pub static FRAG_HEADER: &'static str = r#"
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

pub static FRAG_TAIL: &'static str = r#"
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

pub static VERTEX: &'static str = r#"
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


#[derive(Copy, Clone)]
pub enum InputType {
    Cube,
    D2,
}

impl InputType {
    pub fn ty(&self) -> &'static str {
        match self {
            InputType::Cube => "textureCube",
            InputType::D2 => "texture2D",
        }
    }

    pub fn sampler(&self) -> &'static str {
        match self {
            InputType::Cube => "samplerCube",
            InputType::D2 => "sampler2D",
        }
    }

    pub fn from_ctype(ctype: &str) -> Self {
       if ctype == "cubemap" {
           Self::Cube
       } else {
           Self::D2
       }
    }
    pub fn is_cube(&self) -> bool {
        match self {
            InputType::Cube => true,
            InputType::D2 => false,
        }

    }
}

impl Into<wgpu::TextureViewDimension> for InputType {
    fn into(self) -> wgpu::TextureViewDimension {
        match self {
            InputType::Cube => wgpu::TextureViewDimension::Cube,
            InputType::D2 => wgpu::TextureViewDimension::D2,
        }
    }
}





