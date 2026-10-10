# Alternating 100-sample comparison — 2026-10-09

Rustwright completed **1,600/1,600** measured operations; Playwright Core completed
**1,399/1,600** on the same installed Chromium 151.0.7922.173, machine and local
HTTP fixtures. All 16 cases have exactly 100 measured attempts per engine, split
into two 50-sample passes. Order was Rustwright/Playwright, then Playwright/Rustwright.
Each case also has one excluded warmup per pass: Rustwright's 32 warmups succeeded;
Playwright had four warmup failures, separate from its 201 measured failures.

This run includes the [closed-page lifetime correction](../endurance/CACHE_LIFETIME.md)
discovered during the interrupted first comparison. The archived final source
and binary, rather than the Git commit alone, identify the measured implementation.

## Successful latency and outcomes

Latency columns are **median / nearest-rank p95 in milliseconds**, using successful
measured attempts only. A dash means no successful samples. Failed attempts and
their elapsed times remain in the raw report, including all-attempt p95.

| Scenario | Rustwright successes | Playwright successes | Rustwright median/p95, ms | Playwright median/p95, ms |
| --- | --- | --- | --- | --- |
| delayed_click | 100/100 | 100/100 | 176.1/194.3 | 234.7/329.6 |
| delayed_frame | 100/100 | 100/100 | 207.9/242.0 | 339.3/532.5 |
| cross_site_frame | 100/100 | 100/100 | 233.2/314.0 | 328.2/540.0 |
| cross_site_navigation | 100/100 | 100/100 | 294.9/379.4 | 385.7/570.1 |
| disabled_click | 100/100 | 100/100 | 173.5/201.4 | 214.0/268.3 |
| covered_click | 100/100 | 100/100 | 149.8/190.6 | 155.5/266.2 |
| moving_click | 100/100 | 99/100 | 202.8/219.8 | 226.2/316.9 |
| clipped_click | 100/100 | 100/100 | 81.7/118.1 | 104.4/236.7 |
| rotated_clipped_click | 100/100 | 0/100 | 80.1/114.1 | —/— |
| http_disconnect_recovery | 100/100 | 100/100 | 31.4/84.6 | 45.4/154.9 |
| browser_disconnect | 100/100 | 100/100 | 19.3/89.5 | 62.3/160.7 |
| mock_fetch | 100/100 | 100/100 | 10.4/35.1 | 19.0/54.4 |
| mock_navigation | 100/100 | 100/100 | 28.3/76.3 | 49.6/160.5 |
| mock_frame | 100/100 | 100/100 | 66.4/166.7 | 140.3/353.7 |
| mock_return | 100/100 | 0/100 | 33.0/101.2 | —/— |
| mock_clear | 100/100 | 100/100 | 22.0/68.5 | 38.9/151.4 |

Playwright's measured failures were:

- 100 rotated/clipped click timeouts with its default click position. The two
  excluded warmups also timed out. Rustwright's bounded quad-point selection
  and Playwright's default point policy differ; neither uses forced clicks or
  a supplied click position in this comparison.
- 100 renderer-return mock failures: first native fetch/XHR responses were
  `[200,"network"]` instead of `[201,"mocked"]`. Both excluded warmups failed
  the same check. An OOPIF was verified during untimed setup before return to
  the parent site. Native fetch/XHR were not replaced with JavaScript shims.
- One moving-click actionability postcondition failure in the first pass; the
  other 99 measured attempts and both warmups succeeded. The generic error does
  not distinguish readiness, actual click delivery or scheduling; it should not
  be attributed to a specific Playwright/browser defect from this record alone.

Rustwright's observed successful p95 is lower in the 14 cases with reference
successes. This is fixture-specific evidence, not statistically established
superiority, full feature parity, production tail latency or a SOTA claim.

## Separate memory observations

Maximum **sampled** Linux memory in MiB, observed by the same external Python
monitor at approximately one-second intervals. Each column is its own maximum;
the RSS/PSS maxima need not occur at the same instant.

| Pass | Engine | Samples | Driver RSS max | Driver PSS max | Descendant RSS max | Descendant PSS max |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Rustwright | 169 | 6.19 | 5.40 | 1,185.51 | 518.21 |
| 1 | Playwright | 402 | 239.47 | 237.99 | 855.99 | 379.66 |
| 2 | Playwright | 331 | 238.76 | 237.28 | 704.27 | 379.90 |
| 2 | Rustwright | 177 | 6.16 | 5.37 | 1,103.54 | 477.90 |

The driver is measured separately from all its descendants (browsers and any
harness helpers). Rustwright has lower observed driver memory and higher
descendant peaks in these runs. Memory spans launch, setup, operations and
shutdown, including the temporarily overlapping browser in the disconnect case.
Default launch flags, initial tabs/context policies, errors, GC and retained
allocations differ; this is not a matched single-browser footprint or a leak test.
RSS double-counts shared mappings; PSS apportions them. Missing PSS is null,
not zero. Process-tree reads are sequential, and monitoring can perturb timing.

The interrupted pre-fix 50-sample Rust pass reached 36.21 MiB driver RSS; the two
post-fix passes reached 6.19 and 6.16 MiB. Registry regressions independently
establish the correction. These passes do not constitute a statistically
controlled percentage reduction estimate. The [1,000-cycle default-context
run](../endurance/CACHE_LIFETIME.md) provides separate post-fix endurance evidence.

## Reproduction and preserved records

```sh
cargo build --offline --locked --release --example reliability
python3 bench/reliability/run.py --chrome /workspace/.rustwright-env/chromium \
  --samples 100 --pairs 2 --require-engine-success rustwright \
  --output target/reliability-100-after.json
```

Python's standard library, Node 24.19.0 and locked Playwright Core 1.63.0 were used.
Rustwright was built with Rust 1.99.0 in release mode. Chromium's existing cloud
wrapper accommodates container OS sandbox restrictions. All fixtures are HTTP;
no browser URL policy was changed. No heavy builds or other browser suites ran
concurrently. The [scenario/reporting definitions](../README.md#reliability-and-tail-latency)
explain the two-second operation/postcondition guard, ten-second initial-navigation
timeout, intentional delays, excluded setup and graceful browser disconnection.

[Full report](results/100-samples/report.json), runner log, source archive and
source hashes preserve all 3,264 records (3,200 measured, 64 warmups), versions,
timestamps, Git status/revision, toolchain, platform and 1,079 memory samples.
[Verification](results/100-samples/source-check.json) confirms all four passes,
alternating order, 100 measured records per case/engine, exact Rust binary hash
and 108 code/fixture/config source hashes. Driver PIDs exited after completion;
every former browser descendant PID was not tracked through shutdown.

The fixtures retain adversarial cases that can fail with the reference defaults;
no failing case was removed to improve latency or success rate. Two alternating
pairs do not randomize scenario order or provide many independent host repeats.
Firefox, headed browsers, real sites, Windows/macOS and unexpected transport
crashes are outside this comparison. Broader evidence is still required for the
[90% checkpoint](../../docs/RELIABILITY_ROADMAP.md).
