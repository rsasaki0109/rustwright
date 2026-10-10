# Firefox monitoring before subscription acknowledgment

The existing diagnostics receiver was registered only after the remote
subscription acknowledgment. A peer can send a request and its completion
before that acknowledgment; those events were then permanently absent from
page diagnostics.

The correction registers the receiver in the bounded, detached setup worker
before awaiting acknowledgment and passes that receiver to the pump after
successful subscription. It preserves serialized readiness, cancellation-safe
ownership, reuse, failed-setup behavior and close handling.

Two identical regression sources fail against the old production implementation
and pass after correction: normal setup and cancellation of the first waiter.
A transport-delivery barrier holds acknowledgment until both early events have
arrived, avoiding a timing-dependent reproduction. The full readiness group
passes 8 cases; the full BiDi library passes 86. These totals overlap. One
native Firefox 157 test completes ten monitoring/fetch cycles and closure;
those cycles are not ten separate tests. The first native command named a
nonexistent package and ran no tests; its failure log is preserved separately.
Rust 1.85, Clippy, formatting and whitespace checks pass.

`before/` and `after/` hold exact sources, with matching added-regression hashes.
Commands, exit codes, logs and `report.json` identify each check. Three large
compiled test ELFs remain in the local execution workspace; their hashes and
sizes are preserved in `uncommitted-binary-identities.json`. The original
artifact manifest includes those uncommitted binaries and should not be used
as a completeness check of this smaller source/log archive.

Verify the files actually included here from the repository root:

```sh
sha256sum -c docs/ci/results/firefox-monitor-readiness/SHA256SUMS
```

Existing broadcast capacity/lag handling, diagnostics history, top-context
filtering and historical activity before subscription remain unchanged.
Firefox network-idle parity remains unimplemented. The earlier
[protocol investigation](../firefox-network-probe/README.md) describes its
frozen inspected sources before this receiver correction; that evidence is
preserved unchanged.
