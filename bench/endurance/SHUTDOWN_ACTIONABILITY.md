# Interrupted shutdown and clickable controls — 2026-10-09

Dropping a close future previously stopped context cleanup midway through its
protocol exchange. Firefox could leave helper registrations behind even though
the browser had already removed the context. Chrome also returned from a second
close before the first close completed. Separate native regressions found Firefox
clicking disabled or covered controls, and both backends missing the visible
interior strip of oversized controls clipped by overflow ancestors.

## Shutdown ownership

Chrome browser/context closure now starts one independently owned cleanup worker.
Concurrent and later callers await its completion and receive its cached result,
including protocol failures. Context shutdown announces closure immediately,
closes every tracked page concurrently and attempts isolated-context disposal
even after another phase fails. Attachment, page closure and disposal each have
a five-second bound; this is not a single five-second context deadline. Closing
the default context preserves the connection and other contexts. Browser closure
bounds graceful shutdown to five seconds, then terminates its owned process and
closes the transport. Connecting to an external Chrome instance grants no process
ownership: closing that handle disconnects it.

Firefox page/user-context cleanup runs in detached workers with five-second
overall deadlines, including serialization behind another close of that same
resource. Appropriate missing-resource errors allow repeated closure. Cleanup
attempts every matching helper and returns the first removal error instead of
concealing it. A failed helper keeps its registration identifier, allowing an
explicit subsequent close to retry removal even after the browser removed the
context. Active workers share per-resource locks; weak registry entries are
pruned rather than retaining every historical identifier. Firefox browser
closure owns session termination and process/transport cleanup independently of
the waiting caller and bounds graceful session termination to five seconds.

These workers require a live Tokio runtime. Remote cleanup refusal, connection
loss and deadline expiry remain failures rather than proof of remote reclamation.
Chrome caches a failed shutdown result instead of implicitly restarting cleanup;
Firefox permits explicit cleanup retries. Chrome's existing individual
`Page::close` still suppresses `Target.closeTarget` protocol errors, so successful
default-context shutdown is not a guarantee that a refusing browser closed every
target. Raw standalone BiDi `session.end` is unchanged. Session invalidation does
not reset the local helper diagnostic ledger retained by page handles.

## Deterministic regression evidence

Chrome's first four context regressions failed before correction. Three further
browser-shutdown regressions failed with its old implementation; the external
process preservation check already passed. The final **13 Chrome shutdown tests
pass**, including completion sharing, failure replay, cancelled attachment waits,
bounded silent peers and process/profile/socket cleanup. Browser ownership tests
use a controllable protocol peer and a disposable fake executable that owns a
real Linux child process; these are distinct from installed-browser tests.

Firefox's strengthened eleven-test suite records **nine failures and two passes
before correction, eleven passes after**. It covers cancellation before replies
and during helper removal, concurrent/repeated closure, helper-removal failures
and retries, preserving unrelated resources, and retained transport closure after
browser shutdown. The original protocol-side ledgers and partial pre-fix source
copies are retained in the evidence directory. These are targeted before/after
records, not an exact snapshot of the entire earlier workspace.

## Native cancellation checks

The `http_shutdown` target connects through a local WebSocket proxy to an actual
installed browser. It holds one successful response after remote execution,
cancels and joins the waiting caller, starts another close and verifies it remains
pending, then releases the original response. The proxy does not remove resources
or substitute protocol acknowledgments. Other events and commands keep flowing.

| Backend | Held response | Cycles |
| --- | --- | ---: |
| Chrome | First `Target.closeTarget` during context shutdown | 10 |
| Chrome | `Target.disposeBrowserContext` | 10 |
| Firefox | `browser.removeUserContext` | 10 |
| Firefox | `script.removePreloadScript` during user-context shutdown | 10 |
| Firefox | `script.removePreloadScript` during page shutdown | 10 |

All **50 cancellations pass**. Each context starts with two usable pages and
retains both handles throughout cleanup, preventing handle Drop from masking
helper leaks. Exact sorted page/context identifiers, pending commands and the
Firefox helper ledger return to baseline within a five-second test watchdog after
release. Page-only cleanup preserves its live sibling and owner context before
owner shutdown. Every cycle preserves a separate keeper page and successfully
creates, navigates and fills a subsequent isolated page with Unicode text.

