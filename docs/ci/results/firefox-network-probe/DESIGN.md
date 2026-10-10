# Read-only Firefox network-idle parity investigation

This directory is exploratory evidence. No production Rust source, manifest,
workflow or tracked documentation was edited by this probe. Tests ran only on
Firefox 157.0.1, Linux headless, local HTTP, fresh disposable profiles, through
independent Python/WebSocket BiDi commands with no Rustwright helpers.

## Reproduce

Activate `/workspace/.rustwright-env/activate.sh`. Python 3.12.14 with websockets
16.0 was already installed. Commands used:

```
python3 target/firefox-network-probe/probe.py
PROBE_OUTPUT=target/firefox-network-probe/run2 python3 target/firefox-network-probe/probe.py
PROBE_OUTPUT=target/firefox-network-probe/run3 python3 target/firefox-network-probe/fresh-subscription.py
python3 target/firefox-network-probe/summarize.py
```

The first-run script is frozen in run1/probe.py; the top-level probe.py was
extended before run2. Each run has its exact source, full timestamped protocol
commands/replies/events, HTTP header/body timestamps and Firefox stderr. Original
run.log lives in run1; run2.log and run3.log are at this directory's root.
summary.json contains assertions against those captured events. sha256.json
covers source, results and inspected production source snapshots. Each run's
metadata records the actual Firefox ELF binary hash and launch command.

## Concrete findings

1. CDP NetworkActivity already tracks live requests independently from diagnostic
   history and waits for a 500 ms quiet window. BiDi has only a diagnostic Vec;
   it does not expose this network-idle operation.
2. BiDi dispatch accepts only params.context == root. Both same-origin and
   cross-origin child fetches use distinct child context IDs. contextCreated
   carrying parent=root preceded their first request in the actual Firefox run.
3. The BiDi monitor installs its local event receiver after the remote subscribe
   acknowledgment. An event emitted before that acknowledgment can be lost.
4. Streamed-body responseStarted precedes responseCompleted by approximately
   one second; responseStarted is not a live-request terminal event.
5. Redirects use one request ID, increment redirectCount 0/1/2, and complete all
   three hops individually (302, 307, 200). Counting each start without removing
   each completed hop would leak pending counts.
6. User abort and child removal produce fetchError(NS_BINDING_ABORTED). Early
   body close produces fetchError(NS_ERROR_NET_PARTIAL_TRANSFER). Child removal
   emitted error before contextDestroyed; a new navigation emitted its new
   request start before the old document's pending fetch abort.
7. Subscribing after a request has begun is especially unsafe for lazy idle
   setup: Firefox emitted only a late beforeRequestSent at body completion, with
   no terminal event for at least 2.10 seconds after that late event. This
   reproduced both after unsubscribe/re-subscribe and on the first-ever network
   subscription in a new browser. The request had already been active for about
   100 ms before subscribe. Subsequent newly-begun requests emit all three normal
   events. The late before event carries request.timings.responseEnd > 0.
   Before that late event arrives, a lazy observer could falsely conclude zero
   activity while the body is still arriving. This cannot be repaired by merely
   starting the quiet timer after the subscription acknowledgment.

The first run validated five observations, the extended run nine, and the
fresh-first-subscription run five. These are repeated observations across three
native sessions, not nineteen distinct regression tests or cross-platform proof.

## Proposed finite implementation

* Add a separate per-root live activity object: request ID -> current hop
  redirectCount, owning context, navigation ID when present. Never derive the
  count from retained diagnostic entries or their optional response status.
* Create the local receiver before sending subscribe, and acknowledge readiness
  only after its pump and required subscriptions/context ownership bootstrap are
  ready. Use cancellation-safe, budgeted shared setup as the existing monitor
  does. Establish monitoring before navigation/evaluation for pages created by
  Rustwright; expose/operate a new page only after this readiness. Do not claim
  complete pre-subscription activity for discovery of already-active contexts.
* Subscribe to contextCreated/contextDestroyed and initial getTree when existing
  contexts need ownership bootstrap. Track root plus descendants, recursively;
  ignore unrelated tabs. Remove destroyed descendants and their pending requests.
  Retain only live context/request ownership, rather than a completed-request ID
  census. A page-scoped remote subscription alone is not sufficient local
  isolation because the session's broadcast can also contain events generated
  by another page's subscription.
* A beforeRequestSent for an owned HTTP(S) request starts or replaces its hop.
  A completed/error event finishes only the matching current hop. Duplicate or
  stale terminal events must not restart the quiet timer or remove a newer hop.
  responseStarted updates diagnostics but does not finish live activity.
* A late before event with a nonzero completed responseEnd timestamp can be
  recognized as already-finished history; validate this interpretation against
  protocol replay and native observations before using it. Do not use arbitrary
  expiry of pending requests to manufacture idle. Even handling this historical
  event does not reconstruct requests active before a lazy subscription.
* Zero pending starts a 500 ms quiet window. A new start cancels it. Wait with a
  watch notification and sleep_until deadline, subscribe before checking state,
  and recheck under lock when the timer fires. User timeout releases the waiter
  without tearing down shared monitoring or changing pending state.
* Close/disconnect wakes waiters with a typed closed error. Broadcast lag must
  invalidate the observation and return an explicit error; silently continuing
  could report false idle, and BiDi has no request-list snapshot for recovery.
* Main-document replacement must reconcile old-context requests without deleting
  the new navigation request that may already have started. Aborts/destruction
  naturally handle the observed Firefox sequence; document generation and
  navigation IDs are needed for a deterministic protocol replay of missing or
  reordered old-document terminal events.

## Minimal meaningful regression cases

Protocol tests (paused time where applicable):

* Register receiver before acknowledgment; buffered early start cannot be lost.
* Concurrent/canceled setup yields one acknowledged pump and finite timeout.
* Quiet timer interrupted by new request; duplicate terminal does not reset it.
* Same ID with hop counts; stale old-hop terminal cannot finish the new hop.
* Body header event leaves request pending; fetchError finishes it.
* Existing/new nested child ownership, foreign-tab isolation, child destruction.
* Close/disconnect and broadcast lag release waiters with errors.
* Timed-out wait leaves monitoring valid for another waiter.
* Document replacement preserves a newly-started navigation and drops proven-old
  requests only; no perpetual census of completed contexts or requests.

Native HTTP tests: one delayed-body case (>500 ms), redirect chain with delayed
final body, AbortController/short-body failure, same/cross-origin child delayed
fetch, iframe removal during fetch, navigation replacement, unrelated tab
activity, and repeated wait after timeout. Start observation before fixture
requests. Preserve the first-ever late-subscription probe as evidence defining
unsupported historical reconstruction; do not turn it into an unreliable test
that merely waits a fixed delay before asserting success.

No public site, visible-window, Windows/macOS, persistent/discovered active page,
service worker, WebSocket lifetime, or heap-leak claim is established here.
