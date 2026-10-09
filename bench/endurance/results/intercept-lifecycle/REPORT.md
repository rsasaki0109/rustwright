# Firefox interception lifetime — 2026-10-09

The earlier Firefox page wrapper committed routing rules before registration was
acknowledged, discarded intercept IDs before successful removal, ignored removal
errors and stopped its request pump when the final handle disappeared. A remote
page can outlive those handles: its next request could remain blocked by a
registration that no longer had a driver to answer it. Rediscovered page handles
also allocated independent registrations for the same browsing context.

## Ownership and cancellation

Discovered handles now share one routing core through a weak per-context session
index. Clones and independently discovered handles retain that core until its
last page owner disappears. Caller-managed raw `BidiSession::add_intercept`
registrations remain outside this ownership index.

Protected workers serialize registration, response-phase expansion and clearing.
Local rules are committed through an acknowledged handoff, after the remote ID
is known. Dropping an unaccepted handoff rolls back the candidate and retains
previous rules. Acceptance is the commit point: cancellation after acceptance
can leave a committed route, which a later clear, resource close or final-owner
cleanup retires. Allocation acknowledgment has a 30-second grace; this is a
per-command limit, not an overall deadline including queued operations.

Response-phase expansion installs the broader registration before retiring the
old one. A single pump handles an event listing both IDs once. Failed expansion
preserves previous rules and known registration IDs. If cancellation follows
successful retirement of the old registration, the broader replacement remains
to serve the prior rules; the canceled candidate is discarded.

Removal keeps each known ID until acknowledgment, reports typed errors and
allows explicit retry with that same ID. Only the protocol's `no such intercept`
error confirms an already absent registration. Each removal attempt has a
five-second deadline. Before activating another route, failed or timed-out removals
are reconciled using the same IDs; committed response rules determine the phases
needed by any replacement. Clearing removes the local rules first and keeps a neutral
pump while registration retirement is outstanding. Final-owner Drop schedules
retirement on the captured original Tokio runtime. Page/user-context closure
marks all relevant cores closed before waiting, attempts remote resource closure
and helper cleanup, and also retires routing with retained page handles.

The event receiver is registered before sending `addIntercept`. Early events
with unknown IDs are deferred until the returned ID can be identified. Existing
owned-ID events keep dispatching while a broader registration's reply is held,
including when a new-only response event arrived first. Dispatch matches owned intercept
IDs, including descendant-frame events, rather than requiring the event's frame
ID to equal the top-level page. A removal drain barrier processes the finite
queued prefix before pruning that ID, avoiding a historical-ID cache across
canceled phase expansions.

## Native verification

`http_intercept_lifecycle` uses installed, unmodified Firefox 157.0.1, an explicit
disposable profile and a local HTTP fixture. A transparent WebSocket proxy holds
successful acknowledgments after the actual browser has executed each command.
It forwards events and other commands throughout the hold; it neither fabricates
intercept IDs nor removes registrations itself.

Four identical tests fail with the previous implementation: a pre-ack blocked
request is stranded after registration cancellation, a child document cannot
load while routing is active, and a rediscovered handle loses the pump when the
original handle disappears. The fourth verifies prior-rule traffic during an
unacknowledged phase expansion; the old implementation cannot advance its
background request to the response phase. These tests use explicit watchdogs. An earlier
exploratory run lacked the child-navigation watchdog and was stopped by
terminating its owned Firefox root; that run is preserved separately and is not
the claimed identical baseline.

The post-fix scenarios exercise ten repetitions of each of these seven lifecycle
cases:

- Cancel an allocation while a real request is already blocked; the candidate is
  rolled back and the request completes with the live HTTP body.
- Cancel clearing after remote removal but before its reply; a concurrent clear
  waits, and subsequent live traffic completes.
- Cancel response-phase expansion, preserve the previous mock and discard the
  candidate header change; a later successful expansion changes the header.
