# AGENTS.md

*For the coding agent operating within the VZGLYD codebase.*

---

## What this project is

VZGLYD is a Raspberry Pi 4 display engine for ambient 3D art. It renders slides — small Rust crates that compile to `wasm32-wasip1` — sixty times per second to a television. The constraint is real: 60,000 vertices, 512×512 textures, four texture slots, a bounded shader contract, and a Pi 4 performance floor. Those limits are the medium. A single mandatory house style is not.

## Agent mindset

You are here because the project operates as a constrained medium. That is the manifesto: the vertex budget, texture budget, shader contract, and hardware floor are not arbitrary ceilings but the formal condition by which a slide argues for its presence. Keep the constraint visible in your work, ask what choices the budget forces, and treat every slide as a small world that earns its place through deliberate decisions rather than the illusion of more.

Think of the work as Level Six of the Constraint Principle—knowledge of the later levels is welcome but the practice is about returning to the discipline of early hardware with intention. Your role is not to chase the next shading trick but to keep the formal structure alive so that decisions stay legible on the screen.

The principle lives in `docs/constraint-principle.md`; treat it as the manifesto for our constraints and refer to it whenever a decision has to respect the same rules.

The workspace has three layers:

- **Engine** (`src/`) — the Rust/wgpu/wasmtime runtime. Native target only.
- **Spec crates** (`VRX-64-slide/`, `VRX-64-sidecar/`) — the ABI and networking contracts. `wasm32-wasip1` + native (for tests).
- **Slides** (`slides/*/`) — each an independent `cdylib + rlib` crate. `wasm32-wasip1` for deployment, native for tests.

---

## Build commands

```bash
# Build and test the full workspace (native, for CI and local dev)
cargo build
cargo test

# Build a single slide to WASM (run from the slide directory)
cd slides/flat
cargo build --target wasm32-wasip1 --release

# Build all slides to WASM
for dir in slides/*/; do
  (cd "$dir" && cargo build --target wasm32-wasip1 --release 2>/dev/null) || true
done

# Run the engine against a specific slide directory
cargo run -- --scene slides/flat

# Add the WASM target if missing
rustup target add wasm32-wasip1
```

Tests include shader validation, package manifest validation, transition math, and ABI compliance. Run `cargo test` before claiming any engine or spec change complete.

---

## Slide anatomy

Every slide crate must:

- Be `crate-type = ["cdylib", "rlib"]` — `cdylib` for WASM deployment, `rlib` for native test compilation
- Depend on `VRX-64-slide` for `SlideSpec<V>`, `Limits`, `StaticMesh`, `DynamicMesh`, etc.
- Export `vzglyd_abi_version() -> u32` returning `1`
- Export `vzglyd_update(dt: f32) -> i32` returning `0` (continue) or `1` (stop)
- Optionally export `vzglyd_init()` and `vzglyd_teardown()`

The slide's entry point is `vzglyd_spec() -> SlideSpec<V>`. It returns the full geometric description — meshes, textures, camera path, scene space — serialised with `postcard`. The engine calls it once on load.

Slides that need network data use a **sidecar**: a separate `wasm32-wasip1` process from `VRX-64-sidecar` that pushes data to the slide via channel. The slide calls `channel_poll()` from `vzglyd_host`. The sidecar calls `channel_push()`. They share nothing else.

---

## Limits

These are the Pi 4 limits. They apply to every slide without exception.

```rust
Limits::pi4() == Limits {
    max_vertices: 60_000,
    max_indices: 120_000,
    max_static_meshes: 4,
    max_dynamic_meshes: 4,
    max_textures: 4,
    max_texture_bytes: 4_194_304, // four 512×512 RGBA8 textures
    max_texture_dim: 512,
}
```

When writing geometry, count. The terrain slide covers an entire landscape in 4,225 vertices. A face budget is a canvas, not a ceiling to approach.

---

## Shaders

Slides use WGSL shaders placed in the package's `shaders/` directory. The engine injects a prelude before compiling, so the slide does not declare reserved bindings or uniforms. It declares `vs_main`, `fs_main`, and any helpers it needs.

