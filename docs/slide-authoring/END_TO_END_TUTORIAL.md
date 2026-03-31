# End-to-End Tutorial: Build a Screen-Space Rust Slide

This is how a slide is made. The form is learned by making. This tutorial builds a complete VZGLYD screen-space slide authored in Rust, following the same architecture as the checked-in `slides/flat/` example — because that slide exercises the current authoring surface cleanly: a package directory, embedded `SlideSpec`, custom WGSL, a texture, and runtime overlay updates driven by `vzglyd_update`.

The older Epic 7 ticket proposed a weather dashboard driven by generic host data providers. The current runtime does not ship that key-value ABI yet. The implemented path for live external data is the optional sidecar model used by the terrain slide. This tutorial therefore builds the complete static and overlay path first, then closes with a concrete sidecar extension path for dynamic data.

## Prerequisites

The prerequisites are not bureaucratic hurdles — they are the tools the form requires.

Install the Rust wasm target once:

```bash
rustup target add wasm32-wasip1
```

Use `wasm32-wasip1`, not `wasm32-unknown-unknown`. The older unknown target emits a bare WebAssembly module with no standard WASI surface, which is exactly the loader split the current runtime has removed. For Pi 4-class deployments the important property is not a different code generator, but a single stable runtime contract across ARM Linux and desktop Linux. `wasm32-wasip1` is therefore the target used throughout this tutorial and in the checked-in Rust slide build scripts.

Then create a package directory:

```text
my_clock_slide/
  Cargo.toml
  manifest.json
  src/
    lib.rs
```

## Step 1: create `Cargo.toml`

`Cargo.toml` is the declaration of the slide's existence in the Rust ecosystem. It names the crate, specifies the cdylib target — the slide compiles to a shared library consumed as WebAssembly — and declares its dependencies on `vzglyd-slide`. Use the same dependency surface as the existing Rust slides:

```toml
[package]
name = "my_clock_slide"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
bytemuck = { version = "1", features = ["derive"] }
once_cell = "1"
postcard = { version = "1", features = ["alloc", "use-std"] }
serde = { version = "1", features = ["derive"] }
vzglyd-slide = { path = "../../lume-slide" }
heapless = "0.8"
```

Adjust the `vzglyd-slide` path so that it points at this repository's `vzglyd-slide/` crate from wherever you place `my_clock_slide/`. `heapless` is only needed once you add the runtime clock overlay. Begin with a fully static slide and add it later if you choose.

## Step 2: define the vertex type

The vertex type is the slide's material contract with the renderer. For a `Screen2D` slide, the vertex shape is dictated by the screen-space shader contract:

```rust
use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

#[repr(C)]
#[derive(Copy, Clone, Serialize, Deserialize, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
    pub mode: f32,
}
```

That layout matches the screen-space contract described in [`ABI_REFERENCE.md`](ABI_REFERENCE.md) and [`SHADER_AUTHORING_GUIDE.md`](SHADER_AUTHORING_GUIDE.md).

## Step 3: build a quad and a tiny texture

This is the stage where geometry enters. Start with a single textured quad:

```rust
use vzglyd_slide::{
    DrawSource, DrawSpec, FilterMode, Limits, PipelineKind, SceneSpace, ShaderSources, SlideSpec,
    StaticMesh, TextureDesc, TextureFormat, WrapMode,
};

fn quad() -> (Vec<Vertex>, Vec<u16>) {
    let verts = vec![
        Vertex { position: [-0.8, -0.4, 0.0], tex_coords: [0.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], mode: 1.0 },
        Vertex { position: [ 0.8, -0.4, 0.0], tex_coords: [1.0, 1.0], color: [1.0, 1.0, 1.0, 1.0], mode: 1.0 },
        Vertex { position: [ 0.8,  0.4, 0.0], tex_coords: [1.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], mode: 1.0 },
        Vertex { position: [-0.8,  0.4, 0.0], tex_coords: [0.0, 0.0], color: [1.0, 1.0, 1.0, 1.0], mode: 1.0 },
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    (verts, indices)
}

fn checker_texture() -> Vec<u8> {
    vec![
        255, 255, 255, 255,
         30,  40,  80, 255,
         30,  40,  80, 255,
        255, 255, 255, 255,
    ]
}
```

