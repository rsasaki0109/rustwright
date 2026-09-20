# Benchmarks

Driver-overhead measurements for the README performance table. Both harnesses
drive the **same installed Chrome** and measure the same things:

- `launch_ms` — launch the browser, open a page and load a URL.
- `goto_avg_ms` — per-navigation latency (30 navigations, median of runs).
- `eval_avg_ms` — per-`evaluate` round-trip (2000 calls).
- `vm_hwm_kb` — peak RSS (`VmHWM`) of the **driver process only**; the spawned
  browser is deliberately excluded.

## Rustwright

```sh
cargo run --release -p rustwright-examples --example bench
```

## Playwright

```sh
cd bench/playwright
npm install
node bench.mjs
```

Set `RUSTWRIGHT_BENCH_CHROME` to point Playwright at the installed Chrome (it
defaults to `/usr/bin/google-chrome-stable`). `RUSTWRIGHT_BENCH_GOTOS` and
`RUSTWRIGHT_BENCH_EVALS` change the iteration counts for both harnesses.

Run each a few times and compare medians. These numbers measure the automation
layer, not the browser: because both drive the same engine, navigation latency is
expected to be close, while `evaluate` throughput, driver memory and process
startup reflect the runtime overhead.
