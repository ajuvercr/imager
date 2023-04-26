use std::{borrow::Cow, collections::HashMap, error::Error, io::Write, ops::Deref};

use bytemuck::{Pod, Zeroable};
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, RenderPipeline, Texture};

use crate::{
    shadertoy::{Client, RenderPassInput},
    util::ErrorFuture,
    Renderable, RenderableConfig, Spawner,
};

static FRAG_HEADER: &'static str = r#"
#version 460
layout(location = 0) in vec3      iResolution;           // viewport resolution (in pixels)
layout(location = 1) in float     iTime;                 // shader playback time (in seconds)
layout(location = 2) in float     iTimeDelta;            // render time (in seconds)
layout(location = 3) in float     iFrameRate;            // shader frame rate
layout(location = 4) in int       iFrame;                // shader playback frame
layout(location = 5) in vec4      iMouse;                // mouse pixel coords. xy: current
layout(location = 6) in vec4      iDate;                 // mouse pixel coords. xy: current
layout(location = 7) in vec2      iPos;
layout(location = 8) in vec3      iChannelResolution[4]; // 

out vec4 gl_FragColor;
"#;

static FRAG_TAIL: &'static str = r#"
void main()
{
    // vec2 fragCoord = iPos;
    // vec2 uv = fragCoord.xy / iResolution.xy;
    //
    // gl_FragColor = vec4(uv, 0.0, 1.0);
    // gl_FragColor = texture(sampler2D(u_texture, u_sampler), uv);

    //
    // if (distance(iMouse.xy, fragCoord.xy) <= 10.0) {
    //     gl_FragColor = vec4(vec3(0.0), 1.0);
    // }

    vec4 color = vec4(0.);
    mainImage(color, iPos);
    gl_FragColor = color;
}
"#;

static VERTEX: &'static str = r#"
#version 460
layout(binding = 0) uniform ViewParams {
    vec4 channel_0;
    vec4 channel_1;
    vec4 channel_2;
    vec4 channel_3;
    vec4 res;
    vec4 mouse;
    vec4 date;
    float time;
    float time_delta;
    float frame_rate;
    int frame;
};

in vec4 aPos;

layout(location = 0) out vec3      iResolution;           // viewport resolution (in pixels)
layout(location = 1) out float     iTime;                 // shader playback time (in seconds)
layout(location = 2) out float     iTimeDelta;            // render time (in seconds)
layout(location = 3) out float     iFrameRate;            // shader frame rate
layout(location = 4) out int      iFrame;                // shader playback frame
layout(location = 5) out vec4      iMouse;                // mouse pixel coords. xy: current
layout(location = 6) out vec4      iDate;                // mouse pixel coords. xy: current
layout(location = 7) out vec2      iPos;
layout(location = 8) out vec3      iChannelResolution[4];  // 

void main()
{
    iChannelResolution[0] = channel_0.xyz;
    iChannelResolution[1] = channel_1.xyz;
    iChannelResolution[2] = channel_2.xyz;
    iChannelResolution[3] = channel_3.xyz;
    iResolution = res.xyz;
    iTime = time;
    iTimeDelta = time_delta;
    iFrameRate = frame_rate;
    iFrame = frame;
    iMouse = mouse;
    iDate = date;

    gl_Position = vec4((aPos.xy * 2.0 - vec2(1.0)), 0.0, 1.0);

    // Convert from OpenGL coordinate system (with origin in center
    // of screen) to Shadertoy/texture coordinate system (with origin
    // in lower left corner)
    // iPos = (gl_Position.xy + vec2(1.0)) / vec2(2.0) * res.xy;
    iPos = (gl_Position.xy + vec2(1.0)) / vec2(2.0) * res.xy;
}
"#;

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

    pass: &'a super::shadertoy::RenderPass,
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

fn sampler_string(binding: usize, channel: u64, cubemap: bool) -> String {
    let ty = if cubemap { "textureCube" } else { "texture2D" };
    let sampler = if cubemap { "samplerCube" } else { "sampler2D" };
    format!(
        r#"
layout(set = {binding}, binding = 0) uniform {ty} u_texture_{channel};
layout(set = {binding}, binding = 1) uniform sampler u_sampler_{channel};
#define iChannel{channel} {sampler}(u_texture_{channel}, u_sampler_{channel})
    "#
    )
}

fn to_wgsl(source: &str, stage: naga::ShaderStage, index: usize) -> String {
    use naga::front::glsl::*;
    let mut parser = Parser::default();
    let options = Options::from(stage);
    let glsl = match parser.parse(&options, &source) {
        Ok(x) => x,
        Err(_) => panic!("invalid frag shader!"),
    };

    use naga::valid::*;
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

    use naga::back::wgsl::*;
    let cursor = String::new();
    let mut writer = Writer::new(cursor, WriterFlags::EXPLICIT_TYPES);
    let _ = match writer.write(&glsl, &entry) {
        Ok(s) => s,
        Err(e) => panic!("{}", e),
    };

    let n = if stage == naga::ShaderStage::Vertex {
        "vertex"
    } else {
        "frag "
    };
    let file = format!("./tmp/{}_{}.wgsl", n, index);
    let mut f = std::fs::File::create(file).unwrap();
    let source = writer.finish();
    f.write_all(source.as_bytes()).unwrap();

    source
}

