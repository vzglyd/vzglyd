# VZGLYD Shader Contract

The contract is the law between the engine and the slide. This document is that contract. The engine prepends a prelude before compiling any custom shader body. The slide author writes only the entry points and helper functions that are specific to the slide. The contract version is `1`, exposed in WGSL as `VZGLYD_SHADER_CONTRACT_VERSION`. This number is not a suggestion — it is the prelude.

The contract fixes the shape of the shader interface, not the art style of the slide.

For authoring guidance, examples, and debugging advice that build on this contract, use [docs/slide-authoring/SHADER_AUTHORING_GUIDE.md](docs/slide-authoring/SHADER_AUTHORING_GUIDE.md). This root document is the normative contract text. The other document explains it. This document defines it.

## Overview

Custom slide shaders are body-only WGSL. The engine injects the vertex IO structs, the uniform struct, and the reserved bind-group declarations before validation and pipeline creation. The slide provides `@vertex fn vs_main(...)` and `@fragment fn fs_main(...)`. These are the entry points. There are no others.

The shader does not define compute entry points. The shader does not declare storage buffers. The shader does not use push constants. The shader does not address non-zero bind groups. The shader does not redeclare a reserved prelude binding. The shader does not omit custom shader sources. These are not prohibitions — they are properties of the form. A shader that violates them is not a VZGLYD shader.

## Screen2D Prelude

Screen-space slides operate within the following prelude:

```wgsl
// VZGLYD shader contract v1: Screen2D
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

Binding `0` is the slide texture described by `spec.textures[0]`. Binding `1` is the font atlas when the slide provides one and otherwise aliases the main texture. Binding `2` exposes `spec.textures[1]` when present and otherwise aliases `t_diffuse`. Binding `3` exposes `spec.textures[2]` when present and otherwise aliases `t_detail`. Binding `4` uses the wrap and filter modes from the primary texture, binding `5` is the engine-managed font sampler, and binding `6` is the uniform block.

## World3D Prelude

World-space slides operate within the following prelude:

```wgsl
// VZGLYD shader contract v1: World3D
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

Binding `1` is the font atlas used by the built-in UI overlays. Binding `2` is the second repeating texture slot, exposed to shaders as `t_noise` for historical compatibility. It maps to `spec.textures[1]` when present and otherwise receives a neutral engine-supplied fallback. Slides may ignore it or use it for noise, lookup data, or any other repeating material input that fits the contract. Bindings `3` and `4` expose `spec.textures[2]` and `spec.textures[3]` when present and otherwise receive neutral engine-supplied fallbacks. Binding `5` is the clamp sampler paired with the font-style textures, and binding `6` is the repeat sampler paired with the repeating material textures.

## Required Entry Points

The shader body provides both entry points. The vertex stage is `vs_main`. The fragment stage is `fs_main`. Their signatures use the prelude types for the slide's `SceneSpace`. These are not the preferred entry point names — they are the entry point names.

```wgsl
@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

## Unsupported Features

The shader contract is intentionally constrained. The constraint is the point. The shader does not define compute entry points. The shader does not declare storage buffers. The shader does not use push constants. The shader does not address additional bind groups. The shader does not declare storage textures. The shader does not use binding arrays. The shader does not use comparison samplers. The shader does not target alternate render targets. The engine renders into its configured surface format through bind group `0`. This is where the slide lives. There is no other surface.

## Authoring Guidance

The prelude symbols are reserved names owned by the engine. Every slide ships its own shader body in `SlideSpec.shaders`. The engine does not supply a slide-default shader when `shaders` is absent — a slide without a shader body is not a complete slide. Custom bodies reference `u`, `t_diffuse`, `t_font`, `t_detail`, `t_lookup`, `t_noise`, `t_material_a`, `t_material_b`, `s_diffuse`, `s_font`, `s_clamp`, and `s_repeat` exactly as supplied by the contract, according to the scene space in use. Some names are historical convenience names rather than stylistic directives; the contract cares about slot shape and compatibility, not whether a slide adopts any particular look. Existing examples live in `slides/dashboard/src/dashboard_shader.wgsl`, `slides/flat/src/flat_shader.wgsl`, `slides/terrain/src/terrain_shader.wgsl`, `slides/golf/src/golf_shader.wgsl`, and `slides/beach_dog/src/beach_dog_shader.wgsl`.
