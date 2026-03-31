#!/usr/bin/env bash
set -euo pipefail

cargo build --target wasm32-wasip1 --release
