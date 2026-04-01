// @ts-check
const { defineConfig, devices } = require('@playwright/test');
const path = require('path');

module.exports = defineConfig({
  testDir: './tests',
  timeout: 30_000,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never' }]],

  // Serve web-preview/ over HTTP so WebGPU works (file:// blocks WebGPU on Chromium).
  webServer: {
    command: `node -e "
const http = require('http');
const fs   = require('fs');
const path = require('path');
const dir  = path.join(__dirname, 'web-preview');
const PORT = 8787;
http.createServer((req, res) => {
  const filePath = path.join(dir, req.url === '/' ? 'index.html' : req.url);
  fs.readFile(filePath, (err, data) => {
    if (err) { res.writeHead(404); res.end(); return; }
    const ext = path.extname(filePath);
    const mime = {
      '.html': 'text/html', '.js': 'application/javascript',
      '.css': 'text/css', '.wasm': 'application/wasm',
    }[ext] ?? 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': mime });
    res.end(data);
  });
}).listen(PORT, () => console.log('serving on ' + PORT));
"`,
    url: 'http://localhost:8787',
    reuseExistingServer: false,
    timeout: 10_000,
  },

  projects: [
    {
      name: 'chromium-webgpu',
      use: {
        ...devices['Desktop Chrome'],
        headless: true,
        launchOptions: {
          args: [
            '--enable-unsafe-webgpu',
            '--use-angle=swiftshader',
            '--ignore-gpu-blocklist',
            '--enable-gpu-rasterization',
            '--disable-gpu-sandbox',
            '--no-sandbox',
          ],
        },
      },
    },
  ],
});
