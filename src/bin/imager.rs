use std::{error::Error, fs::read_to_string, time::Instant};

use clap::{Parser, Subcommand};
use imager::{
    francis::Francis,
    screenshot::{scrot_new, Ctx},
    shader_toy,
    shadertoy::{Client, RenderPass},
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

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct FrancisArgs {
    /// Francis location
    addr: String,

    #[arg(short, long)]
    x: Option<u16>,
    #[arg(short, long)]
    y: Option<u16>,

    #[command(subcommand)]
    command: Shader,
}

async fn run_francis() -> Result<(), Box<dyn Error>> {
    let args = FrancisArgs::parse();
    let mut francis = Francis::new(&args.addr, args.x, args.y).await.unwrap();
    println!("Created francis");

    let ctx = Ctx::new::<shader_toy::Example>().await;
    println!("Got GPU Ctx");

    let spawner = Spawner::new();

    let mut client = None;
    let rps = match args.command {
        Shader::Source { location } => {
            let source = match location {
                Some(name) => std::fs::read_to_string(name)?,
                None => include_str!("../../shaders/cyber_fuji.glsl").to_string(),
            };

            vec![RenderPass {
                inputs: vec![],
                outputs: vec![],
                code: source,
                name: "Source Shader".into(),
                description: "".into(),
                pass_type: "image".into(),
            }]
        }
        Shader::Local { api, location } => {
            let st = read_to_string(location)?;
            let shader: imager::shadertoy::Shader = serde_json::from_str(&st)?;
            println!(
                "Runnering shader toy shader {} by {}",
                shader.info.name, shader.info.username
            );
            client = Some(Client::new(&api));

            shader.renderpass
        }
        Shader::Toy {
            api,
            shader_id,
            save,
        } => {
            let c = Client::new(&api);
            let shader = c
                .get_shader(&shader_id, save.as_ref().map(|x| x.as_str()))
                .await?;
            println!(
                "Runnering shader toy shader {} by {}",
                shader.info.name, shader.info.username
            );
            client = Some(c);

            shader.renderpass
        }
    };

    let input = shader_toy::Args {
        rps,
        client: client.unwrap_or(Client::new("".into())),
    };

    let mut runner =
        scrot_new::<shader_toy::Example>(ctx, spawner, francis.width(), francis.height(), input)
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

#[tokio::main]
async fn main() {
    match run_francis().await {
        Ok(_) => {}
        Err(e) => eprintln!("Error {:?}", e),
    };
}
