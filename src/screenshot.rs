use std::num::NonZeroU32;

use wgpu::{Buffer, Texture, TextureView};

use crate::{Renderable, RenderableConfig, Spawner};

pub struct Ctx {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Ctx {
    pub async fn new<E: RenderableConfig>() -> Self {
        let backends = wgpu::util::backend_bits_from_env().unwrap_or_else(wgpu::Backends::all);
        let dx12_shader_compiler = wgpu::util::dx12_shader_compiler_from_env().unwrap_or_default();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            dx12_shader_compiler,
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None, // Some(&surface)
                force_fallback_adapter: false,
            })
            .await
            .expect("No suitable GPU adapters found on the system!");

        let needed_limits = E::required_limits().using_resolution(adapter.limits());

        let optional_features = E::optional_features();
        let required_features = E::required_features();
        let adapter_features = adapter.features();

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

        Self {
            adapter,
            device,
            queue,
        }
    }
}

pub struct AnimScrot<'a, E> {
    ctx: Ctx,
    width: u32,
    height: u32,
    example: E,
    dst_view: TextureView,
    dst_buffer: Buffer,
    dst_texture: Texture,
    spawner: Spawner<'a>,
}

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub buffer: Vec<u8>,
}

pub fn scrot_new<E: Renderable>(
    ctx: Ctx,
    spawner: Spawner,
    width: u32,
    height: u32,
) -> AnimScrot<E> {
    let dst_texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("destination"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let dst_view = dst_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let dst_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("image map buffer"),
        size: width as u64 * height as u64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let example = E::init(
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![wgpu::TextureFormat::Rgba8UnormSrgb],
        },
        &ctx.adapter,
        &ctx.device,
        &ctx.queue,
    );

    AnimScrot {
        ctx,
        width,
        height,
        spawner,
        example,
        dst_view,
        dst_buffer,
        dst_texture,
    }
}

impl<'a, E: Renderable> AnimScrot<'a, E> {
    pub async fn frame(&mut self, time: f32) -> Frame {
        self.example
            .update(time, &self.ctx.device, &self.ctx.queue, &self.spawner);


        self.example.render(
            &self.dst_view,
            &self.ctx.device,
            &self.ctx.queue,
            &self.spawner,
        );

        let mut cmd_buf = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        cmd_buf.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &self.dst_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &self.dst_buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(NonZeroU32::new(self.width * 4).unwrap()),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: self.width as u32,
                height: self.height as u32,
                depth_or_array_layers: 1,
            },
        );

        self.ctx.queue.submit(Some(cmd_buf.finish()));

        let dst_buffer_slice = self.dst_buffer.slice(..);
        dst_buffer_slice.map_async(wgpu::MapMode::Read, |_| ());
        self.ctx.device.poll(wgpu::Maintain::Wait);
        let buffer = dst_buffer_slice.get_mapped_range().to_vec();
        self.dst_buffer.unmap();

        Frame {
            width: self.width as u32,
            height: self.height as u32,
            buffer,
        }
    }
}
