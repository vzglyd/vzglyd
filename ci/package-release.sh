#!/usr/bin/env bash
set -euo pipefail

DIST_DIR="${DIST_DIR:-dist}"
RELEASE_ROOT="${DIST_DIR}/release-root"
SLIDES_DIR="${DIST_DIR}/starter-slides"
VZGLYD_BIN_PATH="${VZGLYD_BIN_PATH:-target/release/vzglyd}"

rm -rf "${RELEASE_ROOT}"
mkdir -p "${DIST_DIR}" "${RELEASE_ROOT}/usr/local/share/vzglyd/systemd"

install -m 755 "${VZGLYD_BIN_PATH}" "${RELEASE_ROOT}/vzglyd"
install -m 644 deploy/weston.ini "${RELEASE_ROOT}/usr/local/share/vzglyd/weston.ini"
install -m 644 deploy/systemd/weston.service "${RELEASE_ROOT}/usr/local/share/vzglyd/systemd/weston.service"
install -m 644 deploy/systemd/vzglyd.service "${RELEASE_ROOT}/usr/local/share/vzglyd/systemd/vzglyd.service"
install -m 644 deploy/systemd/vzglyd-slides.path "${RELEASE_ROOT}/usr/local/share/vzglyd/systemd/vzglyd-slides.path"
install -m 644 deploy/systemd/vzglyd-slides.service "${RELEASE_ROOT}/usr/local/share/vzglyd/systemd/vzglyd-slides.service"

tar -czf "${DIST_DIR}/vzglyd-aarch64-unknown-linux-gnu.tar.gz" -C "${RELEASE_ROOT}" .
(
  cd "${DIST_DIR}"
  sha256sum vzglyd-aarch64-unknown-linux-gnu.tar.gz > vzglyd-aarch64-unknown-linux-gnu.tar.gz.sha256
)

if [[ -d "${SLIDES_DIR}" ]] && compgen -G "${SLIDES_DIR}/*.vzglyd" >/dev/null; then
  tar -czf "${DIST_DIR}/starter-slides.tar.gz" -C "${SLIDES_DIR}" .
  (
    cd "${DIST_DIR}"
    sha256sum starter-slides.tar.gz > starter-slides.tar.gz.sha256
  )
fi

ls -lh "${DIST_DIR}"
