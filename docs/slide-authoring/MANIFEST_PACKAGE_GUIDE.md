# Manifest and Package Guide

The manifest is the contract between the slide and the loader. It declares what the slide is, what assets it needs, how its shaders are supplied, and how long it wants to be visible. The loader reads the manifest, validates its fields, resolves its asset paths against the package root, and refuses what the contract does not permit. The normative loader behavior lives in `src/slide_loader.rs` and `src/slide_manifest.rs`. [`SLIDE_FORMAT.md`](../../SLIDE_FORMAT.md) at the repository root is the short format summary. This document expands that summary with field-by-field notes, current defaults, and caveats where the runtime stores metadata but does not yet act on it.

## Canonical package layout

The form a slide takes when it is complete is:

```text
my_slide/
  manifest.json
  slide.wasm
  assets/
    ...
  shaders/
    ...
```

Only `manifest.json` and `slide.wasm` are required. The current checked-in slides often keep legacy filenames such as `flat_slide.wasm` or `terrain_slide.json` and expose the canonical names as symlinks. The loader resolves the canonical paths from the package root, so both forms are accepted.

## Accepted entry forms

The runtime accepts three entry forms. A package directory such as `slides/flat`. A bare wasm path such as `slides/flat/flat_slide.wasm`, in which case the loader looks for a sibling JSON file with the same stem. A `.vzglyd` archive such as `dist/flat.vzglyd`, which the loader extracts to a temporary cache directory and then loads as a package directory. The package-directory form is the preferred authoring target.

## Manifest schema

The JSON manifest maps to `SlideManifest` in `src/slide_manifest.rs`.

### Top-level fields

| Field | Type | Default | Meaning | Current runtime use |
| --- | --- | --- | --- | --- |
| `name` | `string` | omitted | Human-readable slide name. | Logged and exposed to the UI title path. |
| `version` | `string` | omitted | Slide version label. | Logged only. |
| `author` | `string` | omitted | Slide author label. | Logged only. |
| `description` | `string` | omitted | Free-form description. | Logged only. |
| `abi_version` | `number` | omitted | Declared ABI version. Must equal `1` when present. | Validated only. Runtime ABI authority remains `vzglyd_abi_version`. |
| `scene_space` | `"screen_2d"` or `"world_3d"` | omitted | Declared scene type. | Syntax-validated only. Rendering still follows `SlideSpec.scene_space`. |
| `assets` | object | omitted | External asset overrides and packaged scene inputs. | Used to replace embedded texture bytes and static mesh geometry, and to compile authored scene `GLB` files into world-slide `SlideSpec` data. |
| `shaders` | object | omitted | External shader overrides. | Used to replace embedded WGSL bodies. |
| `display` | object | omitted | Display and transition metadata. | Transition fields are used; duration is currently only validated and stored. |
| `requirements` | object | omitted | Capability metadata. | Stored but not yet enforced. |

### `assets`

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `textures` | array of `AssetRef` | `[]` | Package-relative texture paths that replace the embedded `SlideSpec.textures` payloads. |
| `meshes` | array of `AssetRef` | `[]` | Package-relative `GLB` paths that replace embedded `SlideSpec.static_meshes` geometry. |
| `scenes` | array of `SceneAssetRef` | `[]` | Package-relative Blender-authored scene `GLB` files that compile into existing world-slide `SlideSpec` structures. |

Each `AssetRef` has:

| Field | Type | Default | Meaning | Current runtime use |
| --- | --- | --- | --- | --- |
| `path` | `string` | required | Package-relative file path. | Required and validated. |
| `usage` | `string` | omitted | Author hint such as `"font"` or `"material"`. | Not interpreted by the loader yet. |
| `slot` | `number` | omitted | Explicit `SlideSpec.textures` or `SlideSpec.static_meshes` slot to replace. | Used when present. |
| `label` | `string` | omitted | Explicit `TextureDesc.label` or `StaticMesh.label` to replace. | Used when `slot` is absent. |
| `id` | `string` | omitted | Guest-visible runtime asset key for packaged mesh lookup. | Used by `vzglyd_host::mesh_asset_len` and `vzglyd_host::mesh_asset_read`. |

