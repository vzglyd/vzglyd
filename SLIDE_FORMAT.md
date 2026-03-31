# VZGLYD Slide Package Format

The slide package is the unit of authorship in VZGLYD. This document defines its canonical on-disk form. The format is not a convention — it is the contract between the author and the loader. Every slide that runs in VZGLYD is a slide package. The loader does not guess at structure. The structure is this.

For a field-by-field authoring guide that matches the current runtime, use [docs/slide-authoring/MANIFEST_PACKAGE_GUIDE.md](docs/slide-authoring/MANIFEST_PACKAGE_GUIDE.md). This root document is the format. The other document explains it.

## Canonical Layout

```text
my_slide/
  manifest.json
  slide.wasm
  shaders/
    vertex.wgsl
    fragment.wgsl
  assets/
    noise.png
    font_atlas.png
    world.glb
```

`manifest.json` and `slide.wasm` are required. There is no slide without them. The `shaders/` and `assets/` directories are optional. When present, all package-relative resources are resolved from the package root directory. The package root is the origin from which all paths are measured.

## Manifest Schema

The manifest schema the package loader operates against is:

```json
{
  "name": "Terrain (Rust)",
  "version": "1.0.0",
  "author": "VZGLYD",
  "description": "A procedural terrain with cel shading",
  "abi_version": 1,
  "scene_space": "world_3d",
  "assets": {
    "textures": [
      { "path": "assets/noise.png", "usage": "material" },
      { "path": "assets/font_atlas.png", "usage": "font" }
    ],
    "meshes": [
      { "path": "assets/kart.glb", "label": "kart_body", "id": "kart_model" }
    ],
    "scenes": [
      {
        "path": "assets/world.glb",
        "id": "hero_world",
        "label": "Hero World",
        "entry_camera": "overview",
        "compile_profile": "default_world"
      }
    ]
  },
  "shaders": {
    "vertex": "shaders/vertex.wgsl",
    "fragment": "shaders/fragment.wgsl"
  },
  "display": {
    "duration_seconds": 20,
    "transition_in": "crossfade",
    "transition_out": "dissolve"
  },
  "requirements": {
    "min_texture_dim": 128,
    "uses_depth_buffer": true,
    "uses_transparency": true
  }
}
```

All fields after `description` are optional. Minimal manifests parse. `abi_version`, `scene_space`, asset paths, shader paths, and `display.duration_seconds` are validated by the loader. The transition preference schema is nested under `display`. The older top-level transition manifest shape does not exist in this format.

`assets.textures` names the textures the slide carries. The loader resolves each path relative to the package root and uses the referenced file bytes in place of the corresponding embedded `SlideSpec` texture payload. Texture entries are matched in manifest order against the existing `SlideSpec.textures` entries so the slide continues to inherit labels, sampler settings, and other texture metadata from the WASM spec. PNG files and raw RGBA8 files with `.rgba` or `.rgba8` extensions are supported. Raw RGBA8 assets inherit width and height from the corresponding embedded texture descriptor.

`assets.meshes` names the meshes the slide carries. The loader resolves each `.glb` path relative to the package root, flattens the `GLB` scene into one mesh payload, and makes it available to the runtime host ABI. Entries that name a static target by `slot`, `label`, or implicit order replace the targeted embedded `SlideSpec.static_meshes` entry. Entries with only `id` are runtime assets and do not implicitly override static geometry. The current importer applies node transforms and reads positions, normals, and optional vertex colors. It does not yet consume skinned animation or textured materials.

`assets.scenes` names the Blender-authored scene inputs for Epic 9. Each scene path is a self-contained `.glb` file in the package. The current loader validates and packs these files but does not yet compile them into `SlideSpec`; later Epic 9 tickets use the same schema to import mesh nodes, cameras, empties, and scene metadata deterministically. The v1 scene contract uses Blender custom properties exported through `glTF` `extras`, with flat keys such as `vzglyd_id`, `vzglyd_material`, `vzglyd_anchor`, and `vzglyd_entry_camera`. When that metadata is absent, the importer falls back to stable node names, material names as material-class hints, and the default `glTF` scene.

`shaders` names the WGSL files that override the corresponding embedded shader source in the `SlideSpec`. A path that is absolute, uses `..`, or resolves through a symlink outside the package root is rejected. There is no path outside the package root.

## Loader Behavior

`slide_loader::load_slide_from_wasm()` accepts three input forms. The input form determines the loading path. There is no other loading path.

- A package directory such as `slides/terrain/`. The loader resolves `manifest.json` and `slide.wasm` from the directory root. This is the canonical form.
- A bare `.wasm` file path such as `slides/terrain/terrain_slide.wasm`. The legacy sibling-manifest rule applies: the loader looks for `slides/terrain/terrain_slide.json`.
- A `.vzglyd` archive such as `dist/terrain.vzglyd`. The loader extracts the zip archive into a temporary package root and then follows the package-directory loading path.

The package directory is the preferred entry point for all future work. The `.wasm` path form exists for backward compatibility. The `.vzglyd` form is the distribution form.

## CLI Behavior

The runtime entrypoint is `--scene <path>`. Built-in aliases such as `terrain` and `dashboard` resolve to their package directories internally. Slide loading goes through the path-based package loader regardless of how the slide is named. The directory form is the preferred invocation because it matches the canonical layout and places assets and shaders beside the slide binary where they belong.

Single-file archives are produced with:

```bash
cargo run -- pack slides/terrain -o dist/terrain.vzglyd
```

The `.vzglyd` format is a standard zip archive written with stored entries. It is inspectable with ordinary zip tooling. The archive overhead stays under the E4 target threshold.

## Current Migration Status

`slides/terrain/` conforms to the canonical package layout: `manifest.json` and `slide.wasm` sit at the package root. The legacy `terrain_slide.json` and `terrain_slide.wasm` paths remain in place for backward compatibility.
