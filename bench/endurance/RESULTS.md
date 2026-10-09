# Linux endurance observations — 2026-10-09

The subsequent [Chrome closed-page retention correction](CACHE_LIFETIME.md)
releases page state without requiring discovery. A separate 1,000-cycle default
context run alternates explicit/external closure, records RSS/PSS and preserves
the protocol regressions and the interrupted comparison that exposed it.

The subsequent [Firefox operation ablation](ABLATION.md) records another 6,000
measured cycles in matched forward/reverse-order runs, including direct BiDi
commands with and without the helper. It adds browser-tree and per-process PSS;
growth also occurs without automatic page/context/helper ownership and remains
unattributed. The full-workload observations below retain their original scope.

The subsequent [independent WebSocket study](INDEPENDENT_BIDI.md) records 6,000
Python cycles and 2,000 matched Rust raw cycles. Browser PSS growth also occurs
without Rustwright's transport, and decreases during some idle tails. It does
not identify unreachable browser heap objects. A separate correction removes
historical frame-ID retention in the driver; native same-page iframe churn and
acknowledged monitoring setup have their own verification records.

The subsequent [Firefox interception correction](INTERCEPTION_LIFECYCLE.md)
verifies delayed-acknowledgment registration/removal, shared page handles,
final-owner retirement and retained-handle resource close with native HTTP
traffic. Actual known intercept IDs are queried after cleanup; this does not
retroactively add an intercept census to the historical endurance runs below.

Post-fix Chrome and Firefox each completed **1,000 measured context/page cycles**
after 100 warmup cycles, with zero functional failures and pending commands at all
sampled checkpoints. A second post-fix Firefox run also completed 1,000 measured
cycles and recorded **zero internal helper preloads awaiting acknowledged removal
at every checkpoint**. Page targets and isolated contexts returned to their
startup counts throughout. These are sampled observations, not proof of absence
of every leak.

There are 5,000 measured cycles across the before, after and instrumented runs,
plus 500 warmup cycles. The runner's per-cycle checks include Unicode form values,
absent-locator deadlines and page discovery. Each measured run includes 100
cancelled, already-started evaluations, 100 mock/clear/live-fetch checks and 300
iframe navigations with trusted clicks. See [the workload and commands](README.md).

## Memory observations

Linux `/proc` `VmRSS`, in **MiB** (KiB / 1024), from the end of warmup to the end
of measurement. Browser memory is the root process plus descendants and
**double-counts shared pages**; it is not PSS or unique allocated memory.

| Pass | Backend | Measured cycles / failures | Driver RSS warmup → end | Browser tree RSS warmup → end (delta) | Helper preload ledger |
| --- | --- | --- | --- | --- | --- |
| Before fix | chrome | 1,000 / 0 | 5.44 → 5.64 | 539.04 → 558.13 (+19.09) | Not recorded |
| Before fix | firefox | 1,000 / 0 | 4.68 → 4.83 | 664.41 → 754.36 (+89.96) | Not recorded |
| After fix | chrome | 1,000 / 0 | 5.21 → 5.45 | 538.45 → 557.09 (+18.63) | Not recorded |
| After fix | firefox | 1,000 / 0 | 4.67 → 4.98 | 652.95 → 706.75 (+53.79) | Not recorded |
| After fix + helper ledger | firefox | 1,000 / 0 | 4.79 → 4.99 | 679.34 → 699.58 (+20.24) | 0 at all checkpoints |

The two post-fix Firefox runs have different endpoint growth. Memory still rises
in parts of their tails, and no steady-state bound has been established. Browser
caches, garbage collection, renderer reuse, allocator retention and remaining
resource defects need separate attribution. Do not turn this single before/after
pair or its follow-up into a percentage leak reduction or a performance claim.
Run order was sequential, timing was not controlled and there was no Playwright
comparison. A longer run and a raw-protocol baseline can help separate driver
behavior from browser behavior before drawing stronger conclusions.

## Confirmed driver defects and corrections

Firefox helper registrations were separate for every discovered page handle.
The protocol regression with one page and 22 discoveries retained 23 scripts;
it now retains one shared registration. Context closure previously left the
helper registered while a page handle was kept alive. Last-handle destruction
also left it registered. Cancelling registration after the remote end created
its script but before replying lost the identifier needed for cleanup.

Five protocol regressions now pass for shared discovery, closing a context with
retained pages, preserving another context's helper, last-handle cleanup and
cancelled registration. The session indexes weak references to helpers; helper
installation is serialized and, once started, completes bookkeeping despite
caller cancellation. Explicit page/context closure removes helpers, and last
handle destruction schedules best-effort cleanup on a live Tokio runtime.

The [independent native Firefox probe](results/before/preload-probe.jsonl)
confirmed that a preload scoped to a context could still be removed successfully
**after that user context was destroyed**. The
[probe source](results/before/preload-probe.py) requires Python `websockets`
(tested with 16.0) and an installed Firefox. The main endurance runner uses only
Python's standard library. Raw user init scripts remain caller-managed; the new
helper ledger counts the driver's own acknowledged registrations/removals, not
an independent enumeration of the browser's script registry.

The common HTTP matrix also exposed intermittent Chrome hover failure in a
newly loaded cross-origin iframe. Hover now awaits two animation frames after
scrolling before measuring and dispatching. The formerly failing native case
passed 30 consecutive repetitions, and the full shared matrix passed 24/24.

## Validation and preserved evidence

- Workspace unit tests: 65 passed, including the five new lifecycle regressions.
- Shared real-browser HTTP matrix: 24 passed, 0 skipped.
- Locked offline Rust 1.85.0 checks across all targets; stable Clippy with
  `-D warnings`, formatting and diff whitespace checks passed.
- Chrome 151.0.7922.173 and Mozilla Firefox 157.0.1, headless Linux, Rust 1.99.0.
  Existing cloud launchers accommodate container OS sandbox restrictions; all
  fixtures are HTTP and browser URL policies were not changed.
- Raw JSONL, summaries and manifests: [before](results/before/summary.json),
  [after](results/after/summary.json),
  [helper-ledger follow-up](results/helper-ledger/summary.json). Each directory
  includes source hashes, binary hash, Git revision/status, UTC start time,
  platform and Rust version. The after/follow-up hash checks matched their
  recorded files and executed binaries before this result document was written.

Each directory also preserves its exact `driver.rs`, verified against the
manifest's `examples/endurance.rs` hash. The post-fix driver retains a page clone
through context closure, explicitly exercising cleanup while handles are alive.
The follow-up adds the helper ledger. These changes and the uncontrolled host
make the memory rows observations rather than a controlled isolated comparison.
The original working tree already contained earlier uncommitted improvements;
the recorded Git HEAD alone does not identify its complete measured source.

Chrome starts with one default page and zero isolated contexts; Firefox starts
with one page and four built-in container contexts. Returning to those values
means newly created contexts/pages were removed. The harness does not enumerate
CDP attached sessions or BiDi intercepts, continuously observe intermediate
state, cancel native context/page creation or shutdown, or establish Windows,
macOS, headed-browser or real-site reliability. Native evaluation cancellation
cleans driver response waiters; browser-side execution can continue until its
document is destroyed. The helper-cancellation regression tests bookkeeping after
a page was already created, not arbitrary cancellation of target creation.
