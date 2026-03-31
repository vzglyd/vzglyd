# E11-T4: Port Tier 2 Slides (Calendar, Servers)

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P2 (medium) |
| **Estimate** | L |
| **Blocked by** | E11-T1 |
| **Blocks** | - |

## Description

Port the calendar and server status slides. These are more complex than Tier 1: calendar requires iCalendar format parsing and timezone handling; servers requires both HTTP and raw TCP health checks with uptime history tracking. Both use `vzglyd_sidecar` for networking but add domain-specific parsing on top.

## Slides

### Calendar

**Dashboard**: `plugins/calendar/tools/calendar_getter.py` (200 lines). Fetches a Google Calendar private ICS URL, parses VEVENT components, extracts upcoming events for the next 7 days.

**VZGLYD sidecar approach**:

The sidecar fetches the ICS URL via `vzglyd_sidecar::https_get_text`, then parses the iCalendar format. For ICS parsing in Rust:

- **Option A**: Use the `ical` crate (mature, handles VEVENT/VTIMEZONE/recurrence). This is the recommended approach — it handles the RFC 5545 complexity that would take weeks to reimplement.
- **Option B**: Minimal hand-written parser (only extract DTSTART, SUMMARY, ATTENDEE from VEVENT blocks). Simpler but misses recurring events, timezone rules, etc.

Recommend Option A. The `ical` crate compiles to WASM without issue (pure Rust, no system dependencies).

**Key parsing logic to port**:
- Filter events by DTSTART falling within next 7 days
- Handle both date-only (all-day) and datetime (timed) events
- Timezone conversion: DTSTART may include TZID parameter or be UTC (suffix Z)
- Attendee count: ATTENDEE property may appear 0, 1, or N times per event
- Event type inference: keyword regex on SUMMARY (standup, retro, 1:1, interview, etc.)

**Auth**: None — Google Calendar private ICS URLs embed an auth token in the URL itself. The URL is passed as an env var (`GCAL_ICS_URL`), read via WASI `environ_get`.

**Poll interval**: 15 minutes.

**Slide**: Two views (matching dashboard):
1. **Calendar**: Week view with events grouped by day
2. **Meetings**: List of upcoming meetings with type, time, attendee count

### Servers

**Dashboard**: `plugins/servers/tools/server_checker.py` (333 lines). Reads `servers.yaml`, performs concurrent HTTP HEAD and TCP connect checks, tracks 24h rolling uptime history in a JSON sidecar file.

**VZGLYD sidecar approach**:

The sidecar runs sequential health checks (WASM is single-threaded). For N servers at 5s timeout each, worst case is N×5s. With 10 servers, that's 50s — acceptable for a 60s check interval if most respond in <1s.

**Health check types**:
1. **HTTP**: `vzglyd_sidecar::https_get` with the server's URL. Check status code < 400. Measure response time.
2. **TCP**: Raw WASI socket connect to host:port. Measure connection time. No TLS needed (just checking if the port is reachable).

`vzglyd_sidecar` already provides HTTPS; for TCP-only checks, expose a lower-level `vzglyd_sidecar::tcp_connect(host, port, timeout_ms) -> Result<Duration, Error>` function (this may need to be added to `vzglyd_sidecar` as part of E11-T1 or as a follow-up).

**Uptime history**: The dashboard tracks a rolling 24h window of `[timestamp, ok]` pairs in a JSON file. In VZGLYD, the sidecar can maintain this in-memory (it's a long-running background thread). On each check cycle:
1. Append `(now, ok)` to an in-memory ring buffer
2. Prune entries older than 24h
3. Calculate uptime % = successful / total
4. Push server status array to channel

**Server list configuration**: The dashboard reads `servers.yaml`. In VZGLYD, the server list can be:
- Embedded at compile time (`include_str!("../config/servers.json")`)
- Read from a bundled asset file in the slide package
- Read from WASI filesystem preopens (if the engine preopens a config directory)

Recommend: bundled JSON file in the slide package (`servers.json` in the manifest's assets).

**Poll interval**: 30 seconds (matching dashboard).

**Slide**: Two views (matching dashboard):
1. **Server Status**: Table with name, status icon (healthy/warning/down), uptime %, region
2. **Job Monitor**: (This was dashboard-specific — procman job status. Skip or replace with a VZGLYD-relevant monitoring view.)

## Step-by-step implementation

### Step 1 — Calendar sidecar

1. Create `slides/calendar/sidecar/` with Cargo.toml depending on `vzglyd_sidecar` + `ical` + `chrono`.
2. Implement ICS fetch: `vzglyd_sidecar::https_get_text(host, path)` using the ICS URL parsed into host + path.
3. Parse with `ical` crate: iterate VEVENT components, filter by date range.
4. Extract: summary, dtstart, attendees, infer type.
5. Serialize as JSON array and `channel_push`.
6. Test with a real Google Calendar ICS URL.

### Step 2 — Calendar slide

1. Create `slides/calendar/` slide crate.
2. In `vzglyd_update`: `channel_poll` for new event data. Deserialize JSON.
3. Render: day headers + event list with time, title, type badge.
4. Handle "no data yet" state (display "Loading..." until first fetch).

### Step 3 — Servers sidecar

1. Create `slides/servers/sidecar/` with Cargo.toml depending on `vzglyd_sidecar` + `serde_json`.
2. Parse server list from embedded config or WASI filesystem.
3. For each server, sequentially:
   - HTTP check: `vzglyd_sidecar::https_get` or `tcp_connect` depending on check type.
   - Record response time and success/failure.
   - Append to in-memory history ring buffer.
4. Calculate uptime % for each server.
5. Serialize status array as JSON and `channel_push`.

### Step 4 — Servers slide

1. Create `slides/servers/` slide crate.
2. Render: table rows with name, status indicator (color-coded), uptime %, response time.
3. Status thresholds: healthy (>99%), warning (>95%), degraded (>90%), down (<90%).

### Step 5 — Add TCP connect to vzglyd_sidecar (if needed)

If the servers sidecar needs raw TCP connect (not HTTPS), add:
```rust
pub fn tcp_connect(host: &str, port: u16, timeout_ms: u32) -> Result<std::time::Duration, Error>;
```
This uses the WASI socket layer directly without TLS. May be added in E11-T1 or here.

## Acceptance criteria

- [ ] Calendar sidecar fetches ICS URL, parses events, pushes to channel
- [ ] Calendar slide displays upcoming events grouped by day
- [ ] Timezone handling works correctly (events display in local time)
- [ ] Attendee count and event type inference match dashboard behavior
- [ ] Servers sidecar performs sequential HTTP + TCP health checks
- [ ] Servers sidecar tracks 24h rolling uptime in memory
- [ ] Servers slide displays status table with uptime percentages
- [ ] Both slides handle sidecar errors gracefully (display last-known data or loading state)
- [ ] `cargo test` passes for each slide and sidecar crate

## Files to create

| Directory | Files |
|-----------|-------|
| `slides/calendar/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh` |
| `slides/calendar/sidecar/` | `Cargo.toml`, `src/main.rs` |
| `slides/servers/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh`, `config/servers.json` |
| `slides/servers/sidecar/` | `Cargo.toml`, `src/main.rs` |
