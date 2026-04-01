/**
 * wasm-host.js — Browser-side host imports for vzglyd slide WASM modules.
 *
 * Implements two import namespaces:
 *   wasi_snapshot_preview1  — WASI preview1 stubs (enough for wasm32-wasip1 slides)
 *   vzglyd_host             — vzglyd engine host functions
 *
 * Slides compiled with wasm32-wasip1 call proc_exit(0) at the end of _start().
 * We catch this as a ProcExitError and treat exit code 0 as a clean exit.
 */

class ProcExitError extends Error {
  constructor(code) {
    super(`proc_exit(${code})`);
    this.code = code;
  }
}

// WASI errno constants
const WASI_ESUCCESS   = 0;
const WASI_EBADF      = 8;
const WASI_EINVAL     = 28;
const WASI_ENOSYS     = 52;

// Clock IDs
const CLOCK_REALTIME  = 0;
const CLOCK_MONOTONIC = 1;

class VzglydWasmHost {
  constructor() {
    /** @type {WebAssembly.Instance|null} */
    this._instance = null;
    this._memory   = null;
    this._startMs  = performance.now();
  }

  /** Call after WebAssembly.instantiate to hand the instance back. */
  setInstance(instance) {
    this._instance = instance;
    this._memory   = instance.exports.memory;
  }

  // ── Memory helpers ──────────────────────────────────────────────────────────

  _memView() {
    // The memory DataView must be re-fetched after any growth.
    return new DataView(this._memory.buffer);
  }

  _memU8() {
    return new Uint8Array(this._memory.buffer);
  }

  /** Read `len` bytes starting at guest address `ptr`. */
  _readBytes(ptr, len) {
    return new Uint8Array(this._memory.buffer, ptr, len).slice();
  }

  /** Read a UTF-8 string from guest memory. */
  _readString(ptr, len) {
    return new TextDecoder().decode(new Uint8Array(this._memory.buffer, ptr, len));
  }

  /** Write `data` bytes into guest memory at `ptr`. Returns bytes written. */
  _writeBytes(ptr, data) {
    const mem = this._memU8();
    mem.set(data, ptr);
    return data.length;
  }

  // ── Public vzglyd exports ───────────────────────────────────────────────────

  /**
   * Run the slide's _start() (WASI command entry).
   * Treats proc_exit(0) as success; re-throws other exit codes.
   */
  runStart() {
    const startFn = this._instance.exports._start;
    if (!startFn) return;
    try {
      startFn();
    } catch (e) {
      if (e instanceof ProcExitError) {
        if (e.code !== 0) throw new Error(`slide _start exited with code ${e.code}`);
        return; // clean exit
      }
      throw e;
    }
  }

  /** Call vzglyd_init() if exported. */
  runInit() {
    const fn = this._instance.exports.vzglyd_init;
    if (!fn) return;
    try { fn(); } catch (e) {
      if (e instanceof ProcExitError && e.code === 0) return;
      throw e;
    }
  }

  /**
   * Read the SlideSpec wire bytes from guest memory.
   * Wire format: [1-byte ABI version][postcard SlideSpec<V>]
   * @returns {Uint8Array}
   */
  readSpecBytes() {
    const ptrFn = this._instance.exports.vzglyd_spec_ptr;
    const lenFn = this._instance.exports.vzglyd_spec_len;
    if (!ptrFn || !lenFn) throw new Error('slide is missing vzglyd_spec_ptr / vzglyd_spec_len exports');
    const ptr = ptrFn() >>> 0;
    const len = lenFn() >>> 0;
    if (len === 0) throw new Error('vzglyd_spec_len returned 0');
    return this._readBytes(ptr, len);
  }

  /**
   * Call vzglyd_update(dt) → i32.
   * Returns 0 (no change) or 1 (meshes/overlay updated).
   * @param {number} dt  seconds since last frame
   */
  update(dt) {
    const fn = this._instance.exports.vzglyd_update;
    if (!fn) return 0;
    return fn(dt) | 0;
  }

