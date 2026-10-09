// Counterpart of examples/reliability.rs; run through bench/reliability/run.py.
import { chromium } from 'playwright-core';
import { performance } from 'node:perf_hooks';

const base = process.env.RUSTWRIGHT_RELIABILITY_URL;
const samples = Number(process.env.RUSTWRIGHT_RELIABILITY_SAMPLES);
const executablePath = process.env.RUSTWRIGHT_BENCH_CHROME;
if (!base || !executablePath || !Number.isInteger(samples) || samples <= 0) {
  throw new Error('fixture URL, browser path and positive sample count required');
}
const launch = () => chromium.launch({ executablePath, headless: true });
const browser = await launch();
const diagnostics = await browser.newBrowserCDPSession();
const records = [];
const cases = ['delayed_click', 'delayed_frame', 'cross_site_frame', 'cross_site_navigation', 'disabled_click', 'covered_click', 'moving_click', 'clipped_click', 'rotated_clipped_click', 'http_disconnect_recovery', 'browser_disconnect', 'mock_fetch', 'mock_navigation', 'mock_frame', 'mock_return', 'mock_clear'];
const selected = process.env.RUSTWRIGHT_RELIABILITY_CASES?.split(',') || cases;
if (selected.some(name => !cases.includes(name))) throw new Error('unknown reliability case');

const installMock = page => page.route('**/api/mock', route => route.fulfill({ status: 201, contentType: 'text/plain', body: 'mocked' }));
function verifyNetwork(result, mocked = true) {
  const response = mocked ? [201, 'mocked'] : [200, 'network'];
  if (JSON.stringify(result.responses) !== JSON.stringify([response, response]) || !result.nativeFetch || !result.nativeXHR) {
    throw new Error(`network postcondition failed: ${JSON.stringify(result)}`);
  }
}
async function networkFrame(page) {
  const element = await page.locator('#late-frame').elementHandle();
  const frame = await element.contentFrame();
  await element.dispose();
  if (!frame) throw new Error('network frame not ready');
  return frame;
}
async function verifyRemoteFrame(frame) {
  const targets = await diagnostics.send('Target.getTargets');
  if (!targets.targetInfos.some(target => target.type === 'iframe' && target.url === frame.url()) || await frame.evaluate(() => location.hostname) !== 'localhost') {
    throw new Error('network scenario did not create an OOPIF');
  }
}

async function runCase(name, page, active) {
  switch (name) {
    case 'mock_fetch': {
      await installMock(page);
      const result = await page.evaluate(() => Promise.all([
        fetch('/api/mock').then(async response => [response.status, await response.text()]),
        new Promise(resolve => {
          const request = new XMLHttpRequest();
          request.open('GET', '/api/mock');
          request.onload = () => resolve([request.status, request.responseText]);
          request.onerror = request.onabort = () => resolve(['error', '']);
          request.send();
        }),
      ]).then(responses => ({ responses, nativeFetch: fetch.toString().includes('[native code]'), nativeXHR: XMLHttpRequest.prototype.open.toString().includes('[native code]') })));
      verifyNetwork(result);
      break;
    }
    case 'mock_navigation':
    case 'mock_clear':
      if (name === 'mock_clear') await page.unrouteAll();
      await page.goto(`${base}/network-frame.html`, { timeout: 2000 });
      verifyNetwork(await page.evaluate(() => window.networkResult), name !== 'mock_clear');
      break;
    case 'mock_frame':
    case 'mock_return': {
      const url = name === 'mock_frame' ? `${base.replace('127.0.0.1', 'localhost')}/network-frame.html` : '/network-frame.html';
      await page.evaluate(url => navigateNetworkFrame(url), url);
      const frame = await networkFrame(page);
      verifyNetwork(await frame.evaluate(() => window.networkResult));
      if (name === 'mock_frame') await verifyRemoteFrame(frame);
      break;
    }
    case 'disabled_click':
    case 'covered_click':
    case 'moving_click':
    case 'clipped_click':
    case 'rotated_clipped_click':
      await page.locator('#target').click({ timeout:2000 });
      if (!await page.evaluate(() => window.clicked && !window.clickedBeforeReady && window.wrongClicks === 0)) {
        throw new Error('actionability postcondition failed');
      }
      break;
    case 'delayed_click':
      await page.locator('#late').click({ timeout: 2000 });
      if (!await page.evaluate(() => window.clicked)) throw new Error('click postcondition failed');
      break;
    case 'delayed_frame':
    case 'cross_site_frame':
    case 'cross_site_navigation': {
      const frame = page.frameLocator('#late-frame');
      const input = name === 'cross_site_navigation' ? '#swap-value' : '#value';
      await frame.locator(input).fill('payload', { timeout: 2000 });
      await frame.locator('#submit').click({ timeout: 2000 });
      await page.evaluate(() => new Promise(resolve => {
        if (window.frameValue !== null) resolve();
        else window.addEventListener('message', () => resolve(), { once: true });
      }));
      if (await page.evaluate(() => window.frameValue) !== 'payload') throw new Error('frame postcondition failed');
      if (name !== 'delayed_frame') {
        const url = await page.locator('#late-frame').getAttribute('src');
        const targets = await diagnostics.send('Target.getTargets');
        if (!targets.targetInfos.some(target => target.type === 'iframe' && target.url === url)) {
          throw new Error('cross-site scenario did not create an OOPIF');
        }
        if (!url.startsWith(base.replace('127.0.0.1', 'localhost'))) throw new Error('unexpected frame origin');
      }
      break;
    }
    case 'http_disconnect_recovery': {
      const result = await page.evaluate(() => fetch('/disconnect')
        .then(() => 'unexpected response')
        .catch(error => error instanceof TypeError ? 'network-error' : String(error)));
      if (result !== 'network-error') throw new Error(`expected network error, got ${result}`);
      await page.goto(`${base}/index.html`, { timeout: 2000 });
      if (!await page.evaluate(() => window.fixtureReady)) throw new Error('recovery postcondition failed');
      break;
    }
    case 'browser_disconnect': {
      // Install the rejection handler before closing the browser.
      const waiting = page.evaluate(() => {
        window.pending = true;
        return new Promise(() => {});
      }).then(() => ({ ok: true }), error => ({ error }));
      while (!await page.evaluate(() => window.pending === true)) {
        await new Promise(resolve => setTimeout(resolve, 5));
      }
      await active.close();
      const result = await waiting;
      if (!result.error?.message.includes('closed')) throw new Error('expected closed connection error');
      break;
    }
    default: throw new Error(`unknown case ${name}`);
  }
}

