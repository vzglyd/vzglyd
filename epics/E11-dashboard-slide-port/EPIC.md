# E11: Dashboard Slide Port

## Summary

The `../dashboard` repository contains 19 plugins producing ~30 slides for a C++/SDL2 KMSDRM display engine. Data collection is handled by Python scripts (cron jobs and daemons) that scrape APIs, parse feeds, and write CSVs consumed by the renderer. This epic ports those slides into VZGLYD's WASM slide + sidecar architecture, replacing Python data collectors with Rust sidecars that use the existing `channel_push`/`channel_poll` IPC.

## Problem

The dashboard system works, but it's a two-service split (Python procman + C++ renderer) with file-based IPC (CSV + inotify), a runtime dependency bootstrap (`lib/bootstrap.py` auto-installs pip packages), and no sandboxing. Each plugin's Python fetcher runs with full host access. VZGLYD's model — sandboxed WASM slides with WASI-constrained sidecars — is more portable, more secure, and eliminates the Python runtime dependency entirely.

The data collection scripts share significant overlap: 12 of 19 plugins do some variant of "HTTP GET → parse JSON/XML → extract fields → push to display." Today each reimplements HTTP client setup, error handling, atomic file writes, and retry logic. A shared sidecar networking crate would eliminate this duplication.

## Strategy

### Shared sidecar networking crate: `vzglyd_sidecar`

Extract the terrain sidecar's DNS-over-HTTPS, TLS (rustls), and HTTP client into a reusable library crate at `VRX-64-slide/vzglyd_sidecar/` (or `vzglyd_sidecar/` at workspace root). This crate provides:

- `https_get(host, path, headers) -> Result<Vec<u8>, Error>` — single-request HTTPS client
- `https_get_with_etag(host, path, etag) -> Result<(Vec<u8>, Option<String>), Error>` — conditional GET with ETag/Last-Modified for RSS feeds
- `channel_push(payload: &[u8])` / `channel_poll(buf: &mut [u8]) -> i32` — re-exported FFI bindings
- `channel_active() -> bool` — visibility check
- `sleep_secs(n: u32)` — WASI clock-based sleep
- `PollLoop` — a simple loop harness: `poll_loop(interval_secs, fetch_fn)` that handles sleep, visibility checks, and error retry with backoff

Every sidecar depends on `vzglyd_sidecar` instead of copy-pasting the 800-line DNS/TLS/HTTP stack from the terrain sidecar. The terrain sidecar itself is refactored to use `vzglyd_sidecar`.

**Why a library crate, not a generic configurable binary:** The simple cases (weather, lastfm, word-of-day) could theoretically share one binary parameterised by a config file. But parsing logic is always custom — BOM returns nested `{ daily_forecasts: [{ temp_max, temp_min, ... }] }` while Last.fm returns `{ recenttracks: { track: [...] } }`. A shared binary either embeds every parser or requires a scripting layer. A shared library keeps each sidecar as a thin ~50-line `main.rs` that calls `vzglyd_sidecar::https_get()` and writes its own 10-line parser. This is simpler, more debuggable, and compiles to smaller WASM.

### Slide porting tiers

**Tier 0 — No sidecar (static or computed in `vzglyd_update`):**

| Dashboard plugin | VZGLYD slide | Notes |
|-----------------|------------|-------|
| clock | `slides/clock/` | System time via WASI clock in `vzglyd_update` — no sidecar needed |
| quotes | `slides/quotes/` | Inline data embedded in `SlideSpec` or WASM binary |
| affirmations | `slides/affirmations/` | Same as quotes — inline strings |
| did-you-know | `slides/did_you_know/` | Embed the CSV fact database at compile time — it's static data, not fetched |

These are trivially portable. Each is a single-file slide with no networking.

**Tier 1 — Simple API poll (sidecar using `vzglyd_sidecar::https_get`):**

| Dashboard plugin | VZGLYD slide | API | Auth | Complexity |
|-----------------|------------|-----|------|------------|
| weather | `slides/weather/` | BOM v1 (public) | None | Very low |
| afl | `slides/afl/` | Squiggle API (public) | None | Low |
| word-of-day | `slides/word_of_day/` | Dictionary API (public) | None | Very low |
| on-this-day | `slides/on_this_day/` | Wikipedia API (public) | None | Very low |
| air-quality | `slides/air_quality/` | Pollen API (public) | None | Low |
| lastfm | `slides/lastfm/` | Last.fm (API key in query param) | API key | Low |
| budget | `slides/budget/` | None — static YAML compiled in | None | Very low |
| chore | `slides/chore/` | None — static YAML + RNG | None | Very low |