Each `SceneAssetRef` has:

| Field | Type | Default | Meaning | Current runtime use |
| --- | --- | --- | --- | --- |
| `path` | `string` | required | Package-relative path to a self-contained Blender-exported `.glb` scene. | Resolved during package load, packed into archives, and imported for scene compilation. |
| `label` | `string` | omitted | Human-readable scene label for tooling and diagnostics. | Used as the compiled `SlideSpec.name` fallback when present. |
| `id` | `string` | omitted | Stable scene identifier for later compiler and runtime metadata surfaces. | Used as the imported scene identifier and as the name fallback when no label is present. |
| `entry_camera` | `string` | omitted | Preferred authored camera name or `vzglyd_id` for the initial world view. | Used during scene compilation to pin a specific authored camera when present. |
| `compile_profile` | `string` | omitted | Scene compilation profile. The standardized v1 default is `default_world`. | Used during scene compilation. Unsupported values are rejected. |

Texture overrides are matched by explicit `slot`, by `label`, or implicitly by order when neither selector is present. `slot` takes priority over `label` when both are present. Implicit entries skip slots that were already claimed by an explicit selector, so mixed manifests remain deterministic. Supported file types are `.png`, `.rgba`, and `.rgba8`. Raw `.rgba` and `.rgba8` files inherit width and height from the embedded `TextureDesc`. A path that escapes the package root after symlink resolution is rejected. The extra screen-space texture slots map cleanly to detail or lookup textures; the extra world-space slots map cleanly to custom material maps such as masks, foam lookups, or packed surface data.

Mesh overrides are matched by explicit `slot`, by `label`, or implicitly by order. A mesh entry that carries only `id` is treated as a runtime-addressable packaged asset and does not implicitly replace a `StaticMesh`. A mesh entry may combine `id` with `slot` or `label` when the same packaged asset should both override a declared static slot and remain visible to guest runtime code. The current mesh importer supports `.glb` package assets only. Imported mesh overrides replace only `StaticMesh.vertices` and `StaticMesh.indices`; labels and draw bindings stay embedded in the slide spec. The importer flattens the `GLB` scene, applies node transforms, and imports triangle primitives. Imported vertex colors are used when present; otherwise the embedded mesh's first vertex color and mode are reused as fallback metadata. This path is intended for static geometry. Animated, skinned, or material-textured `glTF` data is not consumed by the current renderer. Guest-visible runtime lookup keys are derived from `id` when present, otherwise `label`, then the mesh file stem, then the raw package path.

`assets.scenes` establishes Blender-exported binary `glTF` (`.glb`) as the supported interchange for authored scenes in Epic 9. Scene entries are package-scoped resources, not mesh overrides. The loader validates their paths, includes them in `.vzglyd` archives, imports the selected scene, and compiles mesh nodes, cameras, and default textures into the loaded world `SlideSpec`. `compile_profile` is intentionally narrow in v1 — omitted values mean `default_world`, and other names are rejected rather than treated as ad hoc per-slide behavior. `entry_camera` selects a preferred authored camera by stable identifier; compilation resolves this field first against `vzglyd_id`, then against the raw Blender node name or camera name if no `vzglyd_id` matches.

### Blender scene contract

Epic 9 defines a deterministic contract for Blender-authored scenes before the importer is implemented. The supported source is a self-contained binary `glTF` file exported from Blender as `.glb`. The contract assumes the package owns the full scene blob, so external buffers and external images are outside the scene path. This keeps the authored-scene boundary stable for packaging, caching, and future compiler passes. The purpose of the contract is not to preserve every Blender feature, but to define the subset that VZGLYD consumes predictably.

The node mapping is:

| Blender-exported node kind | VZGLYD interpretation in Epic 9 v1 |
| --- | --- |
| Mesh object | Scene mesh node that later compiler stages convert into `StaticMesh` data and draw ownership. |
| Camera object | Authored camera that later compiler stages may convert into an imported entry camera or camera path seed. |
| Empty object | Anchor or marker node used for named positions, spawn points, billboards, and other scene metadata. |
| Other node kinds | Ignored with a warning so unsupported content is explicit rather than silently reinterpreted. |

