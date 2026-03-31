# Blender Scene Workflow

The courtyard slide is a Blender-authored world. This is the workflow that produces it. The Blender file is the authored source; the glTF export is the transfer; the manifest is the declaration. The checked-in reference package is [`slides/courtyard/`](../../slides/courtyard), which demonstrates a packaged `assets.scenes` import, an authored multi-camera glide, and a runtime marker that is positioned from a Blender-authored anchor instead of a hardcoded Rust transform.

## Reference Package Layout

The reference package directory is structured like this:

```text
slides/courtyard/
  Cargo.toml
  build.sh
  courtyard.json
  manifest.json -> courtyard.json
  courtyard.wasm
  slide.wasm -> courtyard.wasm
  assets/
    world.glb
  examples/
    export_package_assets.rs
  src/
    lib.rs
```

The runtime guest lives in `src/lib.rs`. The authored scene asset is `assets/world.glb`. The manifest declares that `.glb` through `assets.scenes` and lets the loader compile it into the existing `SlideSpec` structures. The package does not ship custom WGSL because authored scenes compiled with the `default_world` profile can use the built-in imported-scene shader path.

## Blender Scene Organization

The Blender file is where the world is decided. For the first version, keep the scene intentionally simple. Use mesh objects for visible geometry, camera objects for authored viewpoints, and empties for anchors or markers. The importer walks the default exported `glTF` scene recursively and ignores unsupported node kinds with warnings instead of guessing.

The authoring conventions that follow are the form the scene must take:

1. Use one exported scene root or collection for the content that belongs in the package.
2. Keep transforms static. Do not rely on armatures, constraints that require baking, or Blender light translation.
3. Place empties anywhere runtime code needs a named location, such as spawn points, effect emitters, or billboards.
4. Give nodes stable human-readable names even when you also set `vzglyd_id`, because the node name remains the fallback identifier and the main debug label.

## Units and Transforms

Blender authors normally work in metric units and with the default object transform tools. That is a good fit for this pipeline. The important rule is consistency rather than any single numeric scale. If two objects are meant to be one meter apart in the final world, keep that same relation in Blender and let the `glTF` exporter carry the transform into the packaged scene.

Blender itself is Z-up, while exported `glTF` scenes are represented in the coordinate system expected by the `glTF` format. VZGLYD consumes the exported transform data exactly as it appears in the `.glb`. The safe workflow is to trust the Blender `glTF` exporter for axis conversion instead of trying to compensate manually inside slide code.

## Custom Properties

Custom properties are how Blender speaks to VZGLYD. The author places them deliberately — each key is a declaration, and each value is a commitment that guest code will depend on. They are expressed through Blender custom properties and exported into `glTF` `extras`. The supported key set is documented in [`MANIFEST_PACKAGE_GUIDE.md`](MANIFEST_PACKAGE_GUIDE.md), but the keys that matter most for scene authoring are:

| Key | Typical use |
| --- | --- |
| `vzglyd_id` | Stable runtime lookup id for a mesh, camera, or anchor. |
| `vzglyd_anchor` | Marks an empty or node as an authored anchor. A string value is preserved as anchor metadata, but runtime lookup still prefers `vzglyd_id`. |
| `vzglyd_entry_camera` | Marks the preferred fixed camera when the manifest does not override `entry_camera`. Leave it unset when you want the importer to build a glide from multiple authored cameras. |
| `vzglyd_material` | Selects the default imported-scene material class such as `opaque` or `water`. |
| `vzglyd_pipeline` | Requests opaque or transparent draw routing. |

The checked-in reference package uses these specific values:

1. The scene asset id is `courtyard`.
2. The first authored camera id is `overview`.
3. The authored runtime anchor id is `spawn_marker`.
4. The anchor tag is `spawn`.

Because runtime anchor lookup uses `vzglyd_id` first and the node name second, a missing `vzglyd_id` changes how guest code must query the anchor. The simplest rule is to set `vzglyd_id` on every anchor you expect runtime code to consume.

## Blender Export Settings

Export the scene as a binary `glTF` file (`.glb`). Keep the export self-contained so that buffers are embedded in the file rather than referenced externally. Cameras and custom properties must be included — the importer relies on both. If the exporter offers an explicit custom-properties or extras toggle, enable it. If the exporter offers a cameras toggle, enable that as well.

For the current implementation, avoid exporting animation tracks, skinning, or unsupported light setups as if the engine will replay them. The importer may still parse the file, but those features are outside the supported Epic 9 contract and are therefore not part of the reproducible reference workflow.

## Manifest Wiring

The manifest is the declaration that makes the authored scene first-class. The reference package manifest is [`slides/courtyard/courtyard.json`](../../slides/courtyard/courtyard.json). The scene section is intentionally small:

```json
"assets": {
  "scenes": [
    {
      "path": "assets/world.glb",
      "id": "courtyard",
      "label": "Courtyard",
      "compile_profile": "default_world"
    }
  ]
}
```

That manifest entry is what makes the authored scene first-class. The host loads `world.glb`, imports mesh nodes, cameras, and anchors, compiles the visible geometry into `SlideSpec`, and separately exposes the anchor metadata to guest code through the scene-metadata ABI. Because the manifest does not pin a single `entry_camera`, the importer uses the authored camera nodes in export order to synthesize a looping glide through the courtyard.

## Runtime Anchor Consumption

The guest-side reference implementation is [`slides/courtyard/src/lib.rs`](../../slides/courtyard/src/lib.rs). It imports `vzglyd_host::scene_metadata_len` and `vzglyd_host::scene_metadata_read`, decodes a postcard `SceneAnchorSet`, resolves the `spawn_marker` anchor, and emits one runtime mesh positioned at that authored transform. This is the critical end-to-end proof that Blender-authored anchors are useful for live slide logic and not only for static compilation.

If the anchor key is missing, the reference slide fails during `vzglyd_init` instead of silently binding the wrong location. That behavior is deliberate. Runtime-authored scene metadata should fail loudly when the authored package and guest logic drift out of sync.

## Build, Run, and Pack

From the repository root, build the reference package assets and wasm like this:

```bash
cd slides/courtyard
./build.sh
```

Then run it through the engine:

```bash
cargo run --manifest-path ../../Cargo.toml -- --scene slides/courtyard
```

To verify archive loading as well, pack it and run the archive directly:

```bash
cargo run --manifest-path ../../Cargo.toml -- pack slides/courtyard -o /tmp/courtyard.vzglyd
cargo run --manifest-path ../../Cargo.toml -- --scene /tmp/courtyard.vzglyd
```

## Reproducing the Reference From Blender

The whole path — authored layout in Blender, packaged `assets.scenes` import in the manifest, host-side scene compilation, guest-side runtime anchor lookup — can be reproduced from scratch. Create a small courtyard-style world scene with multiple obvious landmarks such as a ground plane, a path, a raised stage, a pool, and a back wall. Add several cameras in the order you want the glide to traverse them, with the first camera using `vzglyd_id = "overview"` so the opening shot remains stable for tests and documentation. Do not set `entry_camera` in the manifest and do not tag a camera with `vzglyd_entry_camera` if you want the importer to compile the whole authored camera sequence into a loop. Add one empty that serves as the runtime anchor on or near the stage, with `vzglyd_id = "spawn_marker"` and `vzglyd_anchor = "spawn"`. Export the scene as `world.glb`, place it in `assets/`, and keep the manifest `id` as `courtyard` so the reference runtime guest continues to request the correct scene metadata key.

That workflow reproduces the whole path used by the checked-in package.