At this stage custom geometry and a texture exist. The first two visible authoring concerns are satisfied.

## Step 4: add a shader body and assemble the `SlideSpec`

The `SlideSpec` is the slide's complete declaration to the renderer: geometry, textures, shaders, and draw calls assembled into a single structure. The renderer requires custom shaders. For the smallest useful slide, embed a straightforward textured shader:

```rust
fn vzglyd-slide() -> SlideSpec<Vertex> {
    let (verts, indices) = quad();
    let mesh = StaticMesh {
        label: "panel".into(),
        vertices: verts,
        indices: indices.clone(),
    };

    SlideSpec {
        name: "my_clock_slide".into(),
        limits: Limits::pi4(),
        scene_space: SceneSpace::Screen2D,
        camera_path: None,
        shaders: Some(ShaderSources {
            vertex_wgsl: Some(
                "@vertex
fn vs_main(in: VzglydVertexInput) -> VzglydVertexOutput {
    var out: VzglydVertexOutput;
    out.clip_pos = vec4<f32>(in.position, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    out.mode = in.mode;
    return out;
}
".into(),
            ),
            fragment_wgsl: Some(
                "@fragment
fn fs_main(in: VzglydVertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    return tex * in.color;
}
".into(),
            ),
        }),
        overlay: None,
        font: None,
        textures_used: 1,
        textures: vec![TextureDesc {
            label: "checker".into(),
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8Unorm,
            wrap_u: WrapMode::ClampToEdge,
            wrap_v: WrapMode::ClampToEdge,
            wrap_w: WrapMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mip_filter: FilterMode::Nearest,
            data: checker_texture(),
        }],
        static_meshes: vec![mesh],
        dynamic_meshes: vec![],
        draws: vec![DrawSpec {
            label: "panel_draw".into(),
            source: DrawSource::Static(0),
            pipeline: PipelineKind::Opaque,
            index_range: 0..indices.len() as u32,
        }],
    }
}
```

This is the minimal complete rendering payload: geometry, a texture, a shader, and one draw call.

## Step 5: serialize the spec and export the ABI

The host reads a versioned spec blob from linear memory. This is the ABI boundary — the point at which Rust meets the WASM host. Add the same pattern used by the checked-in Rust slides:

```rust
use once_cell::sync::Lazy;

const WIRE_VERSION: u8 = 1;

static SPEC_BYTES: Lazy<Vec<u8>> = Lazy::new(|| {
    let mut bytes = vec![WIRE_VERSION];
    bytes.extend(postcard::to_stdvec(&vzglyd-slide()).expect("serialize slide spec"));
    bytes
});

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_abi_version() -> u32 { 1 }

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_spec_ptr() -> *const u8 { SPEC_BYTES.as_ptr() }

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_spec_len() -> u32 { SPEC_BYTES.len() as u32 }

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_init() -> i32 { 0 }

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_update(_dt: f32) -> i32 { 0 }
```

At this stage the slide exists. It compiles. It can be loaded. The host can instantiate the module, decode the spec, validate it, and render the quad.

## Step 6: add `manifest.json`

The manifest is the slide's declaration to the package loader. It names the slide, versions it, and describes its runtime expectations. Create a manifest beside the wasm:

```json
{
  "name": "My Clock Slide",
  "version": "0.1.0",
  "author": "You",
  "description": "A screen-space tutorial slide",
  "abi_version": 1,
  "scene_space": "screen_2d",
  "display": {
    "transition_in": "crossfade",
    "transition_out": "crossfade"
  }
}
```

This manifest is enough for a working package. The current scheduler still rotates on a fixed `20` second interval, so `display.duration_seconds` is optional metadata for now.

## Step 7: build and run it

This is where the slide first exists outside of source. Build the wasm:

```bash
cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/my_clock_slide.wasm slide.wasm
```

