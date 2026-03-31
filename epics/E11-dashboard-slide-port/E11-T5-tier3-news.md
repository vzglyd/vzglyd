# E11-T5: Port Tier 3 — News Slide

| Field | Value |
|-------|-------|
| **Epic** | E11 Dashboard Slide Port |
| **Priority** | P2 (medium) |
| **Estimate** | XL |
| **Blocked by** | E11-T1 |
| **Blocks** | - |

## Description

Port the news slide — the most complex data collector in the dashboard. The Python version (`news_getter.py`, 512 lines) aggregates three source types in parallel threads: Firebase SSE for HackerNews, JSON API with cursor pagination for Reddit, and RSS/Atom XML feeds for traditional news. It also integrates an LLM subprocess for headline shortening. This ticket designs a practical WASM sidecar that achieves equivalent functionality within the single-threaded WASI constraint.

## Architecture decision: polling over streaming

The dashboard's HackerNews source uses Server-Sent Events (Firebase SSE) for real-time push. In a WASM sidecar, SSE is possible but complex — it requires holding a TCP connection open and parsing an unbounded stream of `data:` lines. Since the sidecar already runs in a background thread and the slide only updates every few seconds, **polling the Firebase REST API** at 30-second intervals achieves equivalent freshness with far less complexity.

Firebase REST API: `GET https://hacker-news.firebaseio.com/v0/topstories.json` returns an array of story IDs. Fetch the top N story details individually: `GET /v0/item/{id}.json`. This is the same data as the SSE stream, just pulled instead of pushed.

## Source types in the sidecar

### 1. HackerNews (Firebase REST)

```
GET /v0/topstories.json → [id1, id2, ...]
GET /v0/item/{id}.json → { title, url, score, ... }
```

- Fetch top 30 story IDs, then fetch each story's detail.
- Sequential HTTP requests (30 × ~100ms = ~3s total — acceptable).
- No auth required.
- Extract: title, URL, score, time.

### 2. Reddit (JSON API)

```
GET /r/{subreddit}/new.json?limit=25 → { data: { children: [...] } }
```

- Fetch newest posts from configured subreddits (e.g., `r/technology`, `r/worldnews`).
- No auth required for public subreddits (Reddit JSON API is public with User-Agent).
- Cursor-based pagination: track `after` token to avoid re-fetching.
- Extract: title, subreddit, score, created_utc.

### 3. RSS/Atom Feeds

```
GET /feed.xml → <rss> or <feed> XML
```

- Fetch configured RSS/Atom feed URLs.
- Use `vzglyd_sidecar::https_get_conditional` with ETag/Last-Modified to avoid re-parsing unchanged feeds.
- XML parsing: Use `quick-xml` crate (compiles to WASM, no system dependencies).
- Handle both RSS 2.0 (`<item><title>`) and Atom 1.0 (`<entry><title>`) formats.
- Date parsing: RFC 2822 (RSS) and ISO 8601 (Atom).

## Headline shortening

The dashboard uses Ollama (local LLM) to shorten long headlines. Options for VZGLYD:

1. **Simple truncation with ellipsis**: `headline[..MAX_LEN] + "…"` — zero external dependencies.
2. **Word-boundary truncation**: Break at the last space before MAX_LEN — slightly better readability.
3. **LLM sidecar** (future): A separate `ollama_sidecar` that other sidecars can query via a shared channel. This is out of scope for this ticket.

Recommend option 2 for initial port. If headline quality matters, add LLM integration later as a separate ticket.

## Sidecar design

```rust
// slides/news/sidecar/src/main.rs
use vzglyd_sidecar::{https_get_text, https_get_conditional, poll_loop, Error};

mod hackernews;   // Firebase REST client
mod reddit;       // Reddit JSON client
mod rss;          // RSS/Atom parser (uses quick-xml)

#[derive(serde::Serialize)]
struct Headline {
    title: String,
    source: String,     // "hackernews", "reddit", "abc", etc.
    category: String,   // "tech", "world", "general"
    timestamp: i64,     // unix seconds
}

fn fetch() -> Result<Vec<u8>, Error> {
    let mut headlines: Vec<Headline> = Vec::new();

    // Fetch each source sequentially
    headlines.extend(hackernews::fetch_top(10)?);
    headlines.extend(reddit::fetch_new("technology", 10)?);
    headlines.extend(reddit::fetch_new("worldnews", 10)?);
    headlines.extend(rss::fetch_feed("https://abc.net.au/news/feed/rss", "abc", 10)?);

    // Sort by timestamp descending
    headlines.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // Truncate headlines to display width
    for h in &mut headlines {
        if h.title.len() > 60 {
            let truncated = h.title[..60].rfind(' ').unwrap_or(57);
            h.title.truncate(truncated);
            h.title.push('…');
        }
    }

    serde_json::to_vec(&headlines).map_err(|e| Error::Io(e.to_string()))
}

fn main() {
    poll_loop(30, fetch);
}
```