These are context/page shutdown checks on Chrome 151.0.7922.173 and Firefox
157.0.1 on headless Linux. They do not verify cancellation of owned native
browser shutdown, a complete attached-session/intercept census or browser memory.
Browser-shutdown cancellation is covered by the separate controlled-peer tests.

## Click actionability and clipping

Firefox locator clicks now share the enabled/stable geometry checks used by
Chrome: disabled fieldset ancestry with the first-legend exception, inherited
`aria-disabled`, two animation frames of stability, and native hit testing.
Trusted pointer movement precedes another enabled/geometry/hit check; button
dispatch does not send a redundant second pointer movement. Waiting retries
occur only before dispatch. An ambiguous input error is returned without retry,
because a click may already have reached the browser. `click_with_timeout` bounds
the entire local protocol evaluation and input wait with one deadline.

The native actionability target passes **12 cases**, six paired scenarios:
fieldset and ARIA ancestry, a temporary overlay, movement on hover, permanently
disabled timeout diagnostics and an unresolved preparation promise. Four original
Firefox regressions failed while their Chrome controls passed. The initial
exploration also included overflow clipping; its failed Chrome observation is
preserved rather than counted as a passing actionability case.

Shared geometry additionally reports a conservative visible rectangle from the
viewport and ordinary two-axis overflow ancestor outer bounds. Chrome main-page
clicks clip painted quad polygons to that rectangle; Firefox clips its own-context
client fragments before candidate sampling. Composed shadow ancestry, display
contents and out-of-flow escape are considered. Native hit testing remains
authoritative, and geometry comparison includes changing clip bounds before input.

The clipping target records **six failures before correction**, then **ten
passes**: 60-pixel and 3-pixel strips, nested clipping, plus paired guards for
fixed descendants escaping overflow and document scrolling. Each expects exactly
one trusted click and no click on the intercepting container. One new geometry
regression also passes. The baseline six-case fixture was not snapshotted before
adding four guards; the retained pre-fix source copies are partial.

This does not establish arbitrary clip-path or transformed-ancestor coverage.
Traversal deliberately stops after absolute/fixed elements to avoid excluding
escaped descendants. Chrome frame projection is unchanged. Firefox tests its own
browsing-context coordinates; transformed parent-frame projection and ancestor
iframe overlays remain gaps. Identical-geometry element replacement between
BiDi evaluations, and hover/fill/keyboard actionability beyond these click checks,
also remain outside this change. Timing out releases the local waiter rather than
cancelling a browser-side unresolved JavaScript promise.

## Verification

```sh
cargo test --locked --workspace --lib -- --test-threads=1
cargo test --locked --no-fail-fast -p rustwright-integration-tests \
  --test http_actionability --test http_clipped_control --test http_shutdown \
  --test http_creation --test http_disconnect --test http_compat \
  --test http_contexts --test http_frames --test http_navigation \
  --test http_network_idle -- --nocapture --test-threads=1
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo +1.85.0 check --locked --workspace --all-targets
cargo fmt --all --check
```

The current workspace library suite passes **121/121** and the ten native HTTP
targets pass **153/153**, with no failed, ignored or filtered tests. The native
run includes 50 shutdown cancellations, 100 creation cancellations and 50
process-loss/restart cycles. Locked Rust 1.85.0 all-target compilation, stable
Clippy with warnings denied, formatting and whitespace checks pass.

The combined result is recorded in
[validation.json](results/shutdown-actionability/validation.json), alongside
library, Clippy, minimum-Rust and formatting results. The archive retains final
124 source/configuration files, source and binary hashes, before/after logs and
per-cycle records. The source hashes match the final tested workspace. Git HEAD
alone does not identify these uncommitted changes. Historical
memory and comparative performance measurements use their own older snapshots;
they were not repeated for these changes. File/data-URL suites, Windows/macOS,
headed browsers, real-site compatibility and remote CI were not executed here.
