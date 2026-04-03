# E10-T1: Slide Template for cargo-generate

| Field | Value |
|-------|-------|
| **Epic** | E10 Slide Scaffolding and Project Decoupling |
| **Priority** | P1 (high) |
| **Estimate** | M |
| **Blocked by** | - |
| **Blocks** | E10-T3, E10-T5 |

## Description

Create a `cargo-generate` template under `templates/slide/` that scaffolds a new slide package with all required boilerplate: Cargo.toml, lib.rs with skeleton `vzglyd_update`, manifest.json, build.sh, and an optional sidecar placeholder. Running `cargo generate --path templates/slide --name my_slide` should produce a directory that builds to a loadable `.wasm` + manifest on the first try.

## Background

Every existing slide follows a near-identical pattern:

- `Cargo.toml`: `crate-type = ["cdylib", "rlib"]`, depends on `VRX-64-slide`, `bytemuck`, `postcard`, `serde`.
- `src/lib.rs`: Defines a `Vertex` type, implements `SlideSpec` construction, exports `vzglyd_update(dt: f32) -> i32`.
- `manifest.json`: Declares name, abi_version, scene_space, optional assets/shaders/display config.
- `build.sh`: Builds to `wasm32-wasip1`, runs asset export, creates compatibility symlinks.

Today, new slide authors copy an existing slide and strip out domain-specific logic. This is error-prone — they often miss the workspace member addition, forget to rename crate artifacts, or leave stale asset declarations.

`cargo-generate` is a well-established tool in the Rust ecosystem for exactly this purpose. It supports template variables (project name, author, scene space), conditional file inclusion, and post-generation hooks.

## Template directory layout

```
templates/slide/
├── cargo-generate.toml         # Template metadata and variable definitions
├── Cargo.toml.liquid           # Templated Cargo.toml
├── src/
│   └── lib.rs.liquid           # Skeleton slide with vzglyd_update
├── manifest.json.liquid        # Templated manifest
├── build.sh.liquid             # Build script
├── shaders/                    # Optional: starter vertex/fragment WGSL
│   ├── vertex.wgsl
│   └── fragment.wgsl
└── assets/
    └── .gitkeep                # Empty assets dir, ready for textures/models
```

## Template variables

Defined in `cargo-generate.toml`:

| Variable | Prompt | Default | Used in |
|----------|--------|---------|---------|
| `project-name` | (built-in) | — | Cargo.toml, manifest.json |
| `scene_space` | "Scene space (world_3d or screen_2d)" | `world_3d` | manifest.json, lib.rs |
| `author` | "Author name" | — | manifest.json |
| `with_sidecar` | "Include sidecar data provider? (true/false)" | `false` | Conditional sidecar/ dir |

## Step-by-step implementation

### Step 1 — Install cargo-generate as a dev dependency

Add a note in the project README or a top-level `CONTRIBUTING.md` that `cargo install cargo-generate` is required for scaffolding. Do not add it to the workspace Cargo.toml (it is a CLI tool, not a library).

### Step 2 — Create `cargo-generate.toml`

```toml
[template]
cargo_generate_version = ">=0.18.0"

[placeholders.scene_space]
type = "string"
prompt = "Scene space (world_3d or screen_2d)?"
choices = ["world_3d", "screen_2d"]
default = "world_3d"

[placeholders.author]
type = "string"
prompt = "Author name?"
default = "VZGLYD contributor"

[placeholders.with_sidecar]
type = "bool"
prompt = "Include sidecar data provider scaffold?"
default = false
```

### Step 3 — Create `Cargo.toml.liquid`

```toml
[package]
name = "{{project-name}}_slide"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
VRX-64-slide = { path = "../../VRX-64-slide" }
serde = { version = "1", features = ["derive"] }
bytemuck = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc", "use-std"] }
```

### Step 4 — Create `src/lib.rs.liquid`

Skeleton that compiles and satisfies the ABI:

- Define a `Vertex` struct deriving `Pod`, `Zeroable`, `Serialize`, `Deserialize`.
- Implement a `spec()` function returning a minimal `SlideSpec<Vertex>`.
- Export `vzglyd_update(dt: f32) -> i32` that returns 0 (no geometry change).
- Include `#[cfg(test)]` module with a basic spec validation test.
- Use `{{scene_space}}` variable to set `SceneSpace::World3d` or `SceneSpace::Screen2d`.

### Step 5 — Create `manifest.json.liquid`

```json
{
  "name": "{{project-name}}",
  "version": "0.1.0",
  "author": "{{author}}",
  "abi_version": 1,
  "scene_space": "{{scene_space}}",
  "display": {
    "duration_seconds": 30
  }
}
```

### Step 6 — Create `build.sh.liquid`

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/{{project-name}}_slide.wasm slide.wasm
```

### Step 7 — Add starter shaders

Provide minimal vertex and fragment WGSL files matching the `SHADER_CONTRACT.md` interface. These should compile against the engine's naga validator without modification. Include the required bind groups, vertex inputs, and fragment outputs.

### Step 8 — Test the template

```bash
cd /tmp
cargo generate --path /path/to/vzglyd/templates/slide --name test_slide
cd test_slide
cargo build --target wasm32-wasip1 --release
# Verify: slide.wasm and manifest.json exist and are well-formed
```

Write an integration test script at `templates/test_template.sh` that automates this round-trip.

## Acceptance criteria

- [ ] `cargo generate --path templates/slide --name foo` produces a directory that compiles to `wasm32-wasip1` without edits
- [ ] Generated `manifest.json` passes the engine's manifest validation
- [ ] Generated `lib.rs` includes a passing `cargo test` with spec validation
- [ ] Template variables (`scene_space`, `author`, `with_sidecar`) are correctly substituted
- [ ] `build.sh` produces `slide.wasm` and `manifest.json` at the package root
- [ ] Starter shaders pass naga WGSL validation
- [ ] `templates/test_template.sh` exercises the full generate-build-validate cycle

## Files to create

| File | Purpose |
|------|---------|
| `templates/slide/cargo-generate.toml` | Template metadata and variables |
| `templates/slide/Cargo.toml.liquid` | Templated crate manifest |
| `templates/slide/src/lib.rs.liquid` | Skeleton slide implementation |
| `templates/slide/manifest.json.liquid` | Templated slide manifest |
| `templates/slide/build.sh.liquid` | Build script |
| `templates/slide/shaders/vertex.wgsl` | Starter vertex shader |
| `templates/slide/shaders/fragment.wgsl` | Starter fragment shader |
| `templates/slide/assets/.gitkeep` | Empty asset directory |
| `templates/test_template.sh` | Round-trip test script |
