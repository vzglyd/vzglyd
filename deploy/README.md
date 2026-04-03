# VZGLYD Deployment on DietPi + Weston

This directory contains the canonical deployment files for running VZGLYD as a fullscreen kiosk on a Raspberry Pi 4.

## Target Environment

- Raspberry Pi 4
- DietPi 64-bit (Debian Bookworm base)
- Weston running in kiosk mode
- `vzglyd` system user
- Slides stored in `/var/lib/vzglyd/slides`

## Manual Setup

1. Install system packages:

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  weston \
  libweston-12-0 \
  libinput10 \
  libdrm2 \
  jq \
  curl
```

2. Create the service user:

```bash
sudo useradd \
  --system \
  --home-dir /var/lib/vzglyd \
  --shell /usr/sbin/nologin \
  --groups video,render,input \
  vzglyd
```

3. Create runtime directories:

```bash
sudo mkdir -p /var/lib/vzglyd/slides /var/log/vzglyd /etc/vzglyd
sudo chown -R vzglyd:vzglyd /var/lib/vzglyd /var/log/vzglyd
```

4. Install the checked-in files:

```bash
sudo install -m 644 deploy/weston.ini /etc/vzglyd/weston.ini
sudo install -m 644 deploy/systemd/weston.service /etc/systemd/system/weston.service
sudo install -m 644 deploy/systemd/vzglyd.service /etc/systemd/system/vzglyd.service
sudo install -m 644 deploy/systemd/VRX-64-slides.path /etc/systemd/system/VRX-64-slides.path
sudo install -m 644 deploy/systemd/VRX-64-slides.service /etc/systemd/system/VRX-64-slides.service
```

5. Install the `vzglyd` binary to `/usr/local/bin/vzglyd`.

6. Copy one or more `.vzglyd` packages into `/var/lib/vzglyd/slides/`.

7. Enable and start the services:

```bash
sudo systemctl daemon-reload
sudo systemctl enable weston.service vzglyd.service VRX-64-slides.path
sudo systemctl start weston.service vzglyd.service VRX-64-slides.path
```

## Service Behavior

- `weston.service` creates `/run/vzglyd` and starts Weston with the kiosk shell.
- `vzglyd.service` waits for `/run/vzglyd/wayland-0` before launching the engine.
- `VRX-64-slides.path` watches the slides directory and restarts `vzglyd.service` when packages change.

## Logs

- Weston log: `/var/log/vzglyd/weston.log`
- VZGLYD log: `journalctl -u vzglyd.service`
- Weston journal: `journalctl -u weston.service`
