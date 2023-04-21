use std::time::Instant;

use clap::Parser;
use imager::{
    francis::Francis,
    screenshot::{scrot_new, Ctx},
    shader_toy, Spawner,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct FrancisArgs {
    /// Francis location
    addr: String,

    #[arg(short, long)]
    x: Option<u16>,
    #[arg(short, long)]
    y: Option<u16>,
}

async fn run_francis() {
    let args = FrancisArgs::parse();
    let mut francis = Francis::new(&args.addr, args.x, args.y).await.unwrap();
    println!("Created francis");

    let ctx = Ctx::new::<shader_toy::Example>().await;
    println!("Got GPU Ctx");

    let spawner = Spawner::new();

    let mut runner =
        scrot_new::<shader_toy::Example>(ctx, spawner, francis.width(), francis.height());

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

fn main() {
    pollster::block_on(run_francis());
}
