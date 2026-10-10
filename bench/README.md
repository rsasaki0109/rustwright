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
npm ci
node bench.mjs
```

Set `RUSTWRIGHT_BENCH_CHROME` to point Playwright at the installed Chrome (it
defaults to `/usr/bin/google-chrome-stable`). `RUSTWRIGHT_BENCH_GOTOS` and
`RUSTWRIGHT_BENCH_EVALS` change the iteration counts for both harnesses.

Run each a few times and compare medians. Timings include both driver and browser
work; driving the same executable does not isolate the cause of their differences.
The historical README snapshot uses an older reference version and has no
archived per-run data. Keep it separate from the repeated comparison below.

## Reliability and tail latency

The [2026-10-09–10 three-job comparison](reliability/COMPARISON_CI.md) records
Rustwright 4,800/4,800 and Playwright 4,200/4,800 measured successes on sixteen
local Chromium cases, with all raw failures, source archives and sampled memory.
It is a scoped fixture result; it does not establish general SOTA.

The [repeated matched-browser protocol](reliability/COMPARISON_PROTOCOL.md)
now aligns viewport and main-context policy and defines three separate CI job
environments. Its fresh schema-version-2 reports preserve failed setup and raw
phase sidecars; the historical single-host records below remain unchanged.

The [2026-10-08 local baseline](reliability/RESULTS.md) records a late-iframe bug
found through the comparison and the measurements after its fix.
The [2026-10-09 100-sample comparison](reliability/COMPARISON_100.md) records all
16 cases, reference failures, successful median/p95 and separate RSS/PSS after
correcting closed-page retention in a long-lived default context.

The reliability comparison uses shared local HTTP fixtures and checks the final
DOM state, rather than counting an API returning `Ok` as success. It exercises:

- Clicking a button inserted after 120 ms.
- Clicking a disabled button, a covered button and an animated button after
  each becomes actionable at 120 ms. The actual click must fire after readiness,
  with no click delivered to the covering element.
- Clicking a wide button clipped by an overflow container and a rotated button
  clipped by the viewport. Both engines use their default click APIs without
  explicit positions or forced clicks; point selection policies can differ.
- Filling and submitting an iframe inserted after 120 ms, with the iframe offset
  from the main page's origin. The submitted value must reach the parent.
- Filling and submitting an iframe from `localhost` while the parent uses
  `127.0.0.1`, both when inserted after 120 ms and when navigating from the
  parent site after 80 ms. The new document inserts its input after another
  120 ms. A CDP iframe target must exist, confirming a separate renderer;
  submission and origin checks are included in the measured postconditions.
- A fetch whose HTTP connection closes without a response, followed by a healthy
  navigation and a DOM readiness check.
- Closing the browser while a never-resolving evaluation is pending. The pending
  operation must return a closed-connection error within the watchdog.
- Mocked native fetch and XHR, including registration, main-document startup,
  new OOPIFs, return to the parent renderer and clearing routes. The first
  responses must have the expected status/body; actual server responses are
  failures while mocking is active.

Install Python 3.9+, Node and the locked Playwright dependency, and use the same
installed Chromium executable for both engines:

```sh
cargo build --release --locked -p rustwright-examples --example reliability
npm ci --prefix bench/playwright
python3 bench/reliability/run.py --chrome /path/to/chromium --require-engine-success rustwright
```

The runner starts and stops its own server. It saves raw observations, errors,
browser versions, source/binary hashes, success rates, median successful latency
and nearest-rank p95 in `target/reliability.json`. By default each engine measures
20 attempts per scenario in two pairs, alternating engine order. Each scenario
has one excluded warmup per pair. Use `--samples`, `--pairs` and `--output` to
change these settings; the sample count must be divisible by the pair count.
Use `--cases` to select a subset; the report records the selected cases and
rejects missing or unexpected samples.
Output files must be new. The runner records hashes of all nonignored repository
files, Git status, the Rust toolchain and OS as well as the harness/binary versions.
On Linux, an external observer also records driver RSS/HWM/PSS separately from
the sum of descendant processes (browsers and any harness helpers), approximately
once per second. Memory spans launch, setup, operations and shutdown, including
the temporary browser in the disconnect case. These samples are not an exact
peak or a steady-state leak test. RSS double-counts shared mappings; unreadable
PSS is null. The same observer runs for both engines and can perturb timing.

For the 100-sample comparison checkpoint, include every scenario:

```sh
python3 bench/reliability/run.py --chrome /path/to/chromium \
  --samples 100 --pairs 2 --require-engine-success rustwright \
  --output target/reliability-100.json
```

This alternates Rustwright/Playwright and Playwright/Rustwright order, with
50 measured attempts and one excluded warmup per case in each pair. Reference
failures remain in the report; successful p95 alone is insufficient to compare
an engine that failed attempts. One alternating pair of passes still measures
local fixture behavior, not statistically established superiority.

To measure only the five network scenarios:

```sh
python3 bench/reliability/run.py --chrome /path/to/chromium \
  --samples 30 --pairs 2 \
  --cases mock_fetch mock_navigation mock_frame mock_return mock_clear \
  --require-engine-success rustwright --output target/network-after.json
```

`mock_fetch` times registration and both requests on the existing page.
`mock_navigation`, `mock_frame` and `mock_return` install routes before timing;
the return case also creates and verifies its OOPIF before timing. `mock_clear`
times removing routes and navigation to a document whose first requests must
reach the server. These cases have no intentional fixture delay. The
[network timing results](reliability/RESULTS.md#network-timing-follow-up-2026-10-08)
include before/after values and reference failures.

The two-second watchdog covers the operation and its postcondition checks.
Browser launch and initial navigation are excluded from latency, but intentional
fixture delays are included. Untimed initial navigation uses ten-second API
timeouts in both engines; setup failures still fail the run. Failed attempts are
reported separately and never
count toward successful-operation p95; all-attempt p95 is also recorded.
`--require-success` makes measured failures fail the command after saving results.
`--require-engine-success rustwright` requires Rustwright's measured operations
to succeed while retaining all reference-engine failures in the report. It is
useful when testing new point selection: the rotated/clipped fixture can time
out with Playwright's default click position. A reference timeout has no
successful p95; it is not treated as a zero-latency observation.
Setup failures always fail the command rather than becoming skipped samples.

These cases establish a small local baseline, not overall superiority: launch
flags and page/context creation retain each engine's defaults, and the fixtures
do not cover all actionability checks, real sites or other browser engines.
Browser closure is graceful, not a process crash or forced transport reset. With
20 samples, p95 is the 19th sorted value; larger samples are needed to assess
production tail latency.

Check reporting semantics and the frame/click regression tests with:

```sh
python3 -m unittest discover -s bench/reliability -p 'test_*.py'
cargo test --locked -p rustwright-integration-tests --test http_frames -- --test-threads=1
```

The [network follow-up](reliability/RESULTS.md#cross-site-network-follow-up-2026-10-08)
covers OOPIF route inheritance, updates, cleanup and cached responses. Reproduce
the Chromium initial-fetch gap when an iframe returns to its parent's renderer
with Playwright's public APIs:

```sh
node bench/reliability/network-swap-probe.mjs /path/to/chromium
```

This diagnostic shows Playwright's default behavior and is separate from the
timed comparison. Rustwright's fix is covered by the HTTP regressions and
documented in the [renderer-return follow-up](reliability/RESULTS.md#renderer-return-route-gate-follow-up-2026-10-08).
