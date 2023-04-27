use std::{borrow::Cow, collections::HashMap, error::Error, io::Write, ops::Deref};

use async_std::fs::read_to_string;
use bytemuck::{Pod, Zeroable};
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, RenderPipeline, Texture};

use crate::{
    shadertoy::{Client, RenderPassInput},
    Renderable, RenderableConfig,
};

use super::util::{InputType, FRAG_HEADER, FRAG_TAIL, VERTEX};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
struct Uniform {
    channel_0: [f32; 4],
    channel_1: [f32; 4],
    channel_2: [f32; 4],
    channel_3: [f32; 4],
    resolution: [f32; 4],
    mouse: [f32; 4],
    date: [f32; 4],
    time: f32,
    time_delta: f32,
    rate: f32,
    frame: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    _pos: [f32; 4],
}

fn vertex(pos: [i8; 3]) -> Vertex {
    Vertex {
        _pos: [pos[0] as f32, pos[1] as f32, pos[2] as f32, 1.0],
    }
}

fn create_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let vertex_data = [
        vertex([0, 0, 0]),
        vertex([1, 0, 0]),
        vertex([1, 1, 0]),
        vertex([0, 1, 0]),
    ];

    let index_data: &[u16] = &[
        0, 1, 2, 2, 3, 0, // top
    ];

    (vertex_data.to_vec(), index_data.to_vec())
}

struct PipelineBuilderCommon<'a> {
    common: String,
    config: &'a wgpu::SurfaceConfiguration,
    _adapter: &'a wgpu::Adapter,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    client: &'a Client,
}

struct PipelineBuilder<'a> {
    common: &'a PipelineBuilderCommon<'a>,
    uniform: &'a mut Uniform,
    textures: &'a mut HashMap<u64, Texture>,

    bind_groups: Vec<wgpu::BindGroup>,
    bind_group_layouts: Vec<wgpu::BindGroupLayout>,

    samplers_made: usize,
    inner_text: String,

    pass: &'a super::RenderPass,
    name: &'a str,
    index: usize,
}

impl<'a> Deref for PipelineBuilder<'a> {
    type Target = PipelineBuilderCommon<'a>;

    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

#[derive(Copy, Clone)]
struct Layouts<'a> {
    uniform_layout: &'a BindGroupLayout,
    vb_layout: &'a [wgpu::VertexBufferLayout<'a>],
}

fn sampler_string(binding: usize, channel: u64, cubemap: InputType) -> String {
    let ty = cubemap.ty();
    let sampler = cubemap.sampler();
    format!(
        r#"
layout(set = {binding}, binding = 0) uniform {ty} u_texture_{channel};
layout(set = {binding}, binding = 1) uniform sampler u_sampler_{channel};
#define iChannel{channel} {sampler}(u_texture_{channel}, u_sampler_{channel})
    "#
    )
}

fn to_wgsl(source: &str, stage: naga::ShaderStage, name: &str, index: usize) -> String {
    use naga::back::wgsl::*;
    use naga::front::glsl::*;
    use naga::valid::*;

    let mut parser = Parser::default();
    let options = Options::from(stage);
    let glsl = match parser.parse(&options, &source) {
        Ok(x) => x,
        Err(_) => panic!("invalid frag shader!"),
    };

    let mut validator = Validator::new(ValidationFlags::empty(), Capabilities::empty());
    let entry = match validator.validate(&glsl) {
        Ok(x) => x,
        Err(r) => {
            eprintln!("{}", r.as_inner());
            for (ref span, ref ctx) in r.spans() {
                eprintln!(" at {:?} {}", span.location(&source), ctx);
            }

            panic!();
        }
    };

    let cursor = String::new();
    let mut writer = Writer::new(cursor, WriterFlags::EXPLICIT_TYPES);
    let _ = match writer.write(&glsl, &entry) {
        Ok(s) => s,
        Err(e) => panic!("{}", e),
    };

    // let n = if stage == naga::ShaderStage::Vertex {
    //     "vertex"
    // } else {
    //     "frag"
    // };
    // let file = format!("./tmp/{}_{}_{}.wgsl", name, n, index);
    // let mut f = std::fs::File::create(file).unwrap();
    let source = writer.finish();
    // f.write_all(source.as_bytes()).unwrap();

    source
}

