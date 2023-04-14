use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::{util::ErrorFuture, Renderable, Spawner};

static FRAG_HEADER: &'static str = r#"
// uniform samplerXX iChannel0..3;          // input channel. XX = 2D/Cube
// uniform vec4      iDate;                 // (year, month, day, time in seconds)
// uniform float     iSampleRate;           // sound sample rate (i.e., 44100)

layout(location = 0) in vec3      iResolution;           // viewport resolution (in pixels)
layout(location = 1) in float     iTime;                 // shader playback time (in seconds)
layout(location = 2) in float     iTimeDelta;            // render time (in seconds)
layout(location = 3) in float     iFrameRate;            // shader frame rate
layout(location = 4) in int       iFrame;                // shader playback frame
layout(location = 5) in vec4      iMouse;                // mouse pixel coords. xy: current (if MLB down), zw: click
layout(location = 6)in vec2 iPos;

out vec4 gl_FragColor;

"#;

static FRAG_TAIL: &'static str = r#"
void main()
{
    vec4 color = vec4(0.);
    mainImage(color, iPos);
    gl_FragColor = color;
}
"#;

static VERTEX: &'static str = r#"
layout(binding = 0) uniform ViewParams {
    vec3 res;
    float time;
    float time_delta;
    float frame_rate;
    int frame;
    vec4 mouse;
};

in vec4 aPos;

layout(location = 0) out vec3      iResolution;           // viewport resolution (in pixels)
layout(location = 1) out float     iTime;                 // shader playback time (in seconds)
layout(location = 2) out float     iTimeDelta;            // render time (in seconds)
layout(location = 3) out float     iFrameRate;            // shader frame rate
layout(location = 4) out int       iFrame;                // shader playback frame
layout(location = 5) out vec4      iMouse;                // mouse pixel coords. xy: current (if MLB down), zw: click
layout(location = 6)out vec2 iPos;

void main()
{
    iResolution = res;
    iTime = time;
    iTimeDelta = time_delta;
    iFrameRate = frame_rate;
    iFrame = frame;
    iMouse = mouse;

    iPos = vec2(aPos.x * res.x, aPos.y * res.y);

    gl_Position = vec4(aPos.xy * 2. - 1., 0.0,  1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Uniform {
    resolution: [f32; 3],
    time: f32,
    time_delta: f32,
    frame: u32,
    mouse: [f32; 4],
    date: [f32; 4],
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

pub struct Example {
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: usize,
    bind_group: wgpu::BindGroup,
    uniform: Uniform,
    uniform_buf: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
}

impl Renderable for Example {
    fn init(
        config: &wgpu::SurfaceConfiguration,
        _adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Self {
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
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create other resources
        // let mx_total = [config.width as f32, config.height as f32];
        let mut uniform = Uniform::default();

        uniform.resolution = [config.width as f32, config.height as f32, 0.];

        let uniform_ref: &[Uniform; 1] = &[uniform];
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(uniform_ref),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
            label: None,
        });

        let source = format!(
            "{}\n{}\n{}",
            FRAG_HEADER,
            include_str!("../shaders/cyber_fuji.glsl"),
            FRAG_TAIL
        );

        let frag_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fragment shader"),
            source: wgpu::ShaderSource::Glsl {
                shader: Cow::Owned(source),
                stage: naga::ShaderStage::Fragment,
                defines: Default::default(),
            },
        });

        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vertex shader"),
            source: wgpu::ShaderSource::Glsl {
                shader: Cow::Borrowed(VERTEX),
                stage: naga::ShaderStage::Vertex,
                defines: Default::default(),
            },
        });

        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: vertex_size as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            }],
        }];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: "main",
                buffers: &vertex_buffers,
            },
            fragment: Some(wgpu::FragmentState {
                module: &frag_shader,
                entry_point: "main",
                targets: &[Some(config.format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Done
        Example {
            vertex_buf,
            index_buf,
            index_count: index_data.len(),
            bind_group,
            pipeline,
            uniform,
            uniform_buf,
        }
    }

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
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
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
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            rpass.pop_debug_group();
            rpass.insert_debug_marker("Draw!");
            rpass.draw_indexed(0..self.index_count as u32, 0, 0..1);
        }

        queue.submit(Some(encoder.finish()));

        // If an error occurs, report it and panic.
        spawner.spawn_local(ErrorFuture {
            inner: device.pop_error_scope(),
        });
    }
}