The prelude provides the scene-space-specific vertex IO, uniform block, and fixed texture/sampler slots documented in `SHADER_CONTRACT.md`. Some world-space texture slots may be backed by neutral fallbacks when a slide does not supply assets for them.

Banded light, Bayer dithering, grain overlays, and similar treatments are slide-level choices. The engine enforces the contract shape and resource limits, not a universal shading doctrine.

The shader does not declare storage buffers. The shader does not declare additional bind groups. The shader does not use `@compute`. These are properties of the form.

See `SHADER_CONTRACT.md` for the full contract.

---

## Package format

A deployable slide is a `.vzglyd` archive (zip) containing:

```
manifest.json   ← required
slide.wasm      ← required
shaders/        ← optional custom WGSL
assets/         ← optional textures, fonts, scene data
```

`manifest.json` declares the slide name, ABI version, scene space (`World3D` or `Screen2D`), and references to any assets or shader overrides.

The `build.sh` in each slide directory builds the WASM and links the manifest. Review existing ones (e.g. `slides/flat/build.sh`) before writing a new one.

See `SLIDE_FORMAT.md` and `docs/slide-authoring/MANIFEST_PACKAGE_GUIDE.md` for the full schema.

---

## Code style

- Rust 2024 edition throughout.
- No `unwrap()` in engine code — propagate errors with `?` or `thiserror`.
- `unwrap()` is acceptable in slide geometry initialization (`Lazy::new(|| ...)`) where the only failure mode is a programming error.
- Slides serialise their vertex type with `serde` + `postcard`. Vertex structs must be `#[repr(C)]`, `Pod`, and `Zeroable`.
- `glam` for all math. `bytemuck` for vertex casting. `once_cell::sync::Lazy` for static slide state.
- No `println!` in the engine. Use `log::debug!` / `log::warn!` / `log::error!`.
- Slides may use `eprintln!` for sidecar debug output; it goes to stderr and does not affect rendering.
- Feature-gate GPU-specific code in slides behind a `gpu` feature flag (see `slides/terrain/Cargo.toml`). This keeps `cargo test` working on the native target without a display.

---

## Adding a slide

1. `cargo new --lib slides/my_slide`
2. Set `crate-type = ["cdylib", "rlib"]` in `Cargo.toml`
3. Add `VRX-64-slide = { path = "../../VRX-64-slide" }` as a dependency
4. Add the crate to the workspace `members` list in the root `Cargo.toml`
5. Implement `vzglyd_abi_version`, `vzglyd_spec`, and `vzglyd_update` (minimum viable surface)
6. Write a `manifest.json` and `build.sh` modelled on an existing slide
7. `cargo test -p my_slide` — must pass before any WASM build
8. `cargo build --target wasm32-wasip1 --release` — must produce a loadable module

The clock slide (`slides/clock/`) is the simplest fully-working reference. The terrain slide (`slides/terrain/`) is the most complete example of dynamic data and sidecar integration.

---

## Testing

`cargo test` runs:
- Engine unit tests (transition math, shader compilation, ABI validation)
- Per-slide tests (geometry invariants, vertex budget compliance)
- Package validation tests (manifest schema, WASM loadability)

When adding a new slide, add at least one test that validates the slide's `SlideSpec` does not exceed `Limits::pi4()`. The pattern is in `src/slide_renderer.rs` — search for `package_slide_validates`.

When modifying the engine's render path, run the full suite. Shader validation runs headlessly via `naga`; no display required.

---

## What not to do

Do not increase `Limits::pi4()` values to make a slide fit. Count the vertices instead.

Do not turn the renderer into an unrestricted PBR or material-system expansion unless the task explicitly calls for that level of change. Keep shading portable, bounded, and legible within the existing contract.

Do not add network calls inside a slide's WASM module. Network access belongs in a sidecar. The slide receives data through `channel_poll()`.

Do not hard-code a universal visual treatment into the ABI or loader. Shared facilities are fine when they improve portability or ergonomics, but slides should remain free to choose whether they use them.

Do not modify `VRX-64-slide` ABI types without understanding that doing so breaks all existing compiled slides. ABI version bumps are a major decision. See `docs/slide-authoring/ABI_REFERENCE.md`.

---

*VZGLYD. Small worlds, well made.*
