# E10-T3: Sidecar Template Variant

| Field | Value |
|-------|-------|
| **Epic** | E10 Slide Scaffolding and Project Decoupling |
| **Priority** | P2 (medium) |
| **Estimate** | M |
| **Blocked by** | E10-T1 |
| **Blocks** | E10-T5 |

## Description

Extend the slide template from E10-T1 with an optional sidecar scaffold. When `with_sidecar = true`, `cargo-generate` includes a `sidecar/` subdirectory with a standalone Rust crate that compiles to `wasm32-wasip1`, connects to a configurable HTTPS endpoint, and pushes JSON payloads through the `vzglyd_host::channel_push` IPC mechanism. The sidecar template extracts and encapsulates the DNS, TLS, and HTTP plumbing currently embedded in `slides/terrain/sidecar/`.

## Background

The terrain sidecar (`slides/terrain/sidecar/`) is the only working example of the sidecar pattern. It contains ~1,200 lines of hand-written DNS-over-HTTPS, rustls TLS, chunked HTTP parsing, and WASI socket plumbing — all of which is reusable for any slide that polls a REST API. Today, a new slide author who wants live data must understand and copy this entire stack.

The goal is to make the common case trivial: "I want my slide to poll `https://api.example.com/data` every N seconds and receive JSON in `vzglyd_update`." The sidecar template provides the networking scaffold; the author only writes the URL and response parsing.

## Template additions when `with_sidecar = true`

```
{{project-name}}/
├── sidecar/
│   ├── Cargo.toml.liquid       # Standalone crate targeting wasm32-wasip1
│   ├── src/
│   │   ├── main.rs.liquid      # Poll loop: fetch → parse → channel_push
│   │   ├── http.rs             # Reusable HTTPS GET client (extracted from terrain sidecar)
│   │   ├── dns.rs              # DNS-over-HTTPS resolver (extracted from terrain sidecar)
│   │   └── tls.rs              # rustls TLS client for WASI sockets (extracted from terrain sidecar)
│   └── build.sh.liquid         # Build sidecar to wasm32-wasip1
└── build.sh.liquid             # Updated: builds sidecar first, then slide
```

## Step-by-step implementation

### Step 1 — Extract reusable networking modules from terrain sidecar

Read through `slides/terrain/sidecar/src/main.rs` and identify the generic networking components:

- **DNS resolution** (DoH via Google): `resolve_hostname` → `dns.rs`
- **TLS client** (rustls over WASI sockets): `tls_connect`, `tls_handshake` → `tls.rs`
- **HTTP GET** (chunked transfer, connection management): `https_get` → `http.rs`

These modules should be parameterised — accepting a hostname and path rather than hardcoding `api.coinbase.com`. The terrain sidecar's Coinbase-specific response parsing stays in terrain; only the transport layer is templated.

### Step 2 — Create sidecar Cargo.toml.liquid

```toml
[package]
name = "{{project-name}}_sidecar"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "{{project-name}}_sidecar"
path = "src/main.rs"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rustls = { version = "0.23", default-features = false, features = ["ring", "logging", "tls12"] }
webpki-roots = "0.26"
```

### Step 3 — Create sidecar main.rs.liquid

Skeleton poll loop:

```rust
// Template variables: {{api_host}}, {{api_path}}, {{poll_interval_secs}}
fn main() {
    loop {
        if !channel_active() {
            sleep_secs(1);
            continue;
        }
        match https_get("{{api_host}}", "{{api_path}}") {
            Ok(body) => {
                // TODO: Parse response and extract the data your slide needs
                let payload = body;
                channel_push(payload.as_bytes());
            }
            Err(e) => {
                // Log and retry on next interval
                eprintln!("fetch error: {e}");
            }
        }
        sleep_secs({{poll_interval_secs}});
    }
}
```

The `channel_active()`, `channel_push()`, and `sleep_secs()` functions use the `vzglyd_host` FFI — provide them as inline `extern "C"` declarations matching the existing ABI.

### Step 4 — Copy and parameterise networking modules

Take `dns.rs`, `tls.rs`, `http.rs` from the terrain sidecar extraction (Step 1). These are not templates (no `.liquid` extension) — they are static Rust source files included verbatim. They accept hostname/path as function arguments.

### Step 5 — Update the parent build.sh.liquid

When `with_sidecar` is true, the slide's `build.sh` must also compile the sidecar:

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

{% if with_sidecar %}
# Build sidecar
cargo build --manifest-path sidecar/Cargo.toml --target wasm32-wasip1 --release
cp sidecar/target/wasm32-wasip1/release/{{project-name}}_sidecar.wasm sidecar.wasm
{% endif %}

# Build slide
cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/{{project-name}}_slide.wasm slide.wasm
```

### Step 6 — Add template variables for sidecar config

Extend `cargo-generate.toml`:

```toml
[conditional.'with_sidecar == true'.placeholders.api_host]
type = "string"
prompt = "API hostname (e.g., api.example.com)?"
default = "api.example.com"

[conditional.'with_sidecar == true'.placeholders.api_path]
type = "string"
prompt = "API path (e.g., /v1/data)?"
default = "/v1/data"

[conditional.'with_sidecar == true'.placeholders.poll_interval_secs]
type = "string"
prompt = "Poll interval in seconds?"
default = "5"
```

### Step 7 — Test the sidecar template variant

```bash
cargo generate --path templates/slide --name test_sidecar \
  --define with_sidecar=true \
  --define api_host=api.coinbase.com \
  --define api_path=/v2/prices/BTC-USD/spot \
  --define poll_interval_secs=1

cd test_sidecar
bash build.sh
# Verify: slide.wasm, sidecar.wasm, manifest.json all present
```

## Acceptance criteria

- [ ] `cargo generate` with `with_sidecar=true` produces a sidecar/ subdirectory with compilable Rust source
- [ ] The sidecar builds to `wasm32-wasip1` and produces `sidecar.wasm` at the package root
- [ ] The sidecar template's `main.rs` uses `channel_push` / `channel_active` correctly against the existing ABI
- [ ] The networking modules (`dns.rs`, `tls.rs`, `http.rs`) are parameterised — no hardcoded hostnames
- [ ] `build.sh` with sidecar variant builds both slide and sidecar in the correct order
- [ ] `cargo generate` with `with_sidecar=false` does NOT include the sidecar/ directory
- [ ] Template round-trip test passes: generate → build → engine loads slide + sidecar successfully

## Files to create

| File | Purpose |
|------|---------|
| `templates/slide/sidecar/Cargo.toml.liquid` | Sidecar crate manifest |
| `templates/slide/sidecar/src/main.rs.liquid` | Poll loop skeleton |
| `templates/slide/sidecar/src/http.rs` | Reusable HTTPS GET client |
| `templates/slide/sidecar/src/dns.rs` | DNS-over-HTTPS resolver |
| `templates/slide/sidecar/src/tls.rs` | TLS client for WASI sockets |
| `templates/slide/sidecar/build.sh.liquid` | Sidecar build script |

## Files to modify

| File | Change |
|------|--------|
| `templates/slide/cargo-generate.toml` | Add conditional sidecar variables |
| `templates/slide/build.sh.liquid` | Add conditional sidecar build step |
