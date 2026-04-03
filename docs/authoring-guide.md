# Slide Authoring Guide

This guide is for a Rust developer who wants to build a VZGLYD slide without reverse-engineering the whole repo first.

## 1. Concepts

### What is a slide?

A VZGLYD slide is a `wasm32-wasip1` module that exports:

- `vzglyd_spec_ptr()`
- `vzglyd_spec_len()`
- `vzglyd_abi_version()`
- `vzglyd_update(dt: f32) -> i32`

The engine deserializes the encoded `SlideSpec`, validates it against the Pi 4 limits, and renders it with the engine-owned wgpu pipeline.

### What is a sidecar?

A sidecar is a second WASI module that fetches live data and pushes serialized payloads into the slide over the VZGLYD host channel. Slides do not make network calls directly.

### What is a `.vzglyd` package?

A `.vzglyd` file is a zip archive containing `manifest.json`, `slide.wasm`, and optional assets, shaders, and `sidecar.wasm`.

## 2. Prerequisites

Install the Rust toolchain and the WASI target:

```bash
rustup target add wasm32-wasip1
```

For local engine work:

```bash
cargo build
cargo test
```

Reference material:

- [ABI reference](slide-authoring/ABI_REFERENCE.md)
- [Manifest/package guide](slide-authoring/MANIFEST_PACKAGE_GUIDE.md)
- [Shader authoring guide](slide-authoring/SHADER_AUTHORING_GUIDE.md)

## 3. Your First Slide

The minimal source used in this guide lives in [docs/examples/minimal-slide](examples/minimal-slide).

The short version is:

1. Create a library crate with `crate-type = ["cdylib", "rlib"]`.
2. Add `VRX-64-slide`, `serde`, `postcard`, `bytemuck`, and `once_cell`.
3. Define a `Vertex` type that is `#[repr(C)]`, `Pod`, and `Zeroable`.
4. Build a `SlideSpec<Vertex>`.
5. Encode it once with `postcard` and expose `vzglyd_spec_ptr()` / `vzglyd_spec_len()`.

Local workflow:

```bash
cd docs/examples/minimal-slide
cargo build --target wasm32-wasip1 --release
bash build.sh
```

To load that package in the engine:

```bash
cargo run -- --scene /absolute/path/to/minimal-slide
```

## 4. Shaders

The minimal shader pair is:

```wgsl
@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 1.0);
    out.uv = in.tex_coords;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

The important rules:

- Match your WGSL vertex inputs to your Rust vertex layout.
- Treat the engine-provided bindings as reserved.
- Keep shader logic within the bounded contract in [shader-contract.md](shader-contract.md).

## 5. Assets

Slides can package:

- textures under `assets/`
- authored 3D scenes such as glTF files
- embedded data blobs used by the slide at runtime

Declare package assets in the slide manifest and keep them within the Pi 4 resource limits.

## 6. Adding a Sidecar

The worked example lives in [docs/examples/sidecar-slide](examples/sidecar-slide).

The sidecar uses `vzglyd_sidecar`:

```rust
use vzglyd_sidecar::{https_get_text, poll_loop};

fn main() {
    poll_loop(300, || {
        let body = https_get_text("api.example.com", "/forecast")?;
        Ok(body.into_bytes())
    });
}
```

The slide side polls the channel inside `vzglyd_update`, deserializes the payload, and updates runtime mesh or overlay state.

Store secrets in environment variables exposed to the sidecar process. Do not embed API keys in the package.

## 7. Building and Packaging

During development, an unpacked slide directory is enough. For distribution, package it:

```bash
cargo run -- pack slides/clock -o /tmp/clock.vzglyd
```

Each slide directory should include a `build.sh` that:

- builds the slide for `wasm32-wasip1`
- builds the sidecar if present
- wires `slide.wasm` and `manifest.json` into place

## 8. Testing Locally

- `cargo test` exercises native logic.
- `cargo run -- --scene slides/clock` runs a slide locally.
- `cargo run -- --slides-dir /path/to/packages` exercises the directory-scanning path used on devices.

For deeper examples, see the current official slides under `slides/`.

## 9. Deploying to Raspberry Pi 4

Copy packaged slides into `/var/lib/vzglyd/slides/` on the device. The packaged deployment uses:

- `weston.service` for the compositor
- `vzglyd.service` for the engine
- `VRX-64-slides.path` to restart the engine when the slides directory changes

The full device setup is documented in [../deploy/README.md](../deploy/README.md).

## 10. Publishing Your Slide

Recommended release flow:

1. Add CI that runs tests, builds WASM, and packages a `.vzglyd` file.
2. Tag a release and attach the `.vzglyd` artifact.
3. Publish any sidecar dependencies through `vzglyd_sidecar`.
4. Submit the released package to the slide registry once the registry repo is live.

The official examples and the existing slide crates are the best current reference implementations until the external slide template repo is finished.
