# E12-T13: Slide Authoring Guide

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P1 (high) |
| **Estimate** | M |
| **Blocked by** | E12-T2 |
| **Blocks** | - |

## Description

Write a comprehensive slide authoring guide at `docs/authoring-guide.md` that takes a Rust developer from zero to a running custom slide — both locally and on a Raspberry Pi. The guide covers: scaffolding, the SlideSpec model, shader authoring, building and packaging, sidecar development, and local testing workflow.

## Background

The authoring guide is what turns VZGLYD from a single-developer project into a platform. Without it, building a slide requires reading the source of an existing slide, which works for experienced Rust developers but is a barrier to everyone else. The guide should be written for a developer who knows Rust but has never touched wgpu, WASM, or embedded displays.

## Audience

A Rust developer who:
- Has written a few Rust projects
- Has never worked with WebAssembly or wgpu
- Wants to build a slide that shows data from a web API on their TV

## Document structure

```
docs/authoring-guide.md
│
├── 1. Concepts
│   ├── What is a slide?
│   ├── The slide lifecycle (vzglyd_update, SlideSpec)
│   ├── What is a sidecar?
│   └── The .vzglyd package format
│
├── 2. Prerequisites
│   ├── Rust toolchain (edition 2024)
│   ├── wasm32-wasip1 target
│   ├── cargo-generate
│   └── Running VZGLYD locally (wgpu window mode)
│
├── 3. Your first slide (no sidecar)
│   ├── Scaffold with cargo-generate
│   ├── Understanding the generated code
│   ├── Modify: display a rotating quad
│   ├── Build and test locally
│   └── Load into VZGLYD
│
├── 4. Shaders
│   ├── The shader contract (bind groups, vertex layout)
│   ├── Vertex and fragment shader walkthrough
│   ├── Passing uniforms from vzglyd_update
│   └── Animation: using dt
│
├── 5. Assets
│   ├── Textures (PNG, loaded from manifest.json)
│   ├── 3D models (glTF, for world_3d slides)
│   └── Embedded data (compile-time include_bytes!)
│
├── 6. Adding a sidecar (live data)
│   ├── When do you need a sidecar?
│   ├── Scaffold the sidecar variant
│   ├── vzglyd_sidecar: https_get and channel_push
│   ├── Deserialising the payload in the slide
│   ├── The poll loop pattern
│   └── Handling API keys (environment variables at launch)
│
├── 7. Building and packaging
│   ├── build.sh walkthrough
│   ├── The .vzglyd zip structure
│   └── Verifying the package (vzglyd --validate)
│
├── 8. Testing locally
│   ├── cargo test for slide logic (host target)
│   ├── Running with the VZGLYD binary (wgpu window)
│   └── Iterating: edit → build → reload
│
├── 9. Deploying to RPi4
│   ├── Copy the .vzglyd file to /var/lib/vzglyd/slides/
│   └── Triggering reload (restart vzglyd service)
│
└── 10. Publishing your slide
    ├── Set up the GitHub repo
    ├── Add CI workflow (E12-T9 template)
    ├── Tag a release
    └── Submit to the registry (E12-T10)
```

## Key sections in detail

### 1. What is a slide?

Open with a mental model paragraph: a slide is a WASM module that the engine calls 60 times per second with a delta-time, and returns a geometry description. The engine renders that geometry using its wgpu pipeline. The slide never touches the GPU directly — it describes what it wants and the engine handles the draw calls.

Include a diagram:

```
┌──────────────────────────────────────────────────────┐
│  slide.wasm                                          │
│                                                      │
│  vzglyd_update(dt: f32) -> i32                         │
│      │                                               │
│      └─ returns serialised SlideSpec<Vertex>         │
│         (postcard bytes via WASM memory)             │
└───────────────────┬──────────────────────────────────┘
                    │
┌───────────────────▼──────────────────────────────────┐
│  vzglyd engine                                         │
│                                                      │
│  deserialise SlideSpec → update GPU buffers          │
│  draw call → wgpu → Wayland/KMS → display            │
└──────────────────────────────────────────────────────┘
```

### 3. Your first slide

Walk through the exact commands:

```bash
cargo install cargo-generate    # one-time setup
cargo generate gh:vzglyd/slide-template --name my_slide
cd my_slide
cargo test                      # runs on host
bash build.sh                   # produces my_slide.wasm + my_slide.vzglyd
vzglyd --slide my_slide.vzglyd      # opens a window (desktop) or renders to display
```

The "rotating quad" example gives the reader something visual immediately without requiring them to understand the full API.

### 4. The shader contract

This is the most common source of confusion for new slide authors. Document:
- Bind group 0: engine-provided uniforms (time, resolution, camera matrix for 3D)
- Bind group 1: slide-provided textures (declared in manifest.json assets)
- Vertex buffer layout: must match the `Vertex` struct in the slide (bytemuck Pod)
- Fragment output: `@location(0) vec4<f32>` — premultiplied RGBA

Include a minimal valid vertex + fragment shader pair that compiles, with inline comments explaining every line.

### 6. Sidecar development

The sidecar section needs a concrete worked example. Use the weather slide:

```rust
// sidecar/src/main.rs
use vzglyd_sidecar::{https_get, channel_push, PollLoop};
use serde::Serialize;

#[derive(Serialize)]
struct Forecast {
    max_temp: f32,
    min_temp: f32,
    condition: String,
}

fn main() {
    PollLoop::new(300).run(|| {  // fetch every 5 minutes
        let body = https_get("api.weather.example.com", "/forecast?id=12345", &[])?;
        let parsed: serde_json::Value = serde_json::from_slice(&body)?;

        let forecast = Forecast {
            max_temp: parsed["daily"][0]["maxTemp"].as_f64().unwrap_or(0.0) as f32,
            min_temp: parsed["daily"][0]["minTemp"].as_f64().unwrap_or(0.0) as f32,
            condition: parsed["daily"][0]["condition"].as_str().unwrap_or("").to_string(),
        };

        let payload = postcard::to_stdvec(&forecast)?;
        channel_push(&payload);
        Ok(())
    });
}
```

Then show the matching `vzglyd_update` code in the slide that calls `channel_poll` and updates the geometry.

### API key handling

Many slides need credentials (Last.fm API key, calendar URL). The correct pattern:

1. Sidecar reads from environment variable: `std::env::var("LASTFM_API_KEY")`
2. In `vzglyd.service`, set: `Environment="LASTFM_API_KEY=your_key_here"`
3. Document in slide README: "Set `LASTFM_API_KEY` in your environment or vzglyd.service before using this slide"

Do **not** embed keys in the sidecar binary or manifest.json.

## Acceptance criteria

- [ ] `docs/authoring-guide.md` exists and covers all 10 sections
- [ ] The "your first slide" walkthrough can be followed start-to-finish by someone who only has Rust installed
- [ ] The shader contract section includes a complete, compilable minimal shader pair
- [ ] The sidecar section includes a worked example with `vzglyd_sidecar`
- [ ] The guide is linked from the top-level README
- [ ] Someone not involved in VZGLYD development reviews the guide and successfully builds a slide following it (dogfood test)

## Files to create

| File | Purpose |
|------|---------|
| `docs/authoring-guide.md` | Main authoring guide |
| `docs/examples/minimal-slide/` | Minimal slide source used in the guide |
| `docs/examples/sidecar-slide/` | Sidecar example from section 6 |
