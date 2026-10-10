// Reproduce the initial-fetch interception gap in an OOPIF -> parent swap.
// This diagnostic uses Playwright's public APIs; it is not a performance case.
import assert from 'node:assert/strict';
import http from 'node:http';
import { createRequire } from 'node:module';

const require = createRequire(new URL('../playwright/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const chrome = process.argv[2] || process.env.RUSTWRIGHT_CHROME;
if (!chrome) throw new Error('Usage: node network-swap-probe.mjs /path/to/chromium');

const server = http.createServer((request, response) => {
  response.setHeader('Content-Type', 'text/html');
  response.end(request.url === '/api/network' ? 'network'
    : request.url === '/network-frame.html'
      ? "<input id=value><script>window.initialResponse = fetch('/api/network').then(r => r.text())</script>"
      : '<body></body>');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  const port = server.address().port;
  browser = await chromium.launch({ executablePath: chrome, headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(5000);
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.route('**/api/network', route => route.fulfill({ status: 200, body: 'mocked' }));
  const samples = [];
  for (const hostname of ['localhost', '127.0.0.1']) {
    await page.evaluate(url => new Promise((resolve, reject) => {
      let frame = document.querySelector('iframe');
      if (!frame) {
        frame = document.createElement('iframe');
      }
      const timer = setTimeout(() => reject(new Error('iframe load deadline')), 5000);
      frame.onload = () => { clearTimeout(timer); resolve(true); };
      frame.src = url;
      if (!frame.isConnected) document.body.append(frame);
    }), `http://${hostname}:${port}/network-frame.html`);
    const frame = page.frames().find(frame => frame.parentFrame() === page.mainFrame());
    samples.push({ hostname, initialResponse: await frame.evaluate(() => window.initialResponse) });
  }
  assert.equal(samples[0].initialResponse, 'mocked', 'control: OOPIF interception must work');
  console.log(JSON.stringify({ chromium: browser.version(), playwright: require('playwright-core/package.json').version, samples }, null, 2));
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
