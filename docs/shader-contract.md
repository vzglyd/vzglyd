# Shader Contract

This document is the documentation entry point for the shader ABI used by VZGLYD slides.

## Summary

- Slides provide WGSL source that plugs into the engine's pipeline contract.
- The engine supplies the reserved bindings, uniforms, and render pipeline layout.
- Slides define `vs_main`, `fs_main`, and any helper functions they need.
- Slides do not create storage buffers, additional bind groups, or compute shaders.

## Scene spaces

- `Screen2D`: screen-aligned slides with overlay-style vertex formats and texture sampling.
- `World3D`: world-space slides with camera, lighting, and authored scene support.

## Texture slots

Slides are limited to four texture slots and must stay within the `Limits::pi4()` texture budget.

## Full reference

The full shader contract continues to live in the root repository document:

- [SHADER_CONTRACT.md](../SHADER_CONTRACT.md)

Use this file for README and authoring-guide links; use the root document for the complete contract text.
