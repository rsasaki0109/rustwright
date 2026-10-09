# Reliability milestones toward the 95% goal

Completion estimates used during development are provisional planning judgments,
not benchmark results or proof of state-of-the-art performance. The initial 90% goal
requires evidence across the following areas; small fixes alone do not establish
it. Keep failing, skipped and unexecuted cases separate from passing cases.

## Verified foundation

- [x] Linux Chrome regressions for frame geometry, nested renderer changes,
  request routing, cancellation, navigation lifecycle and resumed network traffic.
- [x] Context ownership across default/isolated contexts and independent CDP
  clients; popup discovery, concurrent attachment and shutdown notification.
- [x] Transport tests for cancellation, response cleanup and detached sessions.
- [x] CDP/BiDi socket-task lifetime: ten regressions fail before correction and
  pass after it, plus a CDP disconnection-marker regression. Native process loss
  and manual restart succeed 25 times per backend, with pending operations
  reporting closure and event senders released after owner Drop.
  [Evidence and limits](../bench/endurance/TRANSPORT_LIFETIME.md).
- [x] Creation cancellation with delayed acknowledgments and initialization:
  ten failing regressions corrected, with seven more ownership/timeout/thread
  checks. Native Chrome and Firefox each complete 50 controlled cancellations
  with exact page/context IDs restored, a retained page usable and subsequent
  creation successful. [Evidence and limits](../bench/endurance/CREATION_CANCELLATION.md).
- [x] Assertion retry and explicit browser startup failure tests.
- [x] Firefox interception ownership: registration/removal cancellation,
  response-phase expansion, shared discovered handles and retained-handle
  resource closure. Native delayed acknowledgments verify real blocked requests,
  descendant routing and removal of the actual known intercept IDs.
  [Evidence and limits](../bench/endurance/INTERCEPTION_LIFECYCLE.md).
- [x] Independent Python/WebSocket Firefox baseline: 6,000 measured cycles plus
  two matched Rust raw runs totaling 2,000 cycles, with context-only/evaluation
  controls and idle-tail samples. Browser PSS growth occurs without Rustwright's
  transport and can decline afterward; heap attribution remains unresolved.
  Historical frame-ID retention and premature network-monitoring readiness are
  corrected, with native same-page iframe churn and subscription cancellation.
  [Evidence and limits](../bench/endurance/INDEPENDENT_BIDI.md).
- [x] Interrupted shutdown: 24 Chrome/Firefox controlled-peer tests and 50 native
  context/page cancellations verify independent cleanup, concurrent completion,
  bounded waiting and Firefox helper retry. Twelve shared actionability and ten
  overflow-clipping checks exercise trusted clicks on both installed browsers.
  [Evidence and limits](../bench/endurance/SHUTDOWN_ACTIONABILITY.md).
- [x] Locked all-target compilation on Rust 1.85.0, with the matching minimum
  version check defined in CI; stable Clippy and formatting checks.
- [x] Verified packaging of eight actual `.crate` archives with README/license
  assets and required helper JavaScript. An external version-only consumer
  resolves those exact archives, compiles all nine Rust README fences on stable
  and Rust 1.85.0, and has native consumer/macro checks on installed browsers.
  [Release evidence and limits](RELEASE_VERIFICATION.md). Registry publication
  and name ownership remain separate from local packaging success.
- [x] Shared Linux HTTP matrix on Chrome 151 and Firefox 157: 12 scenarios per
  backend, 24 real-browser tests with no missing-browser or startup skips.
  See the [compatibility matrix and its remaining gaps](COMPATIBILITY_MATRIX.md).
- [x] Linux endurance: 1,000 measured context/page cycles per backend after
  warmup, with driver/browser RSS separated, sampled page targets/isolated
  contexts and zero pending commands. A further Firefox 1,000-cycle run records
  zero internal helpers awaiting acknowledged removal at each checkpoint.
  Five helper lifecycle regressions cover discovered handles, context ownership,
  Drop and cancellation. [Raw observations and limits](../bench/endurance/RESULTS.md)
  retain remaining memory growth and unmeasured resources explicitly.
- [x] Firefox workload ablation: 6,000 measured cycles plus 600 warmups, with
  direct BiDi/helper/page APIs in forward/reverse order and separate RSS/PSS.
  Growth also occurs without automatic helpers; its cause and steady bound
  remain unresolved. [Evidence and limits](../bench/endurance/ABLATION.md).
- [x] Chrome closed-page registry lifetime: explicit/external closure and browser
  disconnection remove entries without discovery. Three failing ownership tests
  now pass, and 1,000 native default-context cycles alternate explicit/external
  page closure while retaining handles. [Correction and evidence](../bench/endurance/CACHE_LIFETIME.md).
- [x] Local Chromium comparison: all 16 cases × 100 measured attempts per engine,
  alternating engine order. Rustwright 1,600/1,600, Playwright 1,399/1,600;
  successful median/p95, every failure, separate driver/descendant RSS/PSS and
  exact source/binary/version identifiers are preserved. This does not establish
  broad superiority. [Results and scope](../bench/reliability/COMPARISON_100.md).

