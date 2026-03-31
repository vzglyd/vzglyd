#!/usr/bin/env bash
set -euo pipefail

cargo build --target wasm32-wasip1 --release
cargo build --manifest-path sidecar/Cargo.toml --target wasm32-wasip1 --release