Metadata travels through Blender custom properties exported into `glTF` `extras`. The initial key set is flat on purpose because Blender custom properties are easier to manage when they do not require nested JSON structures:

| Custom property key | Meaning |
| --- | --- |
| `vzglyd_id` | Stable identifier for a node. When present, it takes precedence over the raw Blender object name. |
| `vzglyd_pipeline` | Requested pipeline classification for later compilation, such as opaque or transparent routing. |
| `vzglyd_material` | Author hint for material class selection in later compiler stages. |
| `vzglyd_anchor` | Marks an empty or node as an authored anchor. When this value is a string, the runtime preserves it as anchor metadata, but the stable lookup id still comes from `vzglyd_id` first and the node name second. |
| `vzglyd_hidden` | Excludes a node from default scene compilation while keeping it available as metadata. |
| `vzglyd_billboard` | Marks a node for billboard treatment in later compiler stages. |
| `vzglyd_entry_camera` | Marks a camera as the preferred default when the manifest does not provide `entry_camera`. |

The fallback rules are part of the contract. When `vzglyd_id` is absent, the raw Blender node name remains the stable identifier, including for runtime anchor lookup. When `vzglyd_material` is absent, the Blender material name becomes the default material-class hint. When `entry_camera` is absent from the manifest, later compiler stages resolve the first camera tagged with `vzglyd_entry_camera`, and if none is tagged they compile all visible cameras in export order into a looping path. When no explicit scene selector is present, the default `glTF` scene is the one imported. These rules are written to keep two independent implementations aligned on the same authored input.

The v1 limit set is deliberately explicit. Only triangle meshes are in scope. Only static transforms are in scope — no skeletal animation, no skinning, no time-varying Blender constraints. Blender light objects are not translated into runtime lighting. Arbitrary Blender shader-node graphs are not translated into WGSL or material graphs. These limits narrow the problem enough that future compiler stages can be deterministic rather than heuristic.

### `shaders`

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `vertex` | `string` | omitted | Package-relative WGSL body that replaces `SlideSpec.shaders.vertex_wgsl`. |
| `fragment` | `string` | omitted | Package-relative WGSL body that replaces `SlideSpec.shaders.fragment_wgsl`. |

Screen slides and ordinary world slides still require a custom shader body after manifest overrides are applied. World slides compiled from `assets.scenes` may instead use the built-in imported-scene shader when no custom WGSL is supplied.

### `display`

| Field | Type | Default | Meaning | Current runtime use |
| --- | --- | --- | --- | --- |
| `duration_seconds` | `number` | omitted | Requested display duration between `1` and `300` seconds. | Validated only. The scheduler still rotates slides every fixed `20` seconds. |
| `transition_in` | `string` | omitted | Preferred incoming transition. | Used when the outgoing slide does not specify `transition_out`. |
| `transition_out` | `string` | omitted | Preferred outgoing transition. | Takes priority over the incoming slide's `transition_in`. |

The accepted transition strings are `crossfade`, `wipe_left`, `wipe_down`, `dissolve`, and `cut`. Unknown values log a warning and fall back to `crossfade`.

### `requirements`

| Field | Type | Default | Meaning | Current runtime use |
| --- | --- | --- | --- | --- |
| `min_texture_dim` | `number` | omitted | Metadata describing a preferred texture floor. | Stored only. |
| `uses_depth_buffer` | `boolean` | omitted | Metadata describing whether the slide expects depth usage. | Stored only. |
| `uses_transparency` | `boolean` | omitted | Metadata describing whether the slide expects transparency. | Stored only. |

These fields are forward-looking metadata. They are not yet part of capability negotiation.

## Validation rules

The loader validates these fields. An absent `abi_version` is accepted without default; when present, it must match engine ABI `1`. A `scene_space` value, when present, must be `screen_2d` or `world_3d`. A path that is absolute, contains `..`, or escapes the package root through symlinks is rejected. A `display.duration_seconds` value, when present, must be between `1` and `300`. The loader also enforces a separate `10 MiB` cap on the wasm file itself.

