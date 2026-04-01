/**
 * renderer.js — WebGPU renderer for vzglyd slide preview.
 *
 * Matches the native engine's shader contract, bind group layout,
 * vertex buffer layout, and uniform buffer layout exactly.
 */


// ── Shader preludes (verbatim from src/shader_validation.rs) ─────────────────

const SCREEN2D_PRELUDE = `// VZGLYD shader contract v1: Screen2D
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position:   vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color:      vec4<f32>,
    @location(3) mode:       f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color:      vec4<f32>,
    @location(2) mode:       f32,
};

struct VzglydUniforms {
    time:  f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var t_font:    texture_2d<f32>;
@group(0) @binding(2) var t_detail:  texture_2d<f32>;
@group(0) @binding(3) var t_lookup:  texture_2d<f32>;
@group(0) @binding(4) var s_diffuse: sampler;
@group(0) @binding(5) var s_font:    sampler;
@group(0) @binding(6) var<uniform> u: VzglydUniforms;
`;

const WORLD3D_PRELUDE = `// VZGLYD shader contract v1: World3D
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) color:    vec4<f32>,
    @location(3) mode:     f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal:    vec3<f32>,
    @location(2) color:     vec4<f32>,
    @location(3) mode:      f32,
};

struct VzglydUniforms {
    view_proj:        mat4x4<f32>,
    cam_pos:          vec3<f32>,
    time:             f32,
    fog_color:        vec4<f32>,
    fog_start:        f32,
    fog_end:          f32,
    clock_seconds:    f32,
    _pad:             f32,
    ambient_light:    vec4<f32>,
    main_light_dir:   vec4<f32>,
    main_light_color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u:          VzglydUniforms;
@group(0) @binding(1) var t_font:              texture_2d<f32>;
@group(0) @binding(2) var t_noise:             texture_2d<f32>;
@group(0) @binding(3) var t_material_a:        texture_2d<f32>;
@group(0) @binding(4) var t_material_b:        texture_2d<f32>;
@group(0) @binding(5) var s_clamp:             sampler;
@group(0) @binding(6) var s_repeat:            sampler;

fn vzglyd_ambient_light() -> vec3<f32> { return u.ambient_light.rgb; }

fn vzglyd_main_light_dir() -> vec3<f32> {
    let dir = u.main_light_dir.xyz;
    let len_sq = dot(dir, dir);
    if len_sq <= 0.000001 { return vec3<f32>(0.0, 1.0, 0.0); }
    return normalize(dir);
}

fn vzglyd_main_light_rgb() -> vec3<f32> { return u.main_light_color.rgb; }

fn vzglyd_main_light_strength() -> f32 {
    return max(max(u.main_light_color.r, u.main_light_color.g), u.main_light_color.b);
}

fn vzglyd_direct_light_scale() -> f32 {
    let ambient = vzglyd_ambient_light();
    return max(1.0 - max(max(ambient.r, ambient.g), ambient.b), 0.0);
}

fn vzglyd_main_light_screen_uv() -> vec2<f32> {
    let dir = vzglyd_main_light_dir();
    return clamp(
        vec2<f32>(0.5 + dir.x * 0.22, 0.5 - dir.y * 0.30),
        vec2<f32>(0.05, 0.05),
        vec2<f32>(0.95, 0.95),
    );
}
`;

// ── Default shader bodies (used when the slide provides none) ─────────────────
//
// textureSample() requires uniform control flow (WGSL spec §16.7).
// Our fragment shaders branch on the per-fragment `in.mode` varying, so every
// textureSample inside a conditional is non-uniform.  We use textureSampleLevel
// with explicit LOD 0.0 everywhere — it has no derivative restriction and
// produces identical output at the base mip level.

const SCREEN2D_DEFAULT_BODY = `
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos   = vec4<f32>(in.position.xy, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color      = in.color;
    out.mode       = in.mode;
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    if in.mode > 0.5 {
        return textureSampleLevel(t_font, s_font, in.tex_coords, 0.0) * in.color;
    }
    return textureSampleLevel(t_diffuse, s_diffuse, in.tex_coords, 0.0) * in.color;
}
`;

