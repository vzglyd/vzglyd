# VZGLYD WASM ABI Reference

The ABI is the stable boundary between the engine and the slide. It does not change without a version increment. A slide exports a small fixed set of symbols, exposes one immutable wire-format `SlideSpec` blob from linear memory, and may optionally expose per-frame runtime data such as overlays or dynamic meshes. The host owns validation, resource creation, shader compilation, and rendering. This document states what the ABI is. The implementation lives in `src/slide_loader.rs`.

## ABI version

The current ABI version is `1`.

The loader reads the guest's `vzglyd_abi_version` export first. Any value other than `1` is rejected before the spec is decoded. The manifest field `abi_version` is also validated when present, but the runtime source of truth is the exported function. Optional exports such as `vzglyd_teardown`, `vzglyd_overlay_ptr`, and `vzglyd_dynamic_meshes_ptr` do not change the ABI version because the loader already treats them as optional.

## Required exports

The slide exports `memory`. The slide exports `vzglyd_abi_version`. The slide exports `vzglyd_spec_ptr` and `vzglyd_spec_len`.

| Export | Wasm type | Meaning |
| --- | --- | --- |
| `memory` | linear memory | Guest memory containing the spec blob and any optional runtime payloads. |
| `vzglyd_abi_version` | `() -> i32` or `() -> u32` at the language boundary | Must return `1`. WebAssembly only has `i32`, so signedness is a source-language convention. |
| `vzglyd_spec_ptr` | `() -> i32` | Pointer to the first byte of the versioned spec blob. |
| `vzglyd_spec_len` | `() -> i32` | Length in bytes of the versioned spec blob. |

The spec blob remains valid after instantiation. The loader copies it from guest memory immediately after optional initialization.

## Optional exports

The slide may export `vzglyd_init`. When present, the engine calls it once before the spec is copied. When absent, the engine proceeds without it.

The slide may export `vzglyd_update`. When present, the engine calls it once per rendered frame. When absent, the slide is static.

The slide may export `vzglyd_teardown`. When present, the engine calls it before runtime drop. The call is bounded to `100 ms` by Wasmtime epoch interruption. Stuck slides cannot block shutdown or scene rotation.

| Export | Wasm type | Meaning |
| --- | --- | --- |
| `vzglyd_init` | `() -> i32` | Called once after ABI validation and before the spec is copied. Current implementations conventionally return `0`; the host does not interpret the value yet. |
| `vzglyd_update` | `(f32) -> i32` | Called once per rendered frame with `dt` in seconds. Return `0` for no runtime mesh change and `1` when the host should reread overlay or dynamic-mesh payloads from memory. |
| `vzglyd_teardown` | `() -> i32` | Called before runtime drop. The host bounds the call to `100 ms` using Wasmtime epoch interruption so stuck slides cannot block shutdown or scene rotation. |
| `vzglyd_overlay_ptr` | `() -> i32` | Pointer to a postcard-encoded `RuntimeOverlay<V>` payload in guest memory. |
| `vzglyd_overlay_len` | `() -> i32` | Length of the postcard-encoded overlay payload. A length of `0` means no overlay. |
| `vzglyd_dynamic_meshes_ptr` | `() -> i32` | Pointer to a postcard-encoded `RuntimeMeshSet<V>` payload in guest memory. |
| `vzglyd_dynamic_meshes_len` | `() -> i32` | Length of the postcard-encoded dynamic-mesh payload. A length of `0` means no runtime mesh update. |

Only two `vzglyd_update` return values are meaningful in the current renderer:

| Return code | Meaning |
| --- | --- |
| `0` | No runtime geometry changed. |
| `1` | Runtime overlay and-or dynamic meshes changed; the host will reread the exported postcard payloads. |

Other return codes are treated as guest errors and are logged by the renderer.

## Available imports

