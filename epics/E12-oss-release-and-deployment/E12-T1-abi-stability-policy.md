# E12-T1: ABI Stability Policy and Semver Strategy

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | S |
| **Blocked by** | - |
| **Blocks** | E12-T2 |

## Description

Define and document the semver policy for `VRX-64-slide`: what constitutes a breaking change, what the engine's compatibility window is, and how third-party slide authors should declare their dependency. Write this as a document before the first crates.io publish so the commitment is made publicly from day one.

## Background

`VRX-64-slide` is the ABI boundary between the VZGLYD engine and every slide in the ecosystem. Once a slide is compiled against a specific `VRX-64-slide` version, it produces a `.wasm` binary that the engine loads at runtime. If the ABI changes in a way the engine doesn't handle, that slide stops working — silently or with a cryptic error.

Third-party slide authors need to know:
- Which version of `VRX-64-slide` to depend on
- When they need to recompile to stay compatible with the latest engine
- Whether the engine will load a slide compiled against an older `VRX-64-slide`

Without a documented policy, every engine release is a potential surprise. With one, authors can predict when they need to act.

## Proposed policy

### Semver semantics for VRX-64-slide

`VRX-64-slide` follows standard semver (`MAJOR.MINOR.PATCH`) with the following mapping to WASM ABI impact:

| Change type | Version bump | ABI impact |
|-------------|-------------|------------|
| New exported function the engine calls | MAJOR | Breaking — existing slides don't export it |
| Removing or renaming an exported function | MAJOR | Breaking |
| Adding a field to `SlideSpec` in a binary-breaking way | MAJOR | Breaking |
| New optional field with a default, backward-compatible | MINOR | Non-breaking |
| Doc improvements, test additions, helper functions | PATCH | Non-breaking |
| Publishing `vzglyd_sidecar` changes only | (separate crate) | Independent |

### Engine compatibility window

The engine declares a minimum `VRX-64-slide` major version it will load. Initially this is `abi_version: 1` in each `manifest.json`. The engine rejects slides that declare a version it doesn't understand.

When a breaking `VRX-64-slide` change ships:
- Engine version N supports abi_version N and N-1 (one version of backwards compatibility)
- `manifest.json` in each slide declares `abi_version` — the engine checks this at load time, not at compile time
- A slide compiled against `VRX-64-slide` 1.x with `abi_version: 1` will load on any engine that supports `abi_version: 1`

### What slide authors should write in Cargo.toml

```toml
# Lock to current major — breaking changes will be a new major
VRX-64-slide = "1"
```

Not `"1.0.0"`, not `"^1.0"`, not `"*"`. The `"1"` form picks up minor/patch improvements automatically but never crosses a major boundary.

### What constitutes a breaking change (non-exhaustive list)

Breaking (MAJOR):
- Changing the signature of `vzglyd_update(dt: f32) -> i32`
- Adding a required exported symbol (e.g., `vzglyd_init`, `vzglyd_teardown`)
- Changing the `SlideSpec` wire format (postcard serialisation layout)
- Changing the `Vertex` trait bounds in a way that breaks existing impls
- Removing or renaming any public type in `VRX-64-slide`

Non-breaking (MINOR or PATCH):
- Adding new optional fields to `SlideSpec` that default to existing behavior
- Adding new public helper types or constructors that don't affect existing slides
- Improving error messages or panicking conditions

### Pre-1.0 disclaimer

Until `VRX-64-slide` reaches 1.0.0, minor version bumps may contain breaking changes (standard cargo/semver convention for 0.x versions). The engine repo CHANGELOG will explicitly call out ABI impact for every `VRX-64-slide` release. There will be no silent ABI changes.

## Step-by-step implementation

### Step 1 — Write ABI_POLICY.md in VRX-64-slide/

Create `VRX-64-slide/ABI_POLICY.md` with the above policy as the authoritative document. Link it from the crate-level rustdoc `lib.rs` as `//! See [ABI_POLICY.md](../ABI_POLICY.md) for the versioning and stability contract.`

### Step 2 — Add abi_version constant to VRX-64-slide

```rust
/// The current ABI version. Slides embed this in their manifest.json.
/// The engine checks this at load time.
pub const ABI_VERSION: u32 = 1;
```

This is a compile-time constant that slide authors reference rather than hardcoding `1` in their manifests.

### Step 3 — Record current ABI_VERSION in manifest validation

Verify that the engine's manifest loader rejects slides with an unrecognised `abi_version`. Add a test asserting this rejection.

### Step 4 — Write CHANGELOG.md in VRX-64-slide/

Start a `CHANGELOG.md` at the VRX-64-slide package root. The initial entry:

```markdown
## Unreleased / 0.1.0

Initial public release. ABI version 1.
```

This file will be maintained going forward. Every PR that bumps VRX-64-slide must add a CHANGELOG entry noting whether the change is breaking.

## Acceptance criteria

- [ ] `VRX-64-slide/ABI_POLICY.md` exists and covers: semver mapping, engine compatibility window, recommended dependency declaration, and a non-exhaustive breaking-change list
- [ ] `VRX-64-slide/CHANGELOG.md` exists with an initial entry
- [ ] `pub const ABI_VERSION: u32 = 1` is exported from `VRX-64-slide`
- [ ] Engine manifest loader test asserts rejection of unknown `abi_version`
- [ ] Policy is linked from `VRX-64-slide/src/lib.rs` top-level doc comment

## Files to create/modify

| File | Change |
|------|--------|
| `VRX-64-slide/ABI_POLICY.md` | New — policy document |
| `VRX-64-slide/CHANGELOG.md` | New — release history |
| `VRX-64-slide/src/lib.rs` | Add `ABI_VERSION` constant and doc link |
| `src/` (engine manifest loader) | Add abi_version rejection test |
