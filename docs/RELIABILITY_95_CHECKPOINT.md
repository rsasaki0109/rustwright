# Reliability 95% checkpoint: scoped verification complete

**Status: the specified 95% development milestone is complete.** All seven
required jobs in [run 37994561086](https://github.com/rsasaki0109/rustwright/actions/runs/37994561086)
succeeded, and their source, logs, artifact digests, native matrix and package
provenance are frozen and independently checked. The provisional completion
assessment is **95% for this scoped development milestone**. It is not a
measured probability of successful operations or a percentage computed from
test totals. Broader SOTA and universal leak-freedom claims remain unproven.

The source fixes owned startup cleanup, releases closed Firefox observers,
verifies actual Chrome target closure during discovery initialization, and uses
a bounded native CDP sandbox preflight. Historical studies below retain their
original source identities rather than being relabeled as final-source runs.

## Final source and completed CI

The tested head is `39e1c8951dd2ebd67cd1ef8e7311f582d418d873`; all seven original
checkout logs identify actual PR merge
`a2191f83923497f1e34a7785f316e142206dda54`. The
[source-equality record](ci/results/reliability-95/remote-final/tested-source-equality.json)
checks identical Git blobs and modes for 153 relevant source, test, helper,
workflow and package inputs. The exact-head source archive SHA256 is
`d2d0909443167cee5afc13b50e790607531b4cf4d9ba426fd38f2db696e228da`.
This proves equality for those inputs, not identity of every file or commit.

| Completed check | Recorded result |
| --- | --- |
| Workspace suites | 475 passed, zero failed/filtered; one existing ignored macro doctest |
| Library subset | 211 passed, including 111 BiDi; zero failed/ignored/filtered |
| `compat_report` example subset | Four passed |
| Formatting, all-target Clippy and locked Rust 1.85 all-target compilation | Passed |
| Required headless native HTTP, each of Windows/macOS arm64/Ubuntu | 79 passed: 39 Chrome, 40 Firefox; zero failed/ignored/filtered |
| Additional Linux native HTTP | 114 passed; zero failed/ignored/filtered |
| Linux headed current-version portable suite | 79 passed; zero failed/ignored/filtered |
| Linux headless alternate-version portable suite | 79 passed; zero failed/ignored/filtered |
| Selected headed public sites | Three per backend, all operations and teardown successful, six valid hashed PNGs |
| Distribution | Eight actual crate archives, 57 verified dependency checksums, nine README fences on stable/MSRV |
| Stable packaged native consumers and README entry points | Four consumers and two programs passed; five PNGs retained |

[The frozen final CI record](ci/results/reliability-95/remote-final/README.md)
retains all seven logs and six original ZIPs with verified official digests.
All eight archive VCS records identify the actual tested merge; 101 archived
source files and the 106-input package fingerprint match. This is verified
packaging and consumption, not registry publication. The workspace includes
193 HTTP cases, so overlapping native jobs are not additional unique designs.
Repeated 79-case suites demonstrate the recorded OS/mode/version matrix.

The prior [six-job baseline](ci/results/reliability-95/remote/README.md) at head
`4cdec571` and merge `58ac49f` remains unchanged. Its earlier 152-input proof and
two failed extended attempts remain historical records, separate from the
final 153-input verification above.

## Discovery closure correction

The earlier CI run found `SessionDetached` during `Page.enable` while listing a page that had already closed. The correction verifies browser-level target existence after a typed detached-session initialization failure. Only the exact `Target.getTargetInfo` no-target error permits skipping the closed target; unexpected errors still propagate. It adds no retry or arbitrary initialization-error suppression.

The same eight controlled-peer cases produced **2 passed / 6 failed before**, then **8 passed / 0 failed after**. The native Chrome 155 context suite passed all ten cases. The two frozen final source files match commit `4cdec571` exactly. See the [discovery-closure evidence](ci/results/reliability-95/discovery-closure/summary.json); broader final-source CI is recorded separately above.

## Owned startup and closed-observer resources

The [startup ownership evidence](ci/results/reliability-95/startup-ownership/summary.json) retains controlled real Unix-process regressions for timeout, cancellation, early exit, and successful readiness ownership transfer. Six added regressions failed before the fix; the final suite including its transfer control passed 11/11. These controlled processes are distinct from native browser compatibility observations.

The [Firefox resource audit](ci/results/reliability-95/resource-audit/ARCHIVE_README.md) demonstrates three cases where a retained page handle previously kept an idle or diagnostic pump alive after successful local page/browser closure without another remote destroy event. Those three regressions failed before the fix; the fixed BiDi suite passed 111/111. The local close signal and connection shutdown now finish the shared monitoring workers while preserving setup cancellation ownership. Final combined-source library CI supersedes this earlier scoped suite for the current source.

The [native startup archive](ci/results/reliability-95/native-startup/README.md) adds **ten Chrome 155 and ten Firefox 157 successful early-startup cancellation observations on Linux**. Each cycle observes the actual browser executable through `/proc/PID/exe`, verifies direct parentage and that the public launch API has not returned, and cancels without a scheduling gap in a current-thread runtime. After cancellation, every direct PID is absent and `waitpid` reports `ECHILD`, demonstrating that the library already reaped that direct child. Chrome ephemeral profiles are removed; Firefox caller-profile sentinel bytes are preserved before fixture-owned cleanup. No observer cleanup kill or reap was required.

These native wrappers only record PID/arguments and immediately exec the existing container launcher. They impose no startup gate or synthetic readiness response. The assertions establish native executable loading and direct-child/profile ownership during very early startup; they do not establish how far initialization progressed or cancellation at every later stage. The build records HEAD `908323d` with the then-uncommitted context correction represented by exact source hashes. The frozen context files match the later `4cdec571` files; Firefox/startup sources match `908323d`. The historical run is not relabeled as a later commit. The 20 repetitions are observations of two backend scenarios, not 20 unique test designs.

Protocol setup has finite budgets, but synchronous OS kill/wait and filesystem removal do not have a guaranteed hard wall-clock deadline. OS termination/reap refusal remains possible; the implementation warns and retains an ephemeral profile when exit cannot be confirmed. Direct-child evidence does not establish descendant/process-tree termination. Caller-retained handles can retain diagnostic history and finished task metadata, and dead weak observer keys are pruned on a later lookup rather than synchronously on every drop.

## Rust driver allocation evidence

The [driver-heap archive](ci/results/reliability-95/driver-heap/README.md) preserves two valid Firefox 157 instrumented endurance runs on the same recorded `908323d` source and driver binary. Each includes 100 warmup cycles. Process-map checks demonstrate heaptrack in the Rust driver and its absence from the recorded browser root process. The interpreted traces have positive allocation counts and are preserved with full analyzer/demangled logs, raw native results, and [independent exact event recomputation](ci/results/reliability-95/driver-heap/analysis/recomputed.json).

| Rust driver quantity | 100 measured + 100 warmup | 1,000 measured + 100 warmup |
| --- | ---: | ---: |
| Allocation calls | 1,616,387 | 7,357,224 |
| Exact instantaneous live requested-heap peak | 648,786 B | 649,883 B |
| Outstanding requested heap at exit | 26,020 B | 26,020 B |

Every native checkpoint restores the initial page/context/helper-count baseline and has zero pending commands. The exit accounting is identical in both runs and is fully associated with these recorded initialization stacks:

| Exit allocation stack | Outstanding bytes |
| --- | ---: |
| Tokio `signal::registry::globals_init` / `Vec<SignalInfo>`: one 2,048 B allocation and 64 of 344 B | 24,064 |
| Loader `_dl_allocate_tls` via `pthread_create` and Rust/Tokio worker launch: four of 352 B | 1,408 |
| Rust `stack_overflow::thread_info::set_current_info` during `lang_start_internal`: 544 B and 4 B | 548 |

The analyzer's original `MEMORY LEAKS` / `total memory leaked` labels are retained. Allocation stacks and unchanged exit totals in two bounded runs are not reachability proofs or universal leak-freedom guarantees. They are Rust-driver evidence, not Firefox heap evidence. Two earlier 13-byte empty captures caused by missing `libdw.so.1` are preserved under [invalid instrumentation](ci/results/reliability-95/driver-heap/invalid-instrumentation/VALIDITY.json) and excluded from every allocation finding.

The [standalone heap plot](ci/results/reliability-95/driver-heap/analysis/live-heap.svg) uses recorded massif samples against profiler time, including startup, warmup, instrumentation, and shutdown. Sampled maxima differ from the instantaneous event peaks, which are shown separately. It does not interpolate workload cycles or compare latency.

## Independent Firefox browser memory

The [browser-memory archive](ci/results/reliability-95/browser-memory/README.md) is one Firefox 157.0.1 Linux localhost churn experiment using an independent Python/WebSocket driver, without Rustwright helpers or preloads. All **100 warmup + 3,000 measured cycles** succeeded. All 18 recorded samples restore the original page and user-context lists and have zero pending commands; the final result also has zero pending commands.

Six normal `SIGRTMIN` memory reports were captured. No forced minimization or GC was requested, and no `SIGRTMIN+1` was sent. Ordinary browser reclamation remains allowed. The nominal 0/30/90-second tail remains instrumented with count queries and report collection.

| Firefox main-process quantity (MiB) | After warmup | End of measured cycles | 90-second tail |
| --- | ---: | ---: | ---: |
| Sequential `/proc` PSS | 351.68848 | 419.92871 | 388.89355 |
| Reporter resident amount | 384.91016 | 453.59766 | 422.69531 |
| Allocator heap-allocated | 287.30455 | 282.40331 | 262.84633 |
| Reported explicit heap, kind=1 | 228.48409 | 253.26090 | 238.97343 |
| Explicit js-non-window category | 108.17660 | 92.15747 | 69.82654 |
| Explicit window-objects category | 15.02559 | 51.95078 | 48.73644 |
| Explicit network category | 0.71483 | 21.68246 | 21.67537 |
| Explicit gfx category | 100.72740 | 102.33769 | 101.33504 |

All reported processes have `ghost-windows=0` in all six captures. At 90 seconds, root PSS remains **37.20508 MiB above warmup**, while allocator heap-allocated is **24.45822 MiB below warmup**. The data therefore describe different resident, allocator, and reporter-category quantities; they do not justify treating all retained PSS as leaked allocations or declaring a long-run plateau. Explicit categories include both heap and non-heap reporters and are not disjoint subdivisions of heap-allocated. Some categories remain above warmup and their retention is not fully attributed.

The original run manifest records HEAD `3e52b79` with dirty source. Archived Python hashes match later `908323d`, but the run did not occur on that later commit. The upstream memory-dumper source establishes provenance for the signal distinction, not exact installed Firefox source identity. Raw reports, binaries' identity records, summary, and [recomputed category amounts](ci/results/reliability-95/browser-memory/analysis.json) retain the actual run scope. Sequential `/proc` reads and later reports are not atomic; shared mappings affect RSS. Reporter values and zero ghost-window counts do not identify unreachable objects or prove browser leak freedom.

## Completed headed, version and site checkpoint

The [extended proof](ci/results/reliability-95/remote-final/extended-verification.json)
verifies the full five-target 79-case suite headed on Linux with Chrome
`155.0.8059.39` / Firefox `157.0.1`, and headless with Chrome `151.0.7922.138` /
Firefox `153.4.0esr`. Exact CLI identities, executable bindings and raw protocol
versions are retained; ESR's numeric protocol value is `153.4.0`. The selected
alternate Chrome differs from local `151.0.7922.173`.

Owned mapped-window proof is sampled per headed command/backend using Xvfb
virtual X11 `IsViewable`, `WM_CLASS` and a live descendant PID. Four backend
observations cover Chrome/Firefox in the headed suite and each site command.
It does not prove
a window for every case, a physical desktop, human-visible interaction or visual
correctness. Headed and alternate-version proof is Linux-only; the current
headless portable matrix covers all three operating systems.

Both current engines completed all operations and teardown on
`https://example.com/`, `https://developer.mozilla.org/en-US/docs/Web` and
`https://docs.rs/`, with browser-default TLS verification and no explicit proxy.
All six site observations contain HTTP 200 documents, zero observed HTTP
denial/error documents and six SHA-verified 1280×800 PNGs. Firefox repeated
lifecycle statuses are not extra sites. Chrome's URL-matched document candidates
can include same-URL iframes, while Firefox observations use the root context
and navigation ID. These selected passes do not reclassify the broader failed
local nine-site proxy aggregates.

### Preserved failures and sandbox correction

The [frozen local record](ci/results/reliability-95/local/README.md) retains
completed checks and distribution plus failed/interrupted headed, ESR and
container attempts. Native thread/fork errors and 27,449 adopted zombies were
observed, but no initial census establishes causal attribution for every failure.
Only six of nine headless public sites per backend completed all operations;
both aggregate processes exited 1. Successful document or teardown observations
do not convert those failures into passes.

[Run 37987315249](https://github.com/rsasaki0109/rustwright/actions/runs/37987315249)
at head `be1b065` / merge `81864` finished with six successes and one alternate
Chrome CLI sandbox-preflight failure. Its
[partial freeze](ci/results/reliability-95/remote-final-before/README.md) preserves
the original authentication-limited cutoff; the separate
[completed supplement](ci/results/reliability-95/remote-final-before-completed/README.md)
adds recovered final conclusions without rewriting it.
[Run 37990278021](https://github.com/rsasaki0109/rustwright/actions/runs/37990278021)
at direct head `66d98a9` again passed six jobs and failed the same preflight,
despite the supported internal CLI capture timeout. The
[second failed record](ci/results/reliability-95/remote-final-second-before/README.md)
preserves its complete evidence. Both attempts left headed/alternate/site
phases unrun; neither is counted as successful extended verification.

The final correction at `39e1c895` replaces CLI DOM capture with bounded
localhost CDP. Native current/alternate Chrome now prove correlated frame/loader
DOMContentLoaded and document HTTP 200 HTML, completed body, exact evaluated
URL/title/body, actual fixture request/write, Browser.close acknowledgment and
natural child exit 0. Failed cleanup prevents another launch or helper
installation; the existing approved helper-installation function is unchanged.
Retained profiles remain outside artifacts. The native
[sandbox record](ci/results/reliability-95/remote-final/sandbox-verification.json)
is separate from the [26 synthetic controls](ci/results/reliability-95/sandbox-cdp-preflight/ARCHIVE_README.md)
and [earlier CLI correction](ci/results/reliability-95/sandbox-capture-deadline/ARCHIVE_README.md).
The synthetic interruption uses catchable Python `KeyboardInterrupt`, not CI
SIGTERM/SIGKILL or host shutdown. The shared 25-second budget covers operations;
process, server and filesystem cleanup is additional. Direct-child reaping does
not establish descendant quiescence or a hard whole-invocation deadline.

| Final acceptance condition | Verified result |
| --- | --- |
| All seven required current-source jobs succeed | Complete |
| Actual checkout and all 153 relevant input blobs/modes are recorded | Complete; head `39e1` / merge `a219` |
| Workspace, MSRV, formatting, Clippy, three-OS portable and extra Linux suites pass | Complete; scope and overlap recorded above |
| Full headed 79 and alternate headless 79 suites pass without ignored/filtered cases | Complete; Linux-only scope |
| Owned mapped windows and six successful selected site observations with hashed PNGs | Complete; per-command Xvfb and HTTP-attribution limits retained |
| Eight real archives, native consumers and actual package provenance are verified | Complete |
| Original digests, raw reports, source identities and archive hashes are independently audited | Complete; historical failures remain unchanged |
| Scoped milestone assessment | 95% development estimate; no SOTA or universal leak-freedom declaration |

Unknown remote subscription IDs after lost/timed-out acknowledgements remain unreclaimable by the local lifetime fixes; retries can create another remote subscription whose ID was not observed. OS cleanup refusal and descendant-process guarantees remain outside the demonstrated direct-child scope. Unexplained browser category retention, object reachability, longer-run memory bounds, and broad workload/platform/site coverage remain open. A broader SOTA claim requires a separately specified comparative benchmark and reproducible evaluation; this checkpoint provides no such claim.