The relevant browser suites use local HTTP fixtures and an installed, unmodified
Chrome and Firefox. This cloud environment restricts file/data URL browser fixtures, so these
checks do not establish that every integration target passed. Existing benchmark
measurements have their own recorded versions, sample sizes and limits in
[the results](../bench/reliability/RESULTS.md).

## Conditions for the 90% checkpoint

1. **Chrome and Firefox compatibility:** execute a shared local HTTP matrix on
   installed Chrome/CDP and Firefox/BiDi. Cover navigation, fragment/history URLs,
   locator waits, trusted form input, frames, popup ownership, isolated storage,
   cancellation and closure. Document unsupported backend APIs explicitly.
2. **Long-running reliability:** exercise at least 1,000 context/page cycles and
   repeated frame swaps, routing changes and failed/cancelled waits. Record driver
   and browser memory separately, open targets/sessions, pending responses and
   errors. Investigate growth after warmup rather than assuming a passing short
   test establishes absence of leaks.
3. **Reproducible comparisons:** compare Rustwright and Playwright on the same
   browser, machine and fixtures, with at least 100 measured samples per case,
   alternating run order. Publish success counts and failure causes alongside
   median/p95 successful latency and separate memory measurements. Preserve raw
   results and source/binary/version identifiers.
4. **Release checks:** run locked checks at the declared minimum Rust version and
   stable; execute browser tests in CI without silently skipping the required
   browser. Verify documented examples and packaging. Add Windows/macOS evidence
   before making cross-platform reliability claims.
5. **Representative site behavior:** use the existing compatibility report on
   representative real sites, preserving navigation, network and console failures.
   Distinguish server access decisions from driver defects; add generic regression
   fixtures for confirmed driver defects rather than site-specific workarounds.

The initial shared Firefox/Chrome HTTP matrix and 1,000-cycle sampled endurance
checks are verified. Before declaring the long-running checkpoint complete,
attribute the remaining browser RSS/PSS growth with longer runs and heap analysis
(the independent WebSocket baseline reproduces growth and observes idle declines),
measure attached sessions/intercepts, and cover remote allocations whose
identifiers never arrive or arrive after the cleanup grace. Controlled interrupted
shutdown is verified; native owned-browser cancellation, remote cleanup refusal
and runtime loss remain separate limits.
Known page-owned BiDi intercept retirement is verified under acknowledged
removal. Unknown IDs after lost allocation replies, event overflow, overlapping
caller-managed interception and persistent cleanup refusal remain outside that
guarantee; refusal after final-owner Drop can retain a neutral pump/session until
explicit closure.
Controlled creation cancellation is verified; it does not guarantee remote
reclamation after an absent acknowledgment or lost connection.
The local 100-sample alternating comparison is verified; broader
browser/site coverage and independent host repeats remain outstanding.
Continue expanding the shared matrix's remaining geometry, routing and navigation
cases so performance work measures correct behavior.

## Next checkpoint: 95%, before a SOTA claim

The development target is now 95%. Preserve the earlier checkpoint's remaining
limits rather than counting a configured workflow as executed evidence.

Locally verified additions include eleven BiDi navigation protocol cases and
seven native Chrome/Firefox reload/history checks. Required browser policy also
turns explicitly configured legacy discovery/startup errors into failures.
The [CI verification record](CI_VERIFICATION.md) distinguishes configured OS jobs
from executed results and preserves the historical HEAD run separately.

1. **Execute required CI:** test the reviewed changes on Linux, macOS and Windows
   with explicitly installed Chrome and Firefox. Reject missing browsers,
   initialization failure, zero tests, ignored tests and filtered suites. Retain
   logs and actual browser versions; distinguish existing HEAD runs from a future
   run containing the working-tree changes.
2. **Expand backend parity:** cover driver reload/back/forward, nested frame
   transforms and clipping, ancestor overlays, interception during renderer
   changes, and response-body completion during network-idle waits. Document
   backend-specific APIs rather than implying complete parity.
3. **Bound long-running resources:** attribute remaining memory growth, count
   sessions/intercepts as well as pages and contexts, and exercise lost replies,
   remote cleanup refusal and runtime loss. State finite cleanup budgets and
   ownership limits alongside measurements.
4. **Verify representative sites and versions:** preserve operation outcomes,
   console/network errors and browser versions on an agreed site set. Reproduce
   confirmed driver failures in generic local fixtures; access denial alone does
   not establish a driver defect. Include headed sessions and multiple supported
   browser versions before extending compatibility claims.
5. **Verify distribution repeatedly:** run actual archives, version-only consumers,
   minimum-Rust builds, documented entry points and the macro runner in CI. Real
   registry publication remains a separate decision about names and ownership.

Only after these reliability checks should broader SOTA comparisons begin. Use
matched browsers, machines and fixtures, independent repeats and published raw
success/failure, latency and memory results. The existing local comparison is
useful preliminary evidence; it does not establish a general SOTA result.
