# Required browser CI and navigation verification

The current goal is the [95% reliability checkpoint](RELIABILITY_ROADMAP.md)
before making a broader SOTA claim. The required workflow has now succeeded on
Linux, macOS and Windows, including the Firefox network-idle implementation.
Earlier failures and incomplete attempts remain preserved below.

## Current verified source

[Run 37974358667](https://github.com/rsasaki0109/rustwright/actions/runs/37974358667)
tests commit `e3bc08e7ce0a117d173bdcde2924165806e8e854` and completes all six
jobs successfully. Chrome for Testing 155.0.8059.39 and Firefox 157.0.1 run on
Ubuntu, Windows and macOS arm64 with their native CI sandbox configuration.

| Check | Executed result |
| --- | --- |
| Portable native HTTP, each of three OSes | 79 passed; zero failed, ignored or filtered |
| Additional Ubuntu HTTP | 114 passed; zero failed, ignored or filtered |
| Full Ubuntu workspace | 453 passed; zero failed or filtered; one existing intentionally ignored macro doctest |
| Library subset of workspace | 193 passed, including 108 BiDi cases |
| Build checks | Rust 1.85 locked all-target compilation, stable Clippy and formatting passed |
| Distribution | Eight actual archives audited, 57 dependency checksums verified, version-only consumers and nine README fences compiled on stable/MSRV |
| Native distribution entry points | Four consumer cases and both verbatim README programs passed; five PNGs retained |

The workspace total includes its 193 HTTP cases and 193 library cases. Separate
portable jobs repeat coverage on native platforms; do not add these overlapping
counts as distinct tests. The 26 new shared network-idle cases are included in
79, and 22 new backend tests are included in 108/193. See the
[Firefox implementation and limits](FIREFOX_NETWORK_IDLE_IMPLEMENTATION.md).

[The exact-source remote record](ci/results/firefox-network-idle/remote/) retains
all job conclusions, raw logs, native reports, artifact digest checks, package
archives and the relevant Git source archive. A separate
[local record](ci/results/firefox-network-idle/local/) verifies 193 library and
193 HTTP cases, plus distribution, with Chrome 151 and Firefox 157 using
container-specific launchers. Local results do not imply local sandbox support
or a successful local run of every file/data-URL integration target.

Subsequent documentation-only commits preserve this evidence without changing
the tested production, test or workflow source. The verified CI identity is the
commit above, rather than an assertion that every later documentation HEAD ran CI.

## Existing remote evidence

GitHub Actions [run 35488630049](https://github.com/rsasaki0109/rustwright/actions/runs/35488630049)
succeeded on 2026-09-20 for commit
`91e72af388d000266c9b4e7567a3e1fe4007fc33`. Its single Linux job checked formatting,
Clippy and tests. The run predates the working-tree fixes, new MSRV job,
Firefox setup, required multi-OS browser matrix and package-consumer jobs.
It is historical CI evidence for that commit, rather than verification of these
changes. Read-only API responses are retained with the rollout results.

## First execution of the new workflow

Draft [PR #1](https://github.com/rsasaki0109/rustwright/pull/1) contains the
accumulated reliability changes. [Run 37944830831](https://github.com/rsasaki0109/rustwright/actions/runs/37944830831)
executed commit `a8098f2707b70dec44b133d4eb54a99b7a12650f` on 2026-10-09:

- Windows installed Chrome for Testing 155.0.8059.39 and Firefox 157.0.1.
  `http_compat` completed with 13 passing and 11 failing cases, no ignored or
  filtered cases. All twelve Firefox cases passed. Eleven Chrome cases failed
  at their initial HTTP navigation with `net::ERR_ABORTED`; the Chrome
  disconnect case passed. Later portable targets did not execute.
- macOS compiled the first target and printed the Chrome version in its first
  case, but did not complete that case before this run was canceled for a newer
  commit. The last output does not identify the stopped operation.
- Four Linux jobs remained queued without assigned runners and were canceled
  by the newer run. They are unexecuted, rather than passing checks.

The original logs, API snapshots, native artifacts and exact relevant source
archive are retained in [the remote record](ci/results/remote-20261009/).
Linux Chrome for Testing 155 independently passes 23 distinct portable Chrome
cases; that does not establish Windows or macOS behavior.

Failure-only diagnostics were added in commit
`8e94519a62c8fa58ccc334c0cc618f5030e59a4f` and submitted as
[run 37946708417](https://github.com/rsasaki0109/rustwright/actions/runs/37946708417).
The diagnostic keeps the original failure fatal, bounds its extra work to five
seconds, and records the original network state, raw navigation response,
HTTP fixture activity and browser stderr. Its later raw navigation is an
observation, not a retry that can turn a failed test into success.
The Windows diagnostic run completed with 15 passing and nine failing cases.
Each of its nine original navigation failures has a matching browser stderr
error: the sandbox cannot read/execute the downloaded Chrome executable
(`Access is denied`, Windows error 0x5), followed by a network-service restart.
The original request has no HTTP response; after that restart the diagnostic
navigation reaches the fixture and loads successfully with `isDownload=false`.
Those later observations do not erase the original failures.

The workflow now invokes the downloaded Chrome's own `setup.exe` with
`--configure-browser-in-directory` and checks its documented success status,
78. That helper grants Chrome installation capability SIDs read/execute access
to the downloaded tree. It preserves the sandbox and does not add navigation
retries or suppress errors.
The subsequent Windows result below verifies this correction.
Creation/cancellation stage diagnostics also add fatal 30-second test bounds so
an incomplete macOS operation produces usable evidence instead of an indefinite
first test. The staged test code passes all 24 compatibility cases locally with
Chrome 155 and Firefox 157. These are Linux results, not macOS verification.
Successful three-OS CI was not yet established at this diagnostic stage.

## Windows correction and macOS bundle diagnosis

[Run 37948648850](https://github.com/rsasaki0109/rustwright/actions/runs/37948648850)
tested commit `23ee6c5a9ac4c89e9f0eb615a001d6459ae7fd31`. Windows now passes all
53 portable native cases with Chrome 155.0.8059.39 and Firefox 157.0.1, zero
failed/ignored/filtered tests. The job records Chrome's setup success code 78
and the two inherited Chrome capability read/execute grants. This verifies the
installation correction on that Windows runner; its earlier failing attempts
remain preserved separately.

macOS completes `http_compat` with all twelve Firefox cases passing and all
twelve Chrome cases failing bounded page creation. Raw browser-level diagnostic
commands work through target discovery and attachment, then stop at `Page.enable`.
Browser stderr reports denied Mach-port rendezvous lookups and child-process
termination. The action's stable-channel cache removes the outer `.app` suffix
and changes relative framework links. Chrome's bundle/sandbox source and the
official archive layout support correcting that installation, rather than
retrying page creation or disabling the sandbox.

The current workflow re-extracts the exact action-selected macOS Chrome version
with `ditto`, preserving its complete original `.app` and links, then
selects that executable. Native macOS confirmation remains required. Original
job logs, source reviews and the successful Windows record are in
[the remote evidence](ci/results/remote-20261009/). Linux jobs remain queued
without assigned runners; they have not passed. These platform results predate
the following Firefox receiver correction.

## Firefox monitoring receiver readiness

The existing diagnostics receiver now registers before awaiting the remote
subscription acknowledgment, retaining early events in its bounded setup worker.
Two identical controlled-peer regressions fail before the production correction
and pass after it, including cancellation of the first caller. Full BiDi library
tests pass 86 cases; the full workspace library suite passes 171. A native
Firefox test completes ten monitoring/fetch cycles and closure. Workspace Clippy,
Rust 1.85 all-target checks and formatting pass. Actual eight-archive packaging,
version-only consumers on stable/MSRV, four native consumer cases and both
verbatim README entry points also pass with the corrected source.

[Receiver correction evidence](ci/results/firefox-monitor-readiness/) preserves
exact sources, commands, binary identities and logs. Its eight readiness checks
and two new regressions are included in the 86/171 totals, rather than added to
them. The earlier [network-idle investigation](FIREFOX_NETWORK_IDLE.md) describes
the frozen inspected source before this receiver correction. The investigation and receiver record predate the complete network-idle
implementation above. Historical reconstruction still has limits; descendant
tracking and event-loss errors now have separate implementation evidence.

The first bundle-restoration run, 37951867466 at commit `78e95e6`, stopped
before native macOS tests because an added strict `codesign` resource check
rejected the official ZIP's unpacked bundle. The inspected official archive
contains no `_CodeSignature`/`CodeResources` entries; this is an introduced
setup gate failure, not a failing native scenario. Signature-check exit/output
is now retained as diagnostic metadata, while required native tests still
determine compatibility. The original archive and sandbox remain unchanged.

Current Linux workspace and repeated distribution records are frozen in
[the follow-up local record](ci/results/remote-20261009/followup-local/).

The corrected receiver source also passes a fresh local HTTP run at commit
`b8710c968bf0d4f5efbc27843f80fc876aa812d1`: all 53 portable and 114 additional
Linux cases pass, with zero failed, ignored or filtered cases. This repeats the
167-case coverage on the corrected source; it is not additional distinct
coverage. The [current HTTP record](ci/results/remote-20261009/current-http/)
preserves these logs separately from earlier source snapshots.

## Corrected native macOS and Windows results

[Run 37953023827](https://github.com/rsasaki0109/rustwright/actions/runs/37953023827)
at commit `b8710c968bf0d4f5efbc27843f80fc876aa812d1` completes the required
portable matrix successfully on both macOS and Windows: **53 cases per OS**,
zero failed, ignored or filtered cases, using Chrome for Testing 155.0.8059.39
and Firefox 157.0.1. macOS runs natively on arm64. This verifies the bundle
restoration on that runner and repeats the Windows installation correction
with the current receiver source.

The macOS setup record retains the official archive hash, original bundle
identifier and relative framework link. Its strict resource-signature diagnostic
still returns exit code 1; the required native suite passes with the supplied
archive bytes and sandbox intact. Signature metadata is an observation, not a
passing signature check or a reason to count unexecuted browser cases.

Original job logs, structured native reports and exact source identities are
preserved in [the macOS record](ci/results/remote-20261009/macos-bundle-fixed/)
and [the latest Windows record](ci/results/remote-20261009/latest-windows/).
The earlier Windows run with the receiver correction is separately frozen in
[its record](ci/results/remote-20261009/windows-receiver-fixed/).

The frozen macOS/Windows snapshot predates Ubuntu execution. The completed run
subsequently fails three Ubuntu jobs with Chrome SIGABRT; its MSRV job succeeds.
[The completed pre-correction record](ci/results/remote-20261009/linux-before-sandbox/)
preserves those failures. Their original logs lack Chrome stderr, so SIGABRT
alone does not establish the cause.

## Linux sandbox diagnosis and first full workflow success

The bounded real-renderer preflight captures `No usable sandbox!` before setup.
Only recognized sandbox setup failures permit correction. The workflow installs
the action-selected Chrome's identical bundled helper as root-owned mode 4755
under `/usr/local/lib/rustwright-ci`, verifies its hash, and selects it through
`CHROME_DEVEL_SANDBOX`. A fresh-profile renderer preflight must then succeed.
It does not disable Chrome's sandbox, weaken global AppArmor/user-namespace
policy, retry failing tests or suppress unexpected launch errors.

[Run 37972198887](https://github.com/rsasaki0109/rustwright/actions/runs/37972198887)
at commit `4b7226472f3ac4be27e8fcadb536f7019de17894` is the first complete
six-job success: 53 portable cases per OS, 114 additional Ubuntu HTTP cases,
and 405 full-workspace passes with one existing ignored macro doctest. Its
[actual sandbox record](ci/results/remote-20261009/linux-sandbox-fixed/) preserves
before/after stderr, helper identities and successful package checks; the
[three-OS record](ci/results/remote-20261009/three-os-passed/) preserves all jobs.
This run predates Firefox network-idle; the current result above verifies that
feature on the same three native platforms.

## Workflow behavior

`browser-compat` installs both browsers explicitly and runs `http_compat`,
`http_actionability`, `http_clipped_control`, `http_bidi_navigation` and
`http_network_idle_parity` on each
OS. The script requires executable paths, zero retries, headless operation,
positive test counts, and zero failed, ignored or filtered cases. Every portable
target must execute Chrome and Firefox cases. Linux also runs the remaining HTTP
targets. Logs and structured success/failure records become CI artifacts even
when the check fails.

The separate workspace job preserves the full legacy integration suite and now
requires both configured browsers. Optional local discovery remains available
only without explicit browser bindings or the required-browser flag. Explicit
missing paths and Firefox launch errors fail instead of being treated as a
successful skipped case.

The package job fetches exactly locked dependency archives, then checks eight
real `.crate` files and their version-only consumer on stable and Rust 1.85.0.
It requires both native consumer cases per backend, including an executed macro
callback, and executes the verbatim README entry points. Reports are refreshed
at the start of a run so an earlier success cannot survive a failed rerun. Native
screenshots, including the README Firefox PNG, are retained.

Cargo's unpacked local staging cache can retain old unpublished source at the
same name/version and staging location. The harness now derives the packaging
directory from input hashes and audits archived source bytes against the
checkout. A real reuse failure and its corrected verification are retained in
the rollout evidence; the ordinary version number was not changed to hide it.

## Firefox navigation

`BidiPage` now provides `reload`, `go_back` and `go_forward`, plus explicit timeout
variants. `AnyPage` exposes the three default operations on both backends. A
timeout covers subscription setup, the command response and the required load.
Same-document traversal completes on its history event; full-document traversal
waits for a matching navigation ID to load. Missing history entries, closure,
timeout and lost lifecycle events produce errors.

Firefox 157 emits `historyUpdated` for History API entries that change the path
without changing the hash. The driver subscribes before traversal, preserves
events that arrive before the command response, and does not reuse an old or
foreign document's load event. Supported subscriptions are used; the installed
Firefox rejects `navigationAborted` as a subscription name. Operations on one
context should be awaited sequentially. Lifecycle-event loss fails explicitly;
it does not prove successful navigation.

## Local reproduction

The initial Linux rollout on 2026-10-09 passed with Chrome 151.0.7922.173 and
Firefox 157.0.1. Its frozen source predates the two receiver regressions. Current
library, distribution and HTTP reruns are recorded above; the following table
describes the earlier rollout rather than attributing its 169-case total to the
corrected source:

| Check | Executed result |
| --- | --- |
| Workspace library tests | 169 passed |
| Required portable HTTP suite | 53 passed, zero ignored/filtered |
| Additional Linux HTTP suite | 114 passed, zero ignored/filtered |
| Verified archive/native consumer | Eight archives audited, two native cases per browser passed |
| Verbatim README entry points | Chrome and Firefox both passed |
| Strict legacy browser policy | Two previous false successes corrected; 16 real negative cases fail as intended; four native positive cases pass |
| Optional/required policy probes | Eight isolated subprocess outcomes verified |
| CI/report failure guards | Eight expected failures verified, including empty/ignored/filtered suites and stale summaries |
| Final checks | Workspace Clippy, Rust 1.85 all-target check, formatting, actionlint 1.7.12 and whitespace passed |

The seven navigation native cases are included in the portable suite, and the
eleven new BiDi protocol cases are included in the workspace library total. Do
not add those totals again. The missing-navigation-API baseline is a compilation
failure, rather than a failing native run. The strict-policy before cases really
executed the legacy test binaries and falsely reported success after browser
launch failed; their corrected expected failures are separate from passing
browser tests.

With explicit installed browser paths and the Rust environment activated:

```sh
python3 scripts/ci/browser_checks.py --suite portable
python3 scripts/ci/browser_checks.py --suite linux-extra
python3 scripts/release_check.py --run-browser-tests
python3 scripts/release/readme_runtime.py
```

Use `--list` only to inspect target selection; listing is not a test result.
The additional suite requires Linux. Windows/macOS require their native runners;
the Linux cloud machine cannot establish those results. The full legacy suite
contains file/data fixtures restricted by this cloud's browser policy, so local
HTTP success must not be described as every integration target passing.

Local results and frozen source identifiers are retained in
[the CI rollout evidence](ci/results/required-browser/). The earlier package
evidence in [RELEASE_VERIFICATION.md](RELEASE_VERIFICATION.md) remains unchanged.
Headed sessions, representative real sites, broader browser versions and
memory attribution remain outstanding. The current three-OS workflow result
is recorded above.
