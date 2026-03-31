# E12-T14: vzglyd-slide and vzglyd_sidecar Rustdoc

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P1 (high) |
| **Estimate** | M |
| **Blocked by** | E12-T2 |
| **Blocks** | - |

## Description

Write complete rustdoc documentation for every public item in `vzglyd-slide` and `vzglyd_sidecar`. These docs appear on docs.rs automatically after crates.io publish and are the primary API reference for slide authors. Every function, struct, enum, and constant needs at minimum: a one-line summary, a longer description where the behaviour is non-obvious, and a usage example where appropriate.

## Background

docs.rs is where Rust developers go first when they add a new dependency. If the docs page is empty or unhelpful, they clone the repo and read the source — that's friction. If the docs are clear, they can build confidently without ever leaving their editor.

`vzglyd-slide` in particular needs excellent docs because its types and traits define the contract that every slide must implement. A slide author who misunderstands `SlideSpec` will spend hours debugging rendering artefacts.

## vzglyd-slide documentation targets

### Module-level (`lib.rs`)

```rust
//! # vzglyd-slide
//!
//! Type definitions and ABI contract for [VZGLYD](https://github.com/vzglyd/vzglyd) slides.
//!
//! VZGLYD is a Raspberry Pi display engine. Slides are WebAssembly modules that implement
//! the `vzglyd_update` function and return [`SlideSpec`] geometry each frame.
//!
//! ## Quick start
//!
//! Add to your `Cargo.toml`:
//! ```toml
//! [dependencies]
//! vzglyd-slide = "0"
//!
//! [lib]
//! crate-type = ["cdylib"]  # required for WASM compilation
//! ```
//!
//! Implement `vzglyd_update`:
//! ```no_run
//! use vzglyd_slide::{SlideSpec, ABI_VERSION};
//!
//! static mut STATE: Option<MySlideState> = None;
//!
//! #[no_mangle]
//! pub extern "C" fn vzglyd_update(dt: f32) -> i32 {
//!     // Return 0: geometry unchanged, engine reuses last frame's buffers
//!     // Return 1: geometry updated, engine uploads new vertex/index data
//!     0
//! }
//! ```
//!
//! ## ABI stability
//!
//! See [`ABI_POLICY.md`](https://github.com/vzglyd/vzglyd/blob/main/vzglyd-slide/ABI_POLICY.md)
//! for the versioning contract.
```

### `SlideSpec<V>`

Every field documented with:
- What it controls in the render pipeline
- What happens if it's empty/zero/None
- Whether it can change between frames or must remain static

### `SceneSpace` enum

- `World3d`: the camera and coordinate system semantics
- `Screen2d`: pixel-space, origin top-left vs bottom-left

### `vzglyd_update` contract

Not a trait (it's a raw export), but document it as a module-level item explaining:
- The dt parameter (seconds since last call, typically ~0.016 at 60fps)
- Return value semantics (0 = no change, 1 = updated)
- How to pass the `SlideSpec` back to the engine (the WASM memory/pointer convention)
- Threading: called from a single thread, never concurrently

### `ABI_VERSION` constant

```rust
/// Current ABI version. Embed this in your `manifest.json` `abi_version` field.
///
/// The engine checks this at load time. If your slide declares an `abi_version`
/// the engine doesn't recognise, the slide will be rejected with a clear error.
///
/// When this constant increments (a new major version of `vzglyd-slide`), you must
/// recompile your slide and update your `manifest.json`.
pub const ABI_VERSION: u32 = 1;
```

## vzglyd_sidecar documentation targets

### Module-level

```rust
//! # vzglyd_sidecar
//!
//! Networking and IPC utilities for [VZGLYD](https://github.com/vzglyd/vzglyd) slide sidecars.
//!
//! A sidecar is a WebAssembly module compiled to `wasm32-wasip1` that fetches data
//! from external sources and pushes it to its paired slide via a shared memory channel.
//! This crate provides the HTTPS client, channel I/O, and poll loop harness needed
//! to write a sidecar in ~50 lines of Rust.
//!
//! ## Typical sidecar structure
//!
//! ```no_run
//! use vzglyd_sidecar::{https_get, channel_push, PollLoop};
//!
//! fn main() {
//!     PollLoop::new(60).run(|| {
//!         let body = https_get("api.example.com", "/endpoint", &[])?;
//!         channel_push(&body);
//!         Ok(())
//!     });
//! }
//! ```
//!
//! ## Target
//!
//! This crate is only useful on `wasm32-wasip1`. It will not compile on other targets.
```

### `https_get`

Document:
- DNS resolution mechanism (DNS-over-HTTPS — explain why)
- TLS (rustls, no system certs needed)
- What happens on 4xx/5xx vs network error
- Timeout behaviour
- The headers parameter format

### `https_get_with_etag`

Document the conditional GET pattern, why it matters for rate-limited APIs, what the engine does with the returned ETag.

### `channel_push` / `channel_poll`

The most important functions — document clearly:
- The shared memory channel is between the sidecar process and the slide WASM
- `channel_push`: non-blocking, overwrites the previous value if the slide hasn't read it yet
- `channel_poll`: called from `vzglyd_update`, returns -1 if no new data, otherwise the number of bytes written to the buffer
- Buffer sizing: the maximum payload size and what happens if you exceed it
- Include an end-to-end example showing sidecar push and slide poll together

### `PollLoop`

Document:
- The interval parameter (seconds between fetch calls)
- The visibility check (`channel_active`) — loop skips fetches when the slide is not displayed
- Retry semantics on error (exponential backoff, max retry interval)
- How to stop the loop (it doesn't stop — sidecars run for the lifetime of the slide session)

## Doc-tests

Where possible, use `# use` lines to make examples runnable without `no_run`:

```rust
/// # Example
///
/// ```
/// use vzglyd_slide::ABI_VERSION;
/// assert_eq!(ABI_VERSION, 1);
/// ```
```

For WASI-only functions, use `no_run` and note the reason in a `# Platform` section.

## CI integration

Add a doc-test job to the engine CI:

```yaml
- name: Check vzglyd-slide docs
  run: cargo doc -p vzglyd-slide --no-deps 2>&1 | grep -v "^$" | head -20
  # Fails if rustdoc emits warnings about broken links or missing docs
```

Use `#![deny(missing_docs)]` in both crates to enforce documentation as a compile error.

## Acceptance criteria

- [ ] `#![deny(missing_docs)]` is set in both `vzglyd-slide/src/lib.rs` and `vzglyd_sidecar/src/lib.rs`
- [ ] `cargo doc -p vzglyd-slide --no-deps` produces zero warnings
- [ ] `cargo doc -p vzglyd_sidecar --no-deps` produces zero warnings
- [ ] Every public function, struct, enum, and constant has a doc comment
- [ ] `vzglyd_update` contract is documented at the module level with parameter and return value semantics
- [ ] `channel_push` / `channel_poll` include an end-to-end example
- [ ] All doc-tests pass (`cargo test --doc -p vzglyd-slide`)
- [ ] docs.rs pages render correctly after publish (check within 15 minutes)

## Files to modify

| File | Change |
|------|---------|
| `vzglyd-slide/src/lib.rs` | Add `#![deny(missing_docs)]`, module-level docs, all item docs |
| `vzglyd_sidecar/src/lib.rs` | Add `#![deny(missing_docs)]`, module-level docs, all item docs |
| `.github/workflows/ci.yml` | Add doc-check job |