  /** Read RuntimeOverlay bytes from guest memory (null if none / zero length). */
  readOverlayBytes() {
    const ptrFn = this._instance.exports.vzglyd_overlay_ptr;
    const lenFn = this._instance.exports.vzglyd_overlay_len;
    if (!ptrFn || !lenFn) return null;
    const ptr = ptrFn() >>> 0;
    const len = lenFn() >>> 0;
    if (len === 0) return null;
    return this._readBytes(ptr, len);
  }

  /** Read RuntimeMeshSet bytes from guest memory (null if none / zero length). */
  readDynamicMeshBytes() {
    const ptrFn = this._instance.exports.vzglyd_dynamic_meshes_ptr;
    const lenFn = this._instance.exports.vzglyd_dynamic_meshes_len;
    if (!ptrFn || !lenFn) return null;
    const ptr = ptrFn() >>> 0;
    const len = lenFn() >>> 0;
    if (len === 0) return null;
    return this._readBytes(ptr, len);
  }

  // ── Import object ───────────────────────────────────────────────────────────

  /** Build the import object to pass to WebAssembly.instantiate(). */
  buildImports() {
    const wasi = this._buildWasi();
    const host = this._buildVzglydHost();
    return { wasi_snapshot_preview1: wasi, vzglyd_host: host };
  }

  // ── WASI preview1 stubs ─────────────────────────────────────────────────────

