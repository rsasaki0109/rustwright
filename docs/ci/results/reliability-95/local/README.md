# Local final checks, completed distribution, and retained failed observations

This archive preserves local results and failures around source commit `4cdec571a26c64e123a4dde66d4d9b47e8c64f63`. It does **not** establish completion of a headed or older-version native matrix. All original outcome files remain byte-identical, including reports marked failed/running and interrupted observations. The later mandatory hosted extended-verification job is separate evidence.

## Completed local checks and distribution

`final-checks/report.json` and its seven logs record 211 passing library cases, zero failures/ignored/filtered cases, formatting, all-target Clippy, locked Rust1.85 checking, the final compat_report build, 13 Python tests, and actionlint. All seven recorded log hashes were checked during archiving.

`distribution/` retains the **eight actual project .crate archives listed in the original report**, with their original hashes independently checked. It also retains every top-level log/JSON record, the generated consumer Cargo manifest/lock/config and Rust sources, and five produced PNGs. The completed result records:

| Distribution scope | Recorded result |
| --- | --- |
| Project crate archives | eight passed |
| Third-party archive checksums | 57 verified by the original verification pipeline |
| Stable and Rust1.85 consumers | compilation/resolution and basic consumer execution passed |
| README Rust fences | nine compiled |
| Native consumer cases | four passed: two Chrome and two Firefox |
| README native entry points | two passed |
| Preserved screenshots | five valid PNG files |

The native-consumer and README runtime logs identify their actual executable paths. The four native cases must not be relabeled as four MSRV-native runs; MSRV consumer compilation/basic execution is a separate scope. This is a verified local-registry simulation using real archives, not registry publication. Dependency package archives, registry caches, consumer build targets, and profiles are excluded. `registry-checksums.json` retains the original third-party checksum records without copying those dependency archives.

## Failed and interrupted native matrices

| Attempt | Recorded outcome |
| --- | --- |
| Headed Chrome155/Firefox157 initial | compatibility suite completed: 23 passed, one failed; remaining matrix targets unrun |
| Headed Chrome155/Firefox157 adjusted-display attempt (`fixed`) | compatibility suite completed: 14 passed, ten failed; remaining matrix targets unrun |
| Headed Chrome155/Firefox157 serial attempt | interrupted during Chrome isolated storage; no completed suite summary |
| Headless installed Chrome151/Firefox153 ESR | compatibility24 + actionability12 passed; clipped-control then completed six passed / four failed; later targets unrun |
| Final inner-container headed attempt | seven individual Chrome cases logged `ok`, then isolated-popup observation stalled; no completed suite or final run record |

Names such as `fixed` are original output-directory labels, not proof of a successful matrix. `native/` contains raw logs and reports, the nine copied Firefox failure-stderr logs with their original mapping, Xvfb records, serial resource-failure stderr, and explicit serial interruption. The serial Cargo report records exit101, its outer observer exit1, and its underlying test process ended with SIGTERM; no final test-suite pass total is invented. Local ESR/native completeness remains unproven despite the 36 earlier completed passes and six later clipped-control passes.

`run-records/` preserves complete outer JSON records when actually written and X11 window observation JSONL where captured. Window observations do not convert failed tests into passes or prove physical-display visibility. The interrupted container has window JSONL and a process log but **no `run.json`**. That missing result stays missing. `native/xvfb-cleanup.json` records the owned Xvfb PID absent after root stopped it.

## Docker attempts and resource pressure

`container-probe.log` preserves the initial whole-`/usr` mount failure: Docker could not find `/sbin/docker-init`. `container-probe-granular.log` preserves a subsequent successful granular-mount Firefox frame-reload probe: **one selected case passed and 23 were filtered**. That probe is not a portable matrix pass.

`container/` retains the later stdout, metadata/resource snapshot, exact runner/run.sh sources, interruption record, and one copied browser-stderr log with its hash mapping. Its normal final native phase never completed, and its later planned version/site phases did not run. `metadata.json` and `resource-config.json` are historical running-state snapshots taken before interruption, not claims about current container state. Root's command transcript records its eventual stop.

`container/commands.json` was transcribed by root **after execution from actual tool-call argument arrays and shell payloads**. It is command provenance, not an automatic pre-execution snapshot. Similarly, `run-records/runner-sources/runner-initial.py` is a reconstructed earlier helper version obtained by reversing only the known `--mode` option delta. Its byte-identical archival copy does not establish that the helper was automatically snapshotted before execution. Later helper versions and their retained sources remain separately identifiable.

The raw records include `pthread_create`/thread-creation failures and renderer-resource errors. `resource-pressure.json` observes **27,449 zombies, all adopted by PID1**, with no initial census for this continuation. It cannot attribute that entire accumulated count to the new code or these final observations. The inner-container metadata has no configured inner PidsLimit or memory cap. Ancestor quota/resource pressure is a hypothesis supported by observed creation failures, not an independently measured causal attribution. Neither every test failure nor the zombie census is relabeled as proven environmental-only or driver-caused behavior.

## Headless public-site observations

Each final-source headless observer visited the same nine specified sites through the explicit environment proxy. Both aggregate processes exited **1** and reports are `operation_failed`, so neither is a full pass.

| Backend | Site observations | All operations succeeded | Document-observation classifications |
| --- | ---: | ---: | --- |
| Chrome155 | nine | six | seven HTTP documents, one HTTP access denial, one browser error page |
| Firefox157 | nine | six | eight HTTP documents, one unknown document status |

Chrome records an HTTP403 observation for X, a tunnel-connection failure for YouTube, and a five-second network-idle timeout for TikTok. Firefox records network-idle timeouts for Mercari and TikTok and `NS_ERROR_PROXY_FORBIDDEN` during YouTube navigation. These errors and naturally observed statuses are retained without retries or failure suppression. A document HTTP200 observation can coexist with an operation timeout; protocol completion, document status, and website compatibility are distinct measures.

Every recorded site `page_close` operation passed, and global browser close/owned-profile cleanup records remain in each report. This is recorded teardown success, not descendant-process cleanup or a universal site-compatibility guarantee. Public-site copying includes only each original report and nine top-level PNGs; profiles and private certificate stores are excluded.

`sites/environment/` and the four positive/negative trust-probe reports, PNGs, and logs preserve environment setup observations. The default Firefox proxy probe and task-trusted Chrome probe failed; the task-trusted Firefox and normal-NSS Chrome probes completed. These probes happened **before the final compat_report binary build** and must not be claimed as final-source complete public-site proof. Public CA and certutil hashes and restricted wrapper source are retained; no private NSS database/configuration or certificate-verification bypass is bundled.

## Source identity and archive verification

All six completed outer run records record HEAD4c and unchanged tracked input hashes. `observed-source/` contains **149 inputs read from that exact Git commit and matched against every retained source-hash map**; `source-identity.json` records the check. This is source identity evidence, separate from whether a run finished or passed. It excludes later helper/workflow changes for hosted extended verification. Original binary identities remain in `run-records/executables.json`; the binaries themselves are excluded.

`analysis.json` derives and separates completed checks, packaging, failed/interrupted native scopes, public-site outcomes, resource census, and provenance limits. `copied-originals.json` records every original local path, archive path, SHA256, and byte size. `ROOTED_SHA256SUMS` covers every actual archive file except itself, using repository-root-relative paths. `exclusions.json` records exclusions and the deliberately missing container result. No older frozen archive, source file, or root checkpoint document was modified.
