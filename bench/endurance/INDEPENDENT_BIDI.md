# Independent Firefox memory attribution and document helpers — 2026-10-09

The earlier Firefox ablation observed browser PSS growth even without page
wrappers or helper injection, but every workload still used Rustwright's BiDi
transport. This follow-up uses Python `websockets` directly, and separates
context-only churn and repeated evaluation from the matched context/tab workload.
It also corrects a distinct driver retention defect found during source review.

## Independent measurement

The independent runner imports no Rustwright transport, session, page, launcher
library or injected helper. It launches the same installed Firefox executable
through the existing environment wrapper, with an explicit fresh profile and a
local HTTP fixture. It discovers the browser's advertised BiDi endpoint and
starts one session. A strict sequential request loop preserves protocol failures,
JavaScript exceptions and timeouts; its pending command ledger is zero at every
successful checkpoint. No garbage collection or browser preference changes are
used to obtain memory measurements.

Set `RUSTWRIGHT_FIREFOX` to the Firefox executable or existing environment
launcher path before invoking the independent runner.

```sh
# Python 3.11+ and websockets 16.0; Firefox must be installed.
python3 bench/endurance/independent_bidi.py --workload churn \
  --cycles 1000 --warmup 100 --checkpoint 100 --idle-tail 0,10,30 \
  --output target/endurance/independent-churn
```

The `churn` command sequence and HTTP document match the current Rust example's
`raw` workload: create a user context/tab, navigate, read the title, discover the
tab, read the title again, explicitly close the tab on even cycles, and remove
its user context. `context-only` creates/removes user contexts without tabs or
script evaluation. `evaluate` navigates one retained isolated page before taking
its baseline, then repeatedly evaluates `document.title` with
`resultOwnership: "none"`. No scripts or intercepts are registered in these
independent workloads.

Eight runs use fresh browser roots/profiles in this order:

| Order | Driver/workload |
| --- | --- |
| 1 | Rustwright raw BiDi |
| 2 | Independent Python context-only |
| 3 | Independent Python evaluation on one retained page |
| 4 | Independent Python context/tab churn |
| 5 | Independent Python context/tab churn |
| 6 | Independent Python evaluation on one retained page |
| 7 | Independent Python context-only |
| 8 | Rustwright raw BiDi |

Each run executes 100 warmup and 1,000 measured cycles. Independent samples record
exact sorted page/frame and user-context IDs at startup, warmup end, each 100
cycles and measurement end; the IDs must return to each workload's baseline.
Additional samples at 0, 10 and 30 seconds after the measured cycles check an
idle tail. Those samples still issue count queries; they are not protocol silence.
The Rust driver retains its existing sampled count checks and has no idle tail.

RSS and PSS separate driver, browser root, browser tree and individual processes.
The independent sampler excludes only positively identified `State: Z` zombies,
which have released their address spaces, and records their IDs/states. Missing
measurements for a live or unknown-state process remain `null` and make the
relevant aggregate unavailable. Rust's existing sampler only includes processes
with readable RSS. Both read `/proc` sequentially, not atomically; root and tree
metrics can differ because renderer processes exit or change during sampling.
Eleven independent protocol/sampler tests cover these failure and measurement
semantics. An earlier three-cycle smoke with the old sampler is preserved but
excluded from measured results.

All **8,000 measured cycles plus 800 warmups** passed. All 114 checkpoints
reported readable browser-tree PSS, restored sampled resources and zero pending
commands. Values below are endpoint changes after each run's 100-cycle warmup,
in MiB (KiB/1024). Idle deltas compare the final measured sample with the sample
30 seconds later; Rust has no corresponding tail sample.

| Order | Workload | Driver PSS delta | Browser-tree PSS delta | Browser-root PSS delta | Root delta during 30-second idle tail |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | Rust raw | +0.05 | +47.18 | +47.91 | Not measured |
| 2 | Python context-only | +1.04 | +24.62 | +23.96 | −57.80 |
| 3 | Python evaluate | +1.00 | +32.15 | +18.63 | −36.11 |
| 4 | Python churn | +0.89 | +77.04 | +82.23 | −60.79 |
| 5 | Python churn | +1.05 | +34.01 | +40.01 | −41.32 |
| 6 | Python evaluate | +0.60 | +29.04 | +16.07 | −33.39 |
| 7 | Python context-only | +1.03 | +8.51 | +6.75 | −39.74 |
| 8 | Rust raw | +0.02 | +40.54 | +45.77 | Not measured |

Browser growth occurs with an independent transport, so Rustwright's transport
is not necessary for this observation. Every independent root also decreases in
its 30-second tail; some end below their warmup baseline. This is compatible with
delayed cleanup, garbage collection, caches and allocator behavior, without
identifying which explanation applies. It does not establish a steady bound or
absence of unreachable objects. Driver endpoints differ across runtimes, and
these rows are not a memory-efficiency ranking.

![Browser-root PSS relative to each warmup endpoint](results/independent-memory/root-pss.png)

