# E12-T6: DietPi + Weston Kiosk Systemd Deployment

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | L |
| **Blocked by** | - |
| **Blocks** | E12-T7 |

## Description

Codify the canonical VZGLYD deployment configuration as checked-in files: a Weston kiosk configuration, a `vzglyd.service` systemd unit, and a `weston.service` override or drop-in that the installation script can copy verbatim. The output of this ticket is a `deploy/` directory in the engine repo containing everything needed to run VZGLYD on a fresh DietPi + RPi4.

## Background

The current working setup exists only in the developer's head. The concrete requirements:

- **DietPi** base OS (Debian-based, minimal, configured for RPi4)
- **Weston** Wayland compositor running in kiosk mode (no desktop shell, no cursor, no window decorations)
- **vzglyd** binary running as a Weston client, fullscreen on HDMI
- **Systemd** managing both Weston and vzglyd, with vzglyd waiting on Weston to be ready
- **Auto-start on boot** without interactive login

The key challenge is the ordering: vzglyd must not start before the Wayland socket exists. DietPi may use `dietpi-autostart` or raw systemd, depending on version. This ticket targets a pure systemd approach that works regardless of DietPi's autostart menu.

## Target environment

| Component | Version / Notes |
|-----------|-----------------|
| Hardware | Raspberry Pi 4 (any RAM) |
| OS | DietPi (Debian Bookworm base, 64-bit) |
| Display | HDMI at native resolution (1920×1080 typical) |
| Compositor | Weston ≥ 12.0 (from Debian repos) |
| Service manager | systemd |
| User | `vzglyd` system user (non-root, in `video`, `render`, `input` groups) |

## Design decisions

### Why Weston and not direct DRM/KMS?

VZGLYD supports direct DRM/KMS on Linux (`drm` feature in Cargo.toml). On a kiosk device, either approach works. Weston is chosen because:
- It handles HID events (keyboard, touch) gracefully
- It manages output hot-plug
- It is the established path for Wayland kiosk deployments
- DietPi packages it

If future work removes the Weston dependency in favour of direct DRM, this deployment can be updated. For now, Weston is the path of least resistance.

### Run as a dedicated user

Running vzglyd as root is a security risk and unnecessary. A `vzglyd` system user with membership in `video`, `render`, `input`, and `seat` groups is sufficient.

### Weston socket readiness

Weston doesn't provide a `systemd-notify` socket-ready notification by default on all versions. The reliable approach is to use `sd_notify` via a wrapper or use `Type=notify` if Weston's version supports it, falling back to `ExecStartPost` polling the socket path.

## Files to create in deploy/

```
deploy/
├── README.md                        # Human-readable setup narrative
├── weston.ini                       # Kiosk mode Weston config
├── systemd/
│   ├── weston.service               # Weston service unit
│   ├── vzglyd.service                 # VZGLYD service unit
│   └── VRX-64-slides.path             # Optional: watch slides dir for changes
└── skel/
    └── slides/                      # Empty directory placeholder
        └── .gitkeep
```

## Weston configuration (weston.ini)

```ini
[core]
# Kiosk shell: no taskbar, no window decorations, fullscreen
shell=kiosk-shell.so
# Idle time 0 = never blank
idle-time=0

[output]
# Let Weston auto-detect the HDMI output
# Override if you have a specific connector name (e.g. name=HDMI-A-1)
# name=HDMI-A-1

[keyboard]
# No cursor
numlock-on=false
```

The kiosk shell (`kiosk-shell.so`) is bundled with Weston ≥ 9.0 and makes every client fullscreen automatically — VZGLYD doesn't need to request fullscreen itself.

## Weston service unit (systemd/weston.service)

```ini
[Unit]
Description=Weston Wayland Compositor (VZGLYD Kiosk)
After=systemd-logind.service
Requires=systemd-logind.service

[Service]
Type=simple
User=vzglyd
Group=vzglyd
# The Weston config lives at /etc/vzglyd/weston.ini
Environment="XDG_RUNTIME_DIR=/run/user/%i"
ExecStartPre=/bin/mkdir -p /run/user/%i
ExecStartPre=/bin/chown vzglyd:vzglyd /run/user/%i
ExecStartPre=/bin/chmod 700 /run/user/%i
ExecStart=/usr/bin/weston \
    --config=/etc/vzglyd/weston.ini \
    --log=/var/log/vzglyd/weston.log
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Note: `%i` is the UID of the vzglyd user, needed for `XDG_RUNTIME_DIR`. Alternatively, use a tmpfiles.d rule to pre-create this directory.

## VZGLYD service unit (systemd/vzglyd.service)

```ini
[Unit]
Description=VZGLYD Display Engine
After=weston.service
Requires=weston.service