// World3D default body — adapted from src/imported_scene_shader.wgsl.
// All textureSample calls replaced with textureSampleLevel(..., 0.0) so the
// shader compiles under WebGPU's strict uniform-control-flow validation.
const WORLD3D_DEFAULT_BODY = `
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos  = u.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal    = normalize(in.normal);
    out.color     = in.color;
    out.mode      = in.mode;
    return out;
}

fn apply_fog(rgb: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist  = length(world_pos - u.cam_pos);
    let t     = clamp((dist - u.fog_start) / (u.fog_end - u.fog_start), 0.0, 1.0);
    let fog_f = t * t * (3.0 - 2.0 * t);
    return mix(rgb, u.fog_color.rgb, fog_f);
}

fn sky_at(_dir: vec3<f32>) -> vec3<f32> { return vec3<f32>(0.0, 0.0, 0.0); }

fn surface_pattern(world_pos: vec3<f32>) -> vec3<f32> {
    let uv_a = world_pos.xz * 0.065;
    let uv_b = world_pos.xz * 0.025 + vec2<f32>(11.3, 7.1);
    let a = textureSampleLevel(t_material_a, s_repeat, uv_a, 0.0).rgb;
    let b = textureSampleLevel(t_material_b, s_repeat, uv_b, 0.0).rgb;
    return mix(a, b, 0.45);
}

fn lit_surface(base_color: vec4<f32>, normal: vec3<f32>, world_pos: vec3<f32>) -> vec4<f32> {
    let view_dir  = normalize(u.cam_pos - world_pos);
    let light_dir = vzglyd_main_light_dir();
    let diff  = max(dot(normal, light_dir), 0.0);
    let rim   = pow(1.0 - max(dot(normal, view_dir), 0.0), 2.0) * 0.12;
    let light = vzglyd_ambient_light() + vzglyd_main_light_rgb() * diff * vzglyd_direct_light_scale();
    let albedo = base_color.rgb * mix(vec3<f32>(0.88, 0.90, 0.94), surface_pattern(world_pos), 0.20);
    return vec4<f32>(apply_fog(albedo * light + rim * (0.10 + vzglyd_main_light_strength() * 0.02), world_pos), base_color.a);
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    let material_mode = in.mode;
    let base = in.color;

    if material_mode >= 3.5 {
        let uv0 = in.world_pos.xz * 0.06 + vec2<f32>(u.time * 0.030, -u.time * 0.021);
        let uv1 = in.world_pos.xz * 0.11 + vec2<f32>(-u.time * 0.014, u.time * 0.018);
        let n0 = textureSampleLevel(t_material_a, s_repeat, uv0, 0.0).rg * 2.0 - 1.0;
        let n1 = textureSampleLevel(t_material_b, s_repeat, uv1, 0.0).rg * 2.0 - 1.0;
        let water_n  = normalize(vec3<f32>((n0.x + n1.x) * 0.45, 1.0, (n0.y + n1.y) * 0.45));
        let view_dir = normalize(u.cam_pos - in.world_pos);
        let fresnel  = pow(1.0 - max(dot(water_n, view_dir), 0.0), 3.0);
        let refl_col = sky_at(reflect(-view_dir, water_n));
        let water_base = mix(base.rgb, surface_pattern(in.world_pos), 0.35);
        let water_col  = mix(water_base, refl_col, clamp(fresnel * 0.8 + 0.15, 0.0, 1.0));
        return vec4<f32>(apply_fog(water_col, in.world_pos), max(base.a, 0.45));
    }

    var shaded = lit_surface(base, in.normal, in.world_pos);
    if material_mode >= 2.5 {
        let pulse    = 0.65 + 0.35 * sin(u.time * 1.6);
        let emissive = base.rgb * (1.10 + pulse);
        return vec4<f32>(apply_fog(emissive, in.world_pos), base.a);
    }
    if material_mode >= 1.5 { shaded.a = base.a * 0.55; return shaded; }
    if material_mode >= 0.5 {
        if base.a < 0.5 { discard; }
        shaded.a = 1.0;
        return shaded;
    }
    return shaded;
}
`;

// ── Matrix math (column-major, WebGPU convention) ─────────────────────────────

function mat4Identity() {
  return new Float32Array([1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]);
}

function mat4Mul(a, b) {
  const out = new Float32Array(16);
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      let sum = 0;
      for (let k = 0; k < 4; k++) sum += a[k * 4 + row] * b[col * 4 + k];
      out[col * 4 + row] = sum;
    }
  }
  return out;
}

/** Standard perspective projection (right-handed, depth 0..1). */
function mat4Perspective(fovYRad, aspect, near, far) {
  const f = 1.0 / Math.tan(fovYRad / 2);
  const nf = 1 / (near - far);
  // column-major
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0,          f, 0, 0,
    0,          0, (far) * nf,       -1,
    0,          0, (far * near) * nf,  0,
  ]);
}

/** Standard lookAt view matrix (right-handed). */
function mat4LookAt(eye, center, up) {
  const [ex, ey, ez] = eye;
  const [cx, cy, cz] = center;
  const [ux, uy, uz] = up;

  let fx = cx - ex, fy = cy - ey, fz = cz - ez;
  const fl = Math.hypot(fx, fy, fz);
  fx /= fl; fy /= fl; fz /= fl;

  let sx = fy * uz - fz * uy, sy = fz * ux - fx * uz, sz = fx * uy - fy * ux;
  const sl = Math.hypot(sx, sy, sz);
  sx /= sl; sy /= sl; sz /= sl;

  const rx = sy * fz - sz * fy, ry = sz * fx - sx * fz, rz = sx * fy - sy * fx;

  // column-major view matrix
  return new Float32Array([
     sx,  rx, -fx, 0,
     sy,  ry, -fy, 0,
     sz,  rz, -fz, 0,
    -(sx*ex + sy*ey + sz*ez), -(rx*ex + ry*ey + rz*ez), fx*ex + fy*ey + fz*ez, 1,
  ]);
}

function normalize3(v) {
  const len = Math.hypot(v[0], v[1], v[2]);
  if (len === 0) return [0, 1, 0];
  return [v[0]/len, v[1]/len, v[2]/len];
}

function lerp(a, b, t) { return a + (b - a) * t; }