- Hold response-phase registration acknowledgment while a new-only response
  event is blocked; verify existing mock requests still complete, then cancel
  the candidate and release the response unchanged.
- Retain a rediscovered handle while dropping the original, then drop the final
  handle; the remote page stays present and can be rediscovered for live traffic.
- Close a page while retaining page clones; later routing fails and another
  routed page stays usable.
- Close an isolated user context while retaining two routed pages; both owned
  registrations disappear and a separate routed page stays usable.

An additional case covers first-match precedence, request-header replacement,
response-header replacement, abort, and inherited same-/cross-origin child-frame
routing. Requests initiated inside each child use that child's own origin.
At successful checkpoints, pending commands are zero and sorted top-level page
IDs match their expected baseline. Actual allocated intercept IDs are recorded;
each is queried afterward and must return `no such intercept`. This verifies
these known registrations, not a complete remote intercept census.

## Protocol guards and final checks

The initial ten public-API protocol regressions report **nine failures and one
pass before correction**. Their original test source and fixture were run again
against the final production implementation in a separate workspace and compiler
target: **10/10 pass**. The source of those ten tests is byte-identical in both
stages. Sixteen further protocol guards were added during review and are recorded
as additional coverage, rather than claimed pre-fix failures. One additional
queue-capacity guard checks the 2,048-event bound.

These guards cover exact-ID retry after typed failure or a real five-second lost
removal acknowledgment, queued but unobserved handoff, cancellation after phase
retirement/reconciliation, forty canceled upgrades without historical ID growth,
closed contexts with allocation still pending, helper cleanup, foreign-only
events, origin-runtime Drop, and explicit browser closure after persistent
retirement refusal. The final BiDi library has **73 passing tests**.

The full workspace library suite reports **158/158**, and the twelve selected
native HTTP targets report **160/160**, including these four interception tests
and seventy repeated lifecycle checkpoints. Locked offline all-target checks on
Rust 1.85.0, stable Clippy with warnings denied, formatting and diff whitespace
checks pass. These totals describe these specific suites; integration targets
requiring file/data URLs and remote CI are outside the executed scope.

## Limits

An allocation whose ID never arrives, arrives after the acknowledgment grace or
is malformed cannot be reclaimed reliably. Runtime teardown and lost transport
also limit remote cleanup. Retried removal after a lost acknowledgment is safe
for a known ID, but failure does not establish that the remote resource is gone.

After final-owner Drop, persistent remote removal failures retain a neutral pump
and cleanup state and retry with capped backoff. Individual commands are bounded;
the retry lifetime is not. This can retain the session/socket after external
owners disappear while the browser continues rejecting cleanup. Explicit browser
or connection closure ends the fallback. This choice preserves live traffic when
another owner still uses the browser; it is not a universal final-handle resource
release guarantee under cleanup refusal.

Network events still use a bounded broadcast channel, and unknown-ID events are
deferred in a separate queue capped at 2,048. Overflow can lose blocked events;
missing continuation acknowledgments and batches taking longer than the
drain deadline remain limits. Foreign-only intercept events are untouched.
When an event contains both owned and caller-managed IDs, BiDi's continuation
command operates on the request, so complete independent isolation of overlapping
registrations is not established. The raw session API remains caller-managed.
If restoring a previously uncertain registration fails to allocate, the typed
error is retained and the prior rules require an explicit retry before they can
be enforced again.

This is headless Linux/local HTTP evidence. It does not establish Windows/macOS,
headed-browser, real-site or remote CI behavior, comprehensive resource census,
heap leak freedom or current performance superiority. Historical memory and
comparison artifacts retain their original source/binary identities.

Reproduce the native target with installed Firefox:

```sh
cargo test --locked -p rustwright-integration-tests \
  --test http_intercept_lifecycle -- --test-threads=1 --nocapture
```

Exact source archives, executable hashes, baseline/final logs and checkpoint
records are preserved in [the evidence directory](results/intercept-lifecycle).
