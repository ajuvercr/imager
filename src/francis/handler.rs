use std::collections::HashMap;
use std::error::Error;
use std::time::Instant;

use futures_channel::mpsc;
use serde::{Deserialize, Serialize};

use crate::screenshot::scrot_new;
use crate::screenshot::AnimScrot;
use crate::screenshot::Ctx;
use crate::shadertoy::Args;
use crate::shadertoy::Example;

use super::Francis;
use nanorand::{Rng, WyRand};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Options {
    /// Locally downloaded shader toy shaders
    local: Vec<String>,
    /// Remote shader toy shaders
    toy: Vec<String>,
    /// GLSL source files
    source: Vec<String>,

    small_francis: Vec<String>,
    francis: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Send {
    shader: Option<String>,
    target: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Command {
    Start,
    Send(Send),
}

pub struct Handler {
    toys: Vec<AnimScrot<Example>>,
    commands: mpsc::Receiver<Command>,
    rand: WyRand,
    clients: Vec<Francis>,

    start: Instant,
    ctx: Ctx,

    options: HashMap<String, usize>,
}

impl Handler {
    pub async fn new(api: &str, input: Options) -> Handler {
        let rand = WyRand::new();
        let main = Francis::new(&input.francis, None, None).await.unwrap();
        let (w, h) = (main.width(), main.height());
        let mut clients = vec![main];

        for fr in &input.small_francis {
            let francis = Francis::new(fr, None, None).await.unwrap();
            clients.push(francis);
        }

        let ctx = Ctx::new::<Example>().await;
        let mut i = 0;
        let mut options = HashMap::new();

        let mut toys = Vec::new();

        for local in &input.local {
            if let Ok(args) = Args::from_local(api, &local).await {
                add_toy(args, w, h, &mut toys, &mut options, &mut i, &ctx).await;
            }
        }

        for toy in &input.toy {
            if let Ok(args) = Args::from_toy(api, &toy, None).await {
                add_toy(args, w, h, &mut toys, &mut options, &mut i, &ctx).await;
            }
        }

        for source in &input.source {
            if let Ok(args) = Args::from_source(Some(source)).await {
                add_toy(args, w, h, &mut toys, &mut options, &mut i, &ctx).await;
            }
        }

        let (_tx, rx) = mpsc::channel(10);

        Self {
            ctx,
            start: Instant::now(),
            toys,
            options,
            clients,
            rand,
            commands: rx,
        }
    }

    pub async fn start(mut self) -> Result<(), Box<dyn Error>> {
        let start_time = Instant::now();
        loop {
            match self.commands.try_next() {
                Ok(Some(Command::Start)) => {
                    eprintln!("Got unexpected start");
                }
                Ok(Some(Command::Send(send))) => {
                    self.handle_command(send).await?;
                }
                Ok(None) => {}
                Err(e) => return Err(e.into()),
            }

            let francis = {
                let idx = self.rand.generate_range(0usize..self.clients.len());
                &mut self.clients[idx]
            };
            let toy = {
                let idx = self.rand.generate_range(0usize..self.toys.len());
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

    async fn handle_command(
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

async fn add_toy(
    args: Args,
    w: u32,
    h: u32,
    toys: &mut Vec<AnimScrot<Example>>,
    options: &mut HashMap<String, usize>,
    i: &mut usize,
    ctx: &Ctx,
) {
    let name = args.name.clone();
    match scrot_new::<Example>(ctx, w, h, args).await {
        Ok(scrot) => {
            toys.push(scrot);
            options.insert(name, *i);
            *i += 1;
        }
        Err(_) => {
            eprintln!("Couldn't create shadertoy shader {}", name);
        }
    }
}
