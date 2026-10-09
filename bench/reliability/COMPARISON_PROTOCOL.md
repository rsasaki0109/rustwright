# Repeated matched-browser comparison

This protocol follows the completed scoped 95% reliability checkpoint. It
evaluates Rustwright against Playwright Core on sixteen local Chromium cases.
It does not define a general state-of-the-art result or a reliability percentage.

The `Matched Chromium comparison` workflow runs three separate Ubuntu job
environments, with no builds or other browser tests concurrent with measurement
inside a job. Each builds the same release source with locked dependencies and
uses Chrome for Testing `155.0.8059.39`, Playwright Core `1.63.0`, Node `24.19.0`
and Rust `1.99.0`. CLI/protocol versions and executable hashes identify the
browser; job/host identities distinguish the recorded environments. Separate
jobs do not establish physically independent hosts or a population-level sample.

Each environment measures all sixteen existing cases, with 100 attempts per
engine per case. Two passes each measure 50 attempts plus one excluded warmup:
Rustwright then Playwright, followed by Playwright then Rustwright. Case order
is fixed. There is no selection of only favorable cases, adaptive stopping,
automatic retry or replacement of unsuccessful observations.

Both engines use a 1280×720 desktop viewport with device scale factor 1. Every
page verifies its actual inner dimensions and scale before untimed setup. The
main browser reuses one context; the disconnect case owns a fresh browser and
context. Chromium sandboxing is enabled, GPU rendering disabled and site
isolation retained. Both use the same localhost HTTP fixtures, operation
postconditions, two-second watchdog and ten-second initial-navigation budget.
No forced clicks, supplied click position or JavaScript network replacement is
used. Driver-specific flags, initial tabs, click-point policies and debugging
port versus pipe remain different and are recorded rather than described as
identical browser configurations. The main-context policy and viewport alignment
are changes from the historical single-host comparison.

The sixteen cases retain delayed/disabled/covered/moving/clipped controls,
late and cross-site frames, navigation, HTTP-disconnect recovery, graceful
browser closure and native fetch/XHR interception, including renderer return.
Setup and launch are outside operation latency. Intentional fixture delays,
actual operation and postcondition checks are inside it. Browser closure is
graceful, not a forced process crash.

The runner writes schema-version-2 reports plus exclusive raw stdout/stderr and
process sidecars. Failed setup, malformed output, timeout and catchable Python
interruption preserve their available evidence and return failure. Missing or
unrun phases are not manufactured samples. Owned process-group cleanup and
direct-child reaping are recorded with finite waits; descendants that escape
the group and host shutdown are outside that guarantee. Cleanup is additional
to the command operation deadline.

The aggregator requires three distinct environment/run identities, the same
tested commit and declared source inputs, toolchain and exact browser binary.
It checks all four complete phases per environment, exact case/index/order and
warmup flags, raw sidecar hashes and content, launch policy, viewport census,
fixture cleanup and process outcomes. It independently recomputes supplied
summaries. Failed or incomplete setup cannot become a valid comparison.
Rustwright measured failures remain data but fail its declared success gate;
reference measured failures remain visible and do not fail that gate.

Tables show successful counts, separate warmup failures, successful median and
nearest-rank p95, failed observations and all-attempt p95. Ratios are emitted
only where both engines succeed on every measured attempt in the corresponding
scope. Pooled records are descriptive, not statistically independent estimates
or confidence intervals. An engine with failed attempts cannot improve its
claimed advantage by excluding those attempts.

The same external Linux observer samples driver and descendant RSS/PSS about
once per second. Each quantity has its own sampled maximum; missing PSS is
null. Descendants include browsers and helpers, RSS double-counts shared pages,
and readings are sequential. Sampling spans launch, setup, work and shutdown
and can perturb timings. These values are neither exact peaks, matched single
browser footprints nor leak-freedom evidence.

## Reproduce

```sh
npm ci --prefix bench/playwright
cargo build --release --locked -p rustwright-examples --example reliability
python3 bench/reliability/run.py --chrome /path/to/the/pinned/chrome \
  --samples 100 --pairs 2 --require-engine-success rustwright \
  --output target/comparison/report.json
```

For the three-report CI evaluation, preserve each report and its sibling
`report.json.artifacts` directory. Reports must come from the declared distinct
CI jobs and outputs must be new:

```sh
python3 bench/reliability/aggregate.py --reports \
  target/comparison/input/comparison-host-1/report.json \
  target/comparison/input/comparison-host-2/report.json \
  target/comparison/input/comparison-host-3/report.json \
  --output target/comparison/aggregate.json \
  --markdown target/comparison/REPORT.md
```

The workflow uploads all original measurements even when a job fails. Results
must retain the actual tested checkout, source and driver identities, official
artifact digests, failures and this protocol's limits. The historical
`COMPARISON_100.md` is not relabeled as a new run. Firefox, headed comparison,
public-site benchmarks, other operating systems/drivers, randomized workload
order, longer endurance and stronger statistical analysis remain outside this
first repeated comparison.
