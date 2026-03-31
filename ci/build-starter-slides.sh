#!/usr/bin/env bash
set -euo pipefail

VZGLYD_BIN="${VZGLYD_BIN:-target/release/vzglyd}"
OUT_DIR="${OUT_DIR:-dist/starter-slides}"

mkdir -p "${OUT_DIR}"

for slide in clock quotes weather; do
  "${VZGLYD_BIN}" pack "slides/${slide}" -o "${OUT_DIR}/${slide}.vzglyd"
done

ls -lh "${OUT_DIR}"
