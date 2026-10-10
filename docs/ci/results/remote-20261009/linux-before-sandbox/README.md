# Ubuntu run before sandbox diagnostics

Run **37953023827**, exact source `b8710c968bf0d4f5efbc27843f80fc876aa812d1`, is **completed with failure**. The four Ubuntu job results and full run API conclusion are recorded here. Existing records whose API snapshots showed queued jobs remain unchanged as historical observations. Windows and macOS succeeded in this run and have separate evidence directories; their successes do not turn the overall run green.

| Ubuntu job | Verified result |
| --- | --- |
| Rust1.85 workspace / job113896420109 | Success: `cargo check --locked --workspace --all-targets` finished on rustc1.85.0. Full raw job log and API metadata retained. |
| fmt / Clippy / workspace tests / job113896419791 | Formatting and Clippy passed. Executed unit suites passed167 cases: BiDi86, browser4, CDP17, common7, core53, plus a zero-case facade suite. First integration target `advanced` then failed all9 Chrome cases at startup; later targets and the four rustwright-test unit cases were not run. This is partial execution, not full171-library or full-workspace success. |
| Portable HTTP / job113896420140 | First target `http_compat` ran24 cases: Firefox12 passed; Chrome12 failed during launch. Ignored/filtered0. The other three portable targets and all Linux-extra targets were not run. |
| Verified packages / job113896420202 | Eight archive audits and57 third-party checksum verifications passed. Exact registry consumers compiled on stable and Rust1.85, nonbrowser consumer runs passed, and all9 README Rust fences compiled. Both Chrome native consumer cases failed at launch (0passed/2failed, ignored/filtered0). Firefox native consumer and README runtime were not executed after the failure. |

The Chrome version is155.0.8059.39 and Firefox is157.0.1. Retained errors report `EarlyExit` with signal6/SIGABRT. **The original run did not retain Chrome startup stderr, so the root cause is unknown in this stage. SIGABRT alone does not establish a sandbox problem.** Later sandbox preflight changes and diagnostics belong to new sources/runs, not this record.

Original workspace, browser and package job logs, native/package artifacts (including all8 actual crate archives), final jobs API snapshot and additionally fetched job/run API payloads are preserved without rewriting. `report.json` artifacts faithfully indicate failure; audit/compile successes are explicitly separated from native execution. Repeated cases across jobs are not added into one completion total.

Source is bound to GitHub job `head_sha`. The unchanged source archive reference `../macos-bundle-fixed/source.tar.gz` contains125 files; every byte was verified against exact Git commit `b8710c9`. Extra release harness, README, package README and root-license inputs are saved from that same Git commit in `source-inputs/`. `source-equality.json` records all125 source hashes and archive hash. Current uncommitted Firefox/network-idle changes, later workflow edits and current HEAD are outside this earlier run; no current-worktree source equality is claimed. The referenced archive covers relevant native sources and CI runner, while the exact commit remains the source identity for the full all-target workspace check.

Verify files from repository root with `sha256sum -c docs/ci/results/remote-20261009/linux-before-sandbox/SHA256SUMS`. This evidence does not establish later CI success, browser-wide stability or SOTA superiority, and previous frozen directories were not overwritten.
