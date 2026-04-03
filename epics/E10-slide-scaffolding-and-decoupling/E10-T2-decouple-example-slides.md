# E10-T2: Decouple Example Slides from Engine Build

| Field | Value |
|-------|-------|
| **Epic** | E10 Slide Scaffolding and Project Decoupling |
| **Priority** | P1 (high) |
| **Estimate** | M |
| **Blocked by** | - |
| **Blocks** | E10-T4 |

## Description

Move the example slide crate dependencies (`terrain_slide`, `flat_slide`, `golf_slide`, `dashboard_slide`) behind a Cargo feature gate so that the default `cargo build` compiles only the engine and `VRX-64-slide`. This ensures that a breaking change in any example slide — including transitive breakage from upstream API changes in the terrain sidecar's Coinbase client — does not prevent the core plugin from building.

## Background

The root `Cargo.toml` currently declares four slide crates as direct `[dependencies]`:

```toml
terrain_slide = { path = "slides/terrain", features = ["gpu"] }
flat_slide  = { path = "slides/flat" }
golf_slide  = { path = "slides/golf" }
dashboard_slide = { path = "slides/dashboard" }
```

These are used in `src/main.rs` for the built-in scene aliases (`--scene terrain`, `--scene flat`, etc.) where the engine embeds each slide's `SlideSpec` at compile time as a fast path — bypassing the WASM loader for development convenience.

The problem: these are *examples*, not core functionality. The engine's WASM slide loader (`slide_loader.rs`) can load any slide from its package directory at runtime. The embedded compile-time specs are a developer convenience that should not gate the build.

Additionally, the three remaining workspace members without root dependencies (`beach_dog`, `double_dash_benchmark`, `courtyard`) are already decoupled — they build independently. This ticket brings the other four into the same pattern.

## Current state — what to look at

- **`Cargo.toml` lines 36–39**: The four slide dependencies.
- **`src/main.rs`**: Scene alias resolution — `"terrain" => terrain_slide::spec()`, etc. This is the only place these crates are imported by the engine.
- **`slides/terrain/sidecar/`**: The Coinbase BTC price sidecar. Not a dependency of the root crate, but `terrain_slide` itself is — and terrain_slide's tests or examples may transitively pull in sidecar-adjacent code.

## Step-by-step implementation

### Step 1 — Add `examples` feature to root Cargo.toml

```toml
[features]
default = []
examples = ["dep:terrain_slide", "dep:flat_slide", "dep:golf_slide", "dep:dashboard_slide"]

[dependencies]
terrain_slide = { path = "slides/terrain", features = ["gpu"], optional = true }
flat_slide  = { path = "slides/flat", optional = true }
golf_slide  = { path = "slides/golf", optional = true }
dashboard_slide = { path = "slides/dashboard", optional = true }
```

### Step 2 — Gate the compile-time spec embedding in main.rs

Wrap the scene alias code in `#[cfg(feature = "examples")]` blocks:

```rust
#[cfg(feature = "examples")]
fn resolve_builtin_scene(name: &str) -> Option<Box<dyn SlideProvider>> {
    match name {
        "terrain" => Some(Box::new(terrain_slide::spec())),
        "flat" => Some(Box::new(flat_slide::spec())),
        // ...
        _ => None,
    }
}

#[cfg(not(feature = "examples"))]
fn resolve_builtin_scene(_name: &str) -> Option<Box<dyn SlideProvider>> {
    None
}
```

When the `examples` feature is off, the engine falls through to the WASM loader path for all scene names, including the built-in aliases. This means `--scene terrain` still works — it just loads `slides/terrain/slide.wasm` at runtime instead of embedding the spec at compile time.

### Step 3 — Ensure the WASM loader path resolves aliases

In the scene resolution logic, if the built-in function returns `None`, map the short alias to the package directory path:

```rust
fn resolve_scene_path(name: &str) -> PathBuf {
    match name {
        "terrain" => PathBuf::from("slides/terrain"),
        "flat" => PathBuf::from("slides/flat"),
        // ...
        other => PathBuf::from(other),
    }
}
```

This mapping already exists in some form. Verify that the fallback to the WASM loader works for all built-in aliases when the `examples` feature is disabled.

### Step 4 — Update development workflow documentation

Note in the README or CONTRIBUTING.md that:
- `cargo build` builds the core engine only.
- `cargo build --features examples` builds with embedded example slides (faster iteration for slide authors working on built-in slides).
- Individual slides can always be built independently: `cargo build -p terrain_slide --target wasm32-wasip1`.

### Step 5 — Verify clean build without examples

```bash
# Core engine builds clean
cargo build -p vzglyd
cargo test -p vzglyd

# With examples also works
cargo build -p vzglyd --features examples
cargo test -p vzglyd --features examples
```

### Step 6 — Consider removing example slides from workspace members

Optionally, move the example slides out of the `[workspace].members` list entirely. This would mean `cargo test` at the workspace root no longer includes them. If they remain workspace members, their compile errors still appear during `cargo check` at the root (which is confusing). Moving them out means they build only when explicitly targeted or via their own `build.sh`.

Evaluate the trade-off: keeping them as workspace members allows shared `Cargo.lock` and consistent dependency versions. Removing them gives full isolation. A middle ground is a separate `slides/Cargo.toml` workspace that shares `VRX-64-slide` via path dependency but has its own lock file.

## Acceptance criteria

- [ ] `cargo build -p vzglyd` succeeds without compiling any slide crate
- [ ] `cargo test -p vzglyd` passes without slide crate compilation
- [ ] `cargo build -p vzglyd --features examples` compiles all four embedded slides
- [ ] `--scene terrain` loads and renders correctly both with and without the `examples` feature (via WASM loader fallback)
- [ ] The terrain sidecar's Coinbase API endpoint returning an error does not affect `cargo build -p vzglyd`
- [ ] CI default build job uses `cargo build -p vzglyd` (no `--features examples`)

## Files to modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `examples` feature, make four slide deps optional |
| `src/main.rs` | Gate embedded spec resolution behind `#[cfg(feature = "examples")]` |
| `README.md` or `CONTRIBUTING.md` | Document the feature gate and development workflow |
