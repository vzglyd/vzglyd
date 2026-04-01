/**
 * app.js — vzglyd browser preview orchestrator.
 *
 * Flow:
 *   1. File drop / picker → extract .vzglyd zip (fflate)
 *   2. Validate manifest.json
 *   3. Instantiate slide.wasm with WASI + vzglyd_host imports
 *   4. Run _start(), read SlideSpec from guest memory, decode with postcard
 *   5. Init WebGPU renderer
 *   6. requestAnimationFrame loop: update WASM → update GPU → render
 */

// postcard.js, wasm-host.js, renderer.js and fflate (UMD) are loaded as
// plain <script> tags before this file — all their symbols are global.

// ── DOM refs ──────────────────────────────────────────────────────────────────

const dropZone      = document.getElementById('drop-zone');
const fileInput     = document.getElementById('file-input');
const canvasContainer = document.getElementById('canvas-container');
const canvas        = document.getElementById('render-canvas');
const slideName     = document.getElementById('slide-name');
const slideFps      = document.getElementById('slide-fps');
const backBtn       = document.getElementById('back-btn');
const statusBar     = document.getElementById('status-bar');
const statusSpinner = document.getElementById('status-spinner');
const statusText    = document.getElementById('status-text');
const errorBox      = document.getElementById('error-box');
const errorText     = document.getElementById('error-text');
const errorDismiss       = document.getElementById('error-dismiss');
const noWebgpu           = document.getElementById('no-webgpu');
const fileOriginWarning  = document.getElementById('file-origin-warning');

// ── State ─────────────────────────────────────────────────────────────────────

/** @type {VzglydRenderer|null} */
let activeRenderer = null;
let rafId          = null;

// ── Startup checks ────────────────────────────────────────────────────────────

async function checkWebGPU() {
  if (!navigator.gpu) {
    dropZone.hidden = true;
    noWebgpu.hidden = false;
    return;
  }

  if (location.protocol === 'file:') {
    // Chromium-based browsers silently return null from requestAdapter() on
    // file:// origins even when WebGPU is otherwise available.
    dropZone.hidden = true;
    fileOriginWarning.hidden = false;
    return;
  }

  // Probe the adapter now so we surface the "enable the flag" message at page
  // load rather than after the user has already dropped a file.
  const adapter =
    await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' }) ??
    await navigator.gpu.requestAdapter() ??
    await navigator.gpu.requestAdapter({ forceFallbackAdapter: true });

  if (!adapter) {
    dropZone.hidden = true;
    noWebgpu.hidden = false;
  }
}

checkWebGPU();

// ── Copy-command buttons ───────────────────────────────────────────────────────

document.querySelectorAll('.copy-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    const targetId = btn.dataset.target;
    const text = document.getElementById(targetId)?.textContent ?? '';
    navigator.clipboard.writeText(text).then(() => {
      btn.textContent = 'Copied!';
      btn.classList.add('copied');
      setTimeout(() => { btn.textContent = 'Copy'; btn.classList.remove('copied'); }, 2000);
    });
  });
});

// ── UI helpers ────────────────────────────────────────────────────────────────

function setStatus(msg, spinning = false) {
  statusBar.hidden   = false;
  statusText.textContent = msg;
  statusSpinner.hidden   = !spinning;
}

function clearStatus() {
  statusBar.hidden = true;
  statusText.textContent = '';
  statusSpinner.hidden   = true;
}

function showError(msg) {
  errorText.textContent = msg;
  errorBox.hidden = false;
  clearStatus();
}

function clearError() {
  errorBox.hidden = true;
}

function showCanvas(name) {
  dropZone.hidden        = true;
  canvasContainer.hidden = false;
  slideName.textContent  = name;
  clearStatus();
}

function showDropZone() {
  canvasContainer.hidden = true;
  dropZone.hidden        = false;
  slideName.textContent  = '';
  slideFps.textContent   = '';
}

// ── Event wiring ──────────────────────────────────────────────────────────────

errorDismiss.addEventListener('click', clearError);
backBtn.addEventListener('click', reset);

dropZone.addEventListener('dragover', e => {
  e.preventDefault();
  dropZone.classList.add('drag-over');
});
dropZone.addEventListener('dragleave', () => dropZone.classList.remove('drag-over'));
dropZone.addEventListener('drop', e => {
  e.preventDefault();
  dropZone.classList.remove('drag-over');
  const file = e.dataTransfer?.files?.[0];
  if (file) handleFile(file);
});
dropZone.addEventListener('click', () => fileInput.click());
dropZone.addEventListener('keydown', e => {
  if (e.key === 'Enter' || e.key === ' ') fileInput.click();
});

