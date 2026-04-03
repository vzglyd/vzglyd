# VZGLYD Architecture

VZGLYD has three layers:

- `src/`: the native engine, renderer, loader, and platform integration
- `VRX-64-slide/`: the ABI and scene description contract shared by the engine and slides
- `vzglyd_sidecar/`: the WASI-side networking and IPC helpers used by slide sidecars

## Runtime flow

1. The engine resolves a scene path or scans a slide directory.
2. A package is loaded from an unpacked slide directory or a `.vzglyd` archive.
3. `manifest.json` is validated, then the slide's WASM module is loaded with wasmtime.
4. The engine reads the encoded `SlideSpec`, validates it against `Limits::pi4()`, and allocates GPU resources.
5. Every frame, the engine calls `vzglyd_update(dt)` and renders the current scene through wgpu.
6. Optional sidecars fetch data independently and push payloads through the host channel.

## Engine responsibilities

The engine owns:

- package loading and manifest validation
- shader validation and pipeline creation
- texture and mesh buffer uploads
- frame scheduling and slide transitions
- display integration through Wayland or DRM/KMS
- the host ABI exposed to slide and sidecar WASM modules

Slides own:

- scene composition
- animation state
- dynamic mesh updates
- optional shader overrides within the engine's contract

Sidecars own:

- network access
- API polling and caching
- pushing serialized payloads to the slide

## Packaging model

A deployable slide is a `.vzglyd` zip archive containing:

- `manifest.json`
- `slide.wasm`
- optional `sidecar.wasm`
- optional `assets/`
- optional `shaders/`

The engine also supports unpacked slide directories during development.

## Deployment model

The canonical deployment target is a Raspberry Pi 4 running DietPi with Weston in kiosk mode.

- `deploy/` contains the checked-in Weston and systemd configuration.
- `install.sh` installs the release binary, deploy files, and starter slides.
- `.github/workflows/release.yml` builds the aarch64 binary and packages release artifacts.

## Further reading

- [Authoring guide](authoring-guide.md)
- [Shader contract](shader-contract.md)
- [Slide-format guide](../SLIDE_FORMAT.md)