The current loader instantiates every slide through a shared WASI preview1 store and linker. Slide authors build slides for `wasm32-wasip1`, not `wasm32-unknown-unknown`. The runtime surface that exists on both desktop Linux and Raspberry Pi 4-class deployments is `wasi_snapshot_preview1` plus `vzglyd_host`.

The target choice is not cosmetic. The two Rust targets imply different execution models:

| Target | Runtime contract | Status in VZGLYD |
| --- | --- | --- |
| `wasm32-unknown-unknown` | Bare WebAssembly with no standard WASI imports. Any nontrivial host capability must be invented ad hoc through custom imports. | Legacy only. Old modules may still instantiate if their imports happen to resolve, but this is no longer the supported authoring path. |
| `wasm32-wasip1` | WebAssembly plus the `wasi_snapshot_preview1` syscall surface for clocks, randomness, environment access, file descriptors, and future extension points such as the Epic 8 sidecar work. | Supported baseline. This is the single loader path used by the engine and the checked-in slide build scripts. |

On Raspberry Pi 4-class hardware this unification is operationally decisive. The engine requires one predictable sandbox model, one linker configuration, and one debugging story across ARM Linux and desktop Linux. `wasm32-wasip1` provides that shared runtime contract without requiring the renderer to reintroduce a second, Pi-specific slide path.

### Main slide imports

Main slides receive the regular `wasi_snapshot_preview1` surface plus the `vzglyd_host` imports listed below.

| Module | Function | Signature | Meaning |
| --- | --- | --- | --- |
| `vzglyd_host` | `channel_poll` | `(buf_ptr: i32, buf_len: i32) -> i32` | Copy the latest unread sidecar message into guest memory. |
| `vzglyd_host` | `mesh_asset_len` | `(key_ptr: i32, key_len: i32) -> i32` | Return the postcard byte length of a packaged mesh asset visible to the guest. |
| `vzglyd_host` | `mesh_asset_read` | `(key_ptr: i32, key_len: i32, buf_ptr: i32, buf_len: i32) -> i32` | Copy a packaged mesh asset into guest memory. |
| `vzglyd_host` | `scene_metadata_len` | `(key_ptr: i32, key_len: i32) -> i32` | Return the postcard byte length of a packaged scene metadata payload visible to the guest. |
| `vzglyd_host` | `scene_metadata_read` | `(key_ptr: i32, key_len: i32, buf_ptr: i32, buf_len: i32) -> i32` | Copy a packaged scene metadata payload into guest memory. |

The return convention for `channel_poll` is:

| Return code | Meaning |
| --- | --- |
| `>= 0` | Number of bytes written to `buf_ptr`. |
| `-1` | Generic error such as invalid pointers or a memory write failure. |
| `-2` | The provided buffer is too small for the latest unread message. That message remains unread for a later retry. |
| `-3` | The channel is empty. No sidecar message is available on this frame. |

The return convention for `mesh_asset_len`, `mesh_asset_read`, `scene_metadata_len`, and `scene_metadata_read` is:

| Return code | Meaning |
| --- | --- |
| `>= 0` | Number of bytes available or written. |
| `-1` | Generic error such as invalid pointers, invalid UTF-8 key, or write failure. |
| `-2` | The provided buffer is too small for the encoded mesh asset. |
| `-4` | No packaged asset exists for the requested key. |

Implementation details of the current runtime:

- Sidecar payloads are treated as raw bytes by the ABI. The terrain slide currently interprets them as UTF-8 JSON text.
- Packaged mesh asset keys come from manifest `assets.meshes` entries. The host uses `id` when present, otherwise `label`, then the file stem, then the raw package path.
- An `assets.meshes` entry with only `id` is runtime-visible to the guest but does not implicitly replace a `StaticMesh` slot.
- Packaged scene metadata keys come from imported scene ids. In practice this means manifest `assets.scenes[*].id` when present, otherwise the imported scene name, otherwise the `.glb` file stem.
- `scene_metadata_read` currently returns postcard-encoded `SceneAnchorSet` payloads. The first Epic 9 runtime surface is intentionally narrow and exposes authored anchors rather than a generic scene-query API.
- Authored anchor lookup ids use `vzglyd_id` when present. When `vzglyd_id` is absent, the runtime falls back to the Blender node name. If neither exists, the importer synthesizes `anchor_node_<index>` as a last-resort debug identifier.
- `vzglyd_anchor` marks a node as an anchor and, when it is a string, is preserved as `SceneAnchor.tag`. It is not the primary runtime lookup key.
- Slides may import the `vzglyd_host` functions listed above and may also rely on `wasi_snapshot_preview1` when compiled for `wasm32-wasip1`.
- There is no import whitelist beyond the linker itself. If a guest imports a symbol that the current runtime does not provide, the module is rejected at load time with a linker error.
- The checked-in Rust slide packages now all target the WASI-backed loader path.

The current runtime does not implement the older proposed `get_data_f64` and `get_data_str` host calls. If those are added in a later epic, this document will need a versioned update.

## Sidecar (optional)

A slide package may include a `sidecar.wasm` binary alongside the main slide module. When that file is present, the host starts the sidecar on a background thread for the lifetime of the slide and wires it to the main slide through a byte-message mailbox.

### When to use a sidecar

The main slide's `vzglyd_update` callback is part of the render loop. It stays short. The sidecar is where blocking work lives: sockets, sleep between polls, live data. The sidecar publishes new payloads to the main slide whenever data changes. Parked slides remain instantiated, but sidecars consult `channel_active()` to skip work until the slide is presented again.

### Sidecar imports

Sidecars receive their own `wasi_snapshot_preview1` store plus the following `vzglyd_host` import:

| Module | Function | Signature | Meaning |
| --- | --- | --- | --- |
| `vzglyd_host` | `channel_push` | `(ptr: i32, len: i32) -> i32` | Publish a message for the main slide, replacing any older unread payload. |
| `vzglyd_host` | `channel_active` | `() -> i32` | Return `1` when the slide is currently being presented and `0` when it is parked. |

The return convention for `channel_push` is:

| Return code | Meaning |
| --- | --- |
| `0` | The message was published successfully. |
| `-1` | Generic error such as invalid guest memory pointers. |

### WASI TCP socket extension

The sidecar linker additionally exposes the TCP socket functions listed below under `wasi_snapshot_preview1`.

| Function | Meaning |
| --- | --- |
| `sock_open(af, socktype, proto, fd_out*)` | Allocate a socket descriptor. |
| `sock_connect(fd, addr_ptr, addr_len)` | Connect to a remote TCP address. |
| `sock_send(fd, iovec_ptr, iovec_count, flags, written_out*)` | Send bytes. |
| `sock_recv(fd, iovec_ptr, iovec_count, flags, read_out*, roflags_out*)` | Receive bytes. |
| `sock_shutdown(fd, how)` | Shut down the connection. |

These follow the preview1-style socket-extension calling convention implemented by the current host runtime. The terrain sidecar is the canonical reference for this contract because it performs DNS-over-HTTPS, TLS, and HTTP polling through those imported socket calls plus `rustls`, rather than relying on higher-level guest networking shims.

### Sidecar entry points

The host accepts either of the following sidecar entry styles:

| Export | Meaning |
| --- | --- |
| `vzglyd_sidecar_run() -> i32` | Preferred explicit sidecar entry point. |
| `_start()` | Standard binary entry point when the sidecar is compiled as a `wasm32-wasip1` executable with `fn main()`. |

### Main-slide receive pattern

The main slide consumes sidecar output by polling the channel from `vzglyd_update`:

```rust
#[link(wasm_import_module = "vzglyd_host")]
unsafe extern "C" {
    fn channel_poll(buf_ptr: *mut u8, buf_len: i32) -> i32;
}

let mut buf = [0u8; 4096];
let n = unsafe { channel_poll(buf.as_mut_ptr(), buf.len() as i32) };
if n >= 0 {
    let msg = &buf[..n as usize];
    // process msg bytes
}
```