Solid lines show measured cycles, circles mark their end, and dashed lines/squares
show idle samples. Time is elapsed since each run's own warmup endpoint. The
matched churn runs take 84–93 seconds after warmup; the faster controls take only
6–8 seconds, reinforcing the rate/duration limitation below.

Two runs per workload expose run/order variation rather than establish statistical
significance. Context-only and evaluation run much faster than tab/navigation
churn; operation rates and total elapsed durations are not matched. Their deltas
cannot identify which operation causes a heap leak. PSS still includes browser
caches, reachable objects and allocator retention. Idle decreases do not prove
every remaining allocation will be released. The experiment does not enumerate
all sessions/intercepts or inspect reachable browser heap objects.

The two Rust measurements use a frozen release binary built after the previous
shutdown/actionability fixes and before the document-helper/subscription changes
below. Its source/config archive and binary hash are retained. It sends raw BiDi
commands and installs no helper or monitoring subscriptions. Unrelated production
sources were edited during the sequential measurement window, but the frozen
binary, independent runner and measurement sources did not change, and no heavy
compilation or other native browser suites ran concurrently. Per-run manifests
record each observed working tree; their Git HEAD alone does not identify the
executed Rust implementation.

## Historical frame IDs retained by the driver

`PageHelper.contexts` previously remembered every browsing-context ID into which
the helper had been injected. Removing a child iframe did not remove its ID from
that set. A long-lived top-level document therefore retained historical frame
IDs even with no remaining iframe. A same-ID document replacement could also be
mistaken for a document with its helper already installed.

An isolated pre-fix protocol regression creates, injects into and destroys 1,000
different frames. It finds **1,001 retained IDs with only one live document**.
Four meaningful behavior tests record three failures and one pass before
correction: replacement documents, destroyed contexts and late injection replies
fail, while an unchanged live document correctly reuses its helper.

The correction removes the Rust frame-ID set completely. Each helper check probes
the current document with `!!window.__rustwright` and injects only if absent.
Existing shared preload ownership is unchanged; late success replies cannot
populate a stale historical ID cache. This adds one compact evaluation roundtrip
per helper check, and absent helpers require a subsequent injection command.
Earlier comparative latency measurements predate this change.

The probe and locator operation remain separate commands; navigation between them
can still invalidate the operation. The correction does not claim to eliminate
every document replacement race or that Firefox always fails to apply preloads
after navigation. Native tests explicitly remove a document's helper to verify
absence is detected independently of browsing-context identity.

## Acknowledged network monitoring setup

Review also found that session subscription and page monitoring set their ready
flags before awaiting `session.subscribe`. A concurrent caller could receive
success before subscription acknowledgment, and cancellation could leave
monitoring marked ready without a working pump.

Protected workers now serialize setup and publish readiness only after a
successful acknowledgment and pump creation. Cancellation of a waiting caller
does not cancel that worker. Each setup has an overall five-second deadline,
including lock contention; typed failures remain visible and subsequent calls
can retry. The monitoring receiver is subscribed before its task starts so an
immediate event is not lost in the task scheduling gap. Cloned page handles share
one successful setup/pump.

Six identical protocol regressions record **five failures and one pass before,
six passes after**: acknowledgment waiting, cancellation ownership, typed
rejection/queued retry, one reused monitoring pump with received events, retry
after rejected monitoring, and a virtual-clock silent-peer deadline.

This is not exactly-once remote subscription allocation under lost replies: a
browser can subscribe successfully without delivering its acknowledgment before
the deadline. A later retry can then create another subscription. The local
deadline bounds waiting, not proof of remote reclamation. Intercept registration,
removal cancellation and last-handle cleanup remain separate audit findings;
opt-in network monitoring also still retains complete request history. Neither
explains the earlier raw/basic browser PSS growth, whose workloads omit frames,
routing and monitoring.

## Results and verification

Final measured results and source/binary identities are preserved under
[independent-memory](results/independent-memory/summary.json). Native helper and
subscription validation, before/after logs and final source/configuration hashes
are preserved under
[frame-helper-cache](results/frame-helper-cache/validation.json).
The native frame test keeps one top-level document for 1,000 alternating
same-/cross-origin iframe creation, locator and removal cycles; every 100 cycles
also exercises Unicode fill and a trusted click, and exact resource checks follow
frame removal. A separate test checks absent helpers and ten document reloads.
The native subscription test holds an actual successful subscription response,
cancels its caller, verifies another setup still waits, releases the response
and observes ten subsequent HTTP requests with completed 200 responses.

These measurements and regressions use headless Linux/Firefox 157.0.1, local
HTTP and the existing container launcher. Windows/macOS, headed browsers,
representative sites, browser heap profiling and remote CI remain unexecuted.

Final integrated checks pass **131/131 workspace library tests**, **156/156
native HTTP tests** across eleven targets, and **11/11 Python protocol/sampler
tests**. Rust 1.85.0 locked all-target compilation, stable Clippy with warnings
denied, formatting and whitespace checks pass. The archive retains 129 final
code/configuration files, matching source hashes and 19 unit/native executable
hashes. Stage-specific before/after sources and protocol logs are distinct from
the final integrated snapshot and the earlier frozen memory measurement binary.
