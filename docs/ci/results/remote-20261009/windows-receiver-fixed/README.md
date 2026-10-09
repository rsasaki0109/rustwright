# Receiver-fixed Windows native matrix

GitHub Actions run **37951867466**, job **113892422968**, completed successfully on Windows at source `78e95e62b90f4de4d386cf9a3507fb36430b70cb`. The original job JSON, full job log and downloaded native artifact are retained without rewriting CRLFs or paths. The artifact reports **53** successful tests: 24 compatibility, 12 actionability, 10 clipped-control and 7 navigation; every target exits0 with failed, ignored and filtered counts all0. Browser versions in the job log are Chrome155.0.8059.39 and Firefox157.0.1.

This is the second successful Windows run, now including the BiDi pre-ack network-event receiver correction. It is distinct from older `windows-fixed/`, which tested source23ee6c5. The Windows setup applies Chrome sandbox filesystem permissions; it does not use the managed Linux sandbox-disable wrappers. Exact source workflow and HTTP runner are retained in `source-inputs/`.

The frozen source archive is referenced at `../followup-local/source-of-tested-libraries.tar.gz`; every one of its130 regular source files was verified against the exact Windows commit and current production-equivalent commit `b8710c968bf0d4f5efbc27843f80fc876aa812d1`. `source-equality.json` retains per-file hashes and the archive hash. The current commit changes macOS workflow metadata outside the Windows tested-source identity.

This record proves the Windows portable job, not full CI success, Linux-extra execution, workspace/packaging results, macOS status, latest run37953023827, all browser versions, external sites, endurance or SOTA. Workflow steps marked skipped are OS-conditional setup/Linux-extra steps; native test ignored/filtered counts are0. This run's results are not added to the local167 total or earlier repeated Windows53 total.

Verify all files with `sha256sum -c docs/ci/results/remote-20261009/windows-receiver-fixed/SHA256SUMS` from repository root. The existing referenced source archive remains unchanged and is separately bound by its recorded hash.
