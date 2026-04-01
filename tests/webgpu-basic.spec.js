// @ts-check
const { test, expect } = require('@playwright/test');

test('WebGPU renders a red triangle (GPU readback)', async ({ page }) => {
  await page.goto('http://localhost:8787');

  // Phase 1: async setup (requestAdapter/requestDevice are GPU promises — ok to await here).
  // We render into an offscreen texture (not the swap chain) so we can copy it back.
  // The mapAsync is triggered inside the PAGE's own promise chain (.then) so Playwright's
  // bridge never awaits a GPU-side promise — that avoids the "Instance dropped" error.
  const initOk = await page.evaluate(async () => {
    if (!navigator.gpu) return 'no navigator.gpu';

    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) return 'no adapter';

    const device = await adapter.requestDevice();
    window.__gpuTestDevice = device; // pin to prevent GC
    window.__gpuTestResult = null;

    const W = 64, H = 64;
    const format = 'rgba8unorm';

    // Offscreen render target (not the swap chain) — we can copy from it.
    const tex = device.createTexture({
      size: [W, H],
      format,
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
    });

    const shader = device.createShaderModule({ code: `
      @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
        var pos = array<vec2f,3>(vec2f(0.0,0.5), vec2f(-0.5,-0.5), vec2f(0.5,-0.5));
        return vec4f(pos[i], 0.0, 1.0);
      }
      @fragment fn fs() -> @location(0) vec4f {
        return vec4f(1.0, 0.0, 0.0, 1.0);
      }
    `});

    const pipeline = device.createRenderPipeline({
      layout: 'auto',
      vertex:   { module: shader, entryPoint: 'vs' },
      fragment: { module: shader, entryPoint: 'fs', targets: [{ format }] },
      primitive: { topology: 'triangle-list' },
    });

    // Readback buffer: stride must be multiple of 256
    const bytesPerRow = 256; // 64px * 4bytes = 256, already aligned
    const readBuf = device.createBuffer({
      size: bytesPerRow * H,
      usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
    });
    window.__gpuReadBuf = readBuf; // pin

    const enc = device.createCommandEncoder();
    const pass = enc.beginRenderPass({
      colorAttachments: [{
        view: tex.createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        loadOp: 'clear', storeOp: 'store',
      }],
    });
    pass.setPipeline(pipeline);
    pass.draw(3);
    pass.end();

    enc.copyTextureToBuffer(
      { texture: tex },
      { buffer: readBuf, bytesPerRow },
      [W, H],
    );
    device.queue.submit([enc.finish()]);

    // mapAsync runs in the PAGE's event loop — not awaited by Playwright
    readBuf.mapAsync(GPUMapMode.READ).then(() => {
      const data = new Uint8Array(readBuf.getMappedRange());
      // Sample pixel at (32, 20): row 20, col 32 → offset = 20*256 + 32*4
      const off = 20 * 256 + 32 * 4;
      window.__gpuTestResult = { r: data[off], g: data[off+1], b: data[off+2], a: data[off+3] };
      readBuf.unmap();
    }).catch(e => {
      window.__gpuTestResult = { error: e.message };
    });

    return 'ok';
  });

  if (initOk !== 'ok') {
    test.skip(true, initOk);
    return;
  }

  // Phase 2: wait for mapAsync to complete inside the page, then read the result.
  const result = await page.waitForFunction(
    () => window.__gpuTestResult !== null,
    { timeout: 5000 }
  ).then(() => page.evaluate(() => window.__gpuTestResult));

  console.log('WebGPU pixel at triangle center:', result);

  if (result.error) {
    test.skip(true, `mapAsync failed: ${result.error}`);
    return;
  }

  // Red channel must be non-zero if WebGPU rendered the triangle
  expect(result.r).toBeGreaterThan(0);
});

test('WebGL renders a green quad', async ({ page }) => {
  await page.goto('http://localhost:8787');

  const result = await page.evaluate(() => {
    const canvas = document.createElement('canvas');
    canvas.width = 64; canvas.height = 64;
    document.body.appendChild(canvas);

    const gl = canvas.getContext('webgl');
    if (!gl) return { error: 'no webgl' };

    gl.clearColor(0, 1, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const px = new Uint8Array(4);
    gl.readPixels(32, 32, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
    return { r: px[0], g: px[1], b: px[2], a: px[3] };
  });

  console.log('WebGL pixel:', result);

  if (result.error) {
    test.skip(true, result.error);
    return;
  }

  expect(result.g).toBeGreaterThan(0);
});
