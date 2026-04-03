# E12-T5: Prune vzglyd Workspace to Engine-Only

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P1 (high) |
| **Estimate** | S |
| **Blocked by** | E12-T4 |
| **Blocks** | - |

## Description

After all slides have been extracted to their own repos (E12-T4), remove them from the engine's `Cargo.toml` workspace members and from the `slides/` directory. The engine repo ends up containing only the core engine, `VRX-64-slide`, `vzglyd_sidecar`, `slides/loading/`, and the `templates/` directory.

## Background

This is the cleanup step that completes the decoupling. E10-T2 put slide deps behind a feature gate; E12-T4 moved them to separate repos; this ticket deletes the dead wood.

After this ticket, `cargo build` in the engine repo builds only:
- `vzglyd` (the engine binary)
- `VRX-64-slide`
- `vzglyd_sidecar`
- `loading_slide`

A fresh `cargo check` takes seconds instead of minutes. The engine CI is clean and fast.

## Step-by-step implementation

### Step 1 — Confirm all slides are extracted and live

Before deleting anything: verify each slide repo exists on GitHub, builds independently, and has at least one release tag. Only then proceed.

### Step 2 — Remove workspace members from Cargo.toml

Edit the root `Cargo.toml` `[workspace].members` array to contain only:

```toml
[workspace]
members = [
    ".",
    "VRX-64-slide",
    "vzglyd_sidecar",
    "slides/loading",
]
```

### Step 3 — Remove [dependencies] on example slides

The `examples` feature gate from E10-T2 should have already made these optional. Remove them entirely:

```toml
# Remove these lines entirely:
# terrain_slide = { ... }
# flat_slide = { ... }
# golf_slide = { ... }
# dashboard_slide = { ... }
```

And remove the `examples` feature definition since there are no longer any bundled example slides in the repo.

### Step 4 — Remove the slides/ directory (except loading)

```bash
git rm -r slides/terrain slides/flat slides/golf slides/dashboard \
         slides/weather slides/air_quality slides/afl slides/word_of_day \
         slides/on_this_day slides/lastfm slides/calendar slides/servers \
         slides/news slides/reminders slides/quotes slides/affirmations \
         slides/did_you_know slides/clock slides/budget slides/chore \
         slides/beach_dog slides/double_dash_benchmark slides/courtyard
```

`slides/loading/` is NOT removed.

### Step 5 — Update src/main.rs

Remove all references to the (now-gone) embedded example slides. The builtin alias resolution should already fall through to the WASM loader path from E10-T2. Clean up any lingering `#[cfg(feature = "examples")]` blocks and the `resolve_builtin_scene` function that returned non-None variants — replace with a single WASM loader path for all scenes.

### Step 6 — Update CI

The CI workflow no longer needs a separate "build example slides" job. Remove it. The engine CI job is now: `cargo build -p vzglyd` + `cargo test -p vzglyd`.

### Step 7 — Verify a clean build

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

All must pass without reference to any removed slide crates.

## Acceptance criteria

- [ ] `[workspace].members` contains only engine, VRX-64-slide, vzglyd_sidecar, slides/loading
- [ ] `cargo build` completes without building any extracted slide
- [ ] `cargo test` passes
- [ ] `slides/` directory contains only `loading/`
- [ ] No references to removed slide crates remain in `src/`
- [ ] CI workflow does not reference example slide build jobs

## Files to modify

| File | Change |
|------|---------|
| `Cargo.toml` | Prune workspace.members and remove slide deps |
| `src/main.rs` | Remove embedded scene resolution |
| `slides/` | Delete all subdirectories except `loading/` |
| `.github/workflows/ci.yml` | Remove example slide build job |
