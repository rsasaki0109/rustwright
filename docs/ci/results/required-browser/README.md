Frozen required-browser CI rollout evidence, 2026-10-09.

See ../../../CI_VERIFICATION.md for results and limits. Existing remote run metadata describes committed HEAD, not the uncommitted changes. Local portable and additional HTTP results are separate from configured macOS/Windows jobs. navigation/ and rollout/strict-browser-policy/ preserve before/after controls and debug failures; only final/frozen logs establish their success. release/ preserves the corrected current archives and consumer verification after a real Cargo staging-cache reuse failure.

source.tar.gz and source.sha256 freeze the workspace/release/CI inputs, excluding historical result directories. Executable hashes are retained without binaries; agent manifests bind specific executed binaries and stages. Raw paths are original locations. Rerun helpers from the source snapshot with installed toolchains/browsers to generate a usable local registry configuration. SHA256SUMS covers retained files.

executables.sha256 lists available build artifacts, including older or unexecuted binaries. executed-from-final-logs.sha256 identifies only binaries launched in the final parent library/HTTP/consumer/README logs. Strict-policy and navigation stage manifests independently bind their own before/after runs.
