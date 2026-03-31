# E11-T6: Reminders Bridge (External Process Adapter)

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P3 (nice-to-have) |
| **Estimate** | M |
| **Blocked by** | E11-T1 |
| **Blocks** | - |

## Description

The iCloud Reminders integration is the hardest data collector to port to pure Rust — it requires Apple ID authentication with 2FA, session cookie persistence, and a custom protobuf decoder for reminder titles. Rather than fully rewriting this in Rust immediately, this ticket creates a bridge pattern: keep the Python reminders fetcher as an external process, have it write JSON to a well-known file, and add a VZGLYD sidecar that reads from that file via WASI filesystem preopens.

This establishes an "external data adapter" pattern that can be used for any data source where a pure-WASM sidecar is impractical (proprietary auth flows, complex protocol stacks, etc.).

## Why not a full Rust port?

The Python `reminders_getter.py` (407 lines) depends on:
- **`pyicloud`**: A reverse-engineered iCloud client library. No Rust equivalent exists.
- **Apple 2FA flow**: Interactive 6-digit code entry on first setup, session cookie persistence for subsequent runs.
- **Custom protobuf decoder**: Apple encodes reminder titles as gzip-compressed protobuf. The Python code does manual byte-scanning (not a standard protobuf library).

A full Rust port would mean:
1. Reimplementing the pyicloud CloudKit API client (~2,000 lines of Python)
2. Implementing Apple's 2FA flow (undocumented, changes periodically)
3. Porting the gzip+protobuf title decoder (~50 lines, this part is feasible)

The effort is disproportionate and the result would be fragile against Apple API changes. The bridge pattern gets us 90% of the value at 10% of the cost.

## Architecture

```
[Python: reminders_getter.py]
    │
    │  writes JSON every 15 minutes
    ▼
/tmp/vzglyd-reminders/reminders.json    (well-known path)
    │
    │  reads via WASI filesystem preopen
    ▼
[WASM sidecar: reminders_sidecar]
    │
    │  channel_push
    ▼
[WASM slide: reminders_slide]
```

The Python script is the existing dashboard fetcher, modified to write JSON instead of CSV (or kept as CSV with a trivial JSON wrapper). It runs as a systemd service or cron job — independent of the VZGLYD process.

The VZGLYD sidecar is minimal: poll the filesystem, read the JSON file, push to channel. No networking at all.

## Step-by-step implementation

### Step 1 — Modify the Python fetcher output

Fork `dashboard/plugins/reminders/tools/reminders_getter.py` into `tools/reminders_bridge.py` (lives outside the VZGLYD workspace, in a `tools/` or `bridges/` directory).

Change the output from CSV to JSON, written to a configurable path:

```python
output_path = os.environ.get("VZGLYD_REMINDERS_PATH", "/tmp/vzglyd-reminders/reminders.json")
```

Output format:
```json
{
  "fetched_at": "2026-03-29T10:30:00Z",
  "reminders": [
    {"title": "Buy groceries", "due": "2026-03-29", "priority": "normal", "list": "Shopping", "status": "pending"},
    ...
  ]
}
```

### Step 2 — Create the VZGLYD sidecar

The sidecar reads from WASI filesystem instead of making network requests:

```rust
use vzglyd_sidecar::{channel_push, channel_active, sleep_secs};
use std::fs;

fn main() {
    let path = std::env::var("REMINDERS_PATH")
        .unwrap_or_else(|_| "/data/reminders.json".to_string());

    let mut last_mtime = 0u64;
    loop {
        if !channel_active() {
            sleep_secs(1);
            continue;
        }

        // Check if file has been updated
        if let Ok(metadata) = fs::metadata(&path) {
            let mtime = metadata.modified()
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0);
            if mtime > last_mtime {
                if let Ok(data) = fs::read(&path) {
                    channel_push(&data);
                    last_mtime = mtime;
                }
            }
        }
        sleep_secs(5); // Poll file every 5 seconds
    }
}
```

The VZGLYD engine must preopen the directory containing the reminders file when loading this sidecar. This requires a manifest field or engine configuration for WASI filesystem preopens.

### Step 3 — WASI preopen configuration

Add a `wasi_preopens` field to the slide manifest (or sidecar manifest) that tells the engine which host directories to expose to the sidecar:

```json
{
  "name": "reminders",
  "sidecar": {
    "wasi_preopens": ["/tmp/vzglyd-reminders:/data"]
  }
}
```

This maps the host path `/tmp/vzglyd-reminders` to the guest path `/data`. The sidecar sees `/data/reminders.json`.

If this manifest extension is too invasive for one slide, alternatively use environment variables: the engine sets `REMINDERS_PATH=/path/to/file` and the sidecar reads it via `environ_get`.

### Step 4 — Reminders slide

Standard overlay slide: parse the JSON payload from `channel_poll`, render a list of reminders with title, due date, priority indicator, and list name. Color-code by priority (red = high, yellow = normal, green = low) or by overdue status.

### Step 5 — Systemd service for the Python bridge

Create a systemd service template:
```ini
[Unit]
Description=VZGLYD Reminders Bridge
After=network.target

[Service]
ExecStart=/usr/bin/python3 /path/to/reminders_bridge.py
Environment=VZGLYD_REMINDERS_PATH=/tmp/vzglyd-reminders/reminders.json
Environment=ICLOUD_EMAIL=...
Environment=ICLOUD_PASSWORD=...
Restart=on-failure
RestartSec=60

[Install]
WantedBy=multi-user.target
```

### Step 6 — Document the bridge pattern

Document this as a general-purpose pattern for slides that need data from sources too complex for a pure WASM sidecar. Any external process can write JSON to a well-known path, and a trivial "file watcher" sidecar bridges it into the VZGLYD channel.

## Future: full Rust port

If/when a Rust iCloud client library emerges, or if Apple provides a public CalDAV/REST API for Reminders, the bridge can be replaced with a direct sidecar. The slide WASM itself doesn't change — only the sidecar swaps from file-reader to API-fetcher.

## Acceptance criteria

- [ ] Python bridge script writes JSON to configurable path
- [ ] VZGLYD sidecar reads JSON via WASI filesystem and pushes to channel
- [ ] Slide displays reminder list with title, due date, priority, list
- [ ] File polling is efficient (check mtime before reading, 5s interval)
- [ ] Bridge pattern is documented as reusable for other external data sources
- [ ] Systemd service template exists for the Python bridge
- [ ] The slide works end-to-end: Python writes → sidecar reads → slide displays

## Files to create

| File | Purpose |
|------|---------|
| `bridges/reminders/reminders_bridge.py` | Modified Python fetcher |
| `bridges/reminders/reminders.service` | Systemd service template |
| `slides/reminders/` | Slide crate (Cargo.toml, src/lib.rs, manifest.json, build.sh) |
| `slides/reminders/sidecar/` | File-watcher sidecar (Cargo.toml, src/main.rs) |