  _buildWasi() {
    const self = this;

    return {
      // ── fd_write: print to console (stdout fd=1, stderr fd=2) ──────────────
      fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) {
        if (fd !== 1 && fd !== 2) return WASI_EBADF;
        const view = self._memView();
        let total = 0;
        let text = '';
        for (let i = 0; i < iovs_len; i++) {
          const base = view.getUint32(iovs_ptr + i * 8,     true);
          const blen = view.getUint32(iovs_ptr + i * 8 + 4, true);
          if (blen === 0) continue;
          text  += new TextDecoder().decode(new Uint8Array(self._memory.buffer, base, blen));
          total += blen;
        }
        if (text) (fd === 2 ? console.warn : console.log)('[vzglyd]', text.trimEnd());
        view.setUint32(nwritten_ptr, total, true);
        return WASI_ESUCCESS;
      },

      // ── clock_time_get: return real time or monotonic clock ────────────────
      clock_time_get(clock_id, _precision_lo, _precision_hi, out_ptr) {
        let ns;
        if (clock_id === CLOCK_MONOTONIC) {
          ns = BigInt(Math.round((performance.now() - self._startMs) * 1_000_000));
        } else {
          // CLOCK_REALTIME
          ns = BigInt(Math.round(Date.now() * 1_000_000));
        }
        const view = self._memView();
        view.setBigUint64(out_ptr, ns, true);
        return WASI_ESUCCESS;
      },

      // ── random_get: fill with crypto random bytes ──────────────────────────
      random_get(buf_ptr, buf_len) {
        const buf = new Uint8Array(self._memory.buffer, buf_ptr, buf_len);
        crypto.getRandomValues(buf);
        return WASI_ESUCCESS;
      },

      // ── proc_exit: throw sentinel so callers can detect clean exit ─────────
      proc_exit(code) {
        throw new ProcExitError(code);
      },

      // ── args_sizes_get: report 0 args ──────────────────────────────────────
      args_sizes_get(argc_ptr, argv_buf_size_ptr) {
        const view = self._memView();
        view.setUint32(argc_ptr,          0, true);
        view.setUint32(argv_buf_size_ptr, 0, true);
        return WASI_ESUCCESS;
      },

      args_get(_argv_ptr, _argv_buf_ptr) { return WASI_ESUCCESS; },

      // ── environ_sizes_get: report 0 env vars ──────────────────────────────
      environ_sizes_get(env_count_ptr, env_buf_size_ptr) {
        const view = self._memView();
        view.setUint32(env_count_ptr,    0, true);
        view.setUint32(env_buf_size_ptr, 0, true);
        return WASI_ESUCCESS;
      },

      environ_get(_environ_ptr, _environ_buf_ptr) { return WASI_ESUCCESS; },

      // ── fd operations: reject everything except 0/1/2 ─────────────────────
      fd_close(_fd) { return WASI_EBADF; },
      fd_seek(_fd, _lo, _hi, _whence, _out) { return WASI_EBADF; },
      fd_read(_fd, _iovs, _iovs_len, _nread) { return WASI_EBADF; },
      fd_fdstat_get(_fd, _stat_ptr) { return WASI_EBADF; },

      fd_prestat_get(fd, _stat_ptr) {
        // FDs 0/1/2 are stdin/stdout/stderr (no prestat); anything else unknown.
        return WASI_EBADF;
      },

      fd_prestat_dir_name(_fd, _path_ptr, _path_len) { return WASI_EBADF; },

      // ── path operations: not supported ────────────────────────────────────
      path_open(_fd, _dir_flags, _path_ptr, _path_len,
                 _o_flags, _fs_rights_base_lo, _fs_rights_base_hi,
                 _fs_rights_inheriting_lo, _fs_rights_inheriting_hi,
                 _fd_flags, _opened_fd_ptr) {
        return WASI_EBADF;
      },

      path_filestat_get(_fd, _flags, _path_ptr, _path_len, _stat_ptr) { return WASI_EBADF; },
      path_create_directory(_fd, _path_ptr, _path_len)                 { return WASI_EBADF; },
      path_remove_directory(_fd, _path_ptr, _path_len)                 { return WASI_EBADF; },
      path_unlink_file(_fd, _path_ptr, _path_len)                      { return WASI_EBADF; },
      path_rename(_fd, _old_ptr, _old_len, _new_fd, _new_ptr, _new_len){ return WASI_EBADF; },
      path_readlink(_fd, _p, _pl, _buf, _blen, _nread)                 { return WASI_EBADF; },
      path_symlink(_old, _old_len, _fd, _new, _new_len)                { return WASI_EBADF; },

      // ── poll_oneoff: not supported ─────────────────────────────────────────
      poll_oneoff(_in, _out, _nsubscriptions, _nevents_ptr) { return WASI_ENOSYS; },

      // ── sched_yield: no-op ────────────────────────────────────────────────
      sched_yield() { return WASI_ESUCCESS; },
    };
  }

  // ── vzglyd_host imports ─────────────────────────────────────────────────────

  _buildVzglydHost() {
    const self = this;

    // Return codes matching the native engine constants:
    const HOST_ERROR           = -1;
    const HOST_CHANNEL_EMPTY   = -3;
    const HOST_ASSET_NOT_FOUND = -4;

    return {
      /**
       * channel_poll(buf_ptr: i32, buf_len: i32) -> i32
       * The preview has no sidecar channel, so always return HOST_CHANNEL_EMPTY.
       */
      channel_poll(_buf_ptr, _buf_len) {
        return HOST_CHANNEL_EMPTY;
      },

      /**
       * log_info(ptr: i32, len: i32) -> i32
       */
      log_info(ptr, len) {
        try {
          const msg = self._readString(ptr >>> 0, len >>> 0);
          console.log('[vzglyd]', msg);
          return WASI_ESUCCESS;
        } catch {
          return HOST_ERROR;
        }
      },

      /**
       * mesh_asset_len(key_ptr: i32, key_len: i32) -> i32
       * The browser preview has no mesh asset catalog.
       */
      mesh_asset_len(_key_ptr, _key_len) {
        return HOST_ASSET_NOT_FOUND;
      },

      /**
       * mesh_asset_read(key_ptr, key_len, buf_ptr, buf_len) -> i32
       */
      mesh_asset_read(_key_ptr, _key_len, _buf_ptr, _buf_len) {
        return HOST_ASSET_NOT_FOUND;
      },

      /**
       * scene_metadata_len(key_ptr: i32, key_len: i32) -> i32
       */
      scene_metadata_len(_key_ptr, _key_len) {
        return HOST_ASSET_NOT_FOUND;
      },

      /**
       * scene_metadata_read(key_ptr, key_len, buf_ptr, buf_len) -> i32
       */
      scene_metadata_read(_key_ptr, _key_len, _buf_ptr, _buf_len) {
        return HOST_ASSET_NOT_FOUND;
      },
    };
  }
}
