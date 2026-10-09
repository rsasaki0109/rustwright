# Reliability 95% target: blocked evidence checkpoint

**Status: blocked on GitHub authentication. Final headed, browser-version, and public-site verification remains unproven. The provisional development estimate is 92% toward the 95% target; this document does not declare the milestone complete.** The percentage is a development milestone, not a measured probability of successful browser operations or a percentage derived from test totals.

The current source fixes owned startup cleanup, releases closed Firefox observers, and verifies whether a Chrome target actually closed during discovery initialization. Source-specific evidence supports those changes and the tested headless workflows. The remaining final checks and limits are recorded below.

## Source and completed CI

The repaired source commit is `4cdec571a26c64e123a4dde66d4d9b47e8c64f63`, following the startup/Firefox lifetime changes in `908323d38ba036960c41347ecbe2a5dfa8c24271`. [CI run 37983300059](https://github.com/rsasaki0109/rustwright/actions/runs/37983300059) completed with all six jobs successful.

CI actually checked out the PR merge commit `58ac49fda4fd9c85cf7711b9dc99aef32acd1f82`. [The source-equality record](ci/results/reliability-95/remote/tested-merge-source-equality.json) verifies identical Git blob IDs and file modes for 152 relevant source, test, helper, workflow, and package-README inputs between the head and tested merge. All six raw job logs confirm the actual checkout. This proves equality for those inputs, not identity of every file or of the two commits.

| Completed check | Recorded result |
| --- | --- |
| Workspace suites | 475 passed, 0 failed; one existing macro doctest explicitly ignored |
| Library suites within that workspace run | 211 passed, 0 failed, 0 ignored |
| Formatting and all-target Clippy | passed |
| Locked workspace on Rust 1.85.0 | passed |
| Required portable native HTTP cases, Windows | 79 passed; 39 Chrome and 40 Firefox |
| Required portable native HTTP cases, macOS | 79 passed; 39 Chrome and 40 Firefox |
| Required portable native HTTP cases, Ubuntu | 79 passed; 39 Chrome and 40 Firefox |
| Additional Linux native HTTP cases | 114 passed |
| Distribution verification | eight real crate archives; 57 third-party archive checksums verified |
| README Rust fences | nine compiled |
| Packaged native consumers | four passed: two Chrome and two Firefox |
| README native entry points | two passed |

The native HTTP reports have zero failures, ignored cases, and filtered cases. Job-recorded browsers are Chrome `155.0.8059.39` and Firefox `157.0.1`. The workspace total includes 193 HTTP cases, so it must not be added to the native-job totals as new coverage. Repeated runs of the same 79 portable cases across operating systems demonstrate the recorded platform matrix, not 237 unique test designs. The full scope breakdown, raw reports, and artifact digest checks are in the [completed CI summary](ci/results/reliability-95/remote/summary.json) and [native counts](ci/results/reliability-95/remote/native-counts.json).

The [distribution report](ci/results/reliability-95/remote/packages-artifact/report.json) describes a local registry simulation using verified real archives. Stable/MSRV consumer resolution, compilation and basic execution are separate from the four stable native-consumer cases. Archive provenance records the tested merge commit; 101 archived source files and the package fingerprint were independently matched. This is packaging and consumption evidence, not a crates.io publication.

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

## Pending final headed, version, and site verification

**Pending: this section must be replaced with final frozen proof before assessing the 95% milestone.** Earlier harness or exploratory observations do not complete these checks for the final source.

The [frozen local record](ci/results/reliability-95/local/README.md) retains completed final-source checks/distribution and the failed or interrupted headed, ESR and container attempts. Native thread/fork creation errors and a census of 27,449 adopted zombies were observed, but no initial census or causal attribution establishes that every failure was environmental. Nine headless public sites were observed per backend; only six per backend completed all operations, and both aggregate processes exited 1. HTTP document observations and successful teardown do not convert those aggregate failures into passes. These records remain separate from the new required hosted verification.

[Run 37987315249](https://github.com/rsasaki0109/rustwright/actions/runs/37987315249) contains the seven-job workflow at `be1b065aeff87e5c19e53492a19f4465ba4436f4`. The added job failed before native tests: alternate Chrome's configured sandbox preflight produced no DOM before its outer deadline. Its [partial frozen record](ci/results/reliability-95/remote-final-before/README.md) retains the failure and three successful jobs; final conclusions for the other three jobs were unavailable after HTTP401 authentication failures. This is not a seven-job success. The required scope remains sequential complete 79-case headed Chrome `155.0.8059.39` / Firefox `157.0.1` and headless Chrome `151.0.7922.138` / Firefox `153.4.0esr` suites. Hosted Chrome151 differs from the local `151.0.7922.173` build.

Local correction `3d447e72dfedadfd7c8f4e947a348091a6c8b88d` adds the supported 10-second internal DOM-capture deadline while keeping the 25-second outer deadline and strict normal-exit/title/body requirements. [The correction record](ci/results/reliability-95/sandbox-capture-deadline/ARCHIVE_README.md) preserves exact-tag upstream provenance and four synthetic-process controls, including failure on outer timeout even with full DOM and exit0. No native browser was run for these controls. The correction is not pushed; its native effectiveness remains unverified. GitHub authentication must be restored before push, full rerun and final proof retrieval.

Owned mapped-window proof is sampled per headed command/backend using Xvfb virtual X11 `IsViewable`, `WM_CLASS` and a live descendant PID. It does not prove a window for every test, a physical desktop, human-visible interaction or visual correctness. The hosted read-only sites are `https://example.com/`, `https://developer.mozilla.org/en-US/docs/Web` and `https://docs.rs/`: navigation, evaluation, idle observation, screenshot and page closure are required, with six valid PNGs retained. Passing this selected three-site scope would not reclassify the failed broader nine-site proxy observations. The [harness preparation](ci/results/reliability-95/extended-harness/ARCHIVE.md) preserves source and scoped controls, not successful native execution.

| Remaining checkpoint item | Draft status |
| --- | --- |
| Final-source native headed runs and selected headless controls | blocked; hosted attempt failed setup before both phases |
| Actual browser identities, launch modes, and retained results across the selected version matrix | incomplete; final native version matrix unproven |
| Final-source public-site observations and per-site teardown results | local failed aggregates retained; hosted selected sites unrun |
| Independent audit of output completeness, source identities, and failures/limitations | completed for frozen available records; final hosted proof unavailable |
| Completion assessment against the scoped milestone | 92% planning estimate; no 95% declaration |

Public-site reporting must keep HTTP 401/403/429 denial, browser error pages, protocol operation failures, and successful document observations distinct. A report can show completed protocol operations with an HTTP 403 observation; that does not establish successful website access or attribute the refusal to the driver. Screenshots, requested headed flags, and a running process alone do not prove visible-window interaction or broad site compatibility.

Unknown remote subscription IDs after lost/timed-out acknowledgements remain unreclaimable by the local lifetime fixes; retries can create another remote subscription whose ID was not observed. OS cleanup refusal and descendant-process guarantees remain outside the demonstrated direct-child scope. Unexplained browser category retention, object reachability, longer-run memory bounds, and broad workload/platform/site coverage remain open. A broader SOTA claim requires a separately specified comparative benchmark and reproducible evaluation; this checkpoint provides no such claim.
