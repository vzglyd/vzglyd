# VZGLYD Slide Authoring

This directory is the formal study of the slide form. The documents here describe interfaces that are implemented in the current codebase. The host-import surface includes `vzglyd_host::channel_poll` for main slides, `vzglyd_host::channel_push` for optional sidecars, packaged mesh asset reads, packaged scene metadata reads, and the shared `wasi_snapshot_preview1` linker surface used for `wasm32-wasip1` slides. The older proposed generic key-value data-provider ABI is not present in the runtime.

`wasm32-wasip1` is the supported target for both Raspberry Pi 4-class deployments and desktop Linux. Older `wasm32-unknown-unknown` modules are legacy artifacts from the pre-unification loader split. They lack the WASI contract that the current runtime standardizes on. The remaining guides in this directory use `wasm32-wasip1` in both explanations and copy-paste build commands.

The form is learned in this sequence:

1. [`ABI_REFERENCE.md`](ABI_REFERENCE.md) — the exported functions, optional runtime hooks, wire format, and `SlideSpec` type system. This is the law of the machine boundary. It is read first.
2. [`MANIFEST_PACKAGE_GUIDE.md`](MANIFEST_PACKAGE_GUIDE.md) — package layout, manifest fields, asset and shader overrides, and `.vzglyd` archive packing.
3. [`BLENDER_SCENE_WORKFLOW.md`](BLENDER_SCENE_WORKFLOW.md) — the implemented Epic 9 Blender-to-`GLB` scene path and the checked-in authored-scene reference package.
4. [`SHADER_AUTHORING_GUIDE.md`](SHADER_AUTHORING_GUIDE.md) — the WGSL contract that custom slide shaders must satisfy.
5. [`END_TO_END_TUTORIAL.md`](END_TO_END_TUTORIAL.md) — a from-scratch Rust slide walkthrough that mirrors the current `slides/flat/` implementation and then extends it toward sidecar-backed dynamic data.

The repository-backed examples referenced throughout these guides are:

- Rust static screen slide: [`slides/dashboard/src/lib.rs`](../../slides/dashboard/src/lib.rs)
- Rust dynamic overlay slide: [`slides/flat/src/lib.rs`](../../slides/flat/src/lib.rs)
- Rust world slide with host-backed dynamic mesh updates: [`slides/terrain/src/lib.rs`](../../slides/terrain/src/lib.rs)
- Rust authored-scene reference slide: [`slides/courtyard/src/lib.rs`](../../slides/courtyard/src/lib.rs)
- Hand-written WAT ABI shim: [`examples/minimal_screen_slide.wat`](examples/minimal_screen_slide.wat)
