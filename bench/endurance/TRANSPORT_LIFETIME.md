# Socket-task lifetime and process loss — 2026-10-09

CDP and BiDi both retained their connection through background tasks. The writer
held a strong connection reference while awaiting its outbound receiver; that
same connection owned the sender. After peer loss, the reader failed pending
commands but the idle writer remained alive. Dropping the last public handle
also left tasks, event senders and the socket reachable. A writer failure did
not interrupt an idle reader, and a stalled write or close flush could delay
shutdown indefinitely.

## Correction and regressions

Both tasks now hold weak references, upgrading only for synchronous dispatch
or closure bookkeeping. A shared watch signal interrupts reader, outbound and
in-flight write waits. Closing is idempotent, fails pending commands, and attempts
to flush a close frame for at most 250ms before releasing the socket. Dropping
the last owner signals the same shutdown. CDP emits one disconnection marker.
The synchronous `close()` signals asynchronous cleanup; a stalled Tokio runtime
does not provide a wall-clock release guarantee. Already-sent remote commands
are not undone.

Five regressions per backend failed before the correction, then passed:

- Abrupt peer loss with 32 pending commands; typed closure errors, empty pending
  table and connection allocation release after the last owner drops.
- A retained clone remains usable; dropping the final clone releases the
  connection and disconnects an idle peer.
- Writer failure while the reader remains idle releases the entire socket.
- Explicit close interrupts a blocked write and permanently stalled close flush.
- Reader failure interrupts the same blocked writer and close flush.

The fault socket's Drop marker fires only after both split halves are released.
Each resource-release guard is one second. Another CDP test checks repeated close
emits exactly one marker and releases the event sender: **11 new tests pass**.

## Native process-loss verification

```sh
cargo test --locked -p rustwright-integration-tests --test http_disconnect \
  -- --nocapture --test-threads=1
```

Both native tests pass: **25 Chrome + 25 Firefox cycles**, with no startup skips.
Each iteration launches a fresh owned process, connects through the public API,
navigates to local HTTP, checks the title and fills/verifies Unicode input.
It then starts an unresolved evaluation and a 20-second absent-locator wait.
A helper wrapper preserves the actual wait and records that it entered the
document; the evaluation also sets an entry marker. The test observes both
markers and at least two pending commands before killing the process.

There is no graceful `Browser.close`, `session.end` or client `connection.close`
before the checks. After owned-root termination, both operations must fail with
closure errors within two seconds, the pending count must be zero, and a fresh
command must report closure. After dropping all handles, the event receiver must
reach channel closure within another two-second guard. Chrome additionally
requires one disconnection marker. Its receiver belongs to an independent CDP
observer because the high-level browser connection is private; its pending
operations use the actual Browser connection. Firefox's receiver uses the same
connection as the page/session. Firefox uses a disposable profile per cycle.

| Backend | Browser version | Successful cycles | Pending after loss | Owned root exit checks | Event-channel closure checks |
| --- | --- | ---: | ---: | ---: | ---: |
| Chrome/CDP | 151.0.7922.173 | 25/25 | 0 each | 25/25 | 25/25 |
| Firefox/BiDi | 157.0.1 | 25/25 | 0 each | 25/25 | 25/25 |

Observed time from starting process termination through pending/fresh-command
checks was 4.83–26.34ms for Chrome and 12.68–50.33ms for Firefox. Maximum observed
time through handle Drop and event-channel closure was 26.48ms and 50.47ms.
These are observations from one sequential Linux debug test run, not latency
benchmarks, scheduling guarantees or a comparison with another driver.

This verifies manual fresh-process restart, not automatic reconnection or
recovery of old pages. Every owned root process exited; every former descendant
PID was not tracked through termination. Event-channel closure proves sender
ownership is released, while the controlled socket tests verify both task halves.
No browser heap, RSS/PSS, attached-session or intercept count was measured here.
Earlier endurance and 100-sample comparison snapshots precede this correction
and are not measurements of its performance. Remaining browser-memory growth,
creation/shutdown cancellation, real sites, Windows/macOS and remote CI remain
separate gaps.

## Preserved evidence

[Results directory](results/transport-lifetime/summary.json) includes native
per-cycle JSONL, full native and workspace-library logs, pre-fix failure logs,
both pre-fix connection files, a code/config source archive and source/binary
SHA-256 identifiers. Workspace library tests passed **79/79**.

The existing native HTTP suites passed **125/125** after this correction:
shared matrix 24, contexts 10, frames 67, navigation 14 and network-idle 10.
Locked all-target checks passed on Rust 1.85.0; stable Rust 1.99.0 Clippy
(`-D warnings`), formatting and whitespace checks passed. Documentation tests
passed 10, with one pre-existing ignored macro example kept separate.
The full file/data-URL integration suite and remote CI were not run.

To replay the original ten failures, extract `source.tar.gz` into a scratch
checkout. Replace each connection file with the corresponding
`rustwright-{cdp,bidi}-before.rs`, then append this module declaration to each:

```rust
#[cfg(test)]
#[path = "transport_lifetime_tests.rs"]
mod transport_lifetime_tests;
```

```sh
cargo test --locked --no-fail-fast -p rustwright-cdp -p rustwright-bidi \
  --lib transport_lifetime_tests -- --skip repeated_close_emits_one_disconnection_marker
```

The extra marker test was added after the original five-per-backend failure
recording and is excluded from that replay. Restoring the archived corrected
connection files should pass all eleven. Preserve earlier result archives;
the Git revision alone does not identify these uncommitted changes.
