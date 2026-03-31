# E12-T2: Publish vzglyd-slide to crates.io

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | M |
| **Blocked by** | E12-T1 |
| **Blocks** | E12-T3, E12-T4, E12-T13, E12-T14 |

## Description

Prepare `vzglyd-slide` for publication on crates.io: fill out all required metadata, set MSRV, ensure the crate compiles to both `wasm32-wasip1` (slides) and the host target (engine), write enough documentation to be useful on docs.rs, and set up a GitHub Actions workflow that publishes on tag. Perform the initial `cargo publish`.

## Background

`vzglyd-slide` is the single most important crate in the VZGLYD ecosystem. Every slide in the world depends on it. Publishing it to crates.io lets slide authors work in completely isolated repos without path dependencies back to the engine monorepo.

The crate is already well-shaped internally. This ticket is about the publication ceremony: metadata, CI, docs, and the actual publish.

## Current state

`vzglyd-slide/Cargo.toml` is minimal:

```toml
[package]
name = "vzglyd-slide"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
bytemuck = { version = "1", features = ["derive"] }
```

Missing: description, license, repository, homepage, keywords, categories, documentation, MSRV, readme.

## Step-by-step implementation

### Step 1 — Fill out Cargo.toml metadata

```toml
[package]
name = "vzglyd-slide"
version = "0.1.0"
edition = "2024"
description = "ABI contract and data types for VZGLYD display engine slides"
license = "MIT OR Apache-2.0"
repository = "https://github.com/vzglyd/vzglyd"
homepage = "https://github.com/vzglyd/vzglyd"
documentation = "https://docs.rs/vzglyd-slide"
readme = "README.md"
keywords = ["vzglyd", "slide", "display", "wasm", "embedded-display"]
categories = ["rendering", "embedded", "wasm"]
rust-version = "1.85"   # edition 2024 MSRV
```

The license choice of `MIT OR Apache-2.0` is the Rust ecosystem standard dual-license. This should match whatever top-level license is chosen in E12-T12.

### Step 2 — Create vzglyd-slide/README.md

A short README visible on crates.io:

- One paragraph: what is `vzglyd-slide`, what is VZGLYD
- The Cargo.toml dependency snippet (`vzglyd-slide = "0"`)
- The minimal `vzglyd_update` function signature with a code block
- A link to the full slide authoring guide (once E12-T13 exists)
- A link to ABI_POLICY.md

### Step 3 — Verify cross-compilation

`vzglyd-slide` must compile on both targets that use it:

```bash
# Engine (host)
cargo check -p vzglyd-slide

# Slides (WASM)
cargo check -p vzglyd-slide --target wasm32-wasip1
```

Both must succeed without warnings. This is likely already true since the crate has no platform-specific dependencies, but verify explicitly.

### Step 4 — Add a basic doc-test to lib.rs

At minimum, ensure the top-level module doc has a `no_run` example showing how to build a `SlideSpec`. This serves as a smoke-test and gives docs.rs a useful first page.

```rust
//! # vzglyd-slide
//!
//! Type definitions and ABI contract for [VZGLYD](https://github.com/vzglyd/vzglyd) slides.
//!
//! Add to your slide's `Cargo.toml`:
//! ```toml
//! [dependencies]
//! vzglyd-slide = "0"
//! ```
//!
//! Implement `vzglyd_update` to drive your slide:
//! ```no_run
//! use vzglyd_slide::SlideSpec;
//!
//! #[no_mangle]
//! pub extern "C" fn vzglyd_update(dt: f32) -> i32 {
//!     // Return 0 = no geometry change, 1 = geometry updated
//!     0
//! }
//! ```
```

### Step 5 — Dry-run publish

```bash
cd vzglyd-slide
cargo publish --dry-run
```

Fix any errors or warnings crates.io would reject. Common issues: missing license file symlink, missing readme, description too long.

### Step 6 — Set up GitHub Actions release workflow

Create `.github/workflows/publish-slide-spec.yml`:

```yaml
name: Publish vzglyd-slide

on:
  push:
    tags:
      - "vzglyd-slide-v*"

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo publish -p vzglyd-slide
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
```

Tag convention: `vzglyd-slide-v0.1.0`. This allows `vzglyd_sidecar`, the engine, and slide repos to all release independently on their own tags.

### Step 7 — Perform the initial publish

```bash
cd vzglyd-slide
cargo publish
```

Record the crates.io URL in the project README and EPIC.md.

After publishing, update any path dependencies in the workspace that can be switched:

```toml
# Before
vzglyd-slide = { path = "../vzglyd-slide" }

# After (slides that are still in the workspace for now)
vzglyd-slide = { path = "../vzglyd-slide" }  # keep path for workspace builds

# Slides outside the workspace (after T4)
vzglyd-slide = "0.1"
```

The workspace slides keep the path dependency for fast iteration. External slides use the crates.io version.

## Acceptance criteria

- [ ] `vzglyd-slide/Cargo.toml` has description, license, repository, keywords, categories, documentation, readme, rust-version
- [ ] `vzglyd-slide/README.md` exists with dependency snippet, minimal example, ABI policy link
- [ ] `cargo check -p vzglyd-slide` and `cargo check -p vzglyd-slide --target wasm32-wasip1` both pass
- [ ] `cargo publish --dry-run` succeeds without errors or warnings
- [ ] GitHub Actions workflow exists at `.github/workflows/publish-slide-spec.yml`
- [ ] The crate is live on crates.io at `https://crates.io/crates/vzglyd-slide`
- [ ] docs.rs renders successfully (check within ~15 minutes of publish)

## Files to create/modify

| File | Change |
|------|---------|
| `vzglyd-slide/Cargo.toml` | Add all publication metadata |
| `vzglyd-slide/README.md` | New — crates.io landing page |
| `vzglyd-slide/src/lib.rs` | Add top-level module doc with example |
| `.github/workflows/publish-slide-spec.yml` | New — release CI |
