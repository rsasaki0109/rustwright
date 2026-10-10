# Frozen BiDi network-idle backend stage

This archive records the backend implementation and controlled tests. Its 23
source/manifest snapshots match commit
`e3bc08e7ce0a117d173bdcde2924165806e8e854` byte-for-byte; the comparison is in
`source-vs-e3bc08e.json`. It does not contain the root agent's native HTTP,
packaging, README-runtime or remote CI results.

All 54 files listed by the original `backend-artifacts.sha256` were verified
before archival. **53 were copied unchanged.** The compiled `api-probe.rlib`
was excluded; its original SHA-256, size and absolute local path are preserved in
`excluded-binaries.json`. The original manifest is copied unchanged as a stage
record, so it intentionally names that absent compiled file. Verify the actual
archive using the rooted `SHA256SUMS` instead:

```sh
# From the repository root:
sha256sum -c docs/ci/results/firefox-network-idle/backend/SHA256SUMS
```

No compiled Rust library, ELF executable or cache is included. Before/after
external BiDi `.rlib` identities remain in the original `before-api.json` and
`after-api.json`. The exclusion record also records whether those historical
bytes still match their local paths. A historical size is left unknown when
Cargo has overwritten the path, rather than borrowing the size of a different
binary.

## Evidence and limits

- `before-api.log` records two **E0599 missing-method errors**, not a failed
  native test. The identical `api-probe.rs` compiles after implementation;
  `after-api.json` identifies the input library and probe source hash.
- `stale-commit-before.log` records one failing regression: a delayed old
  document commit removed an incoming navigation. `stale-commit-before.rs`
  preserves the tracker before that guard. The full after log contains the
  passing regression and the separate case where the new document request
  precedes its own `navigationStarted` event.
- `final-bidi-frozen.log` records **108 passing BiDi library tests**, including
  **22 new idle cases**. Two subsequent edits only formatted an existing
  `cfg(test)` protocol fixture. The snapshots include that final formatting;
  the root's separate exact-commit workspace validation supersedes this stage.
- `clippy-frozen.log`, `msrv-frozen.log`, `fmt-frozen.log` and
  `whitespace-frozen.log` record the scoped checks. Empty format/whitespace logs
  mean success; `fmt-frozen.exit` records exit zero. Intermediate failed builds
  and tests are retained under their original stage names and are not presented
  as final results.

The API observes owned HTTP(S) requests, including descendants, with a 500 ms
quiet window and a 30-second default timeout. Firefox additionally requires a
fresh minimum 500 ms window per call because request delivery can follow the
triggering script acknowledgment. The deadline is the later of that call floor
and the last observed completion plus 500 ms.

New pages finish acknowledged subscription and ownership bootstrap before
handoff. A discovered handle reuses a complete observer when available;
otherwise it returns `NetworkObservationIncomplete`. This does not reconstruct
pre-subscription activity. Request history remains separate from the live
tracker. Matching hop completion/error, subtree destruction, close/disconnect,
receiver lag, canceled/timed-out waiters, finite setup, reordered document events
and observer/pump release are covered by the captured controlled tests.

This backend stage establishes no independent Windows/macOS native result,
public-site coverage, heap-leak claim, crates.io publication or SOTA comparison.

## Recheck the backend

Use the final commit checkout with its installed stable and Rust 1.85.0
toolchains and normal dependency setup. The original commands are preserved in
`backend-commands.txt`; the cloud activation path there is environment-specific.

```sh
cargo test --offline --locked -p rustwright-bidi --lib
cargo clippy --offline --locked -p rustwright-bidi --all-targets -- -D warnings
cargo +1.85.0 check --offline --locked -p rustwright-bidi --all-targets
```

The source snapshots are provenance records, not a standalone workspace.
Historical before-method failure requires the original pre-implementation
library; executing the probe against the final commit verifies the after API.
Rerun outputs belong in a new writable target directory, never this archive.
