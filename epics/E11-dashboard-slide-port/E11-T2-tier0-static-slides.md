# E11-T2: Port Tier 0 Slides (Clock, Quotes, Affirmations, Did-You-Know)

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P2 (medium) |
| **Estimate** | S |
| **Blocked by** | - |
| **Blocks** | - |

## Description

Port the four dashboard slides that require no sidecar — their data is either computed at runtime (clock) or embedded at compile time (quotes, affirmations, did-you-know). These slides validate the basic slide authoring workflow and serve as the simplest examples in the VZGLYD repository.

## Slides

### Clock

**Dashboard**: Lua script calling `os.date()` each frame, renders HH:MM in large text.

**VZGLYD approach**: Use WASI `clock_time_get` in `vzglyd_update(dt)` to get wall-clock time. Format as HH:MM. Update the overlay text each frame. No sidecar needed.

**Key implementation detail**: The terrain slide already demonstrates text overlay via `RuntimeOverlay` — the clock slide uses the same mechanism with a font atlas texture, updating vertex positions for the time digits each frame.

### Quotes

**Dashboard**: Hardcoded array of quote strings, randomly selected, displayed with author attribution.

**VZGLYD approach**: Embed quotes as a `const` array in `lib.rs`. Select one at init (or rotate on a timer). Render as overlay text. No sidecar.

### Affirmations

**Dashboard**: Identical pattern to quotes — hardcoded affirmation strings, random selection.

**VZGLYD approach**: Same as quotes. Could potentially be the same slide crate with different embedded data, but keeping them separate matches the dashboard's plugin model and is simpler.

### Did-You-Know

**Dashboard**: 20-category CSV fact database (~500 facts), synced on startup, random selection per slide instance (3 slide instances in the dashboard).

**VZGLYD approach**: Embed the fact database as a compiled-in data structure (e.g., `include_str!` of a CSV or a `const` array). Select a random category and fact in `vzglyd_update` using WASI random. Rotate facts on a timer. No sidecar — the data is static and small enough to compile in.

## Step-by-step implementation

### Step 1 — Scaffold each slide

Use the template from E10-T1 (or manually create if E10 isn't done yet):
```
slides/clock/
slides/quotes/
slides/affirmations/
slides/did_you_know/
```

Each with: `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`.

Scene space: `screen_2d` for all four (text overlay, no 3D geometry).

### Step 2 — Implement clock slide

- `lib.rs`: Define a simple `Vertex` (position + tex_coords + color).
- `vzglyd_update(dt)`: Call WASI `clock_time_get(CLOCK_REALTIME)` → format HH:MM → update overlay vertices.
- `spec()`: Set up font atlas texture, overlay region, `SceneSpace::Screen2d`.
- Test: Verify spec validation passes, verify time formatting logic.

### Step 3 — Implement quotes slide

- `lib.rs`: `const QUOTES: &[(&str, &str)] = &[("quote", "author"), ...]`
- Copy quotes from `dashboard/plugins/quotes/plugin.json` inline data.
- `vzglyd_update(dt)`: Track elapsed time, rotate to next quote every N seconds.
- Render: Title "Quote" + quote text + "— Author" attribution.

### Step 4 — Implement affirmations slide

- Same structure as quotes with different data.
- Copy affirmations from `dashboard/plugins/affirmations/plugin.json`.

### Step 5 — Implement did-you-know slide

- `lib.rs`: Embed fact database via `include_str!("../data/facts.csv")` or similar.
- Parse at init into category→facts map.
- `vzglyd_update(dt)`: Rotate facts on timer, pick random category + fact.
- Render: Category header + fact text.
- Copy fact data from `dashboard/plugins/did-you-know/data/`.

### Step 6 — Build and test each slide

```bash
cd slides/clock && bash build.sh
cd slides/quotes && bash build.sh
cd slides/affirmations && bash build.sh
cd slides/did_you_know && bash build.sh

# Verify each loads in the engine
cargo run -- --scene slides/clock
```

## Acceptance criteria

- [ ] Each slide compiles to `wasm32-wasip1` and produces `slide.wasm` + `manifest.json`
- [ ] Each slide loads and renders in the VZGLYD engine
- [ ] Clock displays current time (HH:MM) and updates each frame
- [ ] Quotes and affirmations display text with attribution and rotate on a timer
- [ ] Did-you-know displays a random fact from a random category
- [ ] `cargo test` passes for each slide crate
- [ ] No sidecar is required for any Tier 0 slide

## Files to create

| Directory | Files |
|-----------|-------|
| `slides/clock/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh` |
| `slides/quotes/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh` |
| `slides/affirmations/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh` |
| `slides/did_you_know/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`, `data/facts.csv` |
