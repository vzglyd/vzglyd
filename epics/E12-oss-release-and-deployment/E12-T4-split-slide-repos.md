# E12-T4: Split Slides into Individual Git Repositories

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P1 (high) |
| **Estimate** | XL |
| **Blocked by** | E12-T2, E12-T3 |
| **Blocks** | E12-T5 |

## Description

Each slide (with the exception of the loading slide) moves from `slides/<name>/` in the engine repo into its own dedicated git repository under a shared GitHub organisation. The loading slide stays in the engine repo because it is tightly coupled to the engine's startup sequence. Every other slide becomes a first-class standalone project: its own issues, its own releases, its own CI, its own README.

## Background

The monorepo model made sense during initial development when slides, the engine, and the slide API were all evolving together. Now that the ABI is stable and published, the coupling is a liability:

- A sidecar API change in `slides/weather/` should not show up as noise in the engine's git log.
- A third-party contributor to `slides/afl/` shouldn't need to clone the entire engine repo.
- Each slide should have independent versioning — v2 of the weather slide should be able to ship without bumping the engine.
- The slide authoring guide is more credible if the official slides themselves follow the "external repo" workflow.

The loading slide stays because it is launched by the engine before any WASM loader runs — it is compiled into the engine binary, not loaded at runtime.

## Repository structure

All slide repos live under the `vzglyd` GitHub organisation (or equivalent). Naming convention: `vzglyd/slide-<name>`.

| Repo | Slides | Notes |
|------|--------|-------|
| `vzglyd/slide-terrain` | `slides/terrain/` + sidecar | 3D scene with Coinbase BTC ticker |
| `vzglyd/slide-flat` | `slides/flat/` | Dev/test flat scene |
| `vzglyd/slide-golf` | `slides/golf/` | Golf scores |
| `vzglyd/slide-courtyard` | `slides/courtyard/` | 3D courtyard scene |
| `vzglyd/slide-beach-dog` | `slides/beach_dog/` | |
| `vzglyd/slide-weather` | `slides/weather/` + sidecar | BOM forecast |
| `vzglyd/slide-air-quality` | `slides/air_quality/` + sidecar | Pollen |
| `vzglyd/slide-afl` | `slides/afl/` + sidecar | Squiggle AFL data |
| `vzglyd/slide-word-of-day` | `slides/word_of_day/` + sidecar | Dictionary API |
| `vzglyd/slide-on-this-day` | `slides/on_this_day/` + sidecar | Wikipedia |
| `vzglyd/slide-lastfm` | `slides/lastfm/` + sidecar | Last.fm |
| `vzglyd/slide-calendar` | `slides/calendar/` + sidecar | ICS calendar |
| `vzglyd/slide-servers` | `slides/servers/` + sidecar | Server health |
| `vzglyd/slide-news` | `slides/news/` + sidecar | RSS/Reddit/SSE |
| `vzglyd/slide-reminders` | `slides/reminders/` + sidecar | iCloud bridge |
| `vzglyd/slide-quotes` | `slides/quotes/` | Inline data |
| `vzglyd/slide-affirmations` | `slides/affirmations/` | Inline data |
| `vzglyd/slide-did-you-know` | `slides/did_you_know/` | Inline CSV |
| `vzglyd/slide-clock` | `slides/clock/` | System time |
| `vzglyd/slide-budget` | `slides/budget/` | Static YAML |
| `vzglyd/slide-chore` | `slides/chore/` | Static YAML |
| `vzglyd/slide-dashboard` | `slides/dashboard/` | |
| `vzglyd/slide-double-dash-benchmark` | `slides/double_dash_benchmark/` | Benchmark slide |

**Stays in engine repo:** `slides/loading/` — not a standalone slide, part of the engine.

## Per-repo standard structure

Each slide repository follows this layout:

