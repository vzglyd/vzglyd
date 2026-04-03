# Contributing to VZGLYD

## Ways to contribute

- Build a slide. New slides are the highest-leverage contribution.
- Report bugs with reproduction steps, VZGLYD version, target environment, and logs.
- Improve documentation, especially deployment and slide-authoring material.
- Submit focused fixes with tests.

## Development workflow

```bash
cargo build
cargo test
cargo clippy -- -D warnings
```

Useful local commands:

- Run one slide: `cargo run -- --scene slides/clock`
- Pack a slide: `cargo run -- pack slides/clock -o /tmp/clock.vzglyd`
- Run from a slides directory: `cargo run -- --slides-dir /path/to/slides`

If you are building slides or sidecars, install the WASI target once:

```bash
rustup target add wasm32-wasip1
```

## Pull request guidelines

- Keep each PR to one logical change.
- Run `cargo test` before opening the PR.
- Run `cargo clippy -- -D warnings` before opening the PR.
- Explain any new `unsafe`.
- ABI-breaking changes to `VRX-64-slide` require an explicit changelog entry and version discussion.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
