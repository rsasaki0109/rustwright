# Firefox navigation parity evidence

Before adding methods, `cargo test -p rustwright-bidi --lib navigation_tests --no-run` returned 101 because `BidiPage` lacked `reload`, `go_back`, and `go_forward`; see `before-api.log` and the preserved `browser-before.rs` / `session-before.rs`. This is an API availability baseline, not a native before/after behavior comparison. Native raw commands already worked (`probe.log`), and showed that pushState traversal emits `historyUpdated`, while full navigation emits `navigationStarted` and matching-ID `load`.

After: `unit-all-frozen.log` has 84/84 BiDi units (11 new navigation tests); `native-frozen.log` has 7/7 native HTTP tests (Chrome 3, Firefox 4), no skips. The shared helpers call the portable `AnyPage` facade. The native scenarios cover hash and non-hash pushState history, full-document history with redirects, reload held by a resource gate, absent-entry and closed-page errors, plus Firefox command timeout/cancellation recovery. `clippy-frozen.log` and `msrv-frozen.log` passed.

Only the final frozen logs are success evidence. Earlier `native-first.log`, `native-second.log`, `unit.log`, `unit-final.log`, and `clippy.log` retain implementation/debugging failures. `native-third.log` was successful before the last test refactor; do not sum repeated native executions.

`source/`, `source.sha256`, `executables.sha256` and `report.json` record final provenance. The source snapshot also includes the root-owned facade methods used by the native target. Native browser tests use real loopback HTTP and disposable Firefox profiles, without skipping or file/data fixtures. This result covers Linux Chrome 151.0.7922.173 and Firefox 157.0.1; other OS and Firefox versions are not asserted.
