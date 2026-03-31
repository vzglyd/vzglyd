# Shader Authoring Guide

A VZGLYD slide does not supply a standalone WGSL module. The engine owns the binding layout. The engine prepends the contract prelude for `Screen2D` or `World3D`, concatenates the prelude with the slide's body, and validates the combined module against the expected bindings, vertex interface, fragment interface, and stage names using Naga before pipeline creation. The slide author writes into this structure. The structure is not negotiable.

The authoritative prelude text lives in [`SHADER_CONTRACT.md`](../../SHADER_CONTRACT.md). For package-level shader overrides, see [`MANIFEST_PACKAGE_GUIDE.md`](MANIFEST_PACKAGE_GUIDE.md).

## How the engine and the shader relate

The engine selects the contract prelude for `Screen2D` or `World3D`. It concatenates that prelude with the slide's body-only WGSL source. It validates the combined module. What the prelude declares — bindings, types, struct layouts — is given. All prelude symbols are reserved names owned by the runtime.

The slide author owns the shader logic. The engine owns bind group `0`.

## Required entry points

Every custom shader body must define both of the following entry points:

`@vertex fn vs_main(...)` and `@fragment fn fs_main(...)`

The current renderer rejects slides whose shaders are missing altogether, and the validator rejects bodies that do not define the expected entry points.

## Screen-space contract

For `SceneSpace::Screen2D`, the prelude is given:

```wgsl
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: f32,
};

struct VzglydUniforms {
    time: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var t_font: texture_2d<f32>;
@group(0) @binding(2) var t_detail: texture_2d<f32>;
@group(0) @binding(3) var t_lookup: texture_2d<f32>;
@group(0) @binding(4) var s_diffuse: sampler;
@group(0) @binding(5) var s_font: sampler;
@group(0) @binding(6) var<uniform> u: VzglydUniforms;
```

The bindings carry the following meanings. `t_diffuse` is the slide's primary texture, usually `spec.textures[0]`. `t_font` is the font atlas when one is provided and otherwise aliases the primary texture. `t_detail` exposes `spec.textures[1]` when present and otherwise aliases the primary texture. `t_lookup` exposes `spec.textures[2]` when present and otherwise aliases `t_detail`. `s_diffuse` uses the wrap and filter metadata from the primary texture. `u.time` is elapsed seconds since the slide renderer started.

## World-space contract

For `SceneSpace::World3D`, the prelude is given:

```wgsl
const VZGLYD_SHADER_CONTRACT_VERSION: u32 = 1u;

struct VzglydVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydVertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: f32,
};

struct VzglydUniforms {
    view_proj: mat4x4<f32>,
    cam_pos: vec3<f32>,
    time: f32,
    fog_color: vec4<f32>,
    fog_start: f32,
    fog_end: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: VzglydUniforms;
@group(0) @binding(1) var t_font: texture_2d<f32>;
@group(0) @binding(2) var t_noise: texture_2d<f32>;
@group(0) @binding(3) var t_material_a: texture_2d<f32>;
@group(0) @binding(4) var t_material_b: texture_2d<f32>;
@group(0) @binding(5) var s_clamp: sampler;
@group(0) @binding(6) var s_repeat: sampler;
```

The bindings carry the following meanings. `u.view_proj` is the camera matrix used for clip-space projection. `u.cam_pos`, `u.fog_color`, `u.fog_start`, and `u.fog_end` support common world-space effects. `t_noise` and `s_repeat` are the conventional repeating-texture inputs used by many world-space examples. The name `t_noise` is historical; the slot is not a stylistic requirement. It maps to `spec.textures[1]` when present and otherwise receives a neutral fallback, so a slide may ignore it or use it for any repeating material data that makes sense for that slide. `t_material_a` and `t_material_b` expose `spec.textures[2]` and `spec.textures[3]` when present and otherwise receive neutral fallbacks, so shaders can scale up or down without changing the contract.

## What the shader body contains

The slide's WGSL body defines helper functions, `vs_main`, `fs_main`, and private module constants and structs that do not collide with prelude symbols. The body uses the prelude types directly. The form is:

```wgsl
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

## What the shader body does not contain

The shader does not define compute entry points. The shader does not declare storage buffers. The shader does not declare storage textures. The shader does not declare push constants. The shader does not add bind groups. The shader does not redeclare the reserved `@group(0)` bindings. The shader does not use mismatched vertex input or output types. The shader does not emit fragment outputs other than `@location(0) vec4<f32>`. These are properties of the form, not advisory limits. The validator enforces them and the pipeline refuses what the validator rejects.

## Example 1: colour shader

This is what a colour-only shader looks like in the form — all appearance encoded in vertex colour, textures unused:

```wgsl
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

## Example 2: textured screen-space shader

This is what texture sampling looks like in the form — the primary texture sampled and modulated by vertex colour:

```wgsl
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    return tex * in.color;
}
```

This is the smallest useful textured shader for a screen slide. The minimal WAT example and the end-to-end tutorial both use this pattern.

## Example 3: time-animated shader

This is what time-driven animation looks like in the form — `u.time` used to offset UVs, producing moving backgrounds and dashboard motion without rebuilding geometry:

```wgsl
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.tex_coords + vec2<f32>(0.08 * sin(u.time), 0.0);
    let tex = textureSample(t_diffuse, s_diffuse, uv);
    return tex * in.color;
}
```

## Shaders in the repository

The following shaders in the checked-in slides illustrate specific patterns worked out within the form:

- Font atlas sampling in a screen overlay: [`slides/flat/src/flat_shader.wgsl`](../../slides/flat/src/flat_shader.wgsl)
- Dashboard panel shading: [`slides/dashboard/src/dashboard_shader.wgsl`](../../slides/dashboard/src/dashboard_shader.wgsl)
- World-space repeating-texture and fog shading: [`slides/terrain/src/terrain_shader.wgsl`](../../slides/terrain/src/terrain_shader.wgsl)
- Golf scene shading: [`slides/golf/src/golf_shader.wgsl`](../../slides/golf/src/golf_shader.wgsl)
- Beach scene shading with additional material slots: [`slides/beach_dog/src/beach_dog_shader.wgsl`](../../slides/beach_dog/src/beach_dog_shader.wgsl)

## Diagnostics

Loader failures that mention validation are coming from Naga after the engine prepends the contract prelude. The diagnostic text includes the assembled shader source and points at the offending line within the combined module. When testing shaders with external tools such as `naga-cli`, the appropriate contract prelude must be prepended first — validating the body in isolation is not sufficient because the entry-point signatures depend on the engine-provided types and bindings. When the source of a failure is unclear, starting from a known-good shader in `slides/dashboard`, `slides/flat`, `slides/terrain`, or `slides/golf` and changing one behavior at a time is the reliable path through.
