use clap::Parser;
use imager::{
    framework::{setup, start},
    Args, shader_toy,
};

fn main() {
    let args = Args::parse();

    // if width % 64 != 0 {
    //     width = (width / 64 + 1) * 64;
    // }

    // let output = output.unwrap_or_else(|| String::from("-"));

    let setup = pollster::block_on(setup::<shader_toy::Example>(&args));
    start::<shader_toy::Example>(setup, args);
}
