# First cross-platform CI execution

This record preserves the first run containing the accumulated reliability
changes, rather than the historical main-branch CI result.

`initial/` contains GitHub API snapshots, complete Windows/macOS job logs,
downloaded native test artifacts, `summary.json`, and a relevant-source archive
from exact commit `a8098f2707b70dec44b133d4eb54a99b7a12650f`.
The source archive contains the workflow, manifests/lockfile, crates, test
sources and portable-suite script; it excludes unrelated documentation and
runtime dependencies. Its paths are relative to the repository root.

Run: <https://github.com/rsasaki0109/rustwright/actions/runs/37944830831>.
PR: <https://github.com/rsasaki0109/rustwright/pull/1>.

Windows completed `http_compat`: 13 passed, 11 failed, zero ignored/filtered.
All twelve Firefox cases passed; eleven Chrome cases failed on the initial
HTTP navigation with `net::ERR_ABORTED`. Later targets were not executed.
macOS compiled its first target but had no completed tests before cancellation;
the last version line does not identify the stalled operation. Four Linux jobs
never received runners before cancellation. A newer diagnostic commit canceled
this run, so its overall conclusion is canceled even though Windows failed.

These files establish observed failures and incomplete execution. They do not
establish successful three-OS validation, a corrected defect, or SOTA behavior.
Canceled artifacts can retain `status: running`; their logs and job conclusions
must be read together. No such artifact is counted as passing.

Verify the frozen initial files from the repository root:

```sh
sha256sum -c docs/ci/results/remote-20261009/initial/SHA256SUMS
```

Do not overwrite an attempt's files when rerunning. Store each later commit/run
separately and compare actual platform/browser outcomes.

`diagnostic-windows/` preserves the completed Windows job from run
37946708417 at commit `8e94519a62c8fa58ccc334c0cc618f5030e59a4f`.
Its 15 passes and nine failures differ from the initial run's counts. All nine
original failures have matching sandbox executable-access errors and network
service restarts; later raw diagnostic navigation loads the HTTP fixture in
each case. This directory is a Windows-only record and makes no claim about
that run's other jobs.

```sh
sha256sum -c docs/ci/results/remote-20261009/diagnostic-windows/SHA256SUMS
```

`linux-chrome155/` records local Chrome for Testing 155.0.8059.39 controls.
The unchanged compatibility/actionability/clipping targets execute 23 distinct
Chrome cases successfully; the single navigation case is a repeat, not an
additional distinct pass. First diagnostic instrumentation passes twelve Chrome
compatibility cases, and second stage instrumentation passes all 24 shared
Chrome/Firefox compatibility cases. These repeat runs must not be added together
as distinct coverage.

Exact diagnostic source copies, browser download/executable identities, native
logs and compilation checks are preserved. The scratch consumer source reflects
the second-stage helper exercise, not a frozen source snapshot of every earlier
scratch invocation. The second-stage report predates the remote sandbox trace;
its `remote_failure_cause_identified: false` describes that earlier point in time.
Linux controls establish neither Windows/macOS success nor a corrected CI defect.

```sh
sha256sum -c docs/ci/results/remote-20261009/linux-chrome155/SHA256SUMS
```

`windows-fixed/` preserves all 53 native portable passes at commit `23ee6c5`,
the Chrome helper's success status and capability ACLs. `macos-bundle-before/`
preserves the same commit's twelve Chrome creation failures and twelve Firefox
passes; it shares the exact source archive in `windows-fixed/`.

The `windows-sandbox-review/`, `macos-bundle-review/` and
`macos-workflow-review/` directories preserve authoritative installation source,
archive metadata and reviewed workflow snapshots. Sparse ZIP seek aids and
runtime caches are excluded. Their relative checksum manifests can be checked
from inside each directory. The Windows review snapshot precedes the added
two-minute step limit; the verified installer invocation is unchanged.

```sh
sha256sum -c docs/ci/results/remote-20261009/windows-fixed/SHA256SUMS
sha256sum -c docs/ci/results/remote-20261009/macos-bundle-before/SHA256SUMS
```

`followup-local/` preserves 171 current-source Linux library tests, all-target
Clippy/MSRV checks and repeated real eight-archive distribution verification.
Its native consumer and README results are local, rather than remote Linux CI.