### Build-script convention

The host looks for `sidecar.wasm` in the same package directory as `slide.wasm`. A slide build script compiles the sidecar and copies it into that exact filename:

```bash
cargo build --manifest-path sidecar/Cargo.toml --target wasm32-wasip1 --release
cp sidecar/target/wasm32-wasip1/release/my-sidecar.wasm sidecar.wasm
```

## Wire format

`vzglyd_spec_ptr` and `vzglyd_spec_len` describe a versioned byte sequence:

1. Byte `0` is the wire-format version. The current value is `1`.
2. Bytes `1..` are a postcard-encoded `SlideSpec<V>`.

The loader rejects empty blobs and any wire-format version other than `1`.

## Lifecycle

Initialization happens once. The update loop runs at frame rate. Teardown is bounded to 100ms.

1. Load `manifest.json` and validate any manifest fields that are present.
2. Read the guest module, enforce the `10 MiB` wasm size cap, and prepare it for linker-based import resolution.
3. Instantiate the module with Wasmtime through the shared `wasi_snapshot_preview1` plus `vzglyd_host` linker.
4. Read `vzglyd_abi_version` and reject mismatches.
5. If present, call `vzglyd_init`.
6. Read `memory`, `vzglyd_spec_ptr`, and `vzglyd_spec_len`, then copy the versioned spec blob out of guest memory.
7. Decode postcard into `SlideSpec<V>`, validate the spec, and create GPU resources.
8. On each rendered frame, call `vzglyd_update(dt)` if present.
9. When `vzglyd_update` returns `1`, reread `RuntimeOverlay<V>` and-or `RuntimeMeshSet<V>` from guest memory if the corresponding pointer and length exports exist.
10. On runtime drop, call `vzglyd_teardown` if present and abort the call after `100 ms` if it does not complete.

## `SlideSpec` reference

The `SlideSpec` type lives in `vzglyd-slide/src/lib.rs`. The table below documents every field that the host currently decodes.

### Top-level `SlideSpec<V>`

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | `String` | Human-readable slide identifier used in logs and debugging. |
| `limits` | `Limits` | Resource budgets enforced before resource creation. |
| `scene_space` | `SceneSpace` | Selects the renderer path and shader contract. |
| `camera_path` | `Option<CameraPath>` | Optional world-camera animation path. |
| `shaders` | `Option<ShaderSources>` | Body-only WGSL sources that are validated against the engine prelude. Custom shaders are required by the current renderer. |
| `overlay` | `Option<RuntimeOverlay<V>>` | Optional static overlay submitted with the immutable spec. |
| `font` | `Option<FontAtlas>` | Optional RGBA8 font atlas used by screen-space slides and overlays. |
| `textures_used` | `u32` | Declared number of active textures. Must match `textures.len()`. |
| `textures` | `Vec<TextureDesc>` | Embedded texture payloads and sampling metadata. Manifest texture entries can replace the bytes at load time. |
| `static_meshes` | `Vec<StaticMesh<V>>` | Immutable geometry uploaded once. Package `assets.meshes` entries may replace the vertex and index payloads for declared slots. |
| `dynamic_meshes` | `Vec<DynamicMesh>` | Declared runtime mesh slots whose vertex data can be supplied later from guest memory. |
| `draws` | `Vec<DrawSpec>` | Ordered draw calls referencing static or dynamic meshes. |

### `Limits`

| Field | Type | Meaning |
| --- | --- | --- |
| `max_vertices` | `u32` | Maximum combined vertex budget across static and dynamic meshes. |
| `max_indices` | `u32` | Maximum combined index budget across static and dynamic meshes. |
| `max_static_meshes` | `u32` | Maximum number of static meshes. |
| `max_dynamic_meshes` | `u32` | Maximum number of dynamic mesh declarations. |
| `max_textures` | `u32` | Maximum number of textures. |
| `max_texture_bytes` | `u32` | Maximum total texture payload bytes. |
| `max_texture_dim` | `u32` | Maximum width or height of any texture. |

