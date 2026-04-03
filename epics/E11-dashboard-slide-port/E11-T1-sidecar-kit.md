# E11-T1: Extract `vzglyd_sidecar` Networking Crate

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P1 (high) |
| **Estimate** | L |
| **Blocked by** | - |
| **Blocks** | E11-T3, E11-T4, E11-T5, E11-T6 |

## Description

Extract the DNS-over-HTTPS resolver, TLS client, and HTTP client from `slides/terrain/sidecar/src/main.rs` into a reusable library crate (`vzglyd_sidecar/`) that any sidecar can depend on. Refactor the terrain sidecar to use `vzglyd_sidecar` as validation. The crate targets `wasm32-wasip1` and provides a minimal, ergonomic API for the common sidecar pattern: poll an HTTPS endpoint on an interval, parse the response, push data to the slide.

## Background

The terrain sidecar contains ~1,200 lines of hand-written networking:
- DNS-over-HTTPS via Google DoH (with CNAME traversal, TTL caching)
- TLS 1.2/1.3 via rustls over raw WASI sockets
- HTTP/1.1 GET with chunked transfer-encoding support
- WASI socket FFI (`sock_open`, `sock_connect`, `sock_send`, `sock_recv`)
- `vzglyd_host` FFI bindings (`channel_push`, `channel_poll`, `channel_active`)

This code is correct and battle-tested but monolithic. Extracting it into a library enables the 10+ new sidecars planned in E11 without copy-pasting.

## Crate API surface

```rust
// vzglyd_sidecar/src/lib.rs

/// Perform an HTTPS GET request. Returns the response body.
pub fn https_get(host: &str, path: &str) -> Result<Vec<u8>, Error>;

/// HTTPS GET with conditional request headers (ETag / Last-Modified).
/// Returns (body, new_etag). Body is empty if server returns 304 Not Modified.
pub fn https_get_conditional(
    host: &str,
    path: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<(Vec<u8>, Option<String>, Option<String>), Error>;

/// HTTPS GET returning the response as a UTF-8 string.
pub fn https_get_text(host: &str, path: &str) -> Result<String, Error>;

/// Push a byte payload to the slide via the host channel.
pub fn channel_push(data: &[u8]);

/// Check if the parent slide is currently visible.
pub fn channel_active() -> bool;

/// Sleep for the given number of seconds using WASI clocks.
pub fn sleep_secs(secs: u32);

/// Run a poll loop: calls `fetch` every `interval_secs`, skips when slide
/// is not visible, retries with backoff on error (max 60s).
pub fn poll_loop<F>(interval_secs: u32, fetch: F) -> !
where
    F: FnMut() -> Result<Vec<u8>, Error>;

/// Error type covering DNS, TLS, HTTP, and I/O failures.
pub enum Error {
    Dns(String),
    Tls(String),
    Http { status: u16, body: String },
    Io(String),
    Timeout,
}
```

## Internal modules

```
vzglyd_sidecar/
├── Cargo.toml
└── src/
    ├── lib.rs          # Public API re-exports
    ├── dns.rs          # DoH resolver (extracted from terrain sidecar)
    ├── tls.rs          # rustls client over WASI sockets
    ├── http.rs         # HTTP/1.1 GET with chunked transfer-encoding
    ├── socket.rs       # WASI socket FFI bindings (sock_open, etc.)
    ├── channel.rs      # vzglyd_host FFI bindings (channel_push, etc.)
    └── poll.rs         # poll_loop harness with backoff
```

## Step-by-step implementation

### Step 1 — Create the crate skeleton

```toml
# vzglyd_sidecar/Cargo.toml
[package]
name = "vzglyd_sidecar"
version = "0.1.0"
edition = "2024"

[dependencies]
rustls = { version = "0.23", default-features = false, features = ["ring", "logging", "tls12"] }
webpki-roots = "0.26"
```

Add to workspace members in root `Cargo.toml`. This crate does NOT need `VRX-64-slide` — it is independent.

### Step 2 — Extract `socket.rs`

Move the WASI socket FFI declarations (`sock_open`, `sock_connect`, `sock_send`, `sock_recv`, `sock_shutdown`) from the terrain sidecar. These are `extern "C"` functions linked from `wasi_snapshot_preview1`. Keep them as an internal module — not part of the public API.

### Step 3 — Extract `dns.rs`

Move the DoH resolver. Parameterise the DoH endpoint (default: `dns.google`). Keep the TTL cache as a thread-local (each sidecar WASM instance has its own memory). The resolver should expose:

