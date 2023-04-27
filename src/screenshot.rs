use std::{collections::HashMap, error::Error, num::NonZeroU32};

use wgpu::{Buffer, Texture};

use crate::{Renderable, RenderableConfig};

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

#[derive(Default, Debug)]
struct TextureProvider {
    textures: HashMap<(u32, u32), (Texture, Buffer)>,
}

impl TextureProvider {
    fn get_texture(&mut self, size: (u32, u32), ctx: &Ctx) -> (&Texture, &Buffer) {
        if !self.textures.contains_key(&size) {
            let dst_texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("destination"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let dst_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("image map buffer"),
                size: size.0 as u64 * size.1 as u64 * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            self.textures.insert(size, (dst_texture, dst_buffer));
        }

        let (ref tex, ref buf) = &self.textures[&size];
        (tex, buf)
    }
}

pub struct AnimScrot<E> {
    width: u32,
    height: u32,
    example: E,
    texture_provider: TextureProvider,
}

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub buffer: Vec<u8>,
}

pub async fn scrot_new<'a, E: Renderable + RenderableConfig>(
    ctx: &Ctx,
    width: u32,
    height: u32,
    input: E::Input,
) -> Result<AnimScrot<E>, Box<dyn Error>> {
    let example = E::init(
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ],
        },
        &ctx.adapter,
        &ctx.device,
        &ctx.queue,
        input,
    )
    .await?;

    let texture_provider = TextureProvider::default();

    Ok(AnimScrot {
        width,
        height,
        example,
        texture_provider,
    })
}

impl<E: Renderable> AnimScrot<E> {
    pub async fn frame(&mut self, ctx: &Ctx, time: f32, size: Option<(u32, u32)>) -> Frame {
        let size = size.unwrap_or((self.width, self.height));
        self.example.update(time, size, &ctx.device, &ctx.queue);

        let (tex, buf) = self.texture_provider.get_texture(size, ctx);
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

        self.example.render(&view, &ctx.device, &ctx.queue);

        let mut cmd_buf = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());

        cmd_buf.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(NonZeroU32::new(size.0 * 4).unwrap()),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: size.0 as u32,
                height: size.1 as u32,
                depth_or_array_layers: 1,
            },
        );

        ctx.queue.submit(Some(cmd_buf.finish()));

        let dst_buffer_slice = buf.slice(..);
        dst_buffer_slice.map_async(wgpu::MapMode::Read, |_| ());
        ctx.device.poll(wgpu::Maintain::Wait);
        let buffer = dst_buffer_slice.get_mapped_range().to_vec();
        buf.unmap();

        Frame {
            width: size.0 as u32,
            height: size.1 as u32,
            buffer,
        }
    }
}
