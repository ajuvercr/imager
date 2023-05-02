use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use async_channel as mpsc;
use async_std::task::sleep;
use futures_util::future::select;
use futures_util::future::Either;
use serde::{Deserialize, Serialize};

use crate::screenshot::scrot_new;
use crate::screenshot::AnimScrot;
use crate::screenshot::Ctx;
use crate::shadertoy::Args;
use crate::shadertoy::Example;

use super::froxy_configs;
use super::server::start_server;
use super::Francis;
use super::FroxyConfig;
use nanorand::{Rng, WyRand};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Options {
    /// Locally downloaded shader toy shaders
    local: Vec<String>,
    /// Remote shader toy shaders
    toy: Vec<String>,
    /// GLSL source files
    source: Vec<String>,

    francis: String,
    froxy: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Send {
    shader: Option<String>,
    target: Option<usize>,
    run: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Update {
    running: Option<bool>,
    wait: Option<u64>,
    run: Option<u64>,
    failure: Option<f32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum Command {
    Update(Update),
    Send(Send),
}

struct Params {
    running: bool,
    wait: u64,
    run: u64,
    failure: f32,
}

impl Params {
    fn new() -> Self {
        Params {
            running: true,
            wait: 500,
            run: 2000,
            failure: 0.2,
        }
    }

    fn update(
        &mut self,
        Update {
            running,
            wait,
            run,
            failure,
        }: Update,
    ) {
        if let Some(r) = running {
            self.running = r;
        }
        if let Some(r) = wait {
            self.wait = r;
        }
        if let Some(r) = run {
            self.run = r;
        }
        if let Some(r) = failure {
            self.failure = r;
        }
    }
}

#[derive(Debug)]
struct Current {
    shader_idx: usize,
    francis_idx: usize,
    end: Instant,
}
impl Current {
    fn new() -> Self {
        Self {
            shader_idx: 0,
            francis_idx: 0,
            end: Instant::now(),
        }
    }
}

pub struct Handler {
    toys: Vec<AnimScrot<Example>>,
    clients: Vec<Francis>,

    commands: mpsc::Receiver<Command>,
    rand: WyRand,

    start: Instant,
    ctx: Ctx,

    names: Vec<String>,

    current: Current,
    params: Params,
}

#[derive(Serialize, Debug, Clone)]
pub struct Info {
    froxy: Vec<FroxyConfig>,
    toys: Vec<String>,
}

async fn create_scrot(
    ctx: &Ctx,
    w: u32,
    h: u32,
    args: Args,
) -> Result<(AnimScrot<Example>, String), Box<dyn Error>> {
    let name = args.name.clone();
    let anim = scrot_new(ctx, w, h, args).await?;
    Ok((anim, name))
}

use futures_util::{stream, StreamExt};
impl Handler {
    pub async fn new(api: &str, input: Options, port: u16) -> std::io::Result<Handler> {
        let rand = WyRand::new();

        let froxy = froxy_configs(&input.froxy).await?;

        println!("got froxy config");

        let clients: Vec<_> = stream::iter(&froxy)
            .then(|fr| Francis::new(&input.francis, *fr))
            .map(|x| x.unwrap())
            .collect()
            .await;

        println!("got francis clients");

        let (w, h) = froxy.iter().fold((0, 0), |(w, h), froxy| {
            (w.max(froxy.width), h.max(froxy.height))
        });
        let (w, h) = (w.into(), h.into());

        let (wf, hf) = (w as f32, h as f32);

        let ctx = Ctx::new::<Example>().await;
        let mut options = HashMap::new();

        let locals = stream::iter(input.local)
            .then(|local| Args::from_local(api, local, wf, hf))
            .map(|x| x.unwrap());
        let toys = stream::iter(input.toy)
            .then(|toy| Args::from_toy(api, toy, None, wf, hf))
            .map(|x| x.unwrap());
        let sources = stream::iter(input.source)
            .then(|toy| Args::from_source(Some(toy), wf, hf))
            .map(|x| x.unwrap());

        let toys_and_names: Vec<_> = locals
            .chain(toys)
            .chain(sources)
            .then(|args| create_scrot(&ctx, w, h, args))
            .map(|x| x.unwrap())
            .collect()
            .await;

        println!("got toys");

        let mut toys = Vec::new();
        let mut names = Vec::new();
        toys_and_names
            .into_iter()
            .enumerate()
            .for_each(|(i, (t, n))| {
                options.insert(n.clone(), i);
                toys.push(t);
                names.push(n)
            });

        let info = Info {
            froxy,
            toys: options.keys().cloned().collect(),
        };
        let info = serde_json::to_string_pretty(&info).unwrap();

        let (tx, rx) = mpsc::bounded(10);

        println!("Server starting");
        tokio::spawn(start_server(port, tx, info));

        Ok(Self {
            ctx,
            start: Instant::now(),
            toys,
            clients,
            rand,
            names,
            commands: rx,
            current: Current::new(),
            params: Params::new(),
        })
    }

    pub async fn start(mut self) -> Result<(), Box<dyn Error>> {
        loop {
            if self.current.end < Instant::now() {
                match select(
                    Box::pin(sleep(Duration::from_millis(self.params.wait))),
                    self.commands.recv(),
                )
                .await
                {
                    Either::Left(_) => {
                        self.update_current(Send::default());
                    }
                    Either::Right((comm, _)) => {
                        self.handle_command(comm?);
                        continue;
                    }
                };
            }
            while self.current.end > Instant::now() {
                self.frame(self.params.failure).await?;
                if let Ok(x) = self.commands.try_recv() {
                    self.handle_command(x);
                }
            }
        }
    }

    fn francis_idx(&mut self, run: &Send) -> usize {
        if let Some(ref st) = run.target {
            *st
        } else {
            self.rand.generate_range(0usize..self.clients.len())
        }
    }

    fn shader_idx(&mut self, run: &Send) -> usize {
        if let Some(ref st) = run.shader {
            self.names.iter().position(|x| x == st).unwrap_or(0)
        } else {
            self.rand.generate_range(0usize..self.names.len())
        }
    }

    fn duration(&self, run: &Send) -> u64 {
        run.run.unwrap_or(self.params.run)
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Update(update) => {
                self.params.update(update);
            }
            Command::Send(s) => self.update_current(s),
        }
    }

    async fn frame(&mut self, failure: f32) -> Result<(), Box<dyn Error>> {
        let francis = &mut self.clients[self.current.francis_idx];
        let toy = &mut self.toys[self.current.shader_idx];
        let frame = toy
            .frame(
                &self.ctx,
                self.start.elapsed().as_secs_f32(),
                Some((francis.width(), francis.height())),
            )
            .await;

        francis.write(frame.buffer, 4, failure).await?;
        Ok(())
    }

    fn update_current(&mut self, send: Send) {
        let francis_idx = self.francis_idx(&send);
        let shader_idx = self.shader_idx(&send);

        let duration = self.duration(&send);

        self.current = Current {
            shader_idx,
            end: Instant::now() + Duration::from_millis(duration),
            francis_idx,
        };
    }
}
