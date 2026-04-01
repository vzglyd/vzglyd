/**
 * postcard.js — Binary postcard deserializer for vzglyd SlideSpec types.
 *
 * Postcard encoding rules used here:
 *   u8          → 1 byte
 *   u16/u32/u64 → unsigned LEB128 varint
 *   i16/i32/i64 → zigzag-encode then LEB128 varint
 *   f32         → 4 bytes little-endian (raw IEEE 754)
 *   bool        → 1 byte (0 = false, 1 = true)
 *   String      → varint byte-length + UTF-8 bytes
 *   Vec<T>      → varint element-count + sequential elements
 *   Option<T>   → 0x00 for None, 0x01 + T for Some
 *   [T; N]      → N sequential T values (no length prefix)
 *   enum        → varint discriminant + variant payload
 *   struct      → sequential fields in declaration order
 *
 * Wire format from the engine: [1-byte ABI version][postcard SlideSpec<V>]
 * This module skips the version byte — callers pass `bytes.slice(1)`.
 */

class PostcardDecoder {
  /** @param {Uint8Array} bytes */
  constructor(bytes) {
    this._buf = bytes;
    this._pos = 0;
    this._view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    this._td = new TextDecoder('utf-8');
  }

  get pos() { return this._pos; }
  get remaining() { return this._buf.length - this._pos; }

  // ── Primitives ────────────────────────────────────────────────────────────

  u8() {
    if (this._pos >= this._buf.length) throw new Error('postcard: unexpected end of input');
    return this._buf[this._pos++];
  }

  /** Unsigned LEB128 varint (up to 64-bit, returned as JS number — safe for u32). */
  uvarint() {
    let result = 0;
    let shift = 0;
    while (true) {
      const byte = this.u8();
      result |= (byte & 0x7f) << shift;
      if ((byte & 0x80) === 0) break;
      shift += 7;
      if (shift >= 35) {
        // Handle values > 2^35 (u64 range) — collect into BigInt then convert.
        // For u16/u32 in mesh data we never reach here, but be safe.
        let big = BigInt(result >>> 0);
        let bshift = BigInt(shift);
        while (true) {
          const b2 = this.u8();
          big |= BigInt(b2 & 0x7f) << bshift;
          if ((b2 & 0x80) === 0) break;
          bshift += 7n;
        }
        return Number(big);
      }
    }
    // Treat as unsigned 32-bit (>>> 0 converts signed int32 to uint32).
    return result >>> 0;
  }

  /** Zigzag + LEB128 signed varint. */
  ivarint() {
    const n = this.uvarint();
    // zigzag decode: (n >>> 1) ^ -(n & 1)
    return (n >>> 1) ^ -(n & 1);
  }

  f32() {
    const val = this._view.getFloat32(this._pos, /*littleEndian=*/true);
    this._pos += 4;
    return val;
  }

  bool() { return this.u8() !== 0; }

  string() {
    const len = this.uvarint();
    const slice = this._buf.subarray(this._pos, this._pos + len);
    this._pos += len;
    return this._td.decode(slice);
  }

  bytes() {
    const len = this.uvarint();
    const slice = this._buf.slice(this._pos, this._pos + len);
    this._pos += len;
    return slice;
  }

  /** Read a fixed-size array of f32 values (no length prefix). */
  f32Array(n) {
    const arr = new Float32Array(n);
    for (let i = 0; i < n; i++) arr[i] = this.f32();
    return arr;
  }

  /** Read Vec<T> — varint count then elements. */
  vec(readFn) {
    const count = this.uvarint();
    const arr = new Array(count);
    for (let i = 0; i < count; i++) arr[i] = readFn.call(this, this);
    return arr;
  }

  /** Read Option<T> — 0 = None, 1 = Some(T). */
  option(readFn) {
    const tag = this.u8();
    if (tag === 0) return null;
    return readFn.call(this, this);
  }
}

// ── Enum helpers ──────────────────────────────────────────────────────────────

function decodeSceneSpace(d) {
  const disc = d.uvarint();
  if (disc === 0) return 'Screen2D';
  if (disc === 1) return 'World3D';
  throw new Error(`postcard: unknown SceneSpace discriminant ${disc}`);
}

function decodeWrapMode(d) {
  const disc = d.uvarint();
  return disc === 0 ? 'Repeat' : 'ClampToEdge';
}

function decodeFilterMode(d) {
  const disc = d.uvarint();
  return disc === 0 ? 'Nearest' : 'Linear';
}

function decodeTextureFormat(d) {
  const disc = d.uvarint();
  if (disc === 0) return 'Rgba8Unorm';
  throw new Error(`postcard: unknown TextureFormat discriminant ${disc}`);
}