Then run the package through the engine:

```bash
cargo run --manifest-path /path/to/vzglyd/Cargo.toml -- --scene /path/to/my_clock_slide
```

An independently verifiable checkpoint for this stage is the checked-in static Rust example [`slides/dashboard/src/lib.rs`](../../slides/dashboard/src/lib.rs), which uses the same export pattern with a slightly richer mesh.

## Step 8: add a runtime clock overlay

The second slide is where something is actually decided. The slide has compiled and loaded; now the author begins to choose what it does over time. To evolve the slide from a static panel into a living one, follow the overlay pattern from [`slides/flat/src/lib.rs`](../../slides/flat/src/lib.rs):

1. Add a small font atlas plus glyph map.
2. Build `RuntimeOverlay<Vertex>` geometry for the current time string.
3. Store the encoded overlay bytes in process-local state.
4. Refresh that state from `vzglyd_init` and `vzglyd_update`.
5. Export `vzglyd_overlay_ptr` and `vzglyd_overlay_len`.

The key runtime functions look like this:

```rust
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_init() -> i32 {
    let mut state = runtime_state::state();
    state.elapsed = 0.0;
    state.refresh_overlay();
    0
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_update(dt: f32) -> i32 {
    let mut state = runtime_state::state();
    state.elapsed += dt.max(0.0);
    state.refresh_overlay();
    1
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_overlay_ptr() -> *const u8 {
    runtime_state::state().overlay_bytes.as_ptr()
}

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn vzglyd_overlay_len() -> u32 {
    runtime_state::state().overlay_bytes.len() as u32
}
```

Returning `1` from `vzglyd_update` tells the host to reread the overlay payload.

The checked-in `slides/flat/` package is the complete reference implementation of this step.

## Step 9: add host-backed dynamic data

The third stage is where the author begins to understand what the constraint is for. The current runtime does not yet expose generic key-value data providers. The sidecar model is the honest acknowledgment of where the boundary sits. If you want host-backed dynamic data today, use the optional sidecar model. The terrain package is the canonical example:

- main-slide channel polling and dynamic mesh updates: [`slides/terrain/src/lib.rs`](../../slides/terrain/src/lib.rs)
- sidecar network fetch loop: [`slides/terrain/sidecar/src/main.rs`](../../slides/terrain/sidecar/src/main.rs)

The pattern is:

1. Keep the main slide focused on rendering and import `channel_poll` from `vzglyd_host`.
2. Add a `sidecar/` crate or binary that performs the blocking work and pushes raw bytes with `channel_push`.
3. Compile the sidecar to `sidecar.wasm` in the same package directory as the main `slide.wasm`.
4. Poll from `vzglyd_update`, treat `-3` as "no message yet", and parse any received payload from a guest-owned buffer.
5. When the data changes, rebuild either `RuntimeOverlay<V>` or `RuntimeMeshSet<V>`.
6. Return `1` from `vzglyd_update` so the host rereads the runtime payload.

If you want your tutorial slide to display a weather string or remote metric, this is the extension point to use in the current implementation.

## Step 10: pack and share

The slide author who works within these conditions is not working in a reduced medium. They are working in a specific medium with specific formal properties. The `.vzglyd` archive is that medium's distributable unit. Once the package directory works, archive it:

```bash
cargo run -- pack my_clock_slide -o dist/my_clock_slide.vzglyd
```

The archive is a zip file with `manifest.json`, `slide.wasm`, and any declared external assets or shader files.

## Where to compare your result

These checked-in packages are the form's existing instances. Use them as completion references:

- Static screen slide: [`slides/dashboard/`](../../slides/dashboard)
- Dynamic overlay slide: [`slides/flat/`](../../slides/flat)
- Host-backed dynamic runtime slide: [`slides/terrain/`](../../slides/terrain)

If you can build a package directory that follows this tutorial, load it with `cargo run -- --scene <dir>`, and then pack it with `cargo run -- pack`, you have completed the current end-to-end VZGLYD authoring workflow.
