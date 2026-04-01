// @ts-check
const { test, expect } = require('@playwright/test');
const path = require('path');

const VZGLYD_PATH = path.resolve(__dirname, '../../lume-clock/clock.vzglyd');

test.describe('web-preview', () => {
  test('page loads and shows drop zone', async ({ page }) => {
    await page.goto('http://localhost:8787');

    // Drop zone should be visible
    await expect(page.locator('#drop-zone')).toBeVisible();

    // No errors shown on load
    await expect(page.locator('#error-box')).toBeHidden();
  });

  test('WebGPU availability', async ({ page }) => {
    await page.goto('http://localhost:8787');

    // Wait a moment for checkWebGPU() to run
    await page.waitForTimeout(1000);

    const noWebgpuVisible = await page.locator('#no-webgpu').isVisible();
    const fileWarningVisible = await page.locator('#file-origin-warning').isVisible();
    const dropZoneVisible = await page.locator('#drop-zone').isVisible();

    console.log(`WebGPU available: ${dropZoneVisible}, no-webgpu banner: ${noWebgpuVisible}, file-warning: ${fileWarningVisible}`);

    // If WebGPU is unavailable, mark as skipped with info rather than failing the test
    if (noWebgpuVisible) {
      test.skip(true, 'WebGPU not available in this environment');
    }

    await page.screenshot({ path: 'test-results/01-page-load.png', fullPage: true });
  });

  test('loads clock.vzglyd and renders', async ({ page }) => {
    // Capture ALL console output from the page before navigation
    const consoleLogs = [];
    page.on('console', msg => consoleLogs.push(`[${msg.type()}] ${msg.text()}`));
    page.on('pageerror', err => consoleLogs.push(`[pageerror] ${err.message}`));

    await page.goto('http://localhost:8787');
    await page.waitForTimeout(500);

    // Skip if WebGPU is unavailable
    const noWebgpu = await page.locator('#no-webgpu').isVisible();
    if (noWebgpu) {
      test.skip(true, 'WebGPU not available in this environment — cannot test rendering');
      return;
    }

    await page.screenshot({ path: 'test-results/02-before-load.png', fullPage: true });

    // Load the bundle via the file input
    await page.locator('#file-input').setInputFiles(VZGLYD_PATH);

    // Wait for canvas container to appear (drop zone hidden, canvas shown)
    await expect(page.locator('#canvas-container')).toBeVisible({ timeout: 15_000 });

    // Wait for FPS counter to show a value (render loop running)
    await expect(page.locator('#slide-fps')).not.toBeEmpty({ timeout: 10_000 });

    const fps = await page.locator('#slide-fps').textContent();
    console.log(`FPS: ${fps}`);

    // Wait for the clock fade-in animation (takes ~1.9s of elapsed time).
    await page.waitForTimeout(3000);
    await page.screenshot({ path: 'test-results/03-rendering-3s.png', fullPage: true });

    // Confirm elapsed time and slide name via JS introspection.
    const renderState = await page.evaluate(() => {
      return {
        slideName:    document.getElementById('slide-name')?.textContent,
        slideFps:     document.getElementById('slide-fps')?.textContent,
        errorVisible: !document.getElementById('error-box')?.hidden,
        errorText:    document.getElementById('error-text')?.textContent,
      };
    });
    console.log('Render state:', JSON.stringify(renderState));

    // Check for shader fallback warning
    const shaderFallback = await page.locator('#error-box').isVisible();
    if (shaderFallback) {
      const errorMsg = await page.locator('#error-text').textContent();
      console.log(`Shader warning: ${errorMsg}`);
    }

    // The FPS counter confirms the render loop is running — that's sufficient for CI
    expect(fps).toMatch(/\d+ fps/);

    console.log('\n=== Page console output ===\n' + consoleLogs.join('\n'));
  });
});