```rust
pub(crate) fn resolve(hostname: &str) -> Result<std::net::Ipv4Addr, Error>;
```

### Step 4 — Extract `tls.rs`

Move the rustls client setup. Expose:

```rust
pub(crate) fn tls_connect(host: &str, port: u16) -> Result<TlsStream, Error>;
```

Where `TlsStream` wraps a WASI socket + rustls `ClientConnection`.

### Step 5 — Extract `http.rs`

Move the HTTP/1.1 GET implementation. This handles:
- Request formatting with Host header
- Response status code parsing
- Content-Length and chunked transfer-encoding
- Optional ETag/Last-Modified conditional headers

Expose via the public `https_get` / `https_get_conditional` functions.

### Step 6 — Implement `channel.rs`

Thin wrappers around `vzglyd_host` FFI:

```rust
extern "C" {
    #[link_name = "channel_push"]
    fn host_channel_push(ptr: *const u8, len: u32);
    #[link_name = "channel_active"]
    fn host_channel_active() -> i32;
}

pub fn channel_push(data: &[u8]) {
    unsafe { host_channel_push(data.as_ptr(), data.len() as u32) }
}

pub fn channel_active() -> bool {
    unsafe { host_channel_active() != 0 }
}
```

### Step 7 — Implement `poll.rs`

The `poll_loop` harness:

```rust
pub fn poll_loop<F>(interval_secs: u32, mut fetch: F) -> !
where
    F: FnMut() -> Result<Vec<u8>, Error>,
{
    let mut backoff = interval_secs;
    loop {
        if !channel_active() {
            sleep_secs(1);
            continue;
        }
        match fetch() {
            Ok(payload) => {
                channel_push(&payload);
                backoff = interval_secs; // reset on success
                sleep_secs(interval_secs);
            }
            Err(_) => {
                sleep_secs(backoff);
                backoff = (backoff * 2).min(60); // exponential backoff, cap 60s
            }
        }
    }
}
```

### Step 8 — Refactor terrain sidecar to use `vzglyd_sidecar`

Replace the terrain sidecar's inline networking with `vzglyd_sidecar` calls:

```rust
// slides/terrain/sidecar/src/main.rs (after refactor)
use vzglyd_sidecar::{https_get_text, poll_loop};

fn main() {
    poll_loop(1, || {
        let body = https_get_text("api.coinbase.com", "/v2/prices/BTC-USD/spot")?;
        Ok(body.into_bytes())
    });
}
```

The terrain sidecar should shrink from ~1,200 lines to ~20 lines.

### Step 9 — Tests

- Unit tests for DNS response parsing (mock DoH JSON responses)
- Unit tests for HTTP response parsing (chunked encoding, Content-Length, 304 responses)
- Integration test: build `vzglyd_sidecar` to `wasm32-wasip1` and verify it links
- Regression: terrain sidecar still works end-to-end after refactor

## Acceptance criteria

- [ ] `vzglyd_sidecar` crate exists and compiles to `wasm32-wasip1`
- [ ] `https_get` and `https_get_text` successfully fetch from a public HTTPS endpoint
- [ ] `https_get_conditional` correctly handles ETag/Last-Modified and 304 responses
- [ ] `poll_loop` implements visibility check, backoff, and push
- [ ] Terrain sidecar is refactored to use `vzglyd_sidecar` and still works at runtime
- [ ] Terrain sidecar `main.rs` is under 50 lines after refactor
- [ ] `cargo test -p vzglyd_sidecar` passes
- [ ] `cargo build -p vzglyd_sidecar --target wasm32-wasip1` succeeds

## Files to create

| File | Purpose |
|------|---------|
| `vzglyd_sidecar/Cargo.toml` | Crate manifest |
| `vzglyd_sidecar/src/lib.rs` | Public API |
| `vzglyd_sidecar/src/dns.rs` | DoH resolver |
| `vzglyd_sidecar/src/tls.rs` | TLS client |
| `vzglyd_sidecar/src/http.rs` | HTTP/1.1 client |
| `vzglyd_sidecar/src/socket.rs` | WASI socket FFI |
| `vzglyd_sidecar/src/channel.rs` | vzglyd_host channel FFI |
| `vzglyd_sidecar/src/poll.rs` | Poll loop harness |

## Files to modify

| File | Change |
|------|--------|
| `Cargo.toml` | Add `vzglyd_sidecar` to workspace members |
| `slides/terrain/sidecar/Cargo.toml` | Add `vzglyd_sidecar` dependency |
| `slides/terrain/sidecar/src/main.rs` | Replace inline networking with `vzglyd_sidecar` calls |
