use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use async_channel as mpsc;
use async_std::task::sleep;
use futures_util::future::select;
use futures_util::future::Either;
use serde::{Deserialize, Serialize};
use wgpu::util::align_to;

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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Send {
    shader: Option<String>,
    target: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Update {
    run: Option<bool>,
    sleep: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum Command {
    Update(Update),
    Send(Send),
}

pub struct Handler {
    toys: Vec<AnimScrot<Example>>,
    commands: mpsc::Receiver<Command>,
    rand: WyRand,
    clients: Vec<Francis>,

    start: Instant,
    ctx: Ctx,

    names: Vec<String>,
    options: HashMap<String, usize>,

    running: bool,
    delay: u64,
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

        let clients: Vec<_> = stream::iter(&froxy)
            .then(|fr| Francis::new(&input.francis, *fr))
            .map(|x| x.unwrap())
            .collect()
            .await;

        let (w, h) = froxy.iter().fold((0, 0), |(w, h), froxy| {
            (w.max(froxy.width), h.max(froxy.height))
        });
        let (w, h) = (align_to(w.into(), 64), h.into());

        let ctx = Ctx::new::<Example>().await;
        let mut options = HashMap::new();

        let locals = stream::iter(input.local)
            .then(|local| Args::from_local(api, local))
            .map(|x| x.unwrap());
        let toys = stream::iter(input.toy)
            .then(|toy| Args::from_toy(api, toy, None))
            .map(|x| x.unwrap());
        let sources = stream::iter(input.source)
            .then(|toy| Args::from_source(Some(toy)))
            .map(|x| x.unwrap());

        let toys_and_names: Vec<_> = locals
            .chain(toys)
            .chain(sources)
            .then(|args| create_scrot(&ctx, w, h, args))
            .map(|x| x.unwrap())
            .collect()
            .await;

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

        tokio::spawn(start_server(port, tx, info));

        Ok(Self {
            ctx,
            start: Instant::now(),
            toys,
            options,
            clients,
            rand,
            names,
            commands: rx,
            running: true,
            delay: 200,
        })
    }

    pub async fn start(mut self) -> Result<(), Box<dyn Error>> {
        let start_time = Instant::now();
        loop {
            match select(
                Box::pin(sleep(Duration::from_millis(self.delay))),
                self.commands.recv(),
            )
            .await
            {
                Either::Left(_) => {} // `value1` is resolved from `future1`
                Either::Right((comm, _)) => {
                    self.handle_command(comm?).await?;
                    continue;
                }
            };

            if self.running {
                let francis = {
                    let idx = self.rand.generate_range(0usize..self.clients.len());
                    print!("Using francis {} ", idx);
                    &mut self.clients[idx]
                };
                let toy = {
                    let idx = self.rand.generate_range(0usize..self.toys.len());
                    println!("for shader {}", self.names[idx]);
                    &mut self.toys[idx]
                };
                let x = toy
                    .frame(
                        &self.ctx,
                        start_time.elapsed().as_secs_f32(),
                        Some((francis.width(), francis.height())),
                    )
                    .await;
                francis.write(x.buffer, 4).await?;
            }
        }
    }

    async fn handle_command(&mut self, command: Command) -> Result<(), Box<dyn Error>> {
        match command {
            Command::Update(Update { run, sleep }) => {
                if let Some(run) = run {
                    self.running = run;
                }
                if let Some(sleep) = sleep {
                    self.delay = sleep;
                }
                Ok(())
            }
            Command::Send(s) => self.handle_shader_command(s).await,
        }
    }

    async fn handle_shader_command(
        &mut self,
        Send { shader, target }: Send,
    ) -> Result<(), Box<dyn Error>> {
        let index = if let Some(st) = shader {
            if let Some(index) = self.options.get(&st) {
                *index
            } else {
                eprintln!("Unknown shader {}", st);
                return Ok(());
            }
        } else {
            self.rand.generate_range(0usize..self.toys.len())
        };

        let idx = target.unwrap_or_else(|| self.rand.generate_range(0usize..self.clients.len()));
        let francis = &mut self.clients[idx];

        let toy = &mut self.toys[index];
        let frame = toy
            .frame(
                &self.ctx,
                self.start.elapsed().as_secs_f32(),
                Some((francis.width(), francis.height())),
            )
            .await;

        francis.write(frame.buffer, 4).await?;
        Ok(())
    }
}
