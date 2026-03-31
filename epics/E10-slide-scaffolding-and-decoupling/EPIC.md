# E10: Slide Scaffolding and Project Decoupling

## Summary

VZGLYD's slide ecosystem is growing, but creating a new slide requires manually copying an existing one, adjusting boilerplate, and understanding implicit conventions (Cargo.toml shape, build.sh incantations, manifest fields, asset export patterns). Worse, several example slides are hard-wired as dependencies of the root engine crate — a breaking change in the Coinbase spot API response format, or any third-party data source, directly breaks `cargo build` for the entire project. This epic introduces a `cargo-generate` template for scaffolding new slides, decouples example slides from the engine build, and establishes a CI strategy where the core plugin can ship green even when an upstream API changes.

## Problem

1. **No scaffolding**: Creating a new slide means copying `slides/flat/` or `slides/terrain/`, hand-editing Cargo.toml, build.sh, manifest.json, lib.rs, and the workspace root. Easy to get wrong, tedious to get right.

2. **Coupled build graph**: The root `Cargo.toml` declares `terrain_slide`, `flat_slide`, `golf_slide`, and `dashboard_slide` as direct `[dependencies]`. Any compile error in those crates (including transitive dependency breakage from sidecar API changes) fails the engine build. The terrain sidecar's Coinbase HTTP client — which was always an *example* — is on the critical path for `cargo check`.

3. **No build isolation**: All slides are workspace members, so `cargo test` and `cargo clippy` at the workspace root include every example slide. A new slide author's work-in-progress can break CI for the whole project.

4. **Sidecar boilerplate**: Writing a sidecar involves DNS-over-HTTPS, TLS, chunked HTTP parsing, and WASI socket plumbing — all of which is already solved in `slides/terrain/sidecar/` but not extractable as a reusable starting point.

## Goals

- `cargo generate --path templates/slide` produces a buildable, loadable slide in under 30 seconds.
- The core engine (`cargo build -p vzglyd`) succeeds even if every example slide is deleted.
- Example slides build and test in a separate CI job; their red status does not gate engine releases.
- Sidecar scaffolding is available as an optional template variant for slides that need live data.

## Scope

- Create a `cargo-generate` template under `templates/slide/` with Cargo.toml, lib.rs, manifest.json, build.sh, and optional sidecar scaffolding.
- Move example slide crate dependencies behind a Cargo feature gate (`examples` feature on the root crate) so the default build compiles only the engine + `vzglyd-slide`.
- Restructure CI to build the engine independently, then build example slides as a non-blocking downstream job.
- Extract reusable sidecar networking into a `sidecar_support` crate or inline template.
- Add a `cargo xtask new-slide` (or equivalent) command that wraps `cargo-generate` with project-specific defaults.

## Prerequisites

- All prior epics (E1–E9) are complete — stable ABI, package format, sidecar model, and shader contract are in place.

## Tickets

| ID | Title | Priority | Size | Blocked by | Blocks |
|----|-------|----------|------|------------|--------|
| E10-T1 | Slide template for cargo-generate | P1 | M | - | E10-T3, E10-T5 |
| E10-T2 | Decouple example slides from engine build | P1 | M | - | E10-T4 |
| E10-T3 | Sidecar template variant | P2 | M | E10-T1 | E10-T5 |
| E10-T4 | CI build isolation for example slides | P1 | S | E10-T2 | - |
| E10-T5 | xtask new-slide orchestration command | P2 | S | E10-T1, E10-T3 | - |