`Limits::pi4()` is the repository's baseline budget for Raspberry Pi 4-class hardware.

### `StaticMesh<V>`

| Field | Type | Meaning |
| --- | --- | --- |
| `label` | `String` | Debug label. |
| `vertices` | `Vec<V>` | Fully materialized vertex payload. |
| `indices` | `Vec<u16>` | Triangle index buffer. |

### `DynamicMesh`

| Field | Type | Meaning |
| --- | --- | --- |
| `label` | `String` | Debug label. |
| `max_vertices` | `u32` | Capacity reserved for runtime vertex uploads. |
| `indices` | `Vec<u16>` | Stable index order for the runtime mesh slot. |

### `DrawSpec`

| Field | Type | Meaning |
| --- | --- | --- |
| `label` | `String` | Debug label. |
| `source` | `DrawSource` | Mesh slot used by the draw. |
| `pipeline` | `PipelineKind` | Opaque or transparent pipeline choice. |
| `index_range` | `Range<u32>` | Half-open index range inside the referenced mesh. |

### `TextureDesc`

| Field | Type | Meaning |
| --- | --- | --- |
| `label` | `String` | Debug label. |
| `width` | `u32` | Width in texels. |
| `height` | `u32` | Height in texels. |
| `format` | `TextureFormat` | Texture pixel format. Only `Rgba8Unorm` exists today. |
| `wrap_u` | `WrapMode` | U-axis addressing mode. |
| `wrap_v` | `WrapMode` | V-axis addressing mode. |
| `wrap_w` | `WrapMode` | W-axis addressing mode; currently retained for consistency even though only 2D textures are used. |
| `mag_filter` | `FilterMode` | Magnification filter. |
| `min_filter` | `FilterMode` | Minification filter. |
| `mip_filter` | `FilterMode` | Mipmap filter. |
| `data` | `Vec<u8>` | Embedded RGBA8 bytes. The manifest can replace these bytes from package assets. |

### `ShaderSources`

| Field | Type | Meaning |
| --- | --- | --- |
| `vertex_wgsl` | `Option<String>` | Optional body-only vertex shader source. |
| `fragment_wgsl` | `Option<String>` | Optional body-only fragment shader source. |

The renderer rejects slides where both fields are absent unless the package compiles an authored scene through `assets.scenes`, in which case the loader may select the built-in default world-scene shader path.

### `CameraPath` and `CameraKeyframe`

| Field | Type | Meaning |
| --- | --- | --- |
| `CameraPath.looped` | `bool` | Whether the path loops. |
| `CameraPath.keyframes` | `Vec<CameraKeyframe>` | Ordered camera path samples. |
| `CameraKeyframe.time` | `f32` | Time in seconds; must be non-negative and strictly increasing. |
| `CameraKeyframe.position` | `[f32; 3]` | Camera position. |
| `CameraKeyframe.target` | `[f32; 3]` | Look target. |
| `CameraKeyframe.up` | `[f32; 3]` | Camera up vector. |
| `CameraKeyframe.fov_y_deg` | `f32` | Vertical field of view in degrees. |

### Runtime payload types