impl<'a> PipelineBuilder<'a> {
    pub fn new(
        common: &'a PipelineBuilderCommon<'a>,
        uniform: &'a mut Uniform,
        textures: &'a mut HashMap<u64, Texture>,
        pass: &'a super::shadertoy::RenderPass,
        i: usize,
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
            index: i,
        }
    }

    fn add_sampler(&mut self, channel: u64, cubemap: bool) {
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

        let mut file = std::fs::File::create(format!("tmp/source_{}.glsl", self.index)).unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let frag_shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("fragment shader"),
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(to_wgsl(
                    &source,
                    naga::ShaderStage::Fragment,
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
                    self.index,
                ))), // source: wgpu::ShaderSource::Glsl {
                     //     shader: Cow::Borrowed(VERTEX),
                     //     stage: naga::ShaderStage::Vertex,
                     //     defines: Default::default(),
                     // },
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
            inputs: Vec::new(),
            output,
            name: self.pass.name.to_string(),
            pipeline,
            bind_groups: self.bind_groups,
        }
    }

    pub async fn add_input(&mut self, input: &RenderPassInput) -> Result<(), Box<dyn Error>> {
        if input.ctype == "texture" || input.ctype == "cubemap" {
            let is_cubemap = input.ctype == "cubemap";

            // lets get this image
            let (image, (width, height)) = self.client.get_png(&input.src, is_cubemap).await?;
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
                depth_or_array_layers: if is_cubemap { 6 } else { 1 },
            };

            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&input.src),
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
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

            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            let dimension = if is_cubemap {
                Some(wgpu::TextureViewDimension::Cube)
            } else {
                None
            };

            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: None,
                dimension,
                ..wgpu::TextureViewDescriptor::default()
            });

            let view_dimension = if is_cubemap {
                wgpu::TextureViewDimension::Cube
            } else {
                wgpu::TextureViewDimension::D2
            };
            let sampler_layout =
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: None,
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
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
                    });

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
                layout: &sampler_layout,
                label: Some("bind group"),
            });

            self.bind_groups.push(bind_group);
            self.bind_group_layouts.push(sampler_layout);

            self.add_sampler(input.channel, is_cubemap);
        }

        if input.ctype == "keyboard" {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("destination"),
                size: wgpu::Extent3d::default(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
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

            let sampler = self
                .device
                .create_sampler(&wgpu::SamplerDescriptor::default());
            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler_layout =
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: None,
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
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
                    });

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
                layout: &sampler_layout,
                label: Some("bind group"),
            });

            self.bind_groups.push(bind_group);
            self.bind_group_layouts.push(sampler_layout);

            self.add_sampler(input.channel, false);
        }

        if input.ctype == "buffer" {
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
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });

                self.textures.insert(input.id, texture);
            }

            let texture_view =
                self.textures[&input.id].create_view(&wgpu::TextureViewDescriptor::default());

            let sampler_layout =
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: None,
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
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
                    });

            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

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
                layout: &sampler_layout,
                label: Some("bind group"),
            });

            self.bind_groups.push(bind_group);
            self.bind_group_layouts.push(sampler_layout);
            self.add_sampler(input.channel, false);
        }

        Ok(())
    }
}

pub struct Args {
    pub rps: Vec<super::shadertoy::RenderPass>,
    pub client: Client,
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

        for (i, pass) in args.rps.into_iter().rev().enumerate() {
            let mut builder = PipelineBuilder::new(&common, &mut uniform, &mut textures, &pass, i);

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

        println!("Example created");

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
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _spawner: &Spawner,
    ) {
        let delta = accum_time - self.uniform.time;
        self.uniform.time_delta = delta;
        self.uniform.time = accum_time;
        self.uniform.frame += 1;

        let mx_ref: &[Uniform; 1] = &[self.uniform];
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(mx_ref));
    }

    fn render(
        &mut self,
        view: &wgpu::TextureView,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spawner: &Spawner,
    ) {
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

        // If an error occurs, report it and panic.
        spawner.spawn_local(ErrorFuture {
            inner: device.pop_error_scope(),
        });
    }
}

#[derive(Debug)]
struct RenderHandler {
    rps: Vec<RenderPass>,
}

#[derive(Debug)]
struct Input {
    id: u32,
    channel: u32,
}

#[derive(Debug)]
struct RenderPass {
    inputs: Vec<Input>,
    output: Option<u64>,
    name: String,
    pipeline: RenderPipeline,
    bind_groups: Vec<BindGroup>,
}
