# Shared HTTP compatibility checks

The `http_compat` target executes 12 identical scenarios on each backend, for
24 real-browser tests. Missing browsers and startup failures fail this target;
there are no skip branches. Each case launches a browser and a local HTTP fixture.
Firefox receives a separate disposable profile directory per case.

Locally verified on headless Linux with installed Chrome 151.0.7922.173 (CDP)
and unmodified Mozilla Firefox 157.0.1 (WebDriver BiDi): 24 passed, 0 failed,
0 skipped. The cloud container's launchers accommodate its OS sandbox restrictions;
the tests use HTTP and do not change managed URL policies.

| Scenario | Chrome | Firefox |
| --- | --- | --- |
| Navigation, fragment URLs, `pushState` and same-document history back; form values and trusted click | Pass | Pass |
| Thrown JavaScript errors and rejected promises; successful null/undefined and object values | Pass | Pass |
| Requested wait deadline includes an unresolved browser-side promise | Pass | Pass |
| Cross-origin iframe receives a trusted click at its own viewport coordinates | Pass | Pass |
| Frame locators remain usable after iframe reload | Pass | Pass |
| Closing a page wakes a pending locator wait | Pass | Pass |
| Delayed elements, detachment and timeout for absent elements | Pass | Pass |
| Cancelled wait allows subsequent commands and waits | Pass | Pass |
| Browser disconnection wakes a pending locator wait | Pass | Pass |
| Contexts isolate cookies/localStorage; closing one preserves the other | Pass | Pass |
| Popup discovery stays within its isolated owning context | Pass | Pass |
| Cross-origin iframe receives trusted hover, key press and typed text | Pass | Pass |

Run with both browsers installed, or select their executables through
`RUSTWRIGHT_CHROME` and `RUSTWRIGHT_FIREFOX`:

```sh
cargo test --locked -p rustwright-integration-tests --test http_compat -- --test-threads=1
```

CI is configured to install Firefox 157.0.1 and supply its executable path.
The workflow change has been checked locally; a GitHub Actions run is separate
evidence and has not been executed from this task.

The separate `http_disconnect` target force-terminates a browser root process
after an unresolved evaluation and a locator wait have entered its document.
It verifies closure errors, zero pending commands, failed subsequent commands,
and event-channel termination after all owners are dropped. Each backend
completes 25 fresh-process cycles with successful navigation and Unicode input
before the next termination. Chrome's raw event receiver uses an independent
CDP observer; Firefox's uses the actual page/session transport.
This checks manual restart, not automatic reconnection.
[Transport cleanup evidence](../bench/endurance/TRANSPORT_LIFETIME.md).

```sh
cargo test --locked -p rustwright-integration-tests --test http_disconnect -- --test-threads=1
```

The separate `http_creation` target delays real allocation/initialization replies
through a local WebSocket proxy, then cancels the waiting caller. Fifty cases per
backend cover isolated-context creation and default/isolated page creation or
initialization. Before owner-context shutdown, exact live page/context IDs,
pending commands and Firefox's helper ledger return to baseline. A retained
page remains usable and another page can be created, navigated and filled.
Allocation acknowledgments are retained for up to 30 seconds after cancellation;
creation-guard cleanup commands are also bounded. An absent/late identifier, failed remote
cleanup or runtime shutdown cannot establish remote reclamation.
[Creation cancellation evidence](../bench/endurance/CREATION_CANCELLATION.md).

```sh
cargo test --locked -p rustwright-integration-tests --test http_creation -- --test-threads=1
```

Separate `http_actionability` and `http_clipped_control` targets add twelve and
ten passing native tests, respectively. Both backends cover disabled fieldset/ARIA
ancestry, transient overlays, movement after pointer hover, click deadlines and
oversized controls with narrow visible overflow strips. Each successful click
is trusted; clipping cases require exactly one click and no intercepting click.
The `http_shutdown` target verifies 50 controlled cancellation cycles, retaining
page handles while checking exact page/context identifiers and helper cleanup.
[Shutdown and click evidence](../bench/endurance/SHUTDOWN_ACTIONABILITY.md)
documents backend differences, retry behavior and bounded-cleanup limits.

Firefox's `http_frame_helpers` target additionally keeps one top-level page for
1,000 same-/cross-origin iframe creation/locator/removal cycles and checks absent
helpers across ten document reloads. The driver no longer retains historical
frame IDs. An additional `http_shutdown` case cancels network monitoring startup
while its subscription response is held, verifies a repeated setup awaits the
response and observes ten completed HTTP requests afterward.
[Independent memory and helper evidence](../bench/endurance/INDEPENDENT_BIDI.md)
also records the added helper-check roundtrip and subscription timeout limits.

Firefox's `http_intercept_lifecycle` target covers cancellation of actual remote
registration/removal acknowledgments, response-phase expansion, shared discovered
handles, final-handle Drop and closure with retained routed pages. Seventy lifecycle
checkpoints verify the actual known intercept IDs are absent and pending commands
are zero. Request/response routing also covers same-/cross-origin child frames.
[Interception evidence](../bench/endurance/INTERCEPTION_LIFECYCLE.md) distinguishes
these owned-ID checks from a full intercept census and documents refusal/lost-ack
limits.

## Scope and remaining gaps

This matrix does not establish complete backend parity, cross-platform reliability
or a performance advantage. Chrome's existing `http_frames`, `http_navigation`,
`http_network_idle` and `http_contexts` targets provide additional CDP coverage;
those cases have not all been ported to Firefox. In particular, the shared matrix
does not yet cover transformed/clipped nested frame geometry, ancestor iframe
overlays, renderer churn under interception, or streaming network-idle
semantics. A separate `http_bidi_navigation` target now executes seven native
Chrome/Firefox checks for driver reload/back/forward: hash and History API
entries, redirected documents, resources held during reload, missing entries,
closed pages, and Firefox timeout/cancellation recovery. `AnyPage` exposes the
three shared methods. [CI and navigation verification](CI_VERIFICATION.md)
records the remaining event/subscription and OS limits.

Firefox's `BidiContext::pages` is async discovery, whereas Chrome also exposes
`BrowserContext::wait_for_page`; the shared popup case checks context-scoped
discovery and ownership. Downloads remain documented as CDP-only; a Firefox
download implementation is outside this matrix. Headed browsers, real-site
compatibility, Windows/macOS and the full integration suite using file/data URLs
need separate evidence. The [Linux endurance observations](../bench/endurance/RESULTS.md)
cover 1,000 measured context/page cycles per backend and a further Firefox helper
ledger run, with sampled resource counts and RSS. Follow-up Firefox operation
ablation adds 6,000 matched cycles and RSS/PSS; Chrome default-context churn adds
1,000 cycles without discovery and covers eager closed-page cleanup. Remaining browser memory growth
and unmeasured resources still need attribution. See the
[reliability milestones](RELIABILITY_ROADMAP.md).
