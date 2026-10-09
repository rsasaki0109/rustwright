# First completed three-OS CI pass

GitHub Actions run **37972198887** at exact source `4b7226472f3ac4be27e8fcadb536f7019de17894` is **completed / success**, with **all six jobs completed successfully**. Original full-run, jobs and artifacts API responses and five job logs are retained here. This is the first complete passing CI stage in this work, before the later Firefox network-idle feature and its new tests; it does not certify those later uncommitted changes.

| Native HTTP job | Verified tests |
| --- | --- |
| Windows / job113961373549 | Portable53: Chrome26, Firefox27. |
| macOS ARM64 / job113961373583 | Portable53: Chrome26, Firefox27. |
| Ubuntu / job113961373610 | Portable53 plus Linux-extra114, total167 in this native job. |

Every native target exits0 with failed, ignored and filtered counts all0. The four shared portable suites contribute24 compatibility,12 actionability,10 clipped-control and7 navigation cases on each OS. Browser versions recorded by each job are Chrome155.0.8059.39 and Firefox157.0.1. Actual platform identities are retained in the original reports. Windows sandbox permissions, intact macOS application-bundle setup metadata and Ubuntu sandbox preflight artifacts belong to their corresponding jobs.

Workspace job113961373430 passes formatting, Clippy and the full test command with explicit browser bindings and `RUSTWRIGHT_REQUIRE_BROWSERS=1`. Its separate scopes are: **171 library cases**, **37 legacy integration cases**, **167 HTTP integration cases**, **20 runner integration cases** and **10 passed doctests**, totaling405 passed cases in that workspace invocation. There are0 failures and0 filtered cases. **One existing rustwright-test-macros example explicitly marked `ignore` is intentionally ignored in doctests**; native/unit/integration suites have0 ignores. Thus the entire workspace is not described as zero-ignore. `workspace-counts.json` records all38 result summaries and their source log lines; FIFO matching handles interleaved stdout/stderr Running/result records. The37 legacy cases include file/data fixture scenarios that were blocked in the local managed-cloud browser policy; they ran successfully here, rather than being treated as local positive evidence.

The workspace job contains the same167 HTTP scenarios as the separate Ubuntu native job. These counts are not added together as unique test coverage. Repeated portable cases across OSs and earlier successful runs are also kept as platform execution evidence, not new unique cases.

MSRV job113961373225 passes `cargo check --locked --workspace --all-targets` on Rust1.85.0. Successful package job113961373460 and its original8 crate archives,57 locked third-party checksum verifications, stable/MSRV consumers,9 README Rust fences,4 native consumer cases and2 README entry points are already frozen in **`../linux-sandbox-fixed/`** and are referenced without duplicating the archives.

Both retained Ubuntu native and workspace sandbox preflights initially report `No usable sandbox!` and SIGABRT. Selecting an unchanged matching root-owned4755 helper produces a successful renderer probe with sandbox enabled. Their raw before/after outputs and setup metadata are preserved. This is narrower evidence than attributing every earlier SIGABRT to the sandbox; earlier runs without stderr remain separately documented.

All four downloaded artifact ZIPs are retained, and their SHA256 values exactly match the GitHub artifacts API digests. Extracted artifact bytes are also included. The source reference `../linux-sandbox-fixed/source.tar.gz` has134 regular files, verified byte-for-byte against the exact Git commit and its embedded Git comment. Source archive SHA256 is `673b80c43d49a7f305041ad3541e79c2cf77ab013472e15099b74b17cfafdabc`. `source-equality.json` retains the134 file hashes. Current uncommitted Firefox/network-idle work is excluded from this source identity. Existing frozen directories and earlier queued snapshots were left unchanged.

Verify files from repository root with `sha256sum -c docs/ci/results/remote-20261009/three-os-passed/SHA256SUMS`. This certifies the specified complete CI run and scenarios, not browser-wide correctness, all versions, external-site behavior, memory-leak freedom, registry publication or comparative SOTA superiority.
