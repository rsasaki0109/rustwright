# Firefox network-idle waits

`BidiPage` and `AnyPage` now provide `wait_for_network_idle()` and
`wait_for_network_idle_with_timeout(Duration)`. The default timeout is 30 seconds.
They wait for 500 ms without unfinished **observed HTTP(S) requests** belonging to
the page or its descendant frames. Receiving response headers does not finish a
request; response-body completion or a matching fetch error does.

Firefox starts a fresh 500 ms observation window on every call because its
request notifications can arrive after an initiating command's response. A
Firefox timeout shorter than 500 ms expires even on an already-quiet page.
Chrome can reuse its already-established quiet window. A new request interrupts
quiet; the timer is rechecked against shared live state before returning.

## Readiness, ownership and errors

New default-context and isolated-context Firefox pages acknowledge subscription
and initialize their context ownership before being returned to the caller.
The receiver is registered before subscription. Setup has a five-second budget,
retains a finite prefix of early events, and continues independently of a
canceled first caller. The live pump and observer are shared by page handles.

Discovery reuses an existing complete observer while an owning handle remains.
An already-running discovered context without one returns
`BidiError::NetworkObservationIncomplete`; starting diagnostics later cannot
reconstruct earlier activity. This prevents a successful idle result for a
request that began before complete monitoring.

The tracker includes nested and cross-origin descendants and excludes unrelated
tabs. Redirect hops are matched by request ID and redirect count. Stale terminal
events do not finish a newer hop. Delayed old-document commits do not discard
an incoming navigation, including one observed before its navigation-started
event. Destroyed descendants retire their owned activity.

Timeout and waiter cancellation preserve shared monitoring. Page/context/browser
closure wakes waiters with `BidiError::Closed`. Lost broadcast events produce
`BidiError::NetworkEventsLost { skipped }`, rather than an idle decision from
incomplete observations. There is no request-list snapshot to repair that loss.
The weak session registry does not own page handles, and dropping the last
observer handle stops its pump. Live tracking is separate from retained network
diagnostics and does not keep a census of completed request IDs.

## Verification

Source `e3bc08e7ce0a117d173bdcde2924165806e8e854` passes 22 new backend cases,
all 108 BiDi library cases, and all 193 workspace library cases. The 22 are
included in those totals. The identical API probe fails with missing-method
errors before implementation and compiles afterward; this is a compilation
baseline, not a failing native browser run. A stale-document regression also
records an actual failing intermediate implementation and its correction.

The shared `http_network_idle_parity` target passes 13 scenarios per backend:
held response bodies and timeout reuse, redirects, abort, incomplete body,
nested frames, cross-origin frames, child removal, held replacement navigation,
unrelated tabs, interrupted quiet, waiter cancellation, page closure and browser
closure. The Firefox unrelated-tab case additionally verifies discovered-handle
reuse and an active cold context's typed incomplete-observation error. All 26
cases execute real browsers without ignored or filtered tests.

The current local HTTP total is 193: 79 portable cases (including these 26)
and 114 Linux additions. Stable Clippy, Rust 1.85 all-target compilation,
formatting, eight actual package archives, version-only consumers, four native
consumer cases and both README entry points pass. Local engine versions are
Chrome 151.0.7922.173 and Firefox 157.0.1, with container-specific launchers.
[Run 37974358667](https://github.com/rsasaki0109/rustwright/actions/runs/37974358667)
on that exact source succeeds in all six jobs, including all 26 parity cases
on native Ubuntu, Windows and macOS arm64 with Chrome 155/Firefox 157.
The required portable total is 79 per OS; Ubuntu also passes 114 extra HTTP
cases. [CI_VERIFICATION.md](CI_VERIFICATION.md) records the overlapping full
workspace totals and preserved platform setup diagnostics.

[Backend evidence](ci/results/firefox-network-idle/backend/) and
[exact-source local evidence](ci/results/firefox-network-idle/local/) and
[remote CI evidence](ci/results/firefox-network-idle/remote/) preserve
sources, commands, binary identities and raw results. The earlier
[network-idle investigation](FIREFOX_NETWORK_IDLE.md) remains a frozen
pre-implementation snapshot; its statement that the API was missing describes
that inspected source.

These waits do not establish complete historical reconstruction, service-worker
or WebSocket tracking, complete backend parity, memory attribution, real-site
compatibility or SOTA performance. Remote allocation/subscription replies lost
beyond cleanup budgets and persistent remote cleanup refusal remain separate
ownership limits.
