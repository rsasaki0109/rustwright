# Frozen repeated-comparison evidence, 2026-10-09–10 UTC

The completed comparison records 9,600 measured observations over sixteen local
Chromium fixture cases in three Ubuntu CI job environments. Rustwright passed
4,800/4,800; Playwright Core 1.64.0 passed 4,200/4,800. The 192 warmups are
separate. These are scoped raw observations, not a general SOTA or statistical
population-reliability claim. See [the interpretation](../../COMPARISON_CI.md)
and [the protocol](../../COMPARISON_PROTOCOL.md).

## Completed source and runs

- Branch source: `fadb3463297fb611915cc8fc92a4b1a91f196f15`.
- Actual PR merge checkout: `87f6180e4922ca860e10f91701e6032e6cb85390`.
- [Required CI 38005723888](https://github.com/rsasaki0109/rustwright/actions/runs/38005723888): all seven jobs successful; workspace 480 passed, zero failed,
  one pre-existing ignored macro doctest. All five new unit regressions passed.
- [Comparison 38005723883](https://github.com/rsasaki0109/rustwright/actions/runs/38005723883): three complete measurement jobs and one successful aggregation job.
- Common pins: Chrome for Testing 155.0.8059.39, Playwright Core 1.64.0,
  Node 24.19.0, Rust 1.99.0.
- All 3,641 regular Git blobs/modes match between tested branch and merge;
  all three reports declare the matching file hashes and clean Git status.
- All three Rust release binaries have SHA-256
  `5349c63284929d06fbc7f492478019ae7e70476742b78f588baf5ee4f95a0e65`.

Reporting commits made after measurement are not the tested commit. The source
archives retain the measured source and its original README. README reporting
prose may change afterward while its nine Rust code fences remain identical.
No older 95% checkpoint evidence is changed by this supplement.

## Primary evidence and independent checks

| Evidence | Location |
| --- | --- |
| Per-host success/latency and separate sampled memory | [Generated REPORT.md](attempt-fadb/comparison/artifacts/comparison-summary/REPORT.md) |
| Aggregate, original input reports and failures | [Original aggregate.json](attempt-fadb/comparison/artifacts/comparison-summary/aggregate.json) |
| Three report/source/pin/raw-sidecar audits | [comparison-independent-verification.json](attempt-fadb/comparison/comparison-independent-verification.json) |
| Release binary identities | [comparison-binary-identity-verification.json](attempt-fadb/comparison/comparison-binary-identity-verification.json) |
| Independent aggregate recomputation | [aggregate-independent-recomputation.json](attempt-fadb/comparison/aggregate-independent-recomputation.json) |
| Root's additional raw-summary recomputation | [final-independent-aggregate.json](controls/final-independent-aggregate.json) |
| Workspace result counts | [workspace-counts.json](attempt-fadb/ci/workspace-counts.json) |
| Five new unit regressions in original log | [workspace-new-regressions.json](controls/workspace-new-regressions.json) |
| Per-OS native counts | [native-counts.json](attempt-fadb/ci/native-counts.json) |
| 25 Firefox startup cycles across five native commands | [firefox-startup-cycle-confirmations-final.json](attempt-fadb/ci/firefox-startup-cycle-confirmations-final.json) |
| Headed, alternate versions, mapped windows and public sites | [extended-verification.json](attempt-fadb/ci/extended-verification.json) |
| Six original PNG hashes/dimensions | [public-screenshot-dimensions.json](attempt-fadb/ci/public-screenshot-dimensions.json) |
| Eight actual package archives | [package-archive-verification.json](attempt-fadb/ci/package-archive-verification.json) |
| Eleven actual checkout log confirmations | [checkout-confirmations-final.json](provenance/fadb346/checkout-confirmations-final.json) |
| Ten completed-cohort official ZIP digests | [artifact-digests-fadb.json](controls/artifact-digests-fadb.json) |
| 53 Python and five Node local controls | [local-validation.json](controls/local-validation.json), [Python log](controls/python-tests.log), [Node log](controls/node-tests.log) |
| Host-3 qualitative reference-failure review | [reference-failure-review.json](controls/reference-failure-review.json) |
| Figure derivation and omissions | [figure-metadata.json](figures/figure-metadata.json), [render.py](figures/render.py) |

Original job logs and API responses are under `attempt-fadb/{ci,comparison}/`.
Each artifact's original ZIP and digest response are beside its extracted
directory. Raw setup/harness stdout, stderr, process and observer sidecars are
under each `comparison-host-N/report.json.artifacts/` directory. Cleanup journals
retain exit/reap/process-group and fixture-server checks. They are bounded
observations of owned processes, not a guarantee about escaped descendants or
uninterruptible kernel states.

The package audit verifies eight archives, 102 tracked package sources, a
107-input fingerprint and 57 third-party checksums; archive VCS identity is
the tested merge and clean. Its source fingerprint is
`6d6e4b832e37b0727c66bfb468cc22220096d117425928c28ca2ee86a0ac274a`.
No registry publication is claimed. Headed/window and public-site proofs apply
to the recorded commands and six selected site/browser operations, not every
case or arbitrary sites. HTTP lifecycle duplicates are retained.

## Tested source archives

[The provenance directory](provenance/fadb346/) contains Git-tree, full declared
file SHA-256 and selected input equality records. Source tarballs are built from
the branch commit; selected blobs and modes match the actual merge checkout.

| Archive | Scope | Files | SHA-256 |
| --- | --- | ---: | --- |
| [source.tar.gz](provenance/fadb346/source.tar.gz) | Measurement/build inputs plus protocol and controls; partial source | 108 | `c6954412e30944e0265ef171eec10dfa157aadd7fb912c015ea6512f33f2afb9` |
| [ci-selected-source.tar.gz](provenance/fadb346/ci-selected-source.tar.gz) | CI/runtime/test/package inputs | 155 | `334254da42472eaa68f4fdc49cd7018deb887b375bcb7f10e958dcb3736e3631` |
| [reproducible-source.tar.gz](provenance/fadb346/reproducible-source.tar.gz) | Union including all workspace manifests and documented build inputs | 173 | `ddbad1473340b9c01865efd4e348926e2804efecf98c472cfabb9e8a48327422` |

The union archive passed locked/offline/no-dependency workspace metadata checks
after extraction; that local metadata check is not a rebuild. Actual compilation
and native execution were the recorded CI jobs. Dependency and browser downloads
are still required to rebuild on a fresh machine.

## Earlier failures remain separate

The first started cohort tested branch `d563c82875b3ca820cfa5bd798fcd40ba878763e`
at actual PR merge `06f12ef94b5edce44df68db6c28fb40b50421cb2`.
[Required CI 38002188290](https://github.com/rsasaki0109/rustwright/actions/runs/38002188290)
passed five jobs and failed two. The extended native command refused an existing
cached output directory before execution; its absent new results are unrun scope.
Windows had 78 native passes and one Firefox first-page creation failure.
[Comparison 38002188299](https://github.com/rsasaki0109/rustwright/actions/runs/38002188299)
failed all three measurement jobs and aggregate validation: Rustwright pair one
completed, then Playwright command-line diagnostics failed before observations.

The original **2,400 Rustwright measured successes and 48 warmups** are preserved
under `attempt-d563/`; they are not pooled into the completed cohort. All eleven
earlier checkout logs, ten original artifact ZIPs/digests, partial raw sidecars
and invalid aggregate are retained with their original source archives under
`provenance/d563c82/`. The diagnosis under `firefox-create-analysis/` includes
Mozilla source excerpts, original failure and qualifications: the error supports
a foreground-create visibility-wait race, but its exact discarded window is
not proven by the original error alone.

Two still earlier queued comparisons were cancelled before measurement after
setup corrections. `superseded-unstarted/` records those API snapshots. No started
measurement was discarded, and the completed cohort had no selective case rerun.

The pinned [reference distribution](reference/distribution-verification.json)
has registry, lockfile, tarball SHA-1/SHA-256/SHA-512 and embedded version checks.
Registry metadata describes the recorded retrieval time, not future latestness.

## Interpretation and integrity

Three CI job identities do not prove statistically independent physical hosts.
Default launch/click policies differ. Ratios omit any host/case with measured
failures. Driver/descendant RSS and PSS are separate sampled quantities, each
with its own non-simultaneous maximum; this is not an exact peak or leak test.
Rust descendant RSS was larger in this cohort. Live-descendant PSS was unavailable;
the only readable zeros occurred with no descendants and do not measure browsers.
The main report describes these limits alongside the observations.

`SHA256SUMS` covers every regular file in this directory except itself. Original
remote ZIP digests establish downloaded bytes against the Actions API; local
manifest hashes establish this snapshot's bytes. Neither is independent hardware
attestation. Historical README records retain their original wording and paths;
current interpretation is in this README and the top-level comparison report.