## External assets and overrides

Manifest texture assets replace the texture bytes, not the `TextureDesc` metadata. Labels, dimensions for raw RGBA assets, wrap modes, and filter modes continue to come from the embedded `SlideSpec`.

Manifest mesh assets replace the targeted `StaticMesh` vertex and index buffers, not the draw definitions or slide limits. The embedded slide declares the intended mesh slots and draw ownership. The package provides authored geometry for those slots.

Manifest scene assets mutate the loaded world `SlideSpec` when present. The compiler replaces static scene geometry with imported mesh nodes, selects an authored or synthesized camera path, preserves dynamic draws that already exist in the slide, and installs the built-in imported-scene shader path when no custom WGSL is supplied.

The terrain slide is the canonical example of this pattern:

- Manifest: [`slides/terrain/terrain_slide.json`](../../slides/terrain/terrain_slide.json)
- Asset exporter: [`slides/terrain/examples/export_package_assets.rs`](../../slides/terrain/examples/export_package_assets.rs)

The stable workflow is: build and validate the slide with embedded texture descriptors first, then move the large texture payloads into package assets. Add `assets.textures` entries in the same order as `SlideSpec.textures`, or target them explicitly with `slot` or `label` once the package grows beyond a trivial texture list. For authored scenes, declare an `assets.scenes` `.glb` entry and the loader compiles it into static meshes, ordered draws, textures, and a camera path. For guest-driven runtime geometry, give a packaged mesh an `id` and fetch it through the host mesh-asset ABI instead of generating every vertex procedurally.

## Shader file placement

The loader does not require a specific directory name for shader overrides. The repository convention is:

```text
my_slide/
  shaders/
    fragment.wgsl
    vertex.wgsl
```

The manifest refers to those files with package-relative paths such as `shaders/fragment.wgsl`.

## Building packages

### Rust `wasm32-wasip1`

Rust slides are built for `wasm32-wasip1`. The older `wasm32-unknown-unknown` target produces a bare module with no standard WASI contract, which forced VZGLYD to maintain a second loader model. The current runtime is unified around a single WASI-backed path so that the same package structure, linker surface, and debugging workflow hold on both desktop Linux and Raspberry Pi 4-class hardware.

The checked-in Rust slides follow this pattern:

```bash
rustup target add wasm32-wasip1
cd slides/flat
./build.sh
```

The build script emits `flat_slide.wasm` and refreshes the canonical `slide.wasm` and `manifest.json` symlinks.

### Packing a `.vzglyd` archive

The engine binary packs a package directory:

```bash
cargo run -- pack slides/flat -o dist/flat.vzglyd
```

The resulting archive is a standard zip file using stored entries. The packer includes `manifest.json`, `slide.wasm`, every texture path declared in `assets.textures`, every mesh path declared in `assets.meshes`, every scene path declared in `assets.scenes`, and any shader paths declared in `shaders.vertex` and `shaders.fragment`.

## Testing a package

There is no standalone validation CLI. The current validation path is to load the package through the engine:

```bash
cargo run -- --scene slides/flat
```

For archive testing, the packed `.vzglyd` path loads the same way:

```bash
cargo run -- --scene dist/flat.vzglyd
```

When the package fails validation, the loader reports a manifest, wasm, asset, archive, or spec-decode error before rendering begins.

## Example manifests

Useful concrete examples in the repository are:

- Minimal static manifest: [`slides/dashboard/dashboard_slide.json`](../../slides/dashboard/dashboard_slide.json)
- Minimal screen-space manifest: [`slides/flat/flat_slide.json`](../../slides/flat/flat_slide.json)
- Full package manifest with assets and transitions: [`slides/terrain/terrain_slide.json`](../../slides/terrain/terrain_slide.json)

For ABI and runtime details beyond package metadata, continue with [`ABI_REFERENCE.md`](ABI_REFERENCE.md).