[Service]
Type=simple
User=vzglyd
Group=vzglyd
Environment="XDG_RUNTIME_DIR=/run/user/UID_PLACEHOLDER"
Environment="WAYLAND_DISPLAY=wayland-0"
Environment="RUST_LOG=info"
WorkingDirectory=/var/lib/vzglyd
ExecStartPre=/bin/sleep 2
ExecStart=/usr/local/bin/vzglyd --slides /var/lib/vzglyd/slides
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

The `ExecStartPre=/bin/sleep 2` is a pragmatic guard: even after `weston.service` reports started, the Wayland socket may take a moment to become connectable. A cleaner alternative is a small wrapper that polls `$XDG_RUNTIME_DIR/wayland-0` until it exists (max 5s timeout), then execs vzglyd. Include both and document the trade-off.

## Slide path

VZGLYD looks for `.vzglyd` packages in `/var/lib/vzglyd/slides/`. On first install this directory is empty — the install script populates it with starter slides (see E12-T7).

An optional `VRX-64-slides.path` unit can trigger a vzglyd reload when a new `.vzglyd` file is dropped into the slides directory (using `inotifywait` or the vzglyd binary's own watch mode, if it supports it).

## Step-by-step implementation

### Step 1 — Verify Weston kiosk shell availability

On a test DietPi install, confirm `weston --shell=kiosk-shell.so` works. If the kiosk shell is not available in the DietPi-packaged Weston version, document the workaround (fullscreen-shell or just the regular desktop shell with autohide panel).

### Step 2 — Confirm DRM group membership requirements

On DietPi/RPi4, the `video` group grants DRM access. Confirm the vzglyd user needs: `video`, `render`, `input`, `seat`. Document the `useradd` incantation.

### Step 3 — Write and test the service files

On a real RPi4 running DietPi:
1. Create the `vzglyd` user
2. Copy service files to `/etc/systemd/system/`
3. Copy weston.ini to `/etc/vzglyd/`
4. `systemctl daemon-reload && systemctl enable weston vzglyd && systemctl start weston`
5. Verify Weston starts, then VZGLYD starts and connects

Iterate until both services start reliably across 3 cold reboots.

### Step 4 — Document manual setup in deploy/README.md

The README explains every step a human would take if they're not using the install script:
- Package installation (`apt install weston`)
- User creation
- Directory creation (`/var/lib/vzglyd/slides`, `/var/log/vzglyd`, `/etc/vzglyd`)
- File placement
- Service enable/start
- Log locations

This is the fallback if the install script fails and also the authoritative explanation of what the script does.

### Step 5 — Commit deploy/ directory to engine repo

This directory ships with the engine source. It is not generated — it is the canonical deployment configuration.

## Acceptance criteria

- [ ] `deploy/weston.ini` exists and produces a kiosk-mode Weston session on RPi4/DietPi
- [ ] `deploy/systemd/weston.service` starts Weston and handles restarts on failure
- [ ] `deploy/systemd/vzglyd.service` starts after Weston and connects to the Wayland socket
- [ ] Both services survive 3 successive cold reboots without manual intervention
- [ ] `deploy/README.md` describes manual installation completely
- [ ] A `vzglyd` system user with correct group memberships is documented
- [ ] Slide path `/var/lib/vzglyd/slides/` is documented and created on install

## Files to create

| File | Purpose |
|------|---------|
| `deploy/README.md` | Manual deployment narrative |
| `deploy/weston.ini` | Weston kiosk configuration |
| `deploy/systemd/weston.service` | Weston systemd unit |
| `deploy/systemd/vzglyd.service` | VZGLYD systemd unit |
| `deploy/systemd/VRX-64-slides.path` | Optional: inotify-based slide watcher |
| `deploy/skel/slides/.gitkeep` | Placeholder for slide directory |
