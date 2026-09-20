// Playwright counterpart of examples/bench.rs, for the README comparison.
//
// It measures the same things against the same installed Chrome:
//   - launch -> first page
//   - per-navigation latency
//   - per-evaluate round-trip
//   - driver process peak RSS (VmHWM), excluding the spawned browser
//
// Usage:
//   cd bench/playwright
//   npm install playwright-core
//   node bench.mjs
//
// Env:
//   RUSTWRIGHT_BENCH_CHROME  browser executable (default /usr/bin/google-chrome-stable)
//   RUSTWRIGHT_BENCH_GOTOS   default 30
//   RUSTWRIGHT_BENCH_EVALS   default 2000

import { chromium } from "playwright-core";
import { performance } from "node:perf_hooks";
import fs from "node:fs";

const EXE = process.env.RUSTWRIGHT_BENCH_CHROME || "/usr/bin/google-chrome-stable";
const URL =
  process.env.RUSTWRIGHT_BENCH_URL ||
  "data:text/html,<title>bench</title><h1 id=title>bench</h1><input id=q><button id=go>Go</button><div id=out>ok</div>";
const GOTOS = Number(process.env.RUSTWRIGHT_BENCH_GOTOS || 30);
const EVALS = Number(process.env.RUSTWRIGHT_BENCH_EVALS || 2000);

function vmHwmKb() {
  const status = fs.readFileSync("/proc/self/status", "utf8");
  const match = status.match(/VmHWM:\s+(\d+)\s+kB/);
  return match ? Number(match[1]) : 0;
}

const t0 = performance.now();
const browser = await chromium.launch({ headless: true, executablePath: EXE });
const context = await browser.newContext();
const page = await context.newPage();
await page.goto(URL);
const launchMs = performance.now() - t0;

for (let i = 0; i < 5; i += 1) {
  await page.goto(URL);
}

const g0 = performance.now();
for (let i = 0; i < GOTOS; i += 1) {
  await page.goto(URL);
}
const gotoAvg = (performance.now() - g0) / GOTOS;

for (let i = 0; i < 100; i += 1) {
  await page.evaluate("1+1");
}
const e0 = performance.now();
for (let i = 0; i < EVALS; i += 1) {
  await page.evaluate("1+1");
}
const evalAvg = (performance.now() - e0) / EVALS;

console.log(
  JSON.stringify({
    engine: "playwright-core",
    browser: browser.version(),
    launch_ms: Number(launchMs.toFixed(1)),
    goto_avg_ms: Number(gotoAvg.toFixed(3)),
    eval_avg_ms: Number(evalAvg.toFixed(4)),
    vm_hwm_kb: vmHwmKb(),
    gotos: GOTOS,
    evals: EVALS,
  })
);

await browser.close();
