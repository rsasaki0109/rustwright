# Current-source Linux and distribution checks

Workspace library tests pass **171**, including all **86** BiDi cases and the
two new early-network-event regressions. Clippy with warnings denied, Rust
1.85 all-target checks, formatting and whitespace pass. Root logs are retained
here without adding overlapping agent test totals.

The packaging harness verifies eight actual archives against checkout source
bytes, 57 locked third-party checksums, exact-version registry consumers on
stable and Rust 1.85, and all nine README Rust fences. The four native consumer
cases pass, including real macro callbacks; both verbatim README entry points
also pass. Four consumer PNGs and the README Firefox PNG are retained.
This is a local registry simulation, not publication or remote Linux CI.

`release-report.json` records package input hash
`43b950ee8ebb245e2daa708376597cd8d0ce4c102b5484772518406c404565bf`.
The checks preceded the Git commit of the receiver correction; archive VCS
metadata can identify the earlier parent plus a dirty tree. The library source
bytes are identical to commit `78e95e62b90f4de4d386cf9a3507fb36430b70cb`, whose
relevant source archive is included. The root README and lockfile are unchanged.

The first README runtime invocation omitted browser bindings and correctly
failed before execution; its log is retained. The activated rerun passed both
entry points. It is not an additional browser test or an intentional regression.

Consumer registry paths in generated configuration/metadata identify the
original execution workspace. Recreate them with the harness rather than using
these historical absolute paths in another checkout.

```sh
sha256sum -c docs/ci/results/remote-20261009/followup-local/SHA256SUMS
```
