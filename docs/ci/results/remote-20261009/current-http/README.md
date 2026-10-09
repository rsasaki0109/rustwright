# Current-source Linux HTTP checks

The preserved original reports and native logs show **167** successful tests across 13 targets: **53** portable (24 compatibility, 12 actionability, 10 clipped-control, 7 navigation) and **114** Linux-extra. Each target exits0 with failed, ignored and filtered counts all0. The two orchestration logs are retained unchanged; these are local cloud runs, not remote Linux CI.

Execution source is commit `b8710c968bf0d4f5efbc27843f80fc876aa812d1`. The frozen `../followup-local/source-of-tested-libraries.tar.gz` contains130 regular source files; every archived byte was verified against checkout and both that commit and receiver correction commit `78e95e62b90f4de4d386cf9a3507fb36430b70cb`. `source-equality.json` retains all130 file hashes and the archive hash. Production, test, runner and Cargo inputs are equivalent between the commits; workflow metadata differs. The exact current workflow and HTTP runner are separately retained in `source-inputs/`.

Browsers observed by the tests are Chrome151.0.7922.173 and Firefox157.0.1. `runtime/identities.json` records post-run browser/toolchain identity and wrapper settings. This managed cloud container requires launcher-only sandbox adjustments; its native binaries were not modified. These sandbox settings do not describe Windows or macOS CI. Original report/log paths identify the historical workspace; their bytes are preserved rather than rewritten.

This evidence covers local HTTP scenarios. It does not repeat workspace unit tests, packaging, README runtime, real external sites, memory endurance, remote OS jobs or comparative SOTA measurements, and no totals from those records are added here. Post-run executable hashes are identity records, not pre-run binary provenance assertions.

Verify all files with `sha256sum -c docs/ci/results/remote-20261009/current-http/SHA256SUMS` from repository root. The referenced source archive hash is verified separately in `source-equality.json` and remains in its existing immutable directory.
