# Second diagnostic instrumentation stage

The initial macOS artifact ends after Chrome version output. This alone does not identify whether creation, navigation, JavaScript evaluation, cancellation recovery or shutdown stalled. No production fix is claimed.

Only two test sources change in this stage: common open now logs creation/navigation start and completion; initial Chrome page creation fails explicitly after 30 seconds, with a browser-level probe. The first cancellation scenario logs each evaluation, wait abort, helper restoration, subsequent visible wait, title read, and close, and fails explicitly if its post-open work exceeds 30 seconds. Original errors and assertions remain fatal.

Both failure probes include stderr before asynchronous discovery/connection/command waits. All asynchronous diagnostic work is bounded by five seconds. Browser-only probes log pending-command count, Target.getTargets, Target.getBrowserContexts, raw attach and renderer command names before each await; page probes retain original network snapshots plus full navigation responses and relevant session-scoped events. No retry turns an original failure into a pass.

Local full Chrome155/Firefox157 compatibility: 24/24, no filtering or skips. Clippy with warnings denied, Rust1.85 compile, scoped fmt, and whitespace checks pass. Browser-only diagnostics were executed successfully against native Chrome; an isolated local HTTP attachment yields the intended original ERR_ABORTED, with isDownload=true recorded by the bounded page probe. This validates instrumentation rather than explaining the remote Windows issue.

All original diagnostic-* logs/reports/source copies are preserved. This pass exclusively adds stage-* evidence. Exact source snapshots and SHA256 digests are stage-source/ and stage-source.sha256; stage-report.json records validation hashes. Production source is untouched.
