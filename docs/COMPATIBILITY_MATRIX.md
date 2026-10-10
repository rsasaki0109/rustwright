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

[GitHub Actions run 37994561086](https://github.com/rsasaki0109/rustwright/actions/runs/37994561086)
for source `39e1c8951dd2ebd67cd1ef8e7311f582d418d873` passes all seven jobs.
All seven checkout logs identify tested PR merge
`a2191f83923497f1e34a7785f316e142206dda54`; all 153 relevant input blobs and modes
match the source head. [Frozen final CI evidence](ci/results/reliability-95/remote-final/README.md)
retains raw logs, native counts, versions, sandbox records, six original
artifact digests and verified eight-archive distribution.

| Native scope | Versions | Actual result |
| --- | --- | --- |
| Headless, Ubuntu/Windows/macOS arm64 | Chrome 155.0.8059.39 / Firefox 157.0.1 | 79 per OS, zero failed/ignored/filtered |
| Additional Ubuntu headless HTTP | Same current versions | 114, zero failed/ignored/filtered |
| Headed, Linux Xvfb | Same current versions | Full portable 79, zero failed/ignored/filtered |
| Headless, Linux alternate | Chrome 151.0.7922.138 / Firefox 153.4.0esr | Full portable 79, zero failed/ignored/filtered |
| Headed selected sites, Linux Xvfb | Same current versions | example.com / MDN Web docs / docs.rs: all operations and teardown succeed per backend; HTTP 200 observed, six hashed PNGs |

The 24 compatibility cases above are included in each 79. Each portable suite
has 39 Chrome and 40 Firefox cases across the same five targets. Ubuntu's193
HTTP cases also execute in the workspace job, so overlapping executions are
not counted as new designs. The workspace passes475 cases, including211
library cases and four `compat_report` tests, with one existing ignored macro
doctest and zero failures/filters. Exact CLI/executable identities and raw
protocol versions are retained; ESR reports numeric 153.4.0.

Mapped-window observations establish an owned `IsViewable` virtual X11 window
per headed command/backend using `WM_CLASS` and a live descendant PID. Four
backend observations cover the headed suite and the two site commands. They do
not prove each case's window, physical-desktop interaction or visual correctness.
The site scope uses default browser TLS and no explicit proxy. Repeated Firefox
HTTP lifecycle events are not additional sites; Chrome URL-matched document
candidates can include same-URL iframes. Selected passes do not reclassify the
failed local nine-site aggregates or establish broad website compatibility.
[CI verification](CI_VERIFICATION.md) preserves the earlier failed preflights
and complete historical six-job baseline separately.

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

Browser startup now owns the child immediately after spawn and Chrome's temporary
profile before readiness can suspend or fail. Startup timeout, cancellation and
early-exit regressions that failed before the correction now verify child
termination and reaping before owned-profile removal. Caller-owned Firefox and
persistent Chrome profiles remain intact; Chrome startup failure logs remain
readable outside the temporary profile. A successful temporary Chrome owner
removes its log on Drop. [Startup ownership evidence](ci/results/reliability-95/startup-ownership/EVIDENCE.md)
also verifies successful readiness transfers ownership to the returned handle.
[Twenty native cancellations](ci/results/reliability-95/native-startup/README.md)
observe ten Chrome 155 and ten Firefox 157 processes executing their actual
browser binary before API handoff, then verify direct-child reaping and profile
ownership on Linux. The recorded build used head `908323d` with the then-dirty
context correction, whose frozen files match later `4cdec571`. These are historical
source-scoped observations, not a final `39e1` startup run. These checks cover early
startup and the direct owned child;
descendant trees, later startup stages, OS kill refusal and a hard deadline for
synchronous kill/wait or filesystem cleanup remain outside that guarantee.

Chrome target discovery also handles a tab disappearing after a target snapshot
and successful attachment, while `Page.enable` is still pending. A typed detached
session is skipped only after browser-level `Target.getTargetInfo` returns the
exact missing-target error. A live target's initialization error, other protocol
errors, context closure and socket loss remain errors. [Discovery closure evidence](ci/results/reliability-95/discovery-closure/EVIDENCE.md)
preserves eight controlled cases, the original CI failure and ten passing native
Chrome 155 context cases; the repaired source also passes the full CI workspace
recorded above. This verification adds no retry or blanket error suppression.

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

This matrix establishes the recorded scenarios on the tested platforms and
versions; it does not establish complete backend parity or a performance advantage. Chrome's existing `http_frames`, `http_navigation`,
`http_network_idle` and `http_contexts` targets provide additional CDP coverage;
those cases have not all been ported to Firefox. In particular, the shared matrix
does not yet cover transformed/clipped nested frame geometry, ancestor iframe
overlays or renderer churn under interception. A separate `http_bidi_navigation` target now executes seven native
Chrome/Firefox checks for driver reload/back/forward: hash and History API
entries, redirected documents, resources held during reload, missing entries,
closed pages, and Firefox timeout/cancellation recovery. `AnyPage` exposes the
three shared methods. [CI and navigation verification](CI_VERIFICATION.md)
records the tested source and remaining scope limits.

The shared `http_network_idle_parity` target adds 26 native cases (13 per
backend), included in 79 on each OS. It verifies held/partial response bodies,
redirects, aborts, nested and cross-origin frames, frame removal, replacement
navigation, foreign tabs, interrupted quiet, cancellation and closure.
[Firefox network-idle documentation](FIREFOX_NETWORK_IDLE_IMPLEMENTATION.md)
describes acknowledged page readiness, typed incomplete/lost observation errors
and Firefox's fresh 500 ms window on every call. Service-worker, WebSocket and
missing historical activity reconstruction are outside that guarantee.

Firefox's `BidiContext::pages` is async discovery, whereas Chrome also exposes
`BrowserContext::wait_for_page`; the shared popup case checks context-scoped
discovery and ownership. Downloads remain documented as CDP-only; a Firefox
download implementation is outside this matrix. The specified headed,
alternate-version and selected site checkpoint is now complete; its
[assessment](RELIABILITY_95_CHECKPOINT.md) retains Linux-only headed/version
scope and all failure/ownership/memory limits. Required portable targets have
native Windows/macOS evidence, and Ubuntu passes the full workspace including
legacy file/data fixtures. The [Linux endurance observations](../bench/endurance/RESULTS.md)
cover 1,000 measured context/page cycles per backend and a further Firefox helper
ledger run, with sampled resource counts and RSS. Follow-up Firefox operation
ablation adds 6,000 matched cycles and RSS/PSS; Chrome default-context churn adds
1,000 cycles without discovery and covers eager closed-page cleanup. Remaining browser memory growth
and unmeasured resources still need attribution. See the
[reliability milestones](RELIABILITY_ROADMAP.md).
