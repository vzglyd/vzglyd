# E10-T4: CI Build Isolation for Example Slides

| Field | Value |
|-------|-------|
| **Epic** | E10 Slide Scaffolding and Project Decoupling |
| **Priority** | P1 (high) |
| **Estimate** | S |
| **Blocked by** | E10-T2 |
| **Blocks** | - |

## Description

Restructure the CI pipeline so that the core engine (`vzglyd` + `VRX-64-slide`) builds and tests in an independent job, and example slides build in a separate downstream job whose failure does not block the engine's build status. This is the CI counterpart to E10-T2's Cargo feature gate — together they ensure that a third-party API change (e.g., Coinbase altering their spot price response schema) produces a yellow warning, not a red build.

## Background

Today, if CI runs `cargo build` or `cargo test` at the workspace root, every workspace member compiles. A compile error in `terrain_slide` (or its sidecar's transitive dependencies) fails the entire pipeline. Since the terrain sidecar integrates with the live Coinbase API, any upstream schema change, rate limit, or TLS certificate rotation can cause spurious failures.

After E10-T2 makes the example slides optional, CI needs two jobs:

1. **Engine job** (gate for merges): `cargo build -p vzglyd && cargo test -p vzglyd && cargo build -p VRX-64-slide && cargo test -p VRX-64-slide`
2. **Examples job** (informational): builds each example slide to `wasm32-wasip1`, runs their tests, and runs `build.sh` to produce packages. This job is `allow-failure` / `continue-on-error`.

## Step-by-step implementation

### Step 1 — Define the engine CI job

This job is the merge gate. It must pass for PRs to land.

```yaml
engine:
  steps:
    - cargo build -p vzglyd
    - cargo test -p vzglyd
    - cargo build -p VRX-64-slide
    - cargo test -p VRX-64-slide
    - cargo clippy -p vzglyd -p VRX-64-slide -- -D warnings
```

No slide crates are compiled. This job should be fast (~2 minutes on cached CI).

### Step 2 — Define the examples CI job

This job builds every example slide independently. It is allowed to fail.

```yaml
examples:
  continue-on-error: true
  strategy:
    matrix:
      slide: [terrain, flat, golf, beach_dog, double_dash_benchmark, dashboard, courtyard]
  steps:
    - rustup target add wasm32-wasip1
    - cargo build -p ${slide}_slide --target wasm32-wasip1
    - cargo test -p ${slide}_slide
    - bash slides/${slide}/build.sh
```

The matrix strategy means each slide builds in isolation — one slide's failure doesn't skip the others.

### Step 3 — Add a sidecar build step for terrain

The terrain sidecar is a separate Cargo project (`slides/terrain/sidecar/`). Add it to the terrain matrix entry:

```yaml
    - cargo build --manifest-path slides/terrain/sidecar/Cargo.toml --target wasm32-wasip1 --release
```

### Step 4 — Badge and notification strategy

- The engine job's badge is displayed on the README as the project's build status.
- The examples job's status is visible in CI but does not contribute to the merge gate.
- Optionally, add a separate badge for examples: "Examples: passing/failing" so maintainers can see at a glance when an upstream API change has broken a demo slide.

### Step 5 — Test the isolation

Simulate the scenario: introduce a deliberate compile error in `slides/terrain/src/lib.rs`. Verify:
- Engine CI job passes.
- Examples CI job fails (only the terrain matrix entry).
- The PR is still mergeable.

Revert the deliberate error after verification.

## Acceptance criteria

- [ ] Engine CI job (`vzglyd` + `VRX-64-slide`) passes independently of all slide crates
- [ ] Examples CI job builds each slide in a matrix, with `continue-on-error: true`
- [ ] A compile error in any single example slide does not block PRs from merging
- [ ] The terrain sidecar builds as part of the terrain matrix entry
- [ ] CI configuration is committed and documented

## Files to create or modify

| File | Change |
|------|--------|
| `.github/workflows/ci.yml` (or equivalent) | Split into engine + examples jobs |
| `README.md` | Update build badge to point at engine job |
