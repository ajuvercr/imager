use std::{error::Error, future::Future};

pub mod cube;
pub mod framework;
pub mod francis;
pub mod screenshot;
pub mod shadertoy;
pub mod util;

pub enum Event {
    UpdateArgs(Args),
    Stop,
}

/// Simple program to greet a person
#[derive(Debug)]
pub struct Args {
    pub x_pos: u32,
    pub y_pos: u32,

    pub width: u32,
    pub height: u32,

    pub single: bool,

    pub display: Display,
}

#[derive(Debug)]
pub enum Display {
    Window,
    Desktop,
}
impl Display {
    pub fn needs_override(&self) -> bool {
        match self {
            Display::Window => false,
            Display::Desktop => true,
        }
    }

    pub fn is_desktop(&self) -> bool {
        match self {
            Display::Window => false,
            Display::Desktop => true,
        }
    }
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
    fn update(&mut self, accum_time: f32, size: (u32, u32), device: &wgpu::Device, queue: &wgpu::Queue) {
        let _ = (accum_time, device, queue);
    }

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue);
}
