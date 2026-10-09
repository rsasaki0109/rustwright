# Linux browser endurance check

For delayed-acknowledgment Firefox routing checks and ownership limits, see
[interception lifecycle verification](INTERCEPTION_LIFECYCLE.md).

Run from the repository root with Chrome and Firefox installed. This check uses
local HTTP fixtures; it does not require file/data URL access. Missing browsers,
startup failures, incorrect results and timeouts fail the run.

```sh
cargo build --locked --release --example endurance
python3 bench/endurance/run.py --output target/endurance/run-01
```

The default runs Chrome and then Firefox, each with **100 warmup cycles followed
by 1,000 measured cycles**, using one browser process/session throughout. Firefox
uses a disposable profile. Every cycle creates an isolated context/page,
navigates, fills a Unicode value, verifies an absent-locator timeout, discovers
the page and closes the context. Even cycles explicitly close the page first;
odd cycles close the context with the page still open. Every tenth cycle also:

- Starts a never-resolving browser evaluation, verifies it started, cancels the
  Rust task, and checks a subsequent command succeeds.
- Mocks a fetch response, removes routing and verifies a live server response.
- Navigates an iframe same-origin → cross-origin → same-origin, checking trusted
  clicks after each navigation, then removes the iframe.

Each cycle has a 15-second watchdog; discovery samples and final shutdown have
separate deadlines. Errors produce a nonzero exit. Samples at startup, the end of
warmup, each 100 cycles and the end record page targets, isolated contexts,
pending driver commands and memory. Counts must return to their startup values;
Firefox may start with built-in container contexts. Pending commands must be zero
at sampled checkpoints. These checkpoints do not continuously observe all
intermediate state or count CDP attached sessions or intercepts. Firefox also
records `acknowledged_helper_preloads`, the session's successful internal helper
registrations minus acknowledged removals (including a `no such script` response).
It must return to zero. This client ledger excludes caller-managed raw scripts
and cannot enumerate the browser's registry or independently observe external
removals and session reset. Cancellation removes driver response waiters;
browser-side promises
may keep running until their document is destroyed.

Memory comes from Linux `/proc/<pid>/status` `VmRSS` in KiB, separately for the
Rust driver and the browser process plus its current descendants. The browser
sum double-counts shared pages; it is **not PSS or unique allocated bytes**. A
100ms pause precedes samples; browser caches, process reuse, garbage collection
and allocator retention can affect them. Growth after warmup calls for
investigation; a passing count check is not proof of absence of leaks.

The current harness additionally reads `Pss` from `/proc/<pid>/smaps_rollup`,
which apportions shared mappings instead of counting each mapping in full.
Driver PSS, browser-tree PSS and per-process RSS/PSS are recorded separately.
An unreadable PSS is `null`; a tree total is only available if every sampled
descendant was readable. Processes and mappings can change during sampling, so
these are sequential observations, not an atomic process-tree snapshot. Earlier
recorded runs contain RSS only. PSS still includes retained allocations and
caches and does not identify unreachable objects or prove absence of leaks.

## Firefox operation ablation

```sh
cargo build --locked --release --example endurance
python3 bench/endurance/ablation.py --output target/endurance/ablation-1000
```

This runs three smaller workloads, each with a fresh disposable Firefox profile,
one session, 100 warmup cycles and 1,000 measured cycles, twice in forward/reverse
order: `raw`, `raw-helper`, `basic`, then `basic`, `raw-helper`, `raw`. They use
the same Rust executable, BiDi connection/session, launcher, HTTP fixture,
discovery/count checks and sampling cadence. In each cycle an isolated context
and tab are created, navigated, checked for the expected title, discovered,
checked again and closed. Even cycles explicitly close the page first; odd
cycles close the context with the page open.

- `raw` sends BiDi commands directly, bypassing `BidiPage`, `BidiContext` and
  automatic helper injection.
- `raw-helper` adds the exact driver's helper preload, evaluates it in the
  initial document, and awaits successful removal on page/context closure.
  These caller-managed scripts are excluded from the internal-helper ledger.
- `basic` uses the page/context APIs and their automatic shared-helper ownership.

The `full` workload remains the default of `run.py` and the Rust example.
Select an individual workload with `run.py --engine firefox --workload raw`;
the direct BiDi workloads require Firefox. Ablation omits form input, absent
waits, cancellation, routing and frame swaps, so it does not replace the full
endurance test. All direct workloads still use Rustwright's transport/session;
they are not an independent WebSocket implementation or a Firefox heap profile.
Two runs per workload expose order/run variation but cannot conclusively
attribute memory to a leak, garbage collection or browser caches.

## Chrome default-context churn

```sh
python3 bench/endurance/run.py --engine chrome --workload default \
  --cycles 1000 --warmup 100 --output target/endurance/default-context
```

This reuses the browser's default context rather than disposing an isolated
context after every page. Each cycle creates/navigates a page, fills and verifies
a Unicode value, and closes it while a cloned page handle is still retained.
Half the pages close through the page API; half close through an independent CDP
observer, then await the original page's closed state. The workload deliberately
never calls `pages()` or `refresh_pages()`, because their lazy pruning previously
concealed retained closed handles. A protocol regression directly checks the
registry without invoking either API. Normal sampled target/context/pending
checks and RSS/PSS measurements still apply. This is a different workload from
the Firefox ablation and full isolated-context test.

The runner preserves JSONL samples, stderr, a summary, SHA-256 hashes of repository
files and the exact binary, Git status/revision, Rust version and OS. Browser
versions are recorded by the executable. Build before running; source hashes
alone do not prove a previously built binary matches the source. A shorter smoke
run uses `--cycles 20 --warmup 10`. Output directories must be new so previous
measurements are not overwritten. This is an endurance check, not a comparative
performance benchmark.

[Recorded observations, fixes and remaining limits](RESULTS.md).

[Independent Python/WebSocket attribution](INDEPENDENT_BIDI.md) compares matched
helper-free context/tab churn with the Rust raw workload and adds context-only
and single-page evaluation controls, per-process RSS/PSS and idle-tail samples.
It requires Python 3.11+ with `websockets` (verified with 16.0). Protocol errors,
incorrect titles/resource IDs and unreadable live-process memory are preserved;
missing browsers and startup failure fail rather than skip.
Set `RUSTWRIGHT_FIREFOX` to the installed executable or existing launcher path.

```sh
python3 bench/endurance/independent_bidi.py --workload churn \
  --cycles 1000 --warmup 100 --output target/endurance/independent-churn
python3 -m unittest discover -s bench/endurance -p test_independent_bidi.py
```

The separate [transport lifetime check](TRANSPORT_LIFETIME.md) covers abnormal
disconnection, blocked writes/close flushing, final-handle Drop and 25 actual
process-loss/manual-restart cycles per backend. Its resource-release assertions
complement the sampled memory workloads above; it is not a memory benchmark.

[Creation cancellation checks](CREATION_CANCELLATION.md) delay actual allocation
and initialization acknowledgments, cancel the caller and verify exact resource
IDs before closing any owner context. They also check subsequent page use and
bounded local waiting when an identifier never arrives.