function decodePipelineKind(d) {
  const disc = d.uvarint();
  return disc === 0 ? 'Opaque' : 'Transparent';
}

function decodeDrawSource(d) {
  const disc = d.uvarint();
  const idx = d.uvarint();
  return { kind: disc === 0 ? 'Static' : 'Dynamic', index: idx };
}

// ── Struct decoders ───────────────────────────────────────────────────────────

function decodeLimits(d) {
  return {
    max_vertices:      d.uvarint(),
    max_indices:       d.uvarint(),
    max_static_meshes: d.uvarint(),
    max_dynamic_meshes:d.uvarint(),
    max_textures:      d.uvarint(),
    max_texture_bytes: d.uvarint(),
    max_texture_dim:   d.uvarint(),
  };
}

function decodeTextureDesc(d) {
  return {
    label:      d.string(),
    width:      d.uvarint(),
    height:     d.uvarint(),
    format:     decodeTextureFormat(d),
    wrap_u:     decodeWrapMode(d),
    wrap_v:     decodeWrapMode(d),
    wrap_w:     decodeWrapMode(d),
    mag_filter: decodeFilterMode(d),
    min_filter: decodeFilterMode(d),
    mip_filter: decodeFilterMode(d),
    data:       d.bytes(),
  };
}

function decodeScreenVertex(d) {
  return {
    position:   d.f32Array(3),
    tex_coords: d.f32Array(2),
    color:      d.f32Array(4),
    mode:       d.f32(),
  };
}

function decodeWorldVertex(d) {
  return {
    position: d.f32Array(3),
    normal:   d.f32Array(3),
    color:    d.f32Array(4),
    mode:     d.f32(),
  };
}

/** Pack a ScreenVertex array into a Float32Array (interleaved, GPU-ready). */
function packScreenVertices(verts) {
  // stride: position(3) + tex_coords(2) + color(4) + mode(1) = 10 f32 = 40 bytes
  const buf = new Float32Array(verts.length * 10);
  for (let i = 0; i < verts.length; i++) {
    const v = verts[i];
    const off = i * 10;
    buf[off + 0] = v.position[0];
    buf[off + 1] = v.position[1];
    buf[off + 2] = v.position[2];
    buf[off + 3] = v.tex_coords[0];
    buf[off + 4] = v.tex_coords[1];
    buf[off + 5] = v.color[0];
    buf[off + 6] = v.color[1];
    buf[off + 7] = v.color[2];
    buf[off + 8] = v.color[3];
    buf[off + 9] = v.mode;
  }
  return buf;
}

/** Pack a WorldVertex array into a Float32Array (interleaved, GPU-ready). */
function packWorldVertices(verts) {
  // stride: position(3) + normal(3) + color(4) + mode(1) = 11 f32 = 44 bytes
  const buf = new Float32Array(verts.length * 11);
  for (let i = 0; i < verts.length; i++) {
    const v = verts[i];
    const off = i * 11;
    buf[off + 0] = v.position[0];
    buf[off + 1] = v.position[1];
    buf[off + 2] = v.position[2];
    buf[off + 3] = v.normal[0];
    buf[off + 4] = v.normal[1];
    buf[off + 5] = v.normal[2];
    buf[off + 6] = v.color[0];
    buf[off + 7] = v.color[1];
    buf[off + 8] = v.color[2];
    buf[off + 9] = v.color[3];
    buf[off + 10] = v.mode;
  }
  return buf;
}

/**
 * Pack vertex array (already decoded) into GPU buffer bytes.
 * @param {Array} verts
 * @param {'Screen2D'|'World3D'} sceneSpace
 */
function packVertices(verts, sceneSpace) {
  return sceneSpace === 'Screen2D'
    ? packScreenVertices(verts)
    : packWorldVertices(verts);
}

/** Pack a u16 index array into a Uint16Array for WebGPU. */
function packIndices(indices) {
  const buf = new Uint16Array(indices.length);
  for (let i = 0; i < indices.length; i++) buf[i] = indices[i];
  return buf;
}

function decodeStaticMesh(d, sceneSpace) {
  const label    = d.string();
  const vertices = d.vec(sceneSpace === 'Screen2D' ? decodeScreenVertex : decodeWorldVertex);
  // Vec<u16> — each u16 is varint-encoded in postcard
  const indices  = d.vec(() => d.uvarint());
  return { label, vertices, indices };
}

function decodeDynamicMesh(d) {
  const label        = d.string();
  const max_vertices = d.uvarint();
  const indices      = d.vec(() => d.uvarint());
  return { label, max_vertices, indices };
}