fileInput.addEventListener('change', () => {
  const file = fileInput.files?.[0];
  if (file) handleFile(file);
  fileInput.value = '';
});

// ── Main flow ─────────────────────────────────────────────────────────────────

function reset() {
  if (rafId !== null) { cancelAnimationFrame(rafId); rafId = null; }
  if (activeRenderer) { activeRenderer.stop(); activeRenderer = null; }
  showDropZone();
  clearError();
  clearStatus();
}

async function handleFile(file) {
  if (!file.name.endsWith('.vzglyd')) {
    showError(`Expected a .vzglyd file, got: ${file.name}`);
    return;
  }

  clearError();
  setStatus('Extracting bundle…', true);

  try {
    const bundle = await extractBundle(file);
    setStatus('Instantiating WASM…', true);

    const { spec, host } = await loadWasm(bundle.wasmBytes);
    setStatus('Initialising renderer…', true);

    if (activeRenderer) { activeRenderer.stop(); activeRenderer = null; }
    if (rafId !== null)  { cancelAnimationFrame(rafId); rafId = null; }

    const renderer = new VzglydRenderer(canvas, spec);
    await renderer.init();
    activeRenderer = renderer;

    renderer._onFps = fps => { slideFps.textContent = `${fps} fps`; };

    showCanvas(spec.name || bundle.manifest.name || file.name);

    if (renderer._shaderFallback) {
      showError(
        'Custom shader validation failed — rendering with default shader.\n' +
        'Check the browser console for the specific WGSL error.'
      );
    }

    let lastTime = performance.now();

    function frame(now) {
      const dt = Math.min((now - lastTime) / 1000, 0.1); // cap at 100 ms
      lastTime = now;

      const updated = host.update(dt);
      if (updated === 1) {
        renderer.applyOverlayBytes(host.readOverlayBytes());
        renderer.applyDynamicMeshBytes(host.readDynamicMeshBytes());
      }

      renderer.renderFrame(dt);
      rafId = requestAnimationFrame(frame);
    }

    rafId = requestAnimationFrame(frame);

  } catch (err) {
    console.error(err);
    showError(err?.message ?? String(err));
  }
}

// ── Bundle extraction (fflate) ────────────────────────────────────────────────

async function extractBundle(file) {
  // fflate is loaded as a UMD <script> tag — check it's available.
  if (typeof fflate === 'undefined') {
    throw new Error(
      'fflate not loaded. Check your internet connection (needed to load the zip library from CDN).'
    );
  }

  const arrayBuf = await file.arrayBuffer();
  const zipData  = new Uint8Array(arrayBuf);

  // fflate.unzipSync returns { 'path/name': Uint8Array, ... }
  let files;
  try {
    files = fflate.unzipSync(zipData);
  } catch (e) {
    throw new Error(`Failed to unzip bundle: ${e?.message ?? e}`);
  }

  // manifest.json
  const manifestBytes = files['manifest.json'];
  if (!manifestBytes) throw new Error('Bundle is missing manifest.json');
  const manifest = JSON.parse(new TextDecoder().decode(manifestBytes));

  // slide.wasm
  const wasmBytes = files['slide.wasm'];
  if (!wasmBytes) throw new Error('Bundle is missing slide.wasm');

  return { manifest, wasmBytes, files };
}

// ── WASM loading and spec extraction ─────────────────────────────────────────

async function loadWasm(wasmBytes) {
  const host    = new VzglydWasmHost();
  const imports = host.buildImports();

  let wasmResult;
  try {
    wasmResult = await WebAssembly.instantiate(wasmBytes, imports);
  } catch (e) {
    throw new Error(`WebAssembly.instantiate failed: ${e?.message ?? e}`);
  }

  host.setInstance(wasmResult.instance);

  // Run _start() — WASI command entry. proc_exit(0) is treated as clean exit.
  host.runStart();

  // Optional vzglyd_init()
  host.runInit();

  // Read the wire-format spec bytes: [1-byte ABI version][postcard SlideSpec]
  const specWire = host.readSpecBytes();

  // Validate ABI version
  const abiVersion = specWire[0];
  if (abiVersion !== 1) {
    throw new Error(`Unsupported ABI version ${abiVersion}. Expected 1.`);
  }

  // Decode the postcard payload (skip the version byte)
  const spec = decodeSlideSpec(specWire.slice(1));

  return { spec, host };
}