function lerpVec3(a, b, t) {
  return [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
}

// ── Melbourne clock helper ────────────────────────────────────────────────────

function melbourneClockSeconds() {
  const now = new Date();
  // AEST = UTC+10, AEDT = UTC+11 (Oct–Apr approx)
  const month = now.getUTCMonth(); // 0-based
  const isDst = month >= 9 || month <= 3; // Oct(9)..Mar(3) approximate
  const offsetMs = (isDst ? 11 : 10) * 3600_000;
  const localMs  = (now.getTime() + offsetMs) % 86_400_000;
  return localMs / 1000;
}

// ── Texture helpers ───────────────────────────────────────────────────────────

function wrapModeToGPU(mode) {
  return mode === 'Repeat' ? 'repeat' : 'clamp-to-edge';
}

function filterModeToGPU(mode) {
  return mode === 'Nearest' ? 'nearest' : 'linear';
}

/** Upload a TextureDesc (from postcard) as a GPUTexture and return a GPUTextureView. */
function uploadTextureDesc(device, queue, desc) {
  const tex = device.createTexture({
    label: desc.label,
    size:  [desc.width, desc.height, 1],
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  });
  queue.writeTexture(
    { texture: tex },
    desc.data,
    { bytesPerRow: desc.width * 4 },
    [desc.width, desc.height],
  );
  return tex.createView();
}

/** Upload raw RGBA8 pixel data as a GPUTextureView. */
function uploadRawTexture(device, queue, width, height, pixels, label) {
  const tex = device.createTexture({
    label,
    size:   [width, height, 1],
    format: 'rgba8unorm',
    usage:  GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
  });
  queue.writeTexture(
    { texture: tex },
    pixels,
    { bytesPerRow: width * 4 },
    [width, height],
  );
  return tex.createView();
}

/** Create a 1×1 white fallback texture view. */
function create1x1Texture(device, queue, r = 255, g = 255, b = 255, a = 255) {
  return uploadRawTexture(device, queue, 1, 1, new Uint8Array([r, g, b, a]), 'fallback_1x1');
}

function createSamplerFromDesc(device, desc) {
  return device.createSampler({
    addressModeU:  wrapModeToGPU(desc.wrap_u),
    addressModeV:  wrapModeToGPU(desc.wrap_v),
    addressModeW:  wrapModeToGPU(desc.wrap_w),
    magFilter:     filterModeToGPU(desc.mag_filter),
    minFilter:     filterModeToGPU(desc.min_filter),
    mipmapFilter:  filterModeToGPU(desc.mip_filter),
  });
}

// ── Vertex / index buffer helpers ─────────────────────────────────────────────

function createVertexBuffer(device, floatData, label) {
  const buf = device.createBuffer({
    label,
    size:  floatData.byteLength,
    usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
  });
  device.queue.writeBuffer(buf, 0, floatData);
  return buf;
}

function createIndexBuffer(device, uint16Data, label) {
  const buf = device.createBuffer({
    label,
    size:  Math.ceil(uint16Data.byteLength / 4) * 4, // 4-byte alignment
    usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
  });
  device.queue.writeBuffer(buf, 0, uint16Data);
  return buf;
}

// ── Camera interpolation ──────────────────────────────────────────────────────

function sampleCamera(cameraPath, elapsed) {
  if (!cameraPath || cameraPath.keyframes.length === 0) {
    return { position: [0, 1, 3], target: [0, 0, 0], up: [0, 1, 0], fov_y_deg: 60 };
  }

  const kf = cameraPath.keyframes;
  let t = elapsed;

  if (cameraPath.looped && kf.length >= 2) {
    const duration = kf[kf.length - 1].time;
    if (duration > 0) t = t % duration;
  } else {
    t = Math.min(t, kf[kf.length - 1].time);
  }

  if (t <= kf[0].time)            return kf[0];
  if (t >= kf[kf.length - 1].time) return kf[kf.length - 1];

  for (let i = 0; i < kf.length - 1; i++) {
    const a = kf[i], b = kf[i + 1];
    if (t >= a.time && t <= b.time) {
      const alpha = (b.time === a.time) ? 0 : (t - a.time) / (b.time - a.time);
      return {
        position:  lerpVec3(a.position, b.position, alpha),
        target:    lerpVec3(a.target,   b.target,   alpha),
        up:        normalize3(lerpVec3(a.up, b.up, alpha)),
        fov_y_deg: lerp(a.fov_y_deg, b.fov_y_deg, alpha),
      };
    }
  }

  return kf[kf.length - 1];
}

// ── Bind group layout definitions ─────────────────────────────────────────────

function makeScreen2DBindGroupLayout(device) {
  return device.createBindGroupLayout({
    label: 'screen2d_bgl',
    entries: [
      { binding: 0, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_diffuse
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_font
      { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_detail
      { binding: 3, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_lookup
      { binding: 4, visibility: GPUShaderStage.FRAGMENT, sampler:  {} },                       // s_diffuse
      { binding: 5, visibility: GPUShaderStage.FRAGMENT, sampler:  {} },                       // s_font
      { binding: 6, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
        buffer: { type: 'uniform' } },                                                          // u
    ],
  });
}

function makeWorld3DBindGroupLayout(device) {
  return device.createBindGroupLayout({
    label: 'world3d_bgl',
    entries: [
      { binding: 0, visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
        buffer: { type: 'uniform' } },                                                          // u
      { binding: 1, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_font
      { binding: 2, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_noise
      { binding: 3, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_material_a
      { binding: 4, visibility: GPUShaderStage.FRAGMENT, texture:  { sampleType: 'float' } },  // t_material_b
      { binding: 5, visibility: GPUShaderStage.FRAGMENT, sampler:  {} },                       // s_clamp
      { binding: 6, visibility: GPUShaderStage.FRAGMENT, sampler:  {} },                       // s_repeat
    ],
  });
}

// ── Pipeline helpers ──────────────────────────────────────────────────────────

const SCREEN2D_VERTEX_LAYOUT = {
  arrayStride: 40, // 10 × f32
  stepMode: 'vertex',
  attributes: [
    { shaderLocation: 0, offset: 0,  format: 'float32x3' }, // position
    { shaderLocation: 1, offset: 12, format: 'float32x2' }, // tex_coords
    { shaderLocation: 2, offset: 20, format: 'float32x4' }, // color
    { shaderLocation: 3, offset: 36, format: 'float32'   }, // mode
  ],
};

const WORLD3D_VERTEX_LAYOUT = {
  arrayStride: 44, // 11 × f32
  stepMode: 'vertex',
  attributes: [
    { shaderLocation: 0, offset: 0,  format: 'float32x3' }, // position
    { shaderLocation: 1, offset: 12, format: 'float32x3' }, // normal
    { shaderLocation: 2, offset: 24, format: 'float32x4' }, // color
    { shaderLocation: 3, offset: 40, format: 'float32'   }, // mode
  ],
};

function buildShaderSource(prelude, body) {
  return prelude + '\n' + body;
}

function createPipeline(device, bgl, shaderSrc, vertexLayout, pipelineKind, format, hasDepth) {
  const shaderModule = device.createShaderModule({ code: shaderSrc });

  const colorTarget = { format };
  if (pipelineKind === 'Transparent') {
    colorTarget.blend = {
      color: { srcFactor: 'src-alpha', dstFactor: 'one-minus-src-alpha', operation: 'add' },
      alpha: { srcFactor: 'one',       dstFactor: 'one-minus-src-alpha', operation: 'add' },
    };
  }

  const pipelineLayout = device.createPipelineLayout({
    bindGroupLayouts: [bgl],
  });

  const depthStencil = hasDepth ? {
    format: 'depth32float',
    depthWriteEnabled: pipelineKind === 'Opaque',
    depthCompare: pipelineKind === 'Opaque' ? 'less' : 'less-equal',
  } : undefined;

  return device.createRenderPipeline({
    layout: pipelineLayout,
    vertex: {
      module:     shaderModule,
      entryPoint: 'vs_main',
      buffers:    [vertexLayout],
    },
    fragment: {
      module:     shaderModule,
      entryPoint: 'fs_main',
      targets:    [colorTarget],
    },
    primitive: {
      topology:  'triangle-list',
      cullMode:  'back',
    },
    depthStencil,
  });
}

/**
 * Patch custom WGSL: replace textureSample(t, s, coords) with
 * textureSampleLevel(t, s, coords, 0.0) to comply with WebGPU's strict
 * uniform-control-flow requirement for implicit-derivative texture instructions.
 * textureSampleLevel has an explicit LOD and is allowed anywhere.
 *
 * Matches `textureSample(` (with open paren) only, so it never touches
 * textureSampleLevel, textureSampleBias, textureSampleCompare, etc.
 */
function patchTextureSampleToLevel(wgsl) {
  let result = '';
  let i = 0;
  const needle = 'textureSample(';
  while (i < wgsl.length) {
    const idx = wgsl.indexOf(needle, i);
    if (idx === -1) { result += wgsl.slice(i); break; }

    result += wgsl.slice(i, idx) + 'textureSampleLevel(';
    i = idx + needle.length;

    // Copy all arguments up to the matching closing paren, then append , 0.0.
    // Strip any trailing comma + whitespace before adding the new argument to
    // handle calls written with a trailing comma, e.g.:
    //   textureSample(
    //       t, s,
    //       coords,      ← trailing comma
    //   ).r
    const argStart = i;
    let depth = 1;
    while (i < wgsl.length && depth > 0) {
      if (wgsl[i] === '(') depth++;
      else if (wgsl[i] === ')') { depth--; if (depth === 0) break; }
      i++;
    }
    let args = wgsl.slice(argStart, i);
    // Remove trailing comma/whitespace so we can append ', 0.0' cleanly.
    args = args.replace(/,\s*$/, '');
    result += args + ', 0.0)';
    i++; // skip closing paren
  }
  return result;
}

/**
 * Try creating a pipeline from `customSrc`; if the GPU reports a validation
 * error fall back to `defaultSrc` and return `{ pipeline, usedFallback: true }`.
 * Uses pushErrorScope/popErrorScope so the error does NOT bubble to the device
 * uncaptured-error handler.
 *
 * Before the first attempt, proactively patches textureSample → textureSampleLevel
 * so the shader compiles cleanly on strict WebGPU implementations (Edge, Safari).
 */
async function createPipelineWithFallback(device, bgl, customSrc, defaultSrc, vertexLayout, pipelineKind, format, hasDepth) {
  // Proactively replace textureSample with textureSampleLevel so the shader
  // works under strict uniform-control-flow validation (Edge, Safari).
  const patchedSrc = patchTextureSampleToLevel(customSrc);

  device.pushErrorScope('validation');
  const pipeline = createPipeline(device, bgl, patchedSrc, vertexLayout, pipelineKind, format, hasDepth);
  const err = await device.popErrorScope();
  if (!err) return { pipeline, usedFallback: false };

  console.warn(`[vzglyd] Custom ${pipelineKind} shader failed validation even after textureSample patch — falling back to default shader.\n${err.message}`);
  return {
    pipeline: createPipeline(device, bgl, defaultSrc, vertexLayout, pipelineKind, format, hasDepth),
    usedFallback: true,
  };
}

/**
 * Build the combined shader body from ShaderSources.
 * vzglyd convention:
 *   - fragment_wgsl alone: contains both @vertex vs_main and @fragment fs_main
 *   - vertex_wgsl alone:   contains both entry points
 *   - both set:            concatenated (vertex_wgsl first)
 *   - neither:             returns null → use the built-in default body
 */
function customShaderBody(shaders) {
  if (!shaders) return null;
  const v = shaders.vertex_wgsl;
  const f = shaders.fragment_wgsl;
  if (v && f) return v + '\n' + f;
  if (f)      return f;   // fragment_wgsl carries the full body (most common)
  if (v)      return v;
  return null;
}

// ── VzglydRenderer ────────────────────────────────────────────────────────────

class VzglydRenderer {
  /**
   * @param {HTMLCanvasElement} canvas
   * @param {object}            spec     decoded SlideSpec
   */
  constructor(canvas, spec) {
    this._canvas   = canvas;
    this._spec     = spec;
    this._device   = null;
    this._queue    = null;
    this._context  = null;
    this._format   = null;

    this._uniformBuf  = null;
    this._bindGroup   = null;
    this._pipelines   = {};  // { Opaque: GPURenderPipeline, Transparent: GPURenderPipeline }
    this._staticBufs  = [];  // [{ vertex, index, indexCount }]
    this._dynamicBufs = [];  // [{ vertex, index, indexCount }]
    this._overlayBuf  = null;

    this._elapsed       = 0;
    this._frameCount    = 0;
    this._fpsLastTime   = 0;
    this._fps           = 0;
    this._rafId         = null;
    this._onFps         = null; // optional callback(fps)
  }

  /** Must be called before render(). Returns false if WebGPU is unavailable. */
  async init() {
    const adapter =
      await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' }) ??
      await navigator.gpu.requestAdapter() ??
      await navigator.gpu.requestAdapter({ forceFallbackAdapter: true });
    if (!adapter) throw new Error(
      'WebGPU adapter unavailable. Make sure hardware acceleration is enabled ' +
      '(edge://settings/system) and you are on Edge 113+, Chrome 113+, or Safari 18+.'
    );

    this._device = await adapter.requestDevice();
    this._queue  = this._device.queue;

    this._context = this._canvas.getContext('webgpu');
    this._format  = navigator.gpu.getPreferredCanvasFormat();
    this._context.configure({ device: this._device, format: this._format, alphaMode: 'opaque' });

    // Expose device globally so tests/debug tools can read back pixels.
    window.__vzglyd_device = this._device;

    await this._buildResources();
  }

  async _buildResources() {
    const device = this._device;
    const queue  = this._queue;
    const spec   = this._spec;
    const is3D   = spec.scene_space === 'World3D';

    console.log(`[vzglyd] scene_space=${spec.scene_space} draws=${spec.draws.length} static_meshes=${spec.static_meshes.length} custom_shader=${!!(spec.shaders?.vertex_wgsl || spec.shaders?.fragment_wgsl)}`);

    // ── Textures ──────────────────────────────────────────────────────────────
    const fallback = create1x1Texture(device, queue);

    if (is3D) {
      // World3D: textures[0]=t_font, [1]=t_noise, [2]=t_material_a, [3]=t_material_b
      const views = this._uploadWorld3DTextures(spec, fallback);
      const samplers = this._world3DSamplers(spec);
      await this._buildWorld3DResources(views, samplers);
    } else {
      // Screen2D: textures[0]=t_diffuse, font=t_font, [1]=t_detail, [2]=t_lookup
      const views = this._uploadScreen2DTextures(spec, fallback);
      const samplers = this._screen2DSamplers(spec);
      await this._buildScreen2DResources(views, samplers);
    }
  }

  // ── Screen2D setup ────────────────────────────────────────────────────────

  _uploadScreen2DTextures(spec, fallback) {
    const { _device: device, _queue: queue } = this;
    const t = spec.textures;

    const diffuse = t[0] ? uploadTextureDesc(device, queue, t[0]) : fallback;
    const detail  = t[1] ? uploadTextureDesc(device, queue, t[1]) : diffuse;
    const lookup  = t[2] ? uploadTextureDesc(device, queue, t[2]) : detail;

    let fontView = diffuse;
    if (spec.font) {
      fontView = uploadRawTexture(device, queue,
        spec.font.width, spec.font.height, spec.font.pixels, 'font_atlas');
    }

    return { diffuse, fontView, detail, lookup };
  }

  _screen2DSamplers(spec) {
    const device = this._device;
    const t = spec.textures;
    const mainSampler = t[0] ? createSamplerFromDesc(device, t[0])
      : device.createSampler({ magFilter: 'linear', minFilter: 'linear' });

    const fontSampler = spec.font
      ? device.createSampler({ magFilter: 'linear', minFilter: 'linear', mipmapFilter: 'linear' })
      : mainSampler;

    return { mainSampler, fontSampler };
  }

  async _buildScreen2DResources(views, samplers) {
    const device = this._device;
    const spec   = this._spec;

    // Uniform buffer: 16 bytes (time + 3 pads)
    this._uniformBuf = device.createBuffer({
      label: 'screen2d_uniforms',
      size:  16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    const bgl = makeScreen2DBindGroupLayout(device);
    this._bindGroup = device.createBindGroup({
      layout:  bgl,
      entries: [
        { binding: 0, resource: views.diffuse },
        { binding: 1, resource: views.fontView },
        { binding: 2, resource: views.detail },
        { binding: 3, resource: views.lookup },
        { binding: 4, resource: samplers.mainSampler },
        { binding: 5, resource: samplers.fontSampler },
        { binding: 6, resource: { buffer: this._uniformBuf } },
      ],
    });

    const customBody  = customShaderBody(spec.shaders);
    const hasCustomShader = !!customBody;
    const customSrc  = hasCustomShader
      ? buildShaderSource(SCREEN2D_PRELUDE, customBody)
      : null;
    const defaultSrc = buildShaderSource(SCREEN2D_PRELUDE, SCREEN2D_DEFAULT_BODY);

    const vertLayout = SCREEN2D_VERTEX_LAYOUT;
    const fmt        = this._format;
    const hasDepth   = true;

    if (hasCustomShader) {
      const { pipeline: op, usedFallback: fbO } = await createPipelineWithFallback(device, bgl, customSrc, defaultSrc, vertLayout, 'Opaque',      fmt, hasDepth);
      const { pipeline: tr, usedFallback: fbT } = await createPipelineWithFallback(device, bgl, customSrc, defaultSrc, vertLayout, 'Transparent', fmt, hasDepth);
      this._pipelines.Opaque      = op;
      this._pipelines.Transparent = tr;
      if (fbO || fbT) this._shaderFallback = true;
    } else {
      this._pipelines.Opaque      = createPipeline(device, bgl, defaultSrc, vertLayout, 'Opaque',      fmt, hasDepth);
      this._pipelines.Transparent = createPipeline(device, bgl, defaultSrc, vertLayout, 'Transparent', fmt, hasDepth);
    }

    this._uploadStaticMeshes(SCREEN2D_VERTEX_LAYOUT.arrayStride);
    this._allocDynamicMeshes(SCREEN2D_VERTEX_LAYOUT.arrayStride);
    this._buildDepthTexture();
  }

  // ── World3D setup ──────────────────────────────────────────────────────────

  _uploadWorld3DTextures(spec, fallback) {
    const { _device: device, _queue: queue } = this;
    const t = spec.textures;

    const font       = t[0] ? uploadTextureDesc(device, queue, t[0]) : fallback;
    const noise      = t[1] ? uploadTextureDesc(device, queue, t[1]) : font;
    const materialA  = t[2] ? uploadTextureDesc(device, queue, t[2]) : noise;
    const materialB  = t[3] ? uploadTextureDesc(device, queue, t[3]) : materialA;

    return { font, noise, materialA, materialB };
  }

  _world3DSamplers(spec) {
    const device = this._device;
    const t = spec.textures;

    const clampSampler  = t[0] ? createSamplerFromDesc(device, t[0])
      : device.createSampler({ magFilter: 'linear', minFilter: 'linear' });
    const repeatSampler = t[1] ? createSamplerFromDesc(device, t[1])
      : device.createSampler({
          addressModeU: 'repeat', addressModeV: 'repeat',
          magFilter: 'linear', minFilter: 'linear',
        });

    return { clampSampler, repeatSampler };
  }

  async _buildWorld3DResources(views, samplers) {
    const device = this._device;
    const spec   = this._spec;

    // Uniform buffer: 160 bytes
    // view_proj(64) + cam_pos(12) + time(4) + fog_color(16) +
    // fog_start(4) + fog_end(4) + clock_seconds(4) + _pad(4) +
    // ambient_light(16) + main_light_dir(16) + main_light_color(16)
    this._uniformBuf = device.createBuffer({
      label: 'world3d_uniforms',
      size:  160,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    const bgl = makeWorld3DBindGroupLayout(device);
    this._bindGroup = device.createBindGroup({
      layout:  bgl,
      entries: [
        { binding: 0, resource: { buffer: this._uniformBuf } },
        { binding: 1, resource: views.font },
        { binding: 2, resource: views.noise },
        { binding: 3, resource: views.materialA },
        { binding: 4, resource: views.materialB },
        { binding: 5, resource: samplers.clampSampler },
        { binding: 6, resource: samplers.repeatSampler },
      ],
    });

    const customBody  = customShaderBody(spec.shaders);
    const hasCustomShader = !!customBody;
    const customSrc  = hasCustomShader
      ? buildShaderSource(WORLD3D_PRELUDE, customBody)
      : null;
    const defaultSrc = buildShaderSource(WORLD3D_PRELUDE, WORLD3D_DEFAULT_BODY);

    const vertLayout = WORLD3D_VERTEX_LAYOUT;
    const fmt        = this._format;
    const hasDepth   = true;

    if (hasCustomShader) {
      const { pipeline: op, usedFallback: fbO } = await createPipelineWithFallback(device, bgl, customSrc, defaultSrc, vertLayout, 'Opaque',      fmt, hasDepth);
      const { pipeline: tr, usedFallback: fbT } = await createPipelineWithFallback(device, bgl, customSrc, defaultSrc, vertLayout, 'Transparent', fmt, hasDepth);
      this._pipelines.Opaque      = op;
      this._pipelines.Transparent = tr;
      if (fbO || fbT) this._shaderFallback = true;
    } else {
      this._pipelines.Opaque      = createPipeline(device, bgl, defaultSrc, vertLayout, 'Opaque',      fmt, hasDepth);
      this._pipelines.Transparent = createPipeline(device, bgl, defaultSrc, vertLayout, 'Transparent', fmt, hasDepth);
    }

    this._uploadStaticMeshes(WORLD3D_VERTEX_LAYOUT.arrayStride);
    this._allocDynamicMeshes(WORLD3D_VERTEX_LAYOUT.arrayStride);
    this._buildDepthTexture();
  }

  // ── Mesh buffer management ─────────────────────────────────────────────────

  _uploadStaticMeshes(stride) {
    const device = this._device;
    const spec   = this._spec;

    this._staticBufs = spec.static_meshes.map((mesh, i) => {
      const verts   = packVertices(mesh.vertices, spec.scene_space);
      const indices = packIndices(mesh.indices);
      return {
        vertex:     createVertexBuffer(device, verts,   `static_vertex_${i}`),
        index:      createIndexBuffer (device, indices, `static_index_${i}`),
        indexCount: indices.length,
      };
    });
  }

  _allocDynamicMeshes(stride) {
    const device = this._device;
    const spec   = this._spec;

    this._dynamicBufs = spec.dynamic_meshes.map((mesh, i) => {
      const vertBytes  = mesh.max_vertices * stride;
      const indices    = packIndices(mesh.indices);
      const vertBuf    = device.createBuffer({
        label: `dyn_vertex_${i}`,
        size:  vertBytes,
        usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      });
      const indexBuf   = createIndexBuffer(device, indices, `dyn_index_${i}`);
      return { vertex: vertBuf, index: indexBuf, indexCount: indices.length };
    });
  }

  _buildDepthTexture() {
    const w = this._canvas.width;
    const h = this._canvas.height;
    this._depthTexture = this._device.createTexture({
      label:  'depth',
      size:   [w, h],
      format: 'depth32float',
      usage:  GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this._depthView = this._depthTexture.createView();
  }

  // ── Runtime updates ────────────────────────────────────────────────────────

  /**
   * Apply a RuntimeOverlay decoded from guest memory.
   * @param {Uint8Array|null} overlayBytes
   */
  applyOverlayBytes(overlayBytes) {
    if (!overlayBytes) return;
    const overlay = decodeRuntimeOverlayBytes(overlayBytes, this._spec.scene_space);
    if (!overlay || overlay.vertices.length === 0) { this._overlayBuf = null; return; }

    const verts   = packVertices(overlay.vertices, this._spec.scene_space);
    const indices = packIndices(overlay.indices);
    const device  = this._device;

    if (!this._overlayBuf || this._overlayBuf.indexCount < indices.length) {
      this._overlayBuf = {
        vertex:     createVertexBuffer(device, verts,   'overlay_vertex'),
        index:      createIndexBuffer (device, indices, 'overlay_index'),
        indexCount: indices.length,
      };
    } else {
      device.queue.writeBuffer(this._overlayBuf.vertex, 0, verts);
      device.queue.writeBuffer(this._overlayBuf.index,  0, indices);
      this._overlayBuf.indexCount = indices.length;
    }
  }

  /**
   * Apply a RuntimeMeshSet decoded from guest memory.
   * @param {Uint8Array|null} meshBytes
   */
  applyDynamicMeshBytes(meshBytes) {
    if (!meshBytes) return;
    const meshSet = decodeRuntimeMeshSet(meshBytes, this._spec.scene_space);
    const stride  = this._spec.scene_space === 'Screen2D' ? 40 : 44;

    for (const rm of meshSet.meshes) {
      const buf = this._dynamicBufs[rm.mesh_index];
      if (!buf) continue;
      const verts = packVertices(rm.vertices, this._spec.scene_space);
      this._device.queue.writeBuffer(buf.vertex, 0, verts);
      buf.activeIndexCount = Math.min(rm.index_count, buf.indexCount);
    }
  }

  // ── Uniform updates ────────────────────────────────────────────────────────

  _writeScreen2DUniforms(elapsed) {
    const data = new Float32Array([elapsed, 0, 0, 0]);
    this._queue.writeBuffer(this._uniformBuf, 0, data);
  }

  _writeWorld3DUniforms(elapsed) {
    const spec   = this._spec;
    const cam    = sampleCamera(spec.camera_path, elapsed);
    const aspect = this._canvas.width / this._canvas.height;

    const view     = mat4LookAt(cam.position, cam.target, cam.up);
    const proj     = mat4Perspective(cam.fov_y_deg * Math.PI / 180, aspect, 0.15, 180.0);
    const viewProj = mat4Mul(proj, view);

    const lighting = spec.lighting ?? {
      ambient_color:     [1, 1, 1],
      ambient_intensity: 0.22,
      directional_light: { direction: [0.55, 1.0, 0.38], color: [1, 1, 1], intensity: 1.0 },
    };

    const ambIntensity = Math.max(0, lighting.ambient_intensity);
    const ambLight = [
      lighting.ambient_color[0] * ambIntensity,
      lighting.ambient_color[1] * ambIntensity,
      lighting.ambient_color[2] * ambIntensity,
      0,
    ];

    const dl = lighting.directional_light;
    const mainLightDir = dl
      ? [...normalize3(dl.direction), 1.0]
      : [0, 1, 0, 0];
    const mainLightColor = dl
      ? [dl.color[0] * dl.intensity, dl.color[1] * dl.intensity, dl.color[2] * dl.intensity, 0]
      : [0, 0, 0, 0];

    const data = new Float32Array(40); // 160 bytes = 40 f32
    // view_proj (col-major mat4): indices 0-15
    data.set(viewProj, 0);
    // cam_pos: 16-18, time: 19
    data[16] = cam.position[0];
    data[17] = cam.position[1];
    data[18] = cam.position[2];
    data[19] = elapsed;
    // fog_color: 20-23
    data[20] = 0; data[21] = 0; data[22] = 0; data[23] = 1;
    // fog_start: 24, fog_end: 25, clock_seconds: 26, _pad: 27
    data[24] = 18.0;
    data[25] = 75.0;
    data[26] = melbourneClockSeconds();
    data[27] = 0;
    // ambient_light: 28-31
    data.set(ambLight, 28);
    // main_light_dir: 32-35
    data.set(mainLightDir, 32);
    // main_light_color: 36-39
    data.set(mainLightColor, 36);

    this._queue.writeBuffer(this._uniformBuf, 0, data);
  }

  // ── Render pass ────────────────────────────────────────────────────────────

  renderFrame(dt) {
    this._elapsed += dt;

    const spec = this._spec;
    if (spec.scene_space === 'Screen2D') {
      this._writeScreen2DUniforms(this._elapsed);
    } else {
      this._writeWorld3DUniforms(this._elapsed);
    }

    // FPS counter
    this._frameCount++;
    const now = performance.now();
    if (now - this._fpsLastTime >= 500) {
      this._fps = Math.round(this._frameCount / ((now - this._fpsLastTime) / 1000));
      this._frameCount = 0;
      this._fpsLastTime = now;
      if (this._onFps) this._onFps(this._fps);
    }

    const device = this._device;
    const encoder = device.createCommandEncoder();
    const colorTex = this._context.getCurrentTexture();

    const renderPass = encoder.beginRenderPass({
      colorAttachments: [{
        view:       colorTex.createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        loadOp:     'clear',
        storeOp:    'store',
      }],
      depthStencilAttachment: {
        view:              this._depthView,
        depthClearValue:   1.0,
        depthLoadOp:       'clear',
        depthStoreOp:      'store',
      },
    });

    renderPass.setBindGroup(0, this._bindGroup);

    for (const draw of spec.draws) {
      const pipeline = this._pipelines[draw.pipeline];
      if (!pipeline) continue;
      renderPass.setPipeline(pipeline);

      const { kind, index } = draw.source;
      const buf = kind === 'Static' ? this._staticBufs[index] : this._dynamicBufs[index];
      if (!buf) continue;

      renderPass.setVertexBuffer(0, buf.vertex);
      renderPass.setIndexBuffer(buf.index, 'uint16');
      const count = kind === 'Dynamic'
        ? (buf.activeIndexCount ?? buf.indexCount)
        : buf.indexCount;
      const start = draw.index_range.start;
      const end   = Math.min(draw.index_range.end, count);
      if (end > start) renderPass.drawIndexed(end - start, 1, start, 0);
    }

    // Overlay
    if (this._overlayBuf && this._overlayBuf.indexCount > 0) {
      renderPass.setPipeline(this._pipelines.Transparent);
      renderPass.setBindGroup(0, this._bindGroup);
      renderPass.setVertexBuffer(0, this._overlayBuf.vertex);
      renderPass.setIndexBuffer(this._overlayBuf.index, 'uint16');
      renderPass.drawIndexed(this._overlayBuf.indexCount);
    }

    renderPass.end();
    this._queue.submit([encoder.finish()]);
  }

  stop() {
    if (this._rafId !== null) { cancelAnimationFrame(this._rafId); this._rafId = null; }
  }
}