function decodeDrawSpec(d) {
  const label       = d.string();
  const source      = decodeDrawSource(d);
  const pipeline    = decodePipelineKind(d);
  const index_start = d.uvarint();
  const index_end   = d.uvarint();
  return { label, source, pipeline, index_range: { start: index_start, end: index_end } };
}

function decodeShaderSources(d) {
  const vertex_wgsl   = d.option(function() { return this.string(); });
  const fragment_wgsl = d.option(function() { return this.string(); });
  return { vertex_wgsl, fragment_wgsl };
}

function decodeCameraKeyframe(d) {
  return {
    time:      d.f32(),
    position:  d.f32Array(3),
    target:    d.f32Array(3),
    up:        d.f32Array(3),
    fov_y_deg: d.f32(),
  };
}

function decodeCameraPath(d) {
  const looped    = d.bool();
  const keyframes = d.vec(decodeCameraKeyframe);
  return { looped, keyframes };
}

function decodeRuntimeOverlay(d, sceneSpace) {
  const vertices = d.vec(sceneSpace === 'Screen2D' ? decodeScreenVertex : decodeWorldVertex);
  const indices  = d.vec(() => d.uvarint());
  return { vertices, indices };
}

function decodeFontAtlas(d) {
  const width  = d.uvarint();
  const height = d.uvarint();
  const pixels = d.bytes();
  const glyphs = d.vec(function() {
    return {
      codepoint: this.uvarint(),
      u0: this.f32(),
      v0: this.f32(),
      u1: this.f32(),
      v1: this.f32(),
    };
  });
  return { width, height, pixels, glyphs };
}

function decodeDirectionalLight(d) {
  return {
    direction: d.f32Array(3),
    color:     d.f32Array(3),
    intensity: d.f32(),
  };
}

function decodeWorldLighting(d) {
  return {
    ambient_color:     d.f32Array(3),
    ambient_intensity: d.f32(),
    directional_light: d.option(decodeDirectionalLight),
  };
}

// ── Public entry point ────────────────────────────────────────────────────────

/**
 * Decode a full SlideSpec from postcard bytes.
 *
 * @param {Uint8Array} bytes  — the postcard payload (wire format minus the 1-byte ABI version)
 * @returns {object} decoded SlideSpec with all fields
 */
function decodeSlideSpec(bytes) {
  const d = new PostcardDecoder(bytes);

  const name       = d.string();
  const limits     = decodeLimits(d);
  const scene_space = decodeSceneSpace(d);

  // Fields in declaration order from vzglyd_slide::SlideSpec<V>:
  //   name, limits, scene_space, camera_path, shaders, overlay, font,
  //   textures_used, textures, static_meshes, dynamic_meshes, draws, lighting

  const camera_path = d.option(decodeCameraPath);
  const shaders     = d.option(decodeShaderSources);
  const overlay     = d.option(function() { return decodeRuntimeOverlay(d, scene_space); });
  const font        = d.option(decodeFontAtlas);

  const textures_used  = d.uvarint();
  const textures       = d.vec(decodeTextureDesc);
  const static_meshes  = d.vec(function() { return decodeStaticMesh(d, scene_space); });
  const dynamic_meshes = d.vec(decodeDynamicMesh);
  const draws          = d.vec(decodeDrawSpec);
  const lighting       = d.option(decodeWorldLighting);

  return {
    name,
    limits,
    scene_space,
    camera_path,
    shaders,
    overlay,
    font,
    textures_used,
    textures,
    static_meshes,
    dynamic_meshes,
    draws,
    lighting,
  };
}

/**
 * Decode a RuntimeOverlay<V> from a guest memory slice (postcard, no version byte).
 * @param {Uint8Array} bytes
 * @param {'Screen2D'|'World3D'} sceneSpace
 */
function decodeRuntimeOverlayBytes(bytes, sceneSpace) {
  const d = new PostcardDecoder(bytes);
  return decodeRuntimeOverlay(d, sceneSpace);
}

/**
 * Decode a RuntimeMeshSet<V> from a guest memory slice (postcard, no version byte).
 * @param {Uint8Array} bytes
 * @param {'Screen2D'|'World3D'} sceneSpace
 */
function decodeRuntimeMeshSet(bytes, sceneSpace) {
  const d = new PostcardDecoder(bytes);
  // RuntimeMeshSet { meshes: Vec<RuntimeMesh<V>> }
  // RuntimeMesh { mesh_index: u32, vertices: Vec<V>, index_count: u32 }
  const meshes = d.vec(function() {
    const mesh_index  = this.uvarint();
    const vertices    = this.vec(sceneSpace === 'Screen2D' ? decodeScreenVertex : decodeWorldVertex);
    const index_count = this.uvarint();
    return { mesh_index, vertices, index_count };
  });
  return { meshes };
}