Each sidecar is ~50–80 lines: call `https_get`, parse JSON with `serde_json`, serialise the display payload, `channel_push`. Budget and chore don't need sidecars at all — embed the data or read from a bundled asset file.

**Tier 2 — Moderate complexity (custom sidecar with `vzglyd_sidecar` networking):**

| Dashboard plugin | VZGLYD slide | Challenge |
|-----------------|------------|-----------|
| calendar | `slides/calendar/` | ICS parsing (use `ical` Rust crate), timezone handling |
| servers | `slides/servers/` | Multiple concurrent HTTP + TCP checks, 24h uptime history |
| lastfm (daemon mode) | (upgrade from Tier 1) | 30s poll loop, "now playing" detection |

Calendar requires an iCalendar parser but the Rust `ical` crate handles the heavy lifting. Servers needs sequential HTTP and TCP checks (WASM is single-threaded, but health checks can run sequentially in a loop — 16 servers at 5s timeout worst-case is 80s, acceptable for a 30s check interval if most respond fast).

**Tier 3 — High complexity (custom sidecar, significant effort):**

| Dashboard plugin | VZGLYD slide | Challenge |
|-----------------|------------|-----------|
| news | `slides/news/` | 3 source types: SSE (Firebase), Reddit JSON pagination, RSS/Atom XML. Headline shortening. |
| reminders | `slides/reminders/` | iCloud CloudKit auth with 2FA, custom protobuf title decoder |

News is the hardest port. The Python version uses threads for concurrent source polling, SSE streaming for HackerNews, and an LLM subprocess for headline shortening. In WASM:
- SSE can be implemented as repeated HTTP GETs to the Firebase REST API with `.json` suffix (polling instead of streaming — simpler, good enough at 30s intervals)
- Reddit and RSS are standard HTTP GET + parse
- Run sources sequentially in a loop (no threads in WASM, but the sidecar has its own host thread)
- Headline shortening: either truncate with ellipsis (simpler) or add a second sidecar that talks to a local LLM endpoint

Reminders has the hardest auth story. iCloud CloudKit requires Apple ID + 2FA, session cookies, and a custom protobuf decoder. Options:
1. **Port directly**: Reimplement the CloudKit auth flow in Rust (2–3 weeks, fragile against Apple API changes)
2. **Defer**: Keep the Python reminders fetcher as an external process that writes to a well-known file, and have the VZGLYD sidecar read from that file
3. **Replace**: Use a different reminders backend (CalDAV, Todoist API, etc.)

Recommendation: defer reminders to a later phase and use option 2 as a bridge.

### What about the process manager (procman)?

VZGLYD does not need procman. In the dashboard, procman orchestrates job scheduling, credential storage, and dashboard.json generation. In VZGLYD:
- **Job scheduling**: Each sidecar runs its own poll loop — the VZGLYD engine manages sidecar lifecycle (start when slide loads, stop when unloaded)
- **Credential storage**: Sidecar config files or environment variables at launch time
- **Slide manifest**: Already handled by `manifest.json` per slide package

The procman web UI for entering credentials is useful but orthogonal. It could become a standalone tool or a future VZGLYD feature.

### Rendering differences

The dashboard renders at 256×224 (NES resolution) with CP437 bitmap fonts and OpenGL ES 2.0 shaders. VZGLYD slides render into `SlideSpec` geometry at arbitrary resolution. Each ported slide needs a new visual design — the data model ports directly, but the pixel-art aesthetics are specific to the dashboard's retro theme.

Background shaders (starfield, rain, embers, etc.) from the dashboard can inform VZGLYD shader authoring but aren't directly portable (different shader interface contract).

## Prerequisites

- E10-T1 (slide template) makes scaffolding fast
- E10-T2 (build decoupling) keeps new slides from blocking the engine
- E10-T3 (sidecar template) provides the starting point for Tier 1–3 sidecars

## Tickets

| ID | Title | Priority | Size | Blocked by | Blocks |
|----|-------|----------|------|------------|--------|
| E11-T1 | Extract `vzglyd_sidecar` networking crate | P1 | L | - | E11-T2, E11-T3, E11-T4, E11-T5 |
| E11-T2 | Port Tier 0 slides (clock, quotes, affirmations, did-you-know) | P2 | S | - | - |
| E11-T3 | Port Tier 1 slides (weather, afl, word-of-day, on-this-day, air-quality, lastfm) | P1 | L | E11-T1 | - |
| E11-T4 | Port Tier 2 slides (calendar, servers) | P2 | L | E11-T1 | - |
| E11-T5 | Port Tier 3 slides (news) | P2 | XL | E11-T1 | - |
| E11-T6 | Reminders bridge (external process adapter) | P3 | M | E11-T1 | - |
