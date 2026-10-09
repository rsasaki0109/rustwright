# Cancelled creation and allocation ownership — 2026-10-09

Cancelling `new_page`/`new_context` before the browser replied discarded the
response waiter and therefore the created resource's identifier. A browser could
create a tab/context successfully while the caller never received a handle to
close it. Cancelling or failing page initialization also left the newly created
tab behind. Chrome's previous attachment guard was installed only after the
session identifier arrived, leaving the same gap during discovery attachment.

## Correction

CDP/BiDi allocation workers receive the identifier independently of the caller.
The result channel carries a resource guard, so cancellation before a reply,
after send but before result handoff, or during initialization retains a cleanup
owner. A successful return transfers ownership to the caller; existing public
handle Drop behavior is unchanged. Only the resource created by this operation
is closed. Discovery of an existing CDP target guards/detaches the newly attached
session and preserves the target.

Guards and Firefox helper Drop capture the origin Tokio runtime handle, so
dropping an initializing future on a thread without a runtime entry can still
schedule cleanup on the live origin runtime. Firefox's existing protected helper
registration keeps its acknowledged identifier for later removal.

Cancellation does not synchronously erase every pending allocation: after caller
Drop, its worker waits up to **30 seconds** for an identifier needed for cleanup.
Ordinary uncancelled allocation waiting retains its existing behavior. Guard
cleanup commands also have 30-second response limits. Once the response arrives
and cleanup is acknowledged, the tests require zero pending commands.

If the response never arrives, arrives after that grace, omits its identifier,
the connection is lost, remote cleanup fails, or the origin runtime shuts down,
remote reclamation cannot be guaranteed. In particular, bounded local waiting
does not prove the unknown remote resource was removed. Two virtual-time tests
explicitly verify that a silent peer leaves an unknown mock resource while its
cancelled local response waiter is eventually released. Already-returned handles
and caller-managed raw resources remain the caller's responsibility.

## Protocol regressions

The first **ten tests failed before correction**, then passed: five per backend,
covering cancellation before page creation replies in default and isolated
contexts, before isolated-context creation replies, during page initialization,
and an initialization error. The mock ledger retains an unrelated page/context
and requires their preservation. Isolated-page cleanup is checked while its
owner context is still open, preventing disposal from concealing an orphan tab.

Seven additional tests check result handoff cancellation (two), a permanently
silent peer's 30-second local wait bound using virtual time (two), future Drop
on a thread outside the live runtime (two), and cancelled CDP discovery attachment
without closing the existing page (one). **All 17 new tests pass**.

## Actual-browser verification

```sh
cargo test --locked -p rustwright-integration-tests --test http_creation \
  -- --nocapture --test-threads=1
```

The proxy forwards requests to an actual installed browser. For the selected
method it holds one successful response after remote execution and signals the
test. The test verifies the caller is awaiting a protocol response, cancels and
joins it, then releases the original response. Other commands/events keep flowing;
the proxy does not emulate allocation, substitute identifiers, inject browser
code for resource ownership, or close resources itself.

| Phase | Chrome response held | Firefox response held | Cycles per backend |
| --- | --- | --- | ---: |
| Isolated-context creation | `Target.createBrowserContext` | `browser.createUserContext` | 10 |
| Default-context page creation | `Target.createTarget` | `browsingContext.create` | 10 |
| Default-context page initialization | `Page.enable` | `script.addPreloadScript` | 10 |
| Isolated-context page creation | `Target.createTarget` | `browsingContext.create` | 10 |
| Isolated-context page initialization | `Page.enable` | `script.addPreloadScript` | 10 |

Each backend completes **50 cancellations**, for **100 total**, with no browser
startup skips. Each uses one browser/session and local HTTP; Firefox has a
disposable explicit profile. A keeper page remains open throughout. After every
cancellation, within a five-second watchdog:

- Exact sorted live page and context identifiers return to their baseline,
  not just their counts. Chrome uses an independent CDP observer; Firefox queries
  the actual session. Owner contexts remain open during isolated-page checks.
- Pending commands return to zero; Firefox's acknowledged helper-preload ledger
  returns to one, belonging to the retained keeper page.
- The keeper's title is readable. A new page navigates, checks its title,
  fills/verifies Unicode input, closes successfully and restores the same baseline.

Chrome 151.0.7922.173 starts with zero isolated contexts, Firefox 157.0.1 with
five user contexts; isolated-page phases add one. Each has two live baseline
pages: its startup page and the keeper. These observations do not enumerate every
attached CDP session or BiDi intercept. Protocol regression ledgers verify the
specific attachment cleanup; no memory/heap or comparative latency was measured.
Delayed valid responses are covered; lost/late-beyond-grace responses and remote
cleanup refusal remain distinct limitations.

## Validation and evidence

Workspace library tests pass **96/96**. The native targets pass **129/129**:
the existing 125 HTTP cases, two creation targets with 100 cancellation cycles,
and two actual process-loss targets with 50 kill/restart cycles. Rust 1.85.0
locked all-target checks and stable Clippy with `-D warnings`, formatting and
whitespace checks pass. File/data-URL suites, Windows/macOS and remote CI were
not executed. Historical memory/comparison source snapshots precede this change.

[Evidence](results/creation-cancellation/validation.json) preserves the before
failure log, final library/native logs, per-cycle records, the four pre-fix files,
an exact code/config archive, and source/binary SHA-256 identifiers. The Git
revision alone does not identify these uncommitted changes.

To replay the original ten failures, extract `source.tar.gz` into a scratch
directory, restore `core-browser-before.rs`, `core-context-before.rs`,
`bidi-browser-before.rs` and `bidi-session-before.rs` to their original crate paths.
Append the creation test module to the restored BiDi browser file:

```rust
#[cfg(test)]
#[path = "creation_tests.rs"]
mod creation_tests;
```

```sh
cargo test --locked --no-fail-fast -p rustwright-core -p rustwright-bidi \
  --lib creation_tests -- --test-threads=1 \
  --skip cancelled_after_reply \
  --skip cancelled_creation_with_silent \
  --skip cancelled_initialization_outside \
  --skip cancelled_discovery
```

The seven later-added tests are excluded to match the recorded first ten failures.
Restore the corrected archived files to run all seventeen successfully.
