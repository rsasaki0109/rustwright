# Firefox network-idle investigation

**Firefox network-idle parity is not implemented.** This document records the
requirements for a future implementation; it does not establish a completed API
or increase the project's tested platform coverage.

The independent probe used Firefox **157.0.1**, Linux headless, Python 3.12.14,
and websockets 16.0, with fresh profiles and localhost HTTP. It installed no
Rustwright helpers. Raw commands, events, fixture timestamps, source snapshots,
and the actual Firefox executable hash are frozen in
[the evidence directory](ci/results/firefox-network-probe/README.md).

## Observed behavior

| Case | Firefox BiDi events and consequence |
| --- | --- |
| Delayed response body | `responseStarted` precedes `responseCompleted` by 0.96–0.97 seconds. Receiving headers must not finish a request. |
| Two redirects | One request ID, `redirectCount` 0, 1, 2; each hop completes separately with status 302, 307, 200. |
| User abort | `fetchError` with `NS_BINDING_ABORTED`, without `responseCompleted`. |
| Incomplete body | `fetchError` with `NS_ERROR_NET_PARTIAL_TRANSFER`. |
| Same/cross-origin iframe fetch | Child context IDs differ from the root. `contextCreated(parent=root)` precedes child requests. Cross-origin here means two loopback ports; renderer-process isolation was not measured. |
| Removing an active iframe | Request abort precedes `contextDestroyed`. |
| Replacing the document | New navigation starts before the old document's pending fetch aborts. |
| Subscribing after a fetch starts | Only a late `beforeRequestSent` arrives at body completion; no terminal event was observed for a further 2.10 seconds. |

The last case reproduced both after re-subscription and with the **first-ever
network subscription** in a fresh browser. The fetch started approximately
100 ms before subscribing. The late event has `request.timings.responseEnd > 0`.
Requests started after acknowledged subscription produce normal terminal events.
This is a bounded observation, not a claim that the missing event can never arrive.

A future API that lazily subscribes on its first wait could report 500 ms of
quiet while a one-second response body is still arriving; later, the late start
without a terminal could leave a phantom pending request. Starting the timer
at the subscription acknowledgment alone cannot reconstruct prior activity.

The current BiDi diagnostics also filter events by `params.context == root`,
which omits descendants, and register their receiver after subscription is
acknowledged, leaving an early-event window. The Chrome backend already has a
separate live request tracker and a 500 ms quiet window.

The three runs validated **5 / 9 / 5 observations**. These include repeated
observations and are **not 19 unique regression tests**. The summary assertions
checked captured events; no production Firefox network-idle API was exercised.

## Acceptance criteria for implementation

1. Establish acknowledged monitoring before exposing or operating a newly-created
   page. Register the local receiver before subscribing. Concurrent and canceled
   setup must leave one shared pump with a finite setup budget. Do not promise
   complete activity reconstruction for an already-running discovered context.
2. Track live HTTP(S) requests separately from diagnostic history, retaining only
   current request/hop ownership. Include the root and descendants; exclude
   unrelated tabs. Bootstrap existing ownership safely and prune destroyed
   subtrees. Session-wide broadcasts require local ownership filtering even when
   the remote subscription is scoped to a page.
3. Finish matching hops only on `responseCompleted` or `fetchError`.
   `responseStarted` must leave them pending. Duplicate or stale terminal events
   must neither remove a newer hop nor reset its quiet window. Validate handling
   of historical late starts rather than manufacturing idle through expiration.
4. Require 500 ms with zero observed pending requests; a new request interrupts
   that window. Subscribe to notifications before inspecting state and recheck
   when the timer fires. Timing out one waiter must preserve shared monitoring.
5. Wake waiters with a typed close/disconnect error. Event-buffer lag must produce
   an explicit observation error, since BiDi has no request-list snapshot with
   which to repair lost activity. It must not silently report idle.
6. Document replacement must preserve a new navigation request already in flight
   while reconciling old-document activity. Avoid an accumulating census of
   completed request/context IDs.

Protocol tests must cover early events before subscription acknowledgment,
setup cancellation, interrupted quiet windows, duplicate/stale redirect events,
body headers and errors, nested ownership and foreign tabs, destruction,
close/disconnect/lag, repeated waits after timeout, and document replacement.
Native HTTP tests must cover the delayed body, redirect chain, abort/partial
body, same/cross-origin children, child removal, navigation replacement, unrelated
tab activity, and reuse after timeout. Observe fixture requests from their start.

Windows/macOS, visible windows, public websites, service workers, persistent or
already-running discovered pages, WebSocket lifetime, and heap leaks remain
outside this evidence. A full design note is preserved with the frozen artifacts
as [DESIGN.md](ci/results/firefox-network-probe/DESIGN.md).