Total sidecar is likely ~200–300 lines across 4 files (main + 3 source modules).

## RSS/Atom parser module

The RSS module needs to handle:
1. **RSS 2.0**: `<rss><channel><item><title>`, `<pubDate>` (RFC 2822)
2. **Atom 1.0**: `<feed><entry><title>`, `<updated>` or `<published>` (ISO 8601)
3. **Namespace handling**: Atom may use default namespace `xmlns="http://www.w3.org/2005/Atom"`
4. **Conditional GET**: Pass ETag/Last-Modified from previous fetch to avoid re-downloading unchanged feeds

Using `quick-xml` (event-based XML parser):

```rust
pub fn parse_rss(xml: &str) -> Vec<Headline> { /* ... */ }
pub fn parse_atom(xml: &str) -> Vec<Headline> { /* ... */ }
pub fn parse_feed(xml: &str) -> Vec<Headline> {
    // Detect format from root element, dispatch to parser
}
```

## Slide design

Three sub-views (matching dashboard), rotated by the engine's slide duration:
1. **Tech News**: HackerNews + Reddit r/technology headlines
2. **World News**: Reddit r/worldnews headlines
3. **ABC News**: RSS feed headlines

Or: a single slide that shows all headlines with source icons/labels, paginated by the overlay.

The slide receives the full headline array from `channel_poll`, filters by category for each view, and renders as a scrolling text list.

## Step-by-step implementation

### Step 1 — HackerNews source module

- Implement `hackernews::fetch_top(n)` using Firebase REST API.
- Test with `cargo test` (mock JSON responses).

### Step 2 — Reddit source module

- Implement `reddit::fetch_new(subreddit, n)` using Reddit JSON API.
- Handle cursor tracking (optional for initial port — can always fetch newest).
- Test with mock responses.

### Step 3 — RSS/Atom parser module

- Add `quick-xml` dependency.
- Implement dual-format parser (RSS 2.0 + Atom 1.0).
- Test against real feed samples from ABC News, BBC, etc.
- Date parsing: handle both RFC 2822 and ISO 8601 (chrono crate).

### Step 4 — Assemble sidecar

- Wire the three modules into `fetch()`.
- Implement headline truncation.
- Test end-to-end with `poll_loop`.

### Step 5 — News slide WASM

- Implement slide renderer: headline list with source label, timestamp.
- Handle "loading" state.
- Test in engine.

## Acceptance criteria

- [ ] Sidecar fetches from all three source types (HackerNews, Reddit, RSS)
- [ ] RSS parser handles both RSS 2.0 and Atom 1.0 format
- [ ] Conditional GET (ETag/Last-Modified) works for RSS feeds
- [ ] Headlines are truncated at word boundaries to fit display width
- [ ] Slide displays headlines sorted by recency with source labels
- [ ] Sidecar handles individual source failures without crashing (skip failed source, continue others)
- [ ] `cargo test` passes for sidecar (with mock HTTP responses)
- [ ] Full round-trip works: sidecar fetches → channel_push → slide polls → renders

## Dependencies

| Crate | Purpose |
|-------|---------|
| `vzglyd_sidecar` | Networking, poll loop |
| `quick-xml` | RSS/Atom XML parsing |
| `serde` + `serde_json` | JSON serialization |
| `chrono` | Date parsing (RFC 2822 + ISO 8601) |

## Files to create

| Directory | Files |
|-----------|-------|
| `slides/news/` | `Cargo.toml`, `src/lib.rs`, `manifest.json`, `build.sh` |
| `slides/news/sidecar/` | `Cargo.toml`, `src/main.rs`, `src/hackernews.rs`, `src/reddit.rs`, `src/rss.rs` |