```
slide-<name>/
├── Cargo.toml               # Uses VRX-64-slide = "0.x" from crates.io
├── src/
│   └── lib.rs
├── shaders/
│   ├── vertex.wgsl
│   └── fragment.wgsl
├── assets/                  # (if any)
├── manifest.json
├── build.sh
├── sidecar/                 # (if applicable)
│   ├── Cargo.toml           # Uses vzglyd_sidecar = "0.x" from crates.io
│   └── src/
│       └── main.rs
├── README.md
├── LICENSE
├── CHANGELOG.md
└── .github/
    └── workflows/
        └── ci.yml           # build + test + package .vzglyd artifact
```

## Step-by-step implementation

### Step 1 — Create GitHub organisation

Create `vzglyd` organisation (or confirm it exists). Set org-level settings: visibility defaults, branch protection rules, etc.

### Step 2 — Create slide repository template

Create `vzglyd/slide-template` (the cargo-generate template from E10-T1) as the canonical starting point. This is the reference that `cargo generate gh:vzglyd/slide-template` will use after this ticket.

### Step 3 — Extract slides (repeat per slide)

For each slide in the table above:

1. Create the new repo at `github.com/vzglyd/slide-<name>`.
2. Use `git filter-repo` or subtree split to preserve commit history from `slides/<name>/` (preserving history is preferred over a clean import — it gives proper attribution).
3. Update `Cargo.toml` to replace `VRX-64-slide = { path = "../../VRX-64-slide" }` with `VRX-64-slide = "0.1"` and similarly for `vzglyd_sidecar`.
4. Add `README.md`, `LICENSE`, `CHANGELOG.md`.
5. Add `.github/workflows/ci.yml` (see E12-T9 for the distribution CI definition).
6. Verify `cargo build --target wasm32-wasip1 --release` succeeds in the new repo.
7. Tag initial release `v0.1.0`.

### Step 4 — Preserving history with git filter-repo

```bash
# In a clone of the engine repo
git filter-repo --path slides/weather/ --path-rename slides/weather/:./
```

This rewrites history so that the subtree appears at the root of a new repo. Push this to `github.com/vzglyd/slide-weather`.

### Step 5 — Prioritise based on E11 completion status

Not all slides will be fully implemented when this ticket runs. Extract slides in this order:
1. **Tier 0 first** (clock, quotes, affirmations, did_you_know) — simple, no sidecar, guaranteed to be complete
2. **Tier 1 next** (weather, afl, etc.) — after E11-T3 is complete
3. **Tier 2/3** — after E11-T4/T5

It is acceptable to create the repo with a `main` branch and no initial release for slides that aren't complete yet. The repo signals intent; the tag signals production readiness.

### Step 6 — Update the engine repo's cargo-generate template

After split, the `templates/slide/Cargo.toml.liquid` should reference crates.io versions, not relative paths. The `[workspace]` section in the template's `Cargo.toml.liquid` should be absent — each slide is its own independent crate, not a workspace member.

### Step 7 — Add to registry (feeds into E12-T10)

As each slide repo is created and tagged, add it to the registry index. The registry is the source of truth for which repos exist.

## Acceptance criteria

- [ ] GitHub org `vzglyd` exists with all slide repos created
- [ ] Each slide repo has: Cargo.toml with crates.io deps, README, LICENSE, CHANGELOG, CI workflow
- [ ] Each slide repo builds to `wasm32-wasip1` independently (no path deps outside the repo)
- [ ] Git history is preserved (original commit authors visible in `git log`)
- [ ] `slides/loading/` remains in the engine repo; all others are removed after successful extraction
- [ ] `cargo generate gh:vzglyd/slide-template --name my_slide` produces a buildable crate

## Files to modify (in engine repo, after extraction)

| File | Change |
|------|---------|
| `Cargo.toml` | Remove extracted slides from `[workspace].members` |
| `slides/` | Remove all slide directories except `loading/` |
| `templates/slide/Cargo.toml.liquid` | Switch to crates.io version deps |
