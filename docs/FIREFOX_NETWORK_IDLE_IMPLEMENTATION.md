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
observer handle stops its pump. Dead weak entries are pruned on a later page
lookup; they are not removed synchronously on every handle drop. Live tracking
is separate from retained network diagnostics and does not keep a census of
completed request IDs.

Successful local page or user-context closure broadcasts the observer's shared
close signal. All aliases' idle and diagnostic pumps then stop, even if the
remote peer emits no further context-destroyed event and callers retain their
page handles. Browser/connection shutdown also stops these pumps without
waiting for the event broadcaster to be dropped. Retained closed handles can
still own diagnostic history and finished task metadata; stopping a worker is
not a zero-retained-heap guarantee.

Diagnostic setup checks closure before accepting cached readiness, listens for
the close signal during subscription setup, and checks closure again after the
acknowledgement before installing a pump. The owned subscription worker and
five-second setup budget remain independent of a canceled waiter. A closure
racing just before close-receiver registration can still consume that finite
budget; the post-acknowledgement check prevents installing a pump for a known
closed observer. Unknown remote subscription IDs from lost or timed-out replies
cannot be reclaimed by this local lifetime fix, and a retry can create another
remote subscription whose ID was never observed.

## Verification

### Original feature evidence

Source `e3bc08e7ce0a117d173bdcde2924165806e8e854` passed 22 new backend cases,
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

The original feature's local HTTP total was 193: 79 portable cases (including these 26)
and 114 Linux additions. Stable Clippy, Rust 1.85 all-target compilation,
formatting, eight actual package archives, version-only consumers, four native
consumer cases and both README entry points passed. Local engine versions were
Chrome 151.0.7922.173 and Firefox 157.0.1, with container-specific launchers.
[Run 37974358667](https://github.com/rsasaki0109/rustwright/actions/runs/37974358667)
on that feature source succeeded in all six jobs, including all 26 parity cases
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

### Closed-observer correction and completed combined-source CI

The [resource audit](ci/results/reliability-95/resource-audit/ARCHIVE_README.md)
preserves three controlled regressions that failed before the lifetime
correction: a retained idle pump after local page closure, diagnostic pumps on
retained page aliases after local closure, and retained diagnostic pumps after
browser closure. Each peer deliberately sends no later remote destroy event.
All three pass after correction, within the 111-case BiDi suite. The original
audit snapshots match `908323d38ba036960c41347ecbe2a5dfa8c24271`; its scoped run
is distinct from subsequent combined-source verification.

[Run 37994561086](https://github.com/rsasaki0109/rustwright/actions/runs/37994561086)
completed successfully in all seven jobs for head
`39e1c8951dd2ebd67cd1ef8e7311f582d418d873`. All seven original checkout logs identify
actual PR merge `a2191f83923497f1e34a7785f316e142206dda54`.
[All 153 relevant inputs have identical Git blobs and modes](ci/results/reliability-95/remote-final/tested-source-equality.json)
between that merge and head. This proves those inputs' equality, not whole-commit
identity. The earlier [six-job baseline](ci/results/reliability-95/remote/README.md)
remains a separate historical record.

The completed [CI record](ci/results/reliability-95/remote-final/README.md) verifies:

| Scope | Passing cases |
| --- | ---: |
| BiDi library | 111 |
| All workspace libraries, including BiDi | 211 |
| Full workspace, including libraries and HTTP cases | 475 |
| Required portable HTTP cases on each of Ubuntu, Windows and macOS arm64 | 79 |
| Additional Ubuntu HTTP cases | 114 |

There are zero failures; the full workspace has one existing explicitly ignored
macro doctest. Library and native HTTP suites have zero ignored or filtered
cases. The portable total includes all 26 network-idle parity cases; repeated
operating-system executions and the workspace's 193 HTTP cases are overlapping
coverage. Recorded native engines are Chrome `155.0.8059.39` and Firefox
`157.0.1`. Formatting, all-target Clippy, locked Rust 1.85 compilation, and the
eight-archive distribution checks also passed on that recorded source.

### Completed selected headed/version/site verification

The same five portable targets, including all 26 network-idle parity cases,
pass all 79 cases headed on Linux Xvfb with current Chrome 155 / Firefox 157, and
headless on Linux with Chrome 151.0.7922.138 / Firefox 153.4.0esr. Both phases
have zero failed, ignored or filtered cases. Exact installed and protocol
identities are retained; ESR's protocol value is 153.4.0. Owned mapped-window
observations are per headed command/backend, not every case or a physical
desktop. Both current engines also complete all operations and teardown for
example.com, MDN Web docs and docs.rs, with HTTP 200 documents and six hashed
PNGs. Default browser TLS remains enabled; there is no explicit proxy.
[Extended proof](ci/results/reliability-95/remote-final/extended-verification.json)
retains the actual records and document-attribution limits.

Two earlier extended attempts failed Chrome 151's CLI sandbox preflight before
any headed/alternate/site phases. Their
[first completed supplement](ci/results/reliability-95/remote-final-before-completed/README.md)
and [second failure record](ci/results/reliability-95/remote-final-second-before/README.md)
remain unchanged. The final bounded-CDP preflight has actual current/alternate
Chrome success in the final run; its 26 synthetic controls remain separate from
native proof. [The checkpoint](RELIABILITY_95_CHECKPOINT.md) assesses the scoped
95% milestone and preserves the failed local nine-site aggregates. Selected
three-site observations do not establish broad site compatibility or SOTA.

These waits do not establish complete historical reconstruction, service-worker
or WebSocket tracking, complete backend parity, memory attribution, real-site
compatibility or SOTA performance. Remote allocation/subscription replies lost
beyond cleanup budgets and persistent remote cleanup refusal remain separate
ownership limits.
