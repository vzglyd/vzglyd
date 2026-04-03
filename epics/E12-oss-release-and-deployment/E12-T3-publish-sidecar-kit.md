# E12-T3: Publish vzglyd_sidecar to crates.io

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | M |
| **Blocked by** | E12-T2 |
| **Blocks** | E12-T4 |

## Description

Prepare `vzglyd_sidecar` for publication on crates.io: fill out metadata, write documentation, verify it compiles to `wasm32-wasip1`, set up a release workflow, and publish. This unblocks any slide author who needs live data: they can write a sidecar without vendoring the terrain sidecar's DNS/TLS stack.

## Background

`vzglyd_sidecar` is the companion crate to `VRX-64-slide`. Where `VRX-64-slide` is the WASM slide ABI, `vzglyd_sidecar` is the WASM sidecar standard library: HTTPS client, channel push/poll, poll loop harness. Without it on crates.io, every sidecar author must either copy the 800-line networking stack from `slides/terrain/sidecar/` or vendor the whole engine repo.

The crate was extracted as part of E11-T1 and is already used by all ported sidecars. This ticket is publication-only — the implementation is complete.

## Current state

`vzglyd_sidecar/Cargo.toml` needs inspection. The crate should already expose:

- `https_get(host, path, headers) -> Result<Vec<u8>, Error>`
- `https_get_with_etag(host, path, etag) -> Result<(Vec<u8>, Option<String>), Error>`
- `channel_push(payload: &[u8])` / `channel_poll(buf: &mut [u8]) -> i32`
- `channel_active() -> bool`
- `sleep_secs(n: u32)`
- `PollLoop` harness

## Step-by-step implementation

### Step 1 — Fill out Cargo.toml metadata

```toml
[package]
name = "vzglyd_sidecar"
version = "0.1.0"
edition = "2024"
description = "Networking and IPC utilities for VZGLYD slide sidecars (WASI targets)"
license = "MIT OR Apache-2.0"
repository = "https://github.com/vzglyd/vzglyd"
homepage = "https://github.com/vzglyd/vzglyd"
documentation = "https://docs.rs/vzglyd_sidecar"
readme = "README.md"
keywords = ["vzglyd", "sidecar", "wasm", "wasi", "https"]
categories = ["wasm", "network-programming", "embedded"]
rust-version = "1.85"
```

### Step 2 — Create vzglyd_sidecar/README.md

- What is a VZGLYD sidecar and why does this crate exist
- The `poll_loop` pattern with a short example showing a weather fetch
- Cargo.toml snippet with `wasm32-wasip1` target note
- Link to the slide authoring guide

### Step 3 — Verify wasm32-wasip1 compilation

```bash
cargo check -p vzglyd_sidecar --target wasm32-wasip1
```

`vzglyd_sidecar` is only useful on `wasm32-wasip1`. It likely won't (and shouldn't need to) compile on the host target — document this clearly. If crates.io's docs.rs can't build it on the default target, add a `package.metadata.docs.rs` section:

```toml
[package.metadata.docs.rs]
targets = ["wasm32-wasip1"]
default-target = "wasm32-wasip1"
```

### Step 4 — Write module-level docs and doc-tests

Each public function needs at minimum a one-line doc comment and a `# Errors` section where applicable. The `poll_loop` entry point deserves a full usage example.

Mark examples that use WASI-only bindings as `no_run`:

```rust
/// Run a sidecar fetch loop indefinitely.
///
/// # Example
///
/// ```no_run
/// use vzglyd_sidecar::{https_get, channel_push, PollLoop};
///
/// PollLoop::new(30).run(|| {
///     let body = https_get("api.example.com", "/data", &[])?;
///     channel_push(&body);
///     Ok(())
/// });
/// ```
```

### Step 5 — Dry-run and publish

```bash
cargo publish -p vzglyd_sidecar --dry-run
cargo publish -p vzglyd_sidecar
```

### Step 6 — GitHub Actions release workflow

Create `.github/workflows/publish-sidecar-kit.yml` matching the pattern from E12-T2. Tag convention: `vzglyd_sidecar-v0.1.0`.

### Step 7 — Update existing sidecars to reference crates.io version

After publishing, the sidecars in the workspace still use `{ path = "../../vzglyd_sidecar" }` — that's fine for workspace builds. Document in each sidecar's README that external sidecars should use `vzglyd_sidecar = "0.1"`.

Once slides are split into their own repos (E12-T4), their `Cargo.toml` will switch from path to version dep.

## Acceptance criteria

- [ ] `vzglyd_sidecar/Cargo.toml` has all publication metadata
- [ ] `vzglyd_sidecar/README.md` exists with usage example
- [ ] `cargo check -p vzglyd_sidecar --target wasm32-wasip1` succeeds
- [ ] All public items have doc comments
- [ ] `[package.metadata.docs.rs]` targets `wasm32-wasip1`
- [ ] GitHub Actions workflow exists at `.github/workflows/publish-sidecar-kit.yml`
- [ ] Crate is live on crates.io at `https://crates.io/crates/vzglyd_sidecar`
- [ ] docs.rs renders on the WASM target

## Files to create/modify

| File | Change |
|------|---------|
| `vzglyd_sidecar/Cargo.toml` | Add all publication metadata |
| `vzglyd_sidecar/README.md` | New — crates.io landing page |
| `vzglyd_sidecar/src/lib.rs` | Add doc comments and examples |
| `.github/workflows/publish-sidecar-kit.yml` | New — release CI |
