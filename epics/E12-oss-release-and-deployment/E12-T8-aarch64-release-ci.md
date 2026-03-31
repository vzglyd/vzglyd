# E12-T8: aarch64 Release CI Pipeline

| Field | Value |
|-------|-------|
| **Epic** | E12 OSS Release, Deployment, and Ecosystem |
| **Priority** | P0 (blocker) |
| **Estimate** | M |
| **Blocked by** | - |
| **Blocks** | E12-T7 |

## Description

Create a GitHub Actions workflow that, on every version tag, cross-compiles the `vzglyd` binary for `aarch64-unknown-linux-gnu` (Raspberry Pi 4), packages it with the `deploy/` configuration files, generates a SHA-256 checksum, and attaches everything to a GitHub Release. This is the artifact that E12-T7's install script downloads.

## Background

The target platform (RPi4, aarch64) cannot compile VZGLYD efficiently due to:
- The engine's heavy dependency on `wgpu`, `wasmtime`, and `naga` which have slow compile times
- Limited RAM on RPi4 (the linker can OOM during a full release build)
- Cross-compilation from a fast x86_64 CI host is the practical solution

Cross-compilation from x86_64 to aarch64 is well-supported in the Rust ecosystem via the `cross` tool or via GitHub's native `ubuntu-latest` runners with `aarch64-unknown-linux-gnu` toolchain and `qemu`.

## Artifacts produced per release

| Artifact | Description |
|----------|-------------|
| `vzglyd-aarch64-unknown-linux-gnu.tar.gz` | Binary + deploy/ files |
| `vzglyd-aarch64-unknown-linux-gnu.tar.gz.sha256` | SHA-256 checksum |
| `starter-slides.tar.gz` | Pre-built .vzglyd packages (clock, quotes, weather) |
| `starter-slides.tar.gz.sha256` | SHA-256 checksum |

## Tarball contents

`vzglyd-aarch64-unknown-linux-gnu.tar.gz` extracts to:

```
vzglyd                                      # the binary
usr/local/share/vzglyd/
├── systemd/
│   ├── weston.service
│   └── vzglyd.service
└── weston.ini
```

The install script extracts this and uses the relative paths.

## Workflow design

### Trigger

```yaml
on:
  push:
    tags:
      - "v*"
  workflow_dispatch:
    inputs:
      tag:
        description: "Tag to build (e.g. v0.1.0)"
        required: true
```

`workflow_dispatch` allows building a release manually without a tag — useful for testing the pipeline before cutting a real release.

### Cross-compilation strategy

Option A: **`cross` tool** — A Docker-based cross-compilation tool maintained by the `cross-rs` project. Handles sysroot, linker, and pkg-config setup automatically. Most reliable for crates with C dependencies (wgpu links against system Vulkan/DRM headers).

Option B: **GitHub's aarch64 runners** — GitHub offers `ubuntu-24.04-arm` runners (native aarch64). This eliminates cross-compilation entirely at the cost of runner minutes (arm runners are slower).

Option C: **`cargo-zigbuild`** — uses Zig as the cross-linker, very fast setup but less battle-tested with complex C deps.

**Recommendation: Option B (native aarch64 runner)** — eliminates sysroot complexity entirely. If GitHub's arm runners are unavailable or too slow, fall back to Option A with `cross`.

```yaml
jobs:
  build-aarch64:
    runs-on: ubuntu-24.04-arm   # native aarch64
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install system deps
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libdrm-dev \
            libinput-dev \
            libudev-dev \
            libwayland-dev \
            pkg-config
      - name: Build release binary
        run: cargo build --release -p vzglyd
      - name: Package
        run: bash ci/package-release.sh
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: vzglyd-aarch64
          path: dist/
```

### Release creation

```yaml
  release:
    needs: build-aarch64
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
        with:
          name: vzglyd-aarch64
          path: dist/
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: dist/*
          generate_release_notes: true
```

### Packaging script (ci/package-release.sh)

```bash
#!/usr/bin/env bash
set -euo pipefail

VERSION="${GITHUB_REF_NAME:-dev}"
DIST="dist"
mkdir -p "$DIST"

# Binary
cp target/release/vzglyd .

# Create tarball
tar -czf "$DIST/vzglyd-aarch64-unknown-linux-gnu.tar.gz" \
    vzglyd \
    --transform 's|^deploy/|usr/local/share/vzglyd/|' \
    deploy/weston.ini \
    deploy/systemd/weston.service \
    deploy/systemd/vzglyd.service

# Checksum
(cd "$DIST" && sha256sum vzglyd-aarch64-unknown-linux-gnu.tar.gz \
    > vzglyd-aarch64-unknown-linux-gnu.tar.gz.sha256)

# Starter slides (pre-built .vzglyd packages must exist in ci/starter-slides/)
if [[ -d ci/starter-slides ]]; then
    tar -czf "$DIST/starter-slides.tar.gz" -C ci/starter-slides .
    (cd "$DIST" && sha256sum starter-slides.tar.gz > starter-slides.tar.gz.sha256)
fi

echo "Release artifacts:"
ls -lh "$DIST/"
```

### Starter slides

The starter slides tarball requires pre-built `.vzglyd` packages. Two options:

1. **Build them in CI**: The release workflow builds clock, quotes, and weather slides from their repos (requires wasm32-wasip1 toolchain in the runner) and packages the resulting `.vzglyd` files.

2. **Store pre-built in repo**: Keep committed `.vzglyd` files under `ci/starter-slides/` and update them with each slide release. Less elegant but avoids cross-repo CI dependency.

**Recommendation**: Build them in CI from the published GitHub Release assets of each starter slide repo. Add a step that downloads `vzglyd/slide-clock`, `vzglyd/slide-quotes`, and `vzglyd/slide-weather` release artifacts.

```bash
# In CI, download starter slide artifacts
for slide in clock quotes weather; do
    gh release download \
        --repo "vzglyd/slide-$slide" \
        --pattern "*.vzglyd" \
        --dir ci/starter-slides/
done
```

## Version tagging convention

- Engine releases: `v0.1.0`, `v0.2.0`, etc.
- `vzglyd-slide` releases: `vzglyd-slide-v0.1.0` (separate tag, separate workflow)
- `vzglyd_sidecar` releases: `vzglyd_sidecar-v0.1.0` (separate tag, separate workflow)

These are independent release streams on the same repo.

## Acceptance criteria

- [ ] Pushing a `v*` tag triggers the release workflow
- [ ] Workflow completes on `ubuntu-24.04-arm` (or cross if arm runner unavailable)
- [ ] `vzglyd-aarch64-unknown-linux-gnu.tar.gz` contains the binary and deploy files
- [ ] `sha256sum -c` verifies both tarballs
- [ ] GitHub Release is created with correct tag name and generated release notes
- [ ] The binary in the tarball runs on RPi4 DietPi (`./vzglyd --version` succeeds)
- [ ] Starter slides tarball contains at minimum: clock.vzglyd, quotes.vzglyd
- [ ] `workflow_dispatch` trigger works for manual test builds

## Files to create

| File | Purpose |
|------|---------|
| `.github/workflows/release.yml` | Release CI workflow |
| `ci/package-release.sh` | Packaging script |
| `ci/build-starter-slides.sh` | Downloads starter slide .vzglyd artifacts |
