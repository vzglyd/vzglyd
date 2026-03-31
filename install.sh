#!/usr/bin/env bash
set -euo pipefail

VZGLYD_VERSION="${VZGLYD_VERSION:-latest}"
VZGLYD_USER="${VZGLYD_USER:-vzglyd}"
VZGLYD_HOME="${VZGLYD_HOME:-/var/lib/vzglyd}"
VZGLYD_CONFIG="${VZGLYD_CONFIG:-/etc/vzglyd}"
VZGLYD_LOG="${VZGLYD_LOG:-/var/log/vzglyd}"
VZGLYD_BIN="${VZGLYD_BIN:-/usr/local/bin/vzglyd}"
GITHUB_REPO="${GITHUB_REPO:-vzglyd/vzglyd}"
GITHUB_RELEASES="https://github.com/${GITHUB_REPO}/releases"

log() {
  echo "[vzglyd-install] $*"
}

warn() {
  echo "[vzglyd-install] WARN: $*" >&2
}

err() {
  echo "[vzglyd-install] ERROR: $*" >&2
  exit 1
}

require_root() {
  [[ "${EUID}" -eq 0 ]] || err "run as root: sudo bash install.sh"
}

check_platform() {
  local arch
  arch="$(uname -m)"
  [[ "${arch}" == "aarch64" ]] || err "unsupported architecture: ${arch} (expected aarch64)"
  command -v systemctl >/dev/null 2>&1 || err "systemd is required"
  if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    log "Detected OS: ${PRETTY_NAME:-unknown}"
  fi
}

install_packages() {
  log "Installing system packages"
  apt-get update -qq
  apt-get install -y --no-install-recommends \
    weston \
    libweston-12-0 \
    libinput10 \
    libdrm2 \
    curl \
    jq
}

setup_user() {
  if id "${VZGLYD_USER}" >/dev/null 2>&1; then
    log "User '${VZGLYD_USER}' already exists"
  else
    log "Creating system user '${VZGLYD_USER}'"
    useradd \
      --system \
      --home-dir "${VZGLYD_HOME}" \
      --shell /usr/sbin/nologin \
      --groups video,render,input \
      "${VZGLYD_USER}"
  fi

  for grp in video render input; do
    if getent group "${grp}" >/dev/null; then
      usermod -aG "${grp}" "${VZGLYD_USER}"
    fi
  done
}

setup_directories() {
  log "Creating directory layout"
  mkdir -p "${VZGLYD_HOME}/slides" "${VZGLYD_CONFIG}" "${VZGLYD_LOG}"
  chown -R "${VZGLYD_USER}:${VZGLYD_USER}" "${VZGLYD_HOME}" "${VZGLYD_LOG}"
  chmod 755 "${VZGLYD_HOME}" "${VZGLYD_HOME}/slides"
}

resolve_version() {
  if [[ "${VZGLYD_VERSION}" != "latest" ]]; then
    echo "${VZGLYD_VERSION}"
    return
  fi

  curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
    | jq -r '.tag_name'
}

download_release() {
  local version="$1"
  local tmp_dir="$2"
  local archive_url="${GITHUB_RELEASES}/download/${version}/vzglyd-aarch64-unknown-linux-gnu.tar.gz"
  local checksum_url="${archive_url}.sha256"

  log "Downloading vzglyd ${version}"
  curl -fsSL "${archive_url}" -o "${tmp_dir}/vzglyd.tar.gz"
  curl -fsSL "${checksum_url}" -o "${tmp_dir}/vzglyd.tar.gz.sha256"

  log "Verifying release checksum"
  (
    cd "${tmp_dir}"
    sha256sum -c vzglyd.tar.gz.sha256
  ) || err "checksum verification failed"
}

install_binary_and_assets() {
  local tmp_dir="$1"
  tar -xzf "${tmp_dir}/vzglyd.tar.gz" -C "${tmp_dir}"

  install -m 755 "${tmp_dir}/vzglyd" "${VZGLYD_BIN}"
  install -d /usr/local/share/vzglyd/systemd
  install -m 644 "${tmp_dir}/usr/local/share/vzglyd/weston.ini" /usr/local/share/vzglyd/weston.ini
  install -m 644 "${tmp_dir}/usr/local/share/vzglyd/systemd/weston.service" /usr/local/share/vzglyd/systemd/weston.service
  install -m 644 "${tmp_dir}/usr/local/share/vzglyd/systemd/vzglyd.service" /usr/local/share/vzglyd/systemd/vzglyd.service
  install -m 644 "${tmp_dir}/usr/local/share/vzglyd/systemd/vzglyd-slides.path" /usr/local/share/vzglyd/systemd/vzglyd-slides.path
  install -m 644 "${tmp_dir}/usr/local/share/vzglyd/systemd/vzglyd-slides.service" /usr/local/share/vzglyd/systemd/vzglyd-slides.service
}

install_services() {
  log "Installing systemd service files"
  install -m 644 /usr/local/share/vzglyd/systemd/weston.service /etc/systemd/system/weston.service
  install -m 644 /usr/local/share/vzglyd/systemd/vzglyd.service /etc/systemd/system/vzglyd.service
  install -m 644 /usr/local/share/vzglyd/systemd/vzglyd-slides.path /etc/systemd/system/vzglyd-slides.path
  install -m 644 /usr/local/share/vzglyd/systemd/vzglyd-slides.service /etc/systemd/system/vzglyd-slides.service
  install -m 644 /usr/local/share/vzglyd/weston.ini "${VZGLYD_CONFIG}/weston.ini"

  systemctl daemon-reload
  systemctl enable weston.service vzglyd.service vzglyd-slides.path
}

install_starter_slides() {
  local version="$1"
  local slides_url="${GITHUB_RELEASES}/download/${version}/starter-slides.tar.gz"
  local checksum_url="${slides_url}.sha256"
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir}"' RETURN

  if curl -fsSL "${checksum_url}" -o "${tmp_dir}/starter-slides.tar.gz.sha256"; then
    curl -fsSL "${slides_url}" -o "${tmp_dir}/starter-slides.tar.gz"
    (
      cd "${tmp_dir}"
      sha256sum -c starter-slides.tar.gz.sha256
    ) || err "starter slides checksum verification failed"
    tar -xzf "${tmp_dir}/starter-slides.tar.gz" -C "${VZGLYD_HOME}/slides"
    chown -R "${VZGLYD_USER}:${VZGLYD_USER}" "${VZGLYD_HOME}/slides"
    log "Starter slides installed"
  else
    warn "No starter slide archive published for ${version}"
  fi
}

start_services() {
  log "Starting services"
  systemctl start weston.service
  sleep 3
  systemctl start vzglyd.service
  systemctl start vzglyd-slides.path
}

print_summary() {
  echo
  echo "VZGLYD installed successfully"
  echo "Slides directory: ${VZGLYD_HOME}/slides"
  echo "Logs: journalctl -u vzglyd.service"
  echo "Weston log: ${VZGLYD_LOG}/weston.log"
}

main() {
  require_root
  check_platform
  install_packages
  setup_user
  setup_directories

  local version
  version="$(resolve_version)"
  [[ -n "${version}" && "${version}" != "null" ]] || err "could not resolve a release tag"
  log "Using release ${version}"

  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir}"' EXIT

  download_release "${version}" "${tmp_dir}"
  install_binary_and_assets "${tmp_dir}"
  install_services
  install_starter_slides "${version}"
  start_services
  print_summary
}

main "$@"
