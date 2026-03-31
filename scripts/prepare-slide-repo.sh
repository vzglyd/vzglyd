#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <slide-name> <destination-dir>" >&2
  exit 1
fi

SLIDE_NAME="$1"
DEST_DIR="$2"
SOURCE_DIR="slides/${SLIDE_NAME}"

if [[ ! -d "${SOURCE_DIR}" ]]; then
  echo "unknown slide directory: ${SOURCE_DIR}" >&2
  exit 1
fi

mkdir -p "${DEST_DIR}"
cp -R "${SOURCE_DIR}/." "${DEST_DIR}/"

if [[ -f "${DEST_DIR}/Cargo.toml" ]]; then
  sed -i \
    -e 's|vzglyd_slide = { path = "../../vzglyd_slide" }|vzglyd-slide = "0.1"|' \
    -e 's|vzglyd_sidecar = { path = "../../vzglyd_sidecar" }|vzglyd-sidecar = "0.1"|' \
    "${DEST_DIR}/Cargo.toml"
fi

if [[ -f "${DEST_DIR}/sidecar/Cargo.toml" ]]; then
  sed -i \
    -e 's|vzglyd_slide = { path = "../../../vzglyd_slide" }|vzglyd-slide = "0.1"|' \
    -e 's|vzglyd_sidecar = { path = "../../../vzglyd_sidecar" }|vzglyd-sidecar = "0.1"|' \
    -e 's|vzglyd_sidecar = { path = "../../vzglyd_sidecar" }|vzglyd-sidecar = "0.1"|' \
    "${DEST_DIR}/sidecar/Cargo.toml"
fi

mkdir -p "${DEST_DIR}/.github/workflows"
cp LICENSE-MIT LICENSE-APACHE "${DEST_DIR}/"

if [[ ! -f "${DEST_DIR}/CHANGELOG.md" ]]; then
  cat > "${DEST_DIR}/CHANGELOG.md" <<EOF
# Changelog

## [Unreleased]
EOF
fi

if [[ ! -f "${DEST_DIR}/README.md" ]]; then
  cat > "${DEST_DIR}/README.md" <<EOF
# slide-${SLIDE_NAME}

Standalone VZGLYD slide repository extracted from the monorepo.
EOF
fi

cp templates/slide/.github/workflows/ci.yml.liquid "${DEST_DIR}/.github/workflows/ci.yml"

echo "Prepared ${DEST_DIR}"
echo "Next steps:"
echo "  1. run git filter-repo or subtree split if you want preserved history"
echo "  2. create the remote repository"
echo "  3. review Cargo.toml and README.md"