impl<'a> PipelineBuilder<'a> {
    pub fn new(
        common: &'a PipelineBuilderCommon<'a>,
        uniform: &'a mut Uniform,
        textures: &'a mut HashMap<u64, Texture>,
        pass: &'a super::RenderPass,
        name: &'a str,
        index: usize,
    ) -> Self {
        Self {
            common,
            uniform,
            textures,
            bind_group_layouts: Vec::new(),
            bind_groups: Vec::new(),
            samplers_made: 1,
            inner_text: String::new(),
            pass,
            index,
            name,
        }
    }

    fn add_sampler(&mut self, channel: u64, cubemap: InputType) {
        self.inner_text += &sampler_string(self.samplers_made, channel, cubemap);
        self.samplers_made += 1;
    }

    pub fn build<'b>(self, layouts: Layouts<'b>, common_code: &str) -> RenderPass {
        let mut bind_group_refs: Vec<_> = vec![layouts.uniform_layout];
        bind_group_refs.extend(self.bind_group_layouts.iter());

        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &bind_group_refs,
                push_constant_ranges: &[],
            });

        let source = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            FRAG_HEADER,
            &self.common.common,
            common_code,
            self.inner_text,
            &self.pass.code,
            FRAG_TAIL
        );

        // let mut file =
        //     std::fs::File::create(format!("tmp/{}_source_{}.glsl", self.name, self.index)).unwrap();
        // file.write_all(source.as_bytes()).unwrap();

        let frag_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fragment shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(to_wgsl(
                    &source,
                    naga::ShaderStage::Fragment,
                    self.name,
                    self.index,
                ))),
            });

        let vertex_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("vertex shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(to_wgsl(
                    VERTEX,
                    naga::ShaderStage::Vertex,
                    self.name,
                    self.index,
                ))),
            });

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex_shader,
                    entry_point: "main",
                    buffers: layouts.vb_layout,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &frag_shader,
                    entry_point: "main",
                    targets: &[Some(self.config.format.into())], // This should be changed
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            });

        let output = if self.pass.pass_type == "buffer" {
            Some(self.pass.outputs[0].id)
        } else {
            None
        };

        RenderPass {
            output,
            name: self.pass.name.to_string(),
            pipeline,
            bind_groups: self.bind_groups,
        }
    }

    pub async fn add_input(&mut self, input: &RenderPassInput) -> Result<(), Box<dyn Error>> {
        if input.ctype == "texture" || input.ctype == "cubemap" {
            self.handle_texture_input(input).await?;
        }

        if input.ctype == "keyboard" {
            self.handle_keyboard_input(input);
        }

        if input.ctype == "buffer" {
            self.handle_buffer_input(input);
        }

        Ok(())
    }

    fn handle_keyboard_input(&mut self, input: &RenderPassInput) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("destination"),
            size: wgpu::Extent3d::default(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            texture.as_image_copy(),
            &[255, 0, 0, 255],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4 * 1),
                rows_per_image: None,
            },
            wgpu::Extent3d::default(),
        );

        self.add_renderpass_from_texture(Ok(&texture), input, InputType::D2);
    }

    fn add_renderpass_from_texture(
        &mut self,
        texture: Result<&Texture, wgpu::TextureView>,
        input: &RenderPassInput,
        input_type: InputType,
    ) {
        let texture_view = match texture {
            Ok(texture) => texture.create_view(&wgpu::TextureViewDescriptor {
                label: None,
                dimension: Some(input_type.into()),
                ..wgpu::TextureViewDescriptor::default()
            }),
            Err(t) => t,
        };
        let sampler = self.create_sampler();
        let sampler_layout = self.sampler_layout(input_type.into());
        let bind_group = self.bind_group(texture_view, sampler, &sampler_layout);

        self.bind_groups.push(bind_group);
        self.bind_group_layouts.push(sampler_layout);

        self.add_sampler(input.channel, input_type);
    }

    fn handle_buffer_input(&mut self, input: &RenderPassInput) {
        if !self.textures.contains_key(&input.id) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("destination"),
                size: wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            self.textures.insert(input.id, texture);
        }

        let input_type = InputType::D2;
        let view = self.textures[&input.id].create_view(&wgpu::TextureViewDescriptor {
            label: None,
            dimension: Some(input_type.into()),
            ..wgpu::TextureViewDescriptor::default()
        });
        self.add_renderpass_from_texture(Err(view), input, input_type);
    }

    async fn handle_texture_input(
        &mut self,
        input: &RenderPassInput,
    ) -> Result<(), Box<dyn Error>> {
        let input_type = InputType::from_ctype(&input.ctype);
        let (image, (width, height)) = self.client.get_png(&input.src, input_type).await?;
        println!(
            "Image info {:?} ({} {})",
            width * height,
            input.id,
            input.ctype
        );

        match input.channel {
            0 => self.uniform.channel_0 = [width as f32, height as f32, 0.0, 0.0],
            1 => self.uniform.channel_1 = [width as f32, height as f32, 0.0, 0.0],
            2 => self.uniform.channel_2 = [width as f32, height as f32, 0.0, 0.0],
            3 => self.uniform.channel_3 = [width as f32, height as f32, 0.0, 0.0],
            _ => {}
        }

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: if input_type.is_cube() { 6 } else { 1 },
        };

        let format = if input.sampler.srgb == "true" {
            wgpu::TextureFormat::Bgra8UnormSrgb
        } else {
            wgpu::TextureFormat::Bgra8Unorm
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&input.src),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(4 * width),
                rows_per_image: std::num::NonZeroU32::new(height),
            },
            texture_size, // Fuck you
        );

        self.add_renderpass_from_texture(Ok(&texture), input, input_type);
        Ok(())
    }

    fn bind_group(
        &mut self,
        texture_view: wgpu::TextureView,
        sampler: wgpu::Sampler,
        sampler_layout: &BindGroupLayout,
    ) -> BindGroup {
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            layout: sampler_layout,
            label: Some("bind group"),
        });
        bind_group
    }

    fn sampler_layout(&mut self, view_dimension: wgpu::TextureViewDimension) -> BindGroupLayout {
        self.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            })
    }

    fn create_sampler(&mut self) -> wgpu::Sampler {
        self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        })
    }
}

