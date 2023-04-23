use clap::Parser;
use std::{error::Error, future::Future};

pub mod cube;
pub mod framework;
pub mod francis;
pub mod screenshot;
pub mod shader_toy;
pub mod shadertoy;
pub mod util;

pub enum Event {
    UpdateArgs(Args),
    Stop,
}

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, default_value_t = 0)]
    x_pos: u32,
    #[arg(short, long, default_value_t = 0)]
    y_pos: u32,

    #[arg(short, long, default_value_t = 64)]
    width: u32,
    #[arg(long, default_value_t = 64)]
    height: u32,

    #[arg(short, long, default_value_t = false)]
    single: bool,
    // output: Option<String>,
}

pub struct Spawner<'a> {
    executor: async_executor::LocalExecutor<'a>,
}

impl<'a> Spawner<'a> {
    pub fn new() -> Self {
        Self {
            executor: async_executor::LocalExecutor::new(),
        }
    }

    #[allow(dead_code)]
    pub fn spawn_local(&self, future: impl Future<Output = ()> + 'a) {
        self.executor.spawn(future).detach();
    }

    pub fn run(&self) {
        while self.executor.try_tick() {}
    }

    pub fn run_until_stalled(&self) {
        while self.executor.try_tick() {}
    }
}

#[async_trait::async_trait]
pub trait RenderableConfig: 'static + Sized {
    type Input;
    fn optional_features() -> wgpu::Features {
        wgpu::Features::empty()
    }
    fn required_features() -> wgpu::Features {
        wgpu::Features::empty()
    }
    fn required_downlevel_capabilities() -> wgpu::DownlevelCapabilities {
        wgpu::DownlevelCapabilities {
            flags: wgpu::DownlevelFlags::empty(),
            shader_model: wgpu::ShaderModel::Sm5,
            ..wgpu::DownlevelCapabilities::default()
        }
    }
    fn required_limits() -> wgpu::Limits {
        wgpu::Limits {
            max_bind_groups: 6,
            ..wgpu::Limits::default() // These downlevel limits will allow the code to run on all possible hardware
        }
    }

    async fn init(
        config: &wgpu::SurfaceConfiguration,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: Self::Input,
    ) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

pub trait Renderable: 'static {
    fn update(
        &mut self,
        accum_time: f32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spawner: &Spawner,
    ) {
        let _ = (accum_time, device, queue, spawner);
    }

    fn render(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spawner: &Spawner,
    );
}
