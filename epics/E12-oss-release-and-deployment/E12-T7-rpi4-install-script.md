# E12-T7: RPi4 Installation Script

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | L |
| **Blocked by** | E12-T6, E12-T8 |
| **Blocks** | - |

## Description

Write a single idempotent bash script that takes a fresh DietPi installation on a Raspberry Pi 4 from zero to a running VZGLYD kiosk. The script handles: package installation, user creation, directory layout, binary download, service file placement, and first-boot slide installation. It should be safe to re-run.

The target invocation for a new user is:

```bash
curl -fsSL https://raw.githubusercontent.com/vzglyd/vzglyd/main/install.sh | sudo bash
```

## Background

First impressions matter enormously for an open source project's adoption. The difference between "follow these 14 manual steps" and "run this one command" is the difference between a project people try and a project people actually use. The install script is not a convenience — it is the product's front door.

The script must be honest and cautious: it should print what it is doing, fail loudly if anything goes wrong, and never silently leave the system in a partially-configured state.

## Prerequisites assumed on the target system

- Raspberry Pi 4 (any RAM variant)
- DietPi installed (Debian Bookworm, 64-bit aarch64)
- Network connectivity
- Root access (`sudo` or running as root)
- No existing VZGLYD installation (idempotency handles the re-run case)

## What the script does (in order)

```
1.  Check prerequisites (OS, architecture, root)
2.  Install system packages (weston, libinput, etc.)
3.  Create vzglyd system user and groups
4.  Create directory layout
5.  Download the vzglyd binary from GitHub Releases
6.  Verify binary checksum
7.  Install binary to /usr/local/bin/vzglyd
8.  Install service files from deploy/systemd/
9.  Install weston.ini from deploy/
10. Enable and start services
11. Install starter slides
12. Print success and next steps
```

## Script structure

### Preamble and safety

```bash
#!/usr/bin/env bash
set -euo pipefail

VZGLYD_VERSION="${VZGLYD_VERSION:-latest}"
VZGLYD_USER="vzglyd"
VZGLYD_HOME="/var/lib/vzglyd"
VZGLYD_CONFIG="/etc/vzglyd"
VZGLYD_LOG="/var/log/vzglyd"
VZGLYD_BIN="/usr/local/bin/vzglyd"
GITHUB_REPO="vzglyd/vzglyd"
GITHUB_RELEASES="https://github.com/${GITHUB_REPO}/releases"

log()  { echo "[VRX-64-install] $*"; }
err()  { echo "[VRX-64-install] ERROR: $*" >&2; exit 1; }
warn() { echo "[VRX-64-install] WARN: $*" >&2; }
```

### Step 1 — Prerequisite checks

```bash
check_prerequisites() {
    # Must be root
    [[ "$EUID" -eq 0 ]] || err "Run as root: sudo bash install.sh"

    # Must be aarch64 (RPi4)
    local arch
    arch=$(uname -m)
    [[ "$arch" == "aarch64" ]] || err "Unsupported architecture: $arch (expected aarch64)"

    # Must have systemd
    command -v systemctl >/dev/null 2>&1 || err "systemd required"

    # Check OS (advisory — don't fail on non-DietPi Debian)
    if [[ -f /etc/os-release ]]; then
        . /etc/os-release
        log "Detected OS: $PRETTY_NAME"
    fi
}
```

### Step 2 — Package installation

```bash
install_packages() {
    log "Installing system packages..."
    apt-get update -qq
    apt-get install -y --no-install-recommends \
        weston \
        libweston-12-0 \
        libinput10 \
        libdrm2 \
        curl \
        jq
}
```

Package list verified against DietPi Bookworm. `jq` is used for parsing GitHub API responses to find the latest release.

### Step 3 — User and group setup

```bash
setup_user() {
    if id "$VZGLYD_USER" &>/dev/null; then
        log "User '$VZGLYD_USER' already exists"
    else
        log "Creating system user '$VZGLYD_USER'..."
        useradd \
            --system \
            --no-create-home \
            --home-dir "$VZGLYD_HOME" \
            --shell /usr/sbin/nologin \
            --groups video,render,input \
            "$VZGLYD_USER"
    fi

    # Ensure group memberships (idempotent)
    for grp in video render input; do
        if getent group "$grp" >/dev/null; then
            usermod -aG "$grp" "$VZGLYD_USER"
        fi
    done

    # Create XDG_RUNTIME_DIR for the vzglyd user
    local uid
    uid=$(id -u "$VZGLYD_USER")
    mkdir -p "/run/user/$uid"
    chown "$VZGLYD_USER:$VZGLYD_USER" "/run/user/$uid"
    chmod 700 "/run/user/$uid"
}
```

### Step 4 — Directory layout

```bash
setup_directories() {
    log "Creating directory layout..."
    mkdir -p \
        "$VZGLYD_HOME/slides" \
        "$VZGLYD_CONFIG" \
        "$VZGLYD_LOG"
    chown -R "$VZGLYD_USER:$VZGLYD_USER" "$VZGLYD_HOME" "$VZGLYD_LOG"
    chmod 755 "$VZGLYD_HOME" "$VZGLYD_HOME/slides"
}
```

### Step 5–6 — Binary download and verification