pub struct Args {
    pub rps: Vec<super::RenderPass>,
    pub client: Client,
    pub name: String,
}
impl Args {
    pub async fn from_source(loc: Option<String>) -> Result<Self, Box<dyn Error>> {
        let source = match &loc {
            Some(name) => read_to_string(name).await?,
            None => include_str!("../../shaders/cyber_fuji.glsl").to_string(),
        };

        let rps = vec![super::RenderPass {
            inputs: vec![],
            outputs: vec![],
            code: source,
            name: "Source Shader".into(),
            description: "".into(),
            pass_type: "image".into(),
        }];

        Ok(Args {
            rps,
            client: Client::new("".into()),
            name: loc.unwrap_or("cyber_fuji".to_string()),
        })
    }
    pub async fn from_local(api: &str, loc: String) -> Result<Self, Box<dyn Error>> {
        let st = read_to_string(loc).await?;
        let shader: super::Shader = serde_json::from_str(&st)?;

        let client = Client::new(&api);

        Ok(Args {
            rps: shader.renderpass,
            client,
            name: shader.info.name,
        })
    }

    pub async fn from_toy(
        api: &str,
        shader_id: String,
        save: Option<String>,
    ) -> Result<Self, Box<dyn Error>> {
        let client = Client::new(&api);
        let shader = client
            .get_shader(&shader_id, save.as_ref().map(|x| x.as_str()))
            .await?;

        Ok(Args {
            rps: shader.renderpass,
            client,
            name: shader.info.name,
        })
    }
}

pub struct Example {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: usize,
    bind_group: wgpu::BindGroup,
    uniform: Uniform,
    uniform_buf: wgpu::Buffer,

