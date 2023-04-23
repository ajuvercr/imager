use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};
use std::time::Instant;
use std::{sync::mpsc, thread::sleep, time::Duration};
use wgpu::Texture;

use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{self, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    platform::x11::WindowBuilderExtX11,
};
use x11_dl::xlib::Xlib;

use crate::{Args, Event, Renderable, RenderableConfig, Spawner};

pub struct Rend {
    id: u32,
    texture: Texture,
    renderable: Box<dyn Renderable>,
}

#[allow(unused)]
pub struct Renders {
    window: winit::window::Window,
    event_loop: EventLoop<()>,
    instance: wgpu::Instance,
    surface: wgpu::Surface,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,

    created: u32,
    renderables: Vec<Rend>,
    events: mpsc::Receiver<Event>,

    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_format: wgpu::IndexFormat,
}

impl Renders {
    pub async fn create<I, E: RenderableConfig<Input = I> + Renderable>(
        &mut self,
        args: Args,
        input: I,
    ) -> u32 {
        let id = self.created;
        self.created += 1;

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("output-{}", id)),
            size: wgpu::Extent3d {
                width: args.width,
                height: args.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let renderable = Box::new(
            E::init(
                &wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    width: args.width,
                    height: args.height,
                    present_mode: wgpu::PresentMode::Fifo,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![wgpu::TextureFormat::Rgba8Unorm],
                },
                &self.adapter,
                &self.device,
                &self.queue,
                input,
            )
            .await
            .unwrap(),
        );

        self.renderables.push(Rend {
            texture,
            renderable,
            id,
        });

        id
    }

    pub fn delete(&mut self, index: u32) {
        if let Some(rend) = self.renderables.iter().position(|x| x.id == index) {
            let _ = self.renderables.remove(rend);
        } else {
            eprintln!("Cannot delete id {} (Not found)", index);
        }
    }
}

pub struct Setup {
    window: winit::window::Window,
    event_loop: EventLoop<()>,
    instance: wgpu::Instance,
    size: winit::dpi::PhysicalSize<u32>,
    surface: wgpu::Surface,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

pub async fn setup<E: RenderableConfig>(args: &Args) -> Setup {
    let event_loop = EventLoop::new();
    let xlib = Xlib::open().unwrap();
    let builder = winit::window::WindowBuilder::new()
        .with_inner_size(PhysicalSize {
            width: args.width,
            height: args.height,
        })
        .with_position(PhysicalPosition {
            x: args.x_pos,
            y: args.y_pos,
        })
        .with_override_redirect(true);

    let window = builder.build(&event_loop).unwrap();

    let display_id = match window.raw_display_handle() {
        raw_window_handle::RawDisplayHandle::Xlib(handle) => handle.display,
        _ => panic!(),
    };

    let window_id = match window.raw_window_handle() {
        raw_window_handle::RawWindowHandle::Xlib(handle) => handle.window,
        _ => panic!(),
    };

    unsafe {
        (xlib.XLowerWindow)(display_id.cast(), window_id);
    }

    log::info!("Initializing the surface...");

    let backends = wgpu::util::backend_bits_from_env().unwrap_or_else(wgpu::Backends::all);
    let dx12_shader_compiler = wgpu::util::dx12_shader_compiler_from_env().unwrap_or_default();

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        dx12_shader_compiler,
    });

    let (size, surface) = unsafe {
        let size = window.inner_size();

        let surface = instance.create_surface(&window).unwrap();

        (size, surface)
    };

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .expect("No suitable GPU adapters found on the system!");

    let adapter_info = adapter.get_info();
    eprintln!("Using {} ({:?})", adapter_info.name, adapter_info.backend);

    let optional_features = E::optional_features();
    let required_features = E::required_features();
    let adapter_features = adapter.features();

    // Make sure we use the texture resolution limits from the adapter, so we can support images the size of the surface.
    let needed_limits = E::required_limits().using_resolution(adapter.limits());

    let trace_dir = std::env::var("WGPU_TRACE");
    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: (optional_features & adapter_features) | required_features,
                limits: needed_limits,
            },
            trace_dir.ok().as_ref().map(std::path::Path::new),
        )
        .await
        .expect("Unable to find a suitable GPU adapter!");

    Setup {
        window,
        event_loop,
        instance,
        size,
        surface,
        adapter,
        device,
        queue,
    }
}

pub async fn start<I, E: Renderable + RenderableConfig<Input = I>>(
    Setup {
        window,
        event_loop,
        instance,
        size,
        surface,
        adapter,
        device,
        queue,
    }: Setup,
    args: Args,
    input: I,
) {
    let spawner = Spawner::new();
    let mut config = surface
        .get_default_config(&adapter, size.width, size.height)
        .expect("Surface isn't supported by the adapter.");
    surface.configure(&device, &config);

    log::info!("Initializing the example...");
    let mut example = E::init(&config, &adapter, &device, &queue, input)
        .await
        .unwrap();

    let start = Instant::now();
    let mut last_frame_inst = Instant::now();
    let (mut frame_count, mut accum_time) = (0, 0.0);

    log::info!("Entering render loop...");
    event_loop.run(move |event, _, control_flow| {
        let _ = (&instance, &adapter); // force ownership by the closure
        *control_flow = if cfg!(feature = "metal-auto-capture") {
            ControlFlow::Exit
        } else {
            ControlFlow::Poll
        };
        match event {
            event::Event::RedrawEventsCleared => {
                spawner.run_until_stalled();
                window.request_redraw();
            }
            event::Event::WindowEvent {
                event:
                    WindowEvent::Resized(size)
                    | WindowEvent::ScaleFactorChanged {
                        new_inner_size: &mut size,
                        ..
                    },
                ..
            } => {
                log::info!("Resizing to {:?}", size);
                config.width = size.width.max(1);
                config.height = size.height.max(1);
                // example.resize(&config, &device, &queue);
                surface.configure(&device, &config);
            }
            event::Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput {
                    input:
                        event::KeyboardInput {
                            virtual_keycode: Some(event::VirtualKeyCode::Escape),
                            state: event::ElementState::Pressed,
                            ..
                        },
                    ..
                }
                | WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::KeyboardInput {
                    input:
                        event::KeyboardInput {
                            virtual_keycode: Some(event::VirtualKeyCode::R),
                            state: event::ElementState::Pressed,
                            ..
                        },
                    ..
                } => {
                    eprintln!("{:#?}", instance.generate_report());
                }
                _ => {}
            },
            event::Event::RedrawRequested(_) => {
                let elapsed = start.elapsed().as_secs_f32();
                accum_time += last_frame_inst.elapsed().as_secs_f32();
                last_frame_inst = Instant::now();
                frame_count += 1;

                if frame_count == 100 {
                    eprintln!(
                        "Avg frame time {}ms",
                        accum_time * 1000.0 / frame_count as f32
                    );
                    accum_time = 0.0;
                    frame_count = 0;
                }

                if !args.single {
                    example.update(elapsed, &device, &queue, &spawner);
                }

                let frame = match surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(_) => {
                        surface.configure(&device, &config);
                        surface
                            .get_current_texture()
                            .expect("Failed to acquire next surface texture!")
                    }
                };

                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());

                example.render(&view, &device, &queue, &spawner);

                frame.present();

                let frame_time = start.elapsed().as_secs_f32() - elapsed;

                if frame_time < 0.014 {
                    sleep(Duration::from_secs_f32(0.014 - frame_time));
                }
            }
            _ => {}
        }
    });
}
