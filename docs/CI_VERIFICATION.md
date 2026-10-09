# Required browser CI and navigation verification

The current goal is the [95% reliability checkpoint](RELIABILITY_ROADMAP.md)
before making a broader SOTA claim. The workflow now defines native HTTP jobs on
Linux, macOS and Windows, plus verified package consumers on Linux. Configuring
these jobs does not establish that they have executed successfully.

## Existing remote evidence

GitHub Actions [run 35488630049](https://github.com/rsasaki0109/rustwright/actions/runs/35488630049)
succeeded on 2026-09-20 for commit
`91e72af388d000266c9b4e7567a3e1fe4007fc33`. Its single Linux job checked formatting,
Clippy and tests. The run predates the working-tree fixes, new MSRV job,
Firefox setup, required multi-OS browser matrix and package-consumer jobs.
It is historical CI evidence for that commit, rather than verification of these
changes. Read-only API responses are retained with the rollout results.

## Workflow behavior

`browser-compat` installs both browsers explicitly and runs `http_compat`,
`http_actionability`, `http_clipped_control` and `http_bidi_navigation` on each
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

The final Linux checks on 2026-10-09 passed with Chrome 151.0.7922.173 and Firefox
157.0.1. These are local results for the current working sources:

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
New workflow execution, other operating systems, headed sessions, representative
real sites, broader browser versions and memory attribution remain outstanding.