    rps: Vec<RenderPass>,
    textures: HashMap<u64, Texture>,
}

#[async_trait::async_trait]
impl RenderableConfig for Example {
    type Input = Args;

    async fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        args: Args,
    ) -> Result<Self, Box<dyn Error>> {
        // Create the vertex and index buffers
        let vertex_size = std::mem::size_of::<Vertex>();
        let (vertex_data, index_data) = create_vertices();

        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertex_data),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&index_data),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Create pipeline layout
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // Create other resources
        // let mx_total = [config.width as f32, config.height as f32];
        let mut uniform = Uniform::default();

        uniform.resolution = [config.width as f32, config.height as f32, 0., 0.];

        let vertex_buffer_layouts = [wgpu::VertexBufferLayout {
            array_stride: vertex_size as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            }],
        }];

        let layouts = Layouts {
            uniform_layout: &uniform_layout,
            vb_layout: &vertex_buffer_layouts,
        };

        let common = PipelineBuilderCommon {
            common: String::new(),
            _adapter: &_adapter,
            client: &args.client,
            config: &config,
            device: &device,
            queue: &queue,
        };

        let mut textures = HashMap::new();

        let mut rps = Vec::new();

        let common_code: String = args
            .rps
            .iter()
            .filter(|x| x.pass_type == "common")
            .map(|x| &x.code)
            .fold(String::new(), |acc, st| acc + st);

        for (i, pass) in args
            .rps
            .into_iter()
            .filter(|x| x.pass_type != "common" && x.pass_type != "sound")
            .rev()
            .enumerate()
        {
            let mut builder =
                PipelineBuilder::new(&common, &mut uniform, &mut textures, &pass, &args.name, i);

            for input in &pass.inputs {
                builder.add_input(input).await?;
            }

            rps.push(builder.build(layouts, &common_code));
        }

        let uniform_ref: &[Uniform; 1] = &[uniform];
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(uniform_ref),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind group
        let uniform_buf_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
            label: None,
        });

        // Done
        Ok(Example {
            vertex_buf,
            index_buf,
            index_count: index_data.len(),
            bind_group: uniform_buf_group,
            uniform,
            uniform_buf,
            rps,
            textures,
        })
    }
}

impl Renderable for Example {
    fn update(
        &mut self,
        accum_time: f32,
        size: (u32, u32),
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let delta = accum_time - self.uniform.time;
        self.uniform.time_delta = delta;
        self.uniform.time = accum_time;
        self.uniform.frame += 1;
        self.uniform.resolution = [size.0 as f32, size.1 as f32, 0., 0.];

        let mx_ref: &[Uniform; 1] = &[self.uniform];
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(mx_ref));
    }

    fn render(&mut self, view: &wgpu::TextureView, device: &wgpu::Device, queue: &wgpu::Queue) {
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            for rp in &self.rps {
                let output_view = rp.output.as_ref().map(|id| {
                    self.textures[id].create_view(&wgpu::TextureViewDescriptor::default())
                });
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output_view.as_ref().unwrap_or(view),
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.1,
                                g: 0.2,
                                b: 0.3,
                                a: 1.0,
                            }),
                            store: true,
                        },
                    })],
                    depth_stencil_attachment: None,
                });

                rpass.push_debug_group("Prepare data for draw.");
                rpass.set_pipeline(&rp.pipeline);
                rpass.set_bind_group(0, &self.bind_group, &[]);

                for (i, bg) in rp.bind_groups.iter().enumerate() {
                    rpass.set_bind_group(1 + i as u32, bg, &[]);
                }

                rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
                rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));
                rpass.pop_debug_group();
                rpass.insert_debug_marker("Draw!");
                rpass.draw_indexed(0..self.index_count as u32, 0, 0..1);
            }
        }

        queue.submit(Some(encoder.finish()));
    }
}

#[derive(Debug)]
struct RenderPass {
    output: Option<u64>,
    name: String,
    pipeline: RenderPipeline,
    bind_groups: Vec<BindGroup>,
}
