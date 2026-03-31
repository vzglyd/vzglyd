struct TransitionUniforms {
    blend_factor: f32,
    transition_kind: u32,
    padding0: u32,
    padding1: u32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );

    var out: VertexOutput;
    out.position = vec4<f32>(positions[vi], 0.0, 1.0);
    out.uv = uvs[vi];
    return out;
}

@group(0) @binding(0) var tex_out: texture_2d<f32>;
@group(0) @binding(1) var tex_in: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> uniforms: TransitionUniforms;

const TRANSITION_CROSSFADE: u32 = 0u;
const TRANSITION_WIPE_LEFT: u32 = 1u;
const TRANSITION_WIPE_DOWN: u32 = 2u;
const TRANSITION_DISSOLVE: u32 = 3u;

fn crossfade(outgoing: vec4<f32>, incoming: vec4<f32>, blend: f32) -> vec4<f32> {
    return outgoing * (1.0 - blend) + incoming * blend;
}

fn dissolve_noise(uv: vec2<f32>) -> f32 {
    return fract(sin(dot(uv * 1000.0, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn wipe_reveal(coord: f32, blend: f32) -> f32 {
    let softness = 0.02;
    if blend <= 0.0 {
        return 0.0;
    }
    if blend >= 1.0 {
        return 1.0;
    }
    return 1.0 - smoothstep(blend - softness, blend + softness, coord);
}

@fragment
fn fs(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let outgoing = textureSample(tex_out, samp, uv);
    let incoming = textureSample(tex_in, samp, uv);
    let blend = clamp(uniforms.blend_factor, 0.0, 1.0);

    switch uniforms.transition_kind {
        case TRANSITION_CROSSFADE: {
            return crossfade(outgoing, incoming, blend);
        }
        case TRANSITION_WIPE_LEFT: {
            let reveal = wipe_reveal(uv.x, blend);
            return crossfade(outgoing, incoming, reveal);
        }
        case TRANSITION_WIPE_DOWN: {
            let reveal = wipe_reveal(uv.y, blend);
            return crossfade(outgoing, incoming, reveal);
        }
        case TRANSITION_DISSOLVE: {
            if dissolve_noise(uv) <= blend {
                return incoming;
            }
            return outgoing;
        }
        default: {
            return crossfade(outgoing, incoming, blend);
        }
    }
}
