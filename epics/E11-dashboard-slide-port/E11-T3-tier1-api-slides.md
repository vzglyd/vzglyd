# E11-T3: Port Tier 1 Slides (Weather, AFL, Word-of-Day, On-This-Day, Air Quality, Last.fm)

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P1 (high) |
| **Estimate** | L |
| **Blocked by** | E11-T1 |
| **Blocks** | - |

## Description

Port the six dashboard slides that follow the simple "HTTP GET → parse JSON → push to channel" pattern. Each gets a thin sidecar (~50–80 lines) built on `vzglyd_sidecar`, plus a slide WASM that renders the received data. These slides collectively validate the `vzglyd_sidecar` API and establish the pattern that Tier 2 and Tier 3 slides build on.

## Slides

### Weather

**Source**: Bureau of Meteorology v1 API (public, no auth).
**Sidecar**: `https_get` to `/api/v1/forecasts/3-hourly` → parse JSON → extract day/condition/high/low → push as JSON payload.
**Poll interval**: 30 minutes (weather doesn't change fast).
**Slide**: Render 7-day forecast with day abbreviation, condition text, and high/low temps.
**Dashboard reference**: `plugins/weather/tools/weather_getter.py` (176 lines Python → ~80 lines Rust sidecar).

### AFL

**Source**: Squiggle API (public, no auth).
**Sidecar**: Two endpoints — ladder standings + upcoming games. `https_get` each, merge into a single JSON payload.
**Poll interval**: 1 hour.
**Slide**: Two sub-slides — ladder table (team/W/L/pts) and next round fixtures.
**Dashboard reference**: `plugins/afl/tools/afl_getter.py`.

### Word of the Day

**Source**: Dictionary/word API (public, no auth).
**Sidecar**: Fetch word + definition + example sentence. Push as JSON.
**Poll interval**: Once per day (check if cached word is still today's).
**Slide**: Display word, pronunciation, definition, example.
**Dashboard reference**: `plugins/word-of-day/tools/word_of_day_getter.py`.

### On This Day

**Source**: Wikipedia "On This Day" API (public, no auth).
**Sidecar**: Fetch historical events for today's date. Push top N as JSON.
**Poll interval**: Once per day.
**Slide**: Display date header + list of historical events with years.
**Dashboard reference**: `plugins/on-this-day/tools/on_this_day_getter.py`.

### Air Quality

**Source**: Air quality / pollen API (public, no auth).
**Sidecar**: Fetch current air quality index + pollen levels. Push as JSON.
**Poll interval**: 1 hour.
**Slide**: Display AQI value, category, and pollen breakdown.
**Dashboard reference**: `plugins/air-quality/tools/pollen_getter.py`.

### Last.fm

**Source**: Last.fm API (public, API key required in query params).
**Sidecar**: Fetch recent tracks for configured username. Push as JSON array of {song, artist, album, status, played_at}.
**Poll interval**: 30 seconds (near-realtime for "now playing").
**Auth**: API key passed as compile-time constant or read from sidecar config. No OAuth — just a query parameter `&api_key=KEY`.
**Slide**: Display track list with "Now Playing" indicator.
**Dashboard reference**: `plugins/lastfm/tools/lastfm_getter.py` (205 lines Python → ~60 lines Rust sidecar).

## Common sidecar structure

Every Tier 1 sidecar follows the same shape:

```rust
use vzglyd_sidecar::{https_get_text, poll_loop, Error};
use serde::Serialize;

#[derive(Serialize)]
struct Payload { /* slide-specific fields */ }

fn fetch() -> Result<Vec<u8>, Error> {
    let body = https_get_text("api.example.com", "/v1/data")?;
    let raw: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| Error::Io(e.to_string()))?;
    let payload = Payload {
        // extract fields from raw
    };
    serde_json::to_vec(&payload).map_err(|e| Error::Io(e.to_string()))
}

fn main() {
    poll_loop(INTERVAL_SECS, fetch);
}
```

## Step-by-step implementation

### Step 1 — Start with weather (simplest, validates vzglyd_sidecar end-to-end)

1. Create `slides/weather/` with slide + sidecar scaffold.
2. Implement sidecar: BOM API → JSON parse → payload.
3. Implement slide: Parse channel payload → render forecast rows.
4. Test end-to-end: sidecar fetches, slide displays.
5. Use weather as the reference for all other Tier 1 sidecars.

### Step 2 — Port remaining slides in parallel

Once the weather pattern is proven, the remaining five are mechanical:
- Copy the weather sidecar structure.
- Change the URL, response parser, and payload struct.
- Implement the slide renderer for each data shape.

### Step 3 — Handle Last.fm API key

Last.fm requires an API key. Options:
1. **Compile-time**: `const API_KEY: &str = env!("LASTFM_API_KEY");` — set at build time.
2. **Sidecar config file**: Read a `config.json` from the sidecar's WASI filesystem preopens.
3. **Host environment**: Read `LASTFM_API_KEY` from WASI `environ_get`.

Recommend option 3 (WASI `environ_get`) — it matches the dashboard's approach and doesn't require rebuild for credential changes. `vzglyd_sidecar` can provide a helper: `vzglyd_sidecar::env_var(name) -> Option<String>`.

### Step 4 — Budget and Chore (no sidecar)

Budget and chore read from local YAML files in the dashboard. In VZGLYD:
- **Budget**: Embed budget data as a compiled-in asset (e.g., `include_str!` of a YAML or JSON file in the slide package). No sidecar — the data is local and changes infrequently. To update, rebuild the slide.
- **Chore**: Same approach — embed chore list, use WASI random to select one each hour.

These are effectively Tier 0 (no networking) but grouped here because they originate from the dashboard's cron job plugins.

### Step 5 — Test all slides

Each slide must:
- Compile to `wasm32-wasip1`
- Produce `slide.wasm`, `sidecar.wasm` (where applicable), and `manifest.json`
- Load in the VZGLYD engine
- Display data after sidecar fetches
- Handle sidecar fetch failures gracefully (display "Loading..." or last-known data)

## Acceptance criteria

- [ ] All six sidecar slides compile and produce `slide.wasm` + `sidecar.wasm`
- [ ] Each sidecar uses `vzglyd_sidecar` — no inline DNS/TLS/HTTP code
- [ ] Weather sidecar fetches from BOM API and slide displays 7-day forecast
- [ ] AFL sidecar fetches ladder + games and slide displays both views
- [ ] Last.fm sidecar authenticates with API key via WASI env var
- [ ] All sidecars handle API errors gracefully (backoff, no crash)
- [ ] Budget and chore slides work without sidecars (embedded data)
- [ ] `cargo test` passes for each slide and sidecar crate

## Files to create (per slide)

| Slide | Files |
|-------|-------|
| `slides/weather/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`, `sidecar/{Cargo.toml, src/main.rs}` |
| `slides/afl/` | Same structure |
| `slides/word_of_day/` | Same structure |
| `slides/on_this_day/` | Same structure |
| `slides/air_quality/` | Same structure |
| `slides/lastfm/` | Same structure |
| `slides/budget/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`, `data/budget.json` |
| `slides/chore/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`, `data/chores.json` |
