use std::{error::Error, fs::read_to_string, time::Instant};

use clap::{Parser, Subcommand, ValueEnum};
use imager::{
    francis::Francis,
    screenshot::{scrot_new, Ctx},
    shadertoy::{self as shader_toy, Client, RenderPass},
    Spawner,
};

#[derive(Subcommand, Debug)]
enum Shader {
    Source {
        location: Option<String>,
    },
    Toy {
        #[arg(short, long)]
        api: String,

        #[arg(long)]
        save: Option<String>,

        shader_id: String,
    },

    Local {
        #[arg(short, long)]
        api: String,
        location: String,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Window,
    Francis,
    Desktop,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct FrancisArgs {
    #[arg(value_enum, short, long, default_value_t = Mode::Window)]
    mode: Mode,

    /// Francis location
    addr: Option<String>,

    #[arg(short, long)]
    x: Option<u16>,
    #[arg(short, long)]
    y: Option<u16>,

    #[command(subcommand)]
    command: Shader,
}

fn fuji_args() -> shader_toy::Args {
    let rps = vec![RenderPass {
        inputs: vec![],
        outputs: vec![],
        code: include_str!("../../shaders/splash.glsl").to_string(),
        name: "Source Shader".into(),
        description: "".into(),
        pass_type: "image".into(),
    }];

    shader_toy::Args {
        rps,
        client: Client::new("".into()),
        name: "Splash".into(),
    }
}

async fn run_francis() -> Result<(), Box<dyn Error>> {
    let args = FrancisArgs::parse();

    println!("Got GPU Ctx");

    let spawner = Spawner::new();

    let input = match args.command {
        Shader::Source { location } => {
            let source = match &location {
                Some(name) => std::fs::read_to_string(name)?,
                None => include_str!("../../shaders/cyber_fuji.glsl").to_string(),
            };

            let rps = vec![RenderPass {
                inputs: vec![],
                outputs: vec![],
                code: source,
                name: "Source Shader".into(),
                description: "".into(),
                pass_type: "image".into(),
            }];

            shader_toy::Args {
                rps,
                client: Client::new("".into()),
                name: location.unwrap_or("cyber_fuji".to_string()),
            }
        }
        Shader::Local { api, location } => {
            let st = read_to_string(location)?;
            let shader: imager::shadertoy::Shader = serde_json::from_str(&st)?;
            println!(
                "Runnering shader toy shader {} by {}",
                shader.info.name, shader.info.username
            );

            let client = Client::new(&api);

            shader_toy::Args {
                rps: shader.renderpass,
                client,
                name: shader.info.name,
            }
        }
        Shader::Toy {
            api,
            shader_id,
            save,
        } => {
            let client = Client::new(&api);
            let shader = client
                .get_shader(&shader_id, save.as_ref().map(|x| x.as_str()))
                .await?;

            println!(
                "Runnering shader toy shader {} by {}",
                shader.info.name, shader.info.username
            );

            shader_toy::Args {
                rps: shader.renderpass,
                client,
                name: shader.info.name,
            }
        }
    };

    match args.mode {
        Mode::Window | Mode::Desktop => {
            let display = match args.mode {
                Mode::Window => imager::Display::Window,
                Mode::Desktop => imager::Display::Desktop,
                _ => unreachable!(),
            };

            let args = imager::Args {
                x_pos: 0,
                y_pos: 0,
                width: args.x.unwrap_or(500) as u32,
                height: args.y.unwrap_or(500) as u32,
                single: false,
                display,
            };
            let setup = imager::framework::setup::<shader_toy::Example>(&args).await;
            imager::framework::screen::<shader_toy::Example>(&setup, fuji_args()).await;
            imager::framework::start::<shader_toy::Example>(setup, args, input).await;
            Ok(())
        }
        Mode::Francis => {
            let mut francis =
                Francis::new(&args.addr.expect("Please specify addr"), args.x, args.y)
                    .await
                    .unwrap();
            println!("Created francis");

            let ctx = Ctx::new::<shader_toy::Example>().await;

            let mut runner = scrot_new::<shader_toy::Example>(
                ctx,
                spawner,
                francis.width(),
                francis.height(),
                input,
            )
            .await?;
            let start = Instant::now();
            let mut count = 0;
            let mut fps = Instant::now();
            loop {
                let frame = runner.frame(start.elapsed().as_secs_f32()).await;
                francis.write(frame.buffer, 4).await.unwrap();

                count += 1;
                if fps.elapsed().as_secs_f32() > 1.0 {
                    fps = Instant::now();
                    println!("fps {}", count);
                    count = 0;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    match run_francis().await {
        Ok(_) => {}
        Err(e) => eprintln!("Error {:?}", e),
    };
}