```bash
install_binary() {
    local version="$1"
    local url

    if [[ "$version" == "latest" ]]; then
        log "Resolving latest release..."
        version=$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" \
            | jq -r '.tag_name')
        [[ -n "$version" ]] || err "Could not resolve latest release tag"
        log "Latest version: $version"
    fi

    url="${GITHUB_RELEASES}/download/${version}/VRX-64-aarch64-unknown-linux-gnu.tar.gz"
    local checksum_url="${GITHUB_RELEASES}/download/${version}/VRX-64-aarch64-unknown-linux-gnu.tar.gz.sha256"

    log "Downloading vzglyd $version..."
    local tmp
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT

    curl -fsSL "$url" -o "$tmp/vzglyd.tar.gz"
    curl -fsSL "$checksum_url" -o "$tmp/vzglyd.tar.gz.sha256"

    log "Verifying checksum..."
    (cd "$tmp" && sha256sum -c vzglyd.tar.gz.sha256) || err "Checksum verification failed"

    tar -xzf "$tmp/vzglyd.tar.gz" -C "$tmp"
    install -m 755 "$tmp/vzglyd" "$VZGLYD_BIN"
    log "Installed: $VZGLYD_BIN ($("$VZGLYD_BIN" --version 2>/dev/null || echo 'unknown version'))"
}
```

### Step 7 — Service files

```bash
install_services() {
    log "Installing systemd service files..."
    local uid
    uid=$(id -u "$VZGLYD_USER")

    # Substitute actual UID into vzglyd.service
    sed "s/UID_PLACEHOLDER/$uid/" \
        /usr/local/share/vzglyd/systemd/vzglyd.service \
        > /etc/systemd/system/vzglyd.service

    cp /usr/local/share/vzglyd/systemd/weston.service \
       /etc/systemd/system/weston.service

    cp /usr/local/share/vzglyd/weston.ini \
       "$VZGLYD_CONFIG/weston.ini"

    systemctl daemon-reload
    systemctl enable weston.service vzglyd.service
}
```

The `deploy/` files are bundled into the tarball at `/usr/local/share/vzglyd/`. This avoids a separate download and keeps configuration versioned alongside the binary.

### Step 8 — Starter slides

```bash
install_starter_slides() {
    log "Installing starter slides..."
    local slides_url="${GITHUB_RELEASES}/download/${VZGLYD_VERSION}/starter-slides.tar.gz"

    if curl -fsSL --head "$slides_url" | grep -q "200 OK"; then
        curl -fsSL "$slides_url" | tar -xzf - -C "$VZGLYD_HOME/slides/"
        chown -R "$VZGLYD_USER:$VZGLYD_USER" "$VZGLYD_HOME/slides/"
        log "Starter slides installed"
    else
        warn "No starter slides archive for $VZGLYD_VERSION — slides directory is empty"
        warn "Add .vzglyd packages to $VZGLYD_HOME/slides/ manually"
    fi
}
```

The starter slides archive contains pre-built `.vzglyd` packages for clock, quotes, and weather — the minimal set that demonstrates VZGLYD without requiring API keys.

### Step 9 — Start and verify

```bash
start_services() {
    log "Starting Weston..."
    systemctl start weston.service
    sleep 3

    log "Starting VZGLYD..."
    systemctl start vzglyd.service
    sleep 2

    if systemctl is-active --quiet vzglyd.service; then
        log "VZGLYD is running"
    else
        warn "VZGLYD service failed to start. Check logs:"
        warn "  journalctl -u vzglyd.service -n 50"
        warn "  journalctl -u weston.service -n 20"
    fi
}
```

### Step 10 — Success message

```bash
print_summary() {
    echo ""
    echo "=========================================="
    echo "  VZGLYD installed successfully"
    echo "=========================================="
    echo ""
    echo "  Slides directory:  $VZGLYD_HOME/slides/"
    echo "  Config:            $VZGLYD_CONFIG/weston.ini"
    echo "  Logs:              journalctl -u vzglyd"
    echo ""
    echo "  Add slides:        copy .vzglyd files to $VZGLYD_HOME/slides/"
    echo "  Browse slides:     https://github.com/vzglyd/registry"
    echo ""
    echo "  VZGLYD will start automatically on next boot."
    echo ""
}
```

## Idempotency strategy

Each step is guarded:
- Package install: `apt-get install -y` is idempotent
- User creation: `id "$VZGLYD_USER"` check before `useradd`
- Directory creation: `mkdir -p` is idempotent
- Binary install: always overwrites (intentional — re-running upgrades the binary)
- Service enable: `systemctl enable` is idempotent

## Uninstall script

Create a companion `uninstall.sh` that reverses the installation:
- `systemctl disable --now weston vzglyd`
- Remove service files from `/etc/systemd/system/`
- Remove binary from `/usr/local/bin/`
- Remove `/etc/vzglyd/` and `/usr/local/share/vzglyd/`
- Optionally remove `/var/lib/vzglyd/` (prompt user — slides are user data)
- Remove `vzglyd` user

## Acceptance criteria

- [ ] `curl -fsSL .../install.sh | sudo bash` completes on a fresh DietPi RPi4 in < 5 minutes
- [ ] After install, `systemctl is-active weston vzglyd` both report `active`
- [ ] VZGLYD displays on HDMI after reboot without manual intervention
- [ ] Script fails with a clear error if run on wrong architecture
- [ ] Script fails with a clear error if run without root
- [ ] Re-running the script is safe (idempotent)
- [ ] Checksum verification fails the install if the binary is corrupted
- [ ] `uninstall.sh` reverses the installation cleanly
- [ ] Script has been tested on a real RPi4 running DietPi (Bookworm, 64-bit)

## Files to create

| File | Purpose |
|------|---------|
| `install.sh` | Main installation script |
| `uninstall.sh` | Uninstallation script |
| (binary tarball CI) | See E12-T8 — script depends on this artifact |