try {
  for (const name of selected) {
    for (let index = 0; index <= samples; index++) {
      const owned = name === 'browser_disconnect' ? await launch() : null;
      const active = owned || browser;
      const page = await active.newPage();
      try {
        await page.goto(`${base}/index.html`, { timeout: 10000 }).catch(error => {
          throw new Error(`${name}[${index}] initial navigation: ${error.message}`);
        });
        if (name === 'delayed_click') await page.evaluate(() => scheduleButton(120));
        if (name === 'delayed_frame') await page.evaluate(() => scheduleFrame(120));
        if (name === 'cross_site_frame') await page.evaluate(url => scheduleFrame(120, url), `${base.replace('127.0.0.1', 'localhost')}/frame.html`);
        if (name === 'cross_site_navigation') await page.evaluate(url => scheduleFrameSwap(80, url), `${base.replace('127.0.0.1', 'localhost')}/swap-frame.html`);
        if (['disabled_click', 'covered_click', 'moving_click'].includes(name)) {
          await page.evaluate(kind => prepareActionability(kind,120), name.replace('_click',''));
        }
        if (['clipped_click', 'rotated_clipped_click'].includes(name)) {
          await page.evaluate(kind => prepareGeometry(kind), name.replace(/_click$/, ''));
        }
        if (['mock_navigation', 'mock_frame', 'mock_return', 'mock_clear'].includes(name)) {
          await installMock(page);
          if (name === 'mock_return') {
            await page.evaluate(url => navigateNetworkFrame(url), `${base.replace('127.0.0.1', 'localhost')}/network-frame.html`);
            const frame = await networkFrame(page);
            verifyNetwork(await frame.evaluate(() => window.networkResult));
            await verifyRemoteFrame(frame);
          } else if (name === 'mock_clear') {
            await page.goto(`${base}/network-frame.html`, { timeout: 10000 });
            verifyNetwork(await page.evaluate(() => window.networkResult));
          }
        }
        const start = performance.now();
        let timer;
        let error = null;
        try {
          await Promise.race([
            runCase(name, page, active),
            new Promise((_, reject) => {
              timer = setTimeout(() => reject(new Error('operation exceeded 2000ms watchdog')), 2000);
            }),
          ]);
        } catch (caught) { error = String(caught); }
        finally { clearTimeout(timer); }
        records.push({ case: name, index, warmup: index === 0, ok: error === null, error, elapsed_ms: performance.now() - start });
      } finally {
        if (owned) await owned.close();
        else await page.close();
      }
    }
  }
  console.log(JSON.stringify({ engine: 'playwright-core', browser: browser.version(), records }));
} finally { await browser.close(); }