| Type | Fields | Meaning |
| --- | --- | --- |
| `RuntimeOverlay<V>` | `vertices: Vec<V>`, `indices: Vec<u16>` | Complete overlay mesh reread from guest memory. |
| `RuntimeMesh<V>` | `mesh_index: u32`, `vertices: Vec<V>`, `index_count: u32` | Vertex rewrite for one declared dynamic mesh slot. |
| `RuntimeMeshSet<V>` | `meshes: Vec<RuntimeMesh<V>>` | Batch of runtime mesh updates returned through `vzglyd_dynamic_meshes_ptr`. |
| `MeshAssetVertex` | `position`, `normal`, `tex_coords`, `color` | Decoded vertex payload used by packaged mesh assets exposed through the host ABI. |
| `MeshAsset` | `vertices: Vec<MeshAssetVertex>`, `indices: Vec<u16>` | Postcard-encoded packaged mesh payload returned by `mesh_asset_read`. |
| `SceneAnchor` | `id`, `label`, `node_name`, `tag`, `world_transform` | Runtime-visible authored anchor record imported from a packaged scene. `translation()` returns the world-space origin extracted from the transform. |
| `SceneAnchorSet` | `scene_id`, `scene_label`, `scene_name`, `anchors` | Postcard-encoded scene metadata payload returned by `scene_metadata_read`. `require_anchor()` returns a structured lookup error instead of silently falling back to another anchor. |

### Font atlas types

| Type | Fields | Meaning |
| --- | --- | --- |
| `FontAtlas` | `width`, `height`, `pixels`, `glyphs` | RGBA8 atlas plus glyph metadata. |
| `GlyphInfo` | `codepoint`, `u0`, `v0`, `u1`, `v1` | Glyph-to-atlas mapping. |

### Enums

| Enum | Variants | Meaning |
| --- | --- | --- |
| `SceneSpace` | `Screen2D`, `World3D` | Chooses the renderer path and shader contract. |
| `PipelineKind` | `Opaque`, `Transparent` | Chooses the pipeline blend state. |
| `DrawSource` | `Static(usize)`, `Dynamic(usize)` | References a static or dynamic mesh slot. |
| `TextureFormat` | `Rgba8Unorm` | Current texture format. |
| `WrapMode` | `Repeat`, `ClampToEdge` | Texture addressing modes. |
| `FilterMode` | `Nearest`, `Linear` | Texture filter modes. |

## Complete language examples

The repository contains complete examples in the maintained forms used by the current runtime, plus a narrow Epic 9 scene-metadata reference:

- Rust ABI example: [`slides/dashboard/src/lib.rs`](../../slides/dashboard/src/lib.rs) shows the minimal static-export pattern, and [`slides/flat/src/lib.rs`](../../slides/flat/src/lib.rs) extends it with `vzglyd_update` plus overlay exports.
- Rust scene metadata example: [`slides/courtyard/src/lib.rs`](../../slides/courtyard/src/lib.rs) imports `scene_metadata_len` and `scene_metadata_read`, decodes a `SceneAnchorSet`, and places a runtime marker at an authored Blender anchor.
- WAT ABI example: [`examples/minimal_screen_slide.wat`](examples/minimal_screen_slide.wat) is a hand-written module that embeds a valid versioned `SlideSpec` blob and exports the required entry points.

The terrain slide is the canonical reference for sidecar-backed live data because the main slide imports `vzglyd_host::channel_poll`, the packaged `sidecar.wasm` performs the network fetch, and the slide uses `vzglyd_dynamic_meshes_ptr` to refresh the BTC overlay text. The courtyard slide is the canonical reference for authored-scene metadata imports.

## ABI violations

These are the ways the ABI is violated.

`vzglyd_spec_ptr` and `vzglyd_spec_len` describe the version byte plus postcard payload. A slide that points them at only the postcard bytes has violated the wire format. The loader rejects it.

`textures_used` must exactly match `textures.len()`. A mismatch is a violation. The host does not reconcile the discrepancy.

Draw ranges are half-open and must stay inside the referenced mesh's index buffer. A range that exceeds the buffer is a violation. The spec validator rejects it before GPU resources are created.

Camera keyframes must be strictly increasing in time. Keyframes that repeat or reverse in time are a violation. The path is malformed.

The current renderer requires custom shaders. A slide that leaves `SlideSpec.shaders` empty has violated the shader contract. The load fails.

When `vzglyd_update` returns `1`, the corresponding runtime payload must already be present in guest memory at the moment the host rereads it. A slide that signals a change before the payload is written has violated the update contract. The host reads whatever bytes are there.
