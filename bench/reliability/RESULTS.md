# Local reliability baseline — 2026-10-08

The [2026-10-09 100-sample comparison](COMPARISON_100.md) includes all 16 cases,
alternating order, complete raw records and separate driver/descendant RSS/PSS.
Rustwright completed 1,600/1,600 measured operations; Playwright completed
1,399/1,600. It includes the closed-page registry correction discovered during
the interrupted diagnostic pass. The historical measurements below retain their
original smaller sample sizes and implementations.

## Frame auto-wait baseline

The comparison found and reproduced an auto-wait bug: a locator inside a late
iframe failed immediately before the iframe existed. In the pre-fix diagnostic
run, Rustwright completed 0/5 delayed-frame attempts and Playwright completed
5/5. Three initial Rust regression tests also failed for late iframe resolution,
missing-frame deadlines and a frame whose document was still loading.

After the fix, both engines completed all 20 measured attempts in each scenario.
The seven frame regression tests passed, including navigation while waiting,
one deadline across iframe/child resolution, and prompt JavaScript/closed-page
error propagation.

| Scenario | Rustwright successes | Playwright successes | Rustwright p95 | Playwright p95 |
|---|---:|---:|---:|---:|
| Button inserted after 120 ms | 20/20 | 20/20 | 144.7 ms | 258.5 ms |
| Iframe inserted after 120 ms; fill and submit | 20/20 | 20/20 | 188.5 ms | 328.8 ms |
| HTTP fetch disconnect and healthy navigation | 20/20 | 20/20 | 31.9 ms | 54.9 ms |
| Pending evaluation interrupted by browser closure | 20/20 | 20/20 | 26.9 ms | 81.5 ms |

These are successful-operation nearest-rank p95 values, not averages. All
attempts succeeded, so the all-attempt p95 values were identical. There were also
no warmup failures.

The run used Chromium 151.0.7922.173, Playwright Core 1.63.0 and Node 24.19.0 on
Linux. Rustwright was built in release mode with Rust 1.99.0 from the working
tree based on commit `91e72af388d000266c9b4e7567a3e1fe4007fc33`, including the
uncommitted stability fixes. Both engines used the same executable wrapper,
which adds the container-required `--no-sandbox` and `--disable-dev-shm-usage`.
The two pairs ran in Rustwright/Playwright and Playwright/Rustwright order, with
10 measured attempts plus one excluded warmup per scenario in each pair.

The measurement command was:

```sh
python3 bench/reliability/run.py \
  --chrome /workspace/.rustwright-env/chromium \
  --samples 20 --pairs 2 --require-success \
  --output target/reliability-after.json
```

The raw local report includes every observation and source/binary hashes. The
pre-fix diagnostic report is `target/reliability-before.json` (5 attempts per
scenario, one pair); it is evidence of the bug, not a statistical before/after
latency comparison.

This is a small controlled baseline. Timing excludes launch and initial
navigation, includes the fixture delay and postcondition checks, and keeps each
engine's default flags and page/context creation. The cases do not compare the
full set of actionability checks or establish overall SOTA performance, real-site
compatibility, Firefox support or production tail latency. Browser closure is
graceful, not a crash or a forced TCP reset. See the [run instructions](../README.md#reliability-and-tail-latency)
for the exact scenario definitions and reporting rules.

## Click actionability follow-up

The next comparison added disabled, covered and moving buttons. Before the
click fix, Rustwright completed 0/5 attempts in each new scenario; Playwright
completed 5/5. The Rust API had returned success despite missing a disabled
button, clicking an overlay or clicking during an animation. Six initial
regression tests also failed, including inherited disabled state and an overlay
created by a hover handler.

Chrome/CDP locator clicks now wait for visibility, enabled state, two stable
animation frames and a receiving hit target. They check the retained target
again after mouse movement, map iframe coordinates and root scroll offsets,
and use one explicit `click_with_timeout` deadline. The same checks apply to
checkbox clicks. Blocked timeouts include their last observed reason; unrelated
JavaScript and connection errors are returned immediately. This change does not
add these click checks to the Firefox/BiDi implementation.

All seven scenarios then passed 20/20 measured attempts in both engines. Each
engine ran two pairs in alternating order, using the same Chromium, Node,
Playwright version and container executable described above.

| Scenario | Rustwright successes | Playwright successes | Rustwright p95 | Playwright p95 |
|---|---:|---:|---:|---:|
| Button inserted after 120 ms | 20/20 | 20/20 | 177.3 ms | 246.9 ms |
| Iframe inserted after 120 ms; fill and submit | 20/20 | 20/20 | 211.1 ms | 323.9 ms |
| Disabled button enabled after 120 ms | 20/20 | 20/20 | 196.1 ms | 244.4 ms |
| Overlay removed after 120 ms | 20/20 | 20/20 | 175.2 ms | 238.7 ms |
| Button moving for 120 ms | 20/20 | 20/20 | 209.8 ms | 271.1 ms |
| HTTP fetch disconnect and healthy navigation | 20/20 | 20/20 | 27.6 ms | 55.5 ms |
| Pending evaluation interrupted by browser closure | 20/20 | 20/20 | 23.3 ms | 68.4 ms |

There were no measured or warmup failures. The additional checks increase the
latency of simple clicks: in the earlier 20-attempt baseline, delayed-button p95
was 144.7 ms and delayed-frame p95 was 188.5 ms. These are observations from
separate local runs, not statistically controlled latency regressions.

The raw reports are `target/click-before.json` (5 attempts, one pair) and
`target/click-after.json` (20 attempts, two pairs). The final report's source and
release-binary hashes matched the files used for measurement. The command was:

```sh
python3 bench/reliability/run.py \
  --chrome /workspace/.rustwright-env/chromium \
  --samples 20 --pairs 2 --require-success \
  --output target/click-after.json
```

The combined frame/click suite passed 24 tests, including disabled fieldsets,
the first-legend exception, inherited ARIA state, hover-time replacement and
overlays, a parent overlay above an iframe, offscreen main/frame targets, a
one-pixel target, checkbox actions, stalled preparation deadlines and no late
clicks after blocked preparation times out. Workspace library tests, existing
assertion/startup tests, documentation/reporting tests, formatting and Clippy
also passed.

These remain small local Chromium scenarios. The fixtures do not establish full
Playwright actionability parity: the candidate point is the centre of clipped
bounds, so arbitrary clipping and transformed geometry can need more advanced
point selection. The browser-disconnect and statistical limitations of the
earlier baseline also apply.

## Click geometry follow-up

The next diagnostic run reproduced clicks timing out on a wide button inside an
overflow container and on a rotated button clipped by the viewport. Before this
fix, Rustwright completed 0/2 attempts in both scenarios. Playwright completed
2/2 overflow-clipped attempts and 0/2 rotated/clipped attempts with its default
click position. Three of the four initial geometry regression tests failed in
Rustwright; the perspective-only case already passed. These small diagnostic
samples establish reproducibility, not a before/after latency estimate.

Chrome/CDP clicks now use content quads instead of mapping an axis-aligned
bounding rectangle into a transformed quad. The code clips each quad polygon to
the root viewport, tries fragment centroids followed by interior points near
edges and vertices, and verifies the actual receiving node before and after
mouse movement. The search is bounded to 16 fragments and 256 candidate points;
it does not exhaustively search arbitrary visible slivers or clip paths.
Geometry checks include active transforms, inline fragments and root content
quads, so movement can be detected even when the bounding rectangle is unchanged.

The combined HTTP frame/click suite passed 33 tests. Its nine new cases cover
partial overlays, overflow clipping, clipped rotation, perspective, a rotated
partial overlay, a clip path, multiple inline fragments, a transformed iframe,
and an animated transform with an unchanged bounding rectangle. Six polygon
unit tests, all 39 workspace library tests, 12 assertion tests, five startup
tests, two core documentation tests and four report tests passed. Formatting,
Clippy with warnings denied and the release harness build also passed. This
validation uses local HTTP fixtures; it does not claim completion of the
existing file/data-URL browser suites restricted by this environment's policy.

In the final comparison, Rustwright completed all 180 measured operations across
nine scenarios, with no warmup failures. Playwright completed 20/20 in eight
scenarios; all 20 rotated/clipped operations and its two excluded warmups timed
out. Both engines used the same fixture and default click APIs, without an
explicit click position or forced input. Their different point selection
policies are part of this observation, not evidence of overall superiority.

| Scenario | Rustwright successes | Playwright successes | Rustwright p95 | Playwright p95 |
|---|---:|---:|---:|---:|
| Button inserted after 120 ms | 20/20 | 20/20 | 199.6 ms | 246.3 ms |
| Iframe inserted after 120 ms; fill and submit | 20/20 | 20/20 | 214.0 ms | 341.0 ms |
| Disabled button enabled after 120 ms | 20/20 | 20/20 | 199.7 ms | 247.5 ms |
| Overlay removed after 120 ms | 20/20 | 20/20 | 169.8 ms | 241.8 ms |
| Button moving for 120 ms | 20/20 | 20/20 | 212.0 ms | 276.9 ms |
| Wide button clipped by an overflow container | 20/20 | 20/20 | 90.5 ms | 103.5 ms |
| Rotated button clipped by the viewport | 20/20 | 0/20 | 83.2 ms | — |
| HTTP fetch disconnect and healthy navigation | 20/20 | 20/20 | 26.9 ms | 59.7 ms |
| Pending evaluation interrupted by browser closure | 20/20 | 20/20 | 30.3 ms | 79.3 ms |

The dash means no successful observations, not zero latency. Playwright's
rotated/clipped all-attempt p95 was 2001.1 ms against a 2000 ms watchdog. Successful
p95 values use the nearest-rank method across 20 samples, excluding warmups and
failures. The raw report retains every failed observation and its error.

This run used the same Chromium 151.0.7922.173, Playwright Core 1.63.0, Node
24.19.0, Rust 1.99.0 and container wrapper as the preceding runs. It ran two
pairs in alternating engine order, with ten measured attempts and one warmup
per scenario in each pair. The working tree was still based on commit
`91e72af388d000266c9b4e7567a3e1fe4007fc33` with uncommitted improvements. Recorded
source hashes and the release-binary hash matched the measured files.

```sh
python3 bench/reliability/run.py \
  --chrome /workspace/.rustwright-env/chromium \
  --samples 20 --pairs 2 --require-engine-success rustwright \
  --output target/geometry-after.json
```

The raw local reports are `target/geometry-before.json` and
`target/geometry-after.json`. The engine-specific success requirement enforces
Rustwright's measured outcomes while retaining all Playwright failures. Geometry
point selection replaces the centre-only limitation noted in the earlier click
baseline; the bounded search and Chromium-only scope remain. The small-sample,
real-site, production latency and browser-disconnect limitations above still
apply.

## Cross-site iframe follow-up

After correcting the shared fixture's cross-origin notification handling, a
same-site iframe on another port worked with the previous implementation. The
remaining product regressions were a delayed cross-site iframe and a wait during
a same-site to cross-site process swap: both failed their initial regression
tests. A raw CDP probe confirmed an `iframe` target in a separate session. Its
execution context and content quads were local to that renderer; the root
page's hit test returned the iframe owner rather than its button.

Chrome/CDP now auto-attaches iframe targets recursively while retaining site
isolation, installs the locator helpers before resuming new renderers, and
uses each default execution context's owning session for evaluation and DOM
actions. Context ids and remote object ids are never routed by their numeric
value alone. Frame ancestry survives process swaps; isolated worlds cannot
overwrite the default context. Existing-page attachment initializes loaded
iframe documents and the current page URL.

Clicks project renderer-local quads through iframe content quads into root
coordinates, including nested affine and perspective transforms. Hit testing
checks each ancestor renderer and the child before and after mouse movement,
so parent overlays block input. Owners scroll only when offscreen, avoiding
repeated scroll oscillation between an iframe and its target.

A further regression reproduced a renderer detaching while click preparation
awaited a never-resolving promise. The transport now completes that session's
pending requests with `CdpError::SessionDetached`; destruction of an explicitly
selected evaluation context returns `CdpError::ContextDestroyed`. Other sessions
and contexts remain usable, and late responses cannot complete a newer request.
Frame waits and click probes can retry before input within their original
deadline; input dispatch errors are never retried. Detaching an ancestor also
removes its descendant sessions from the page registry.

The combined HTTP suite passed 44 tests, including 11 new regressions for
another-port frames, actual OOPIF input/submission, process swaps in both
directions, reloads, sibling sessions, transformed/scrolled parent overlays,
nested renderer transforms, removal/recreation, existing-page attachment,
isolated worlds, JavaScript error propagation, preparation deadlines and
detachment during preparation. The workspace's 44 library tests also passed;
five new unit tests exercise projective mapping, pre-input retry classification,
subtree cleanup, session-specific cancellation and context-specific cancellation.
The 12 assertion tests, five runner startup tests, two core documentation tests,
four report tests, formatting and Clippy with warnings denied passed.

The comparison adds two scenarios using `127.0.0.1` and `localhost`. One inserts
the remote iframe at 120 ms. The other navigates a same-site iframe to the remote
site at 80 ms; that document inserts the requested input at 120 ms after load.
Both require the submitted value to reach the parent and an actual `iframe` CDP
target to exist. Target and origin verification are part of the measured
postconditions, with site isolation retained in both engines.

An initial full comparison stopped during untimed setup when `Page.navigate`
exceeded its two-second timeout; no completed full report was produced. Initial
navigation was then configured with ten-second API timeouts in both harnesses.
The measured operation/postcondition watchdog stayed at two seconds, and setup
errors still fail the run rather than becoming successful or skipped samples.

The final complete run measured 20 attempts per engine and scenario in two
pairs with alternating engine order. Rustwright completed all 220 measured
operations across 11 scenarios without warmup failures. Both engines completed
20/20 in each new cross-site scenario. Playwright completed 20/20 in ten
scenarios; the existing rotated/clipped fixture still failed all 20 measured
attempts and both warmups with its default click selection.

| Scenario | Rustwright successes | Playwright successes | Rustwright p95 | Playwright p95 |
|---|---:|---:|---:|---:|
| Button inserted after 120 ms | 20/20 | 20/20 | 224.9 ms | 364.5 ms |
| Same-site iframe inserted after 120 ms; fill and submit | 20/20 | 20/20 | 394.2 ms | 689.1 ms |
| Cross-site iframe inserted after 120 ms; fill and submit | 20/20 | 20/20 | 606.4 ms | 635.2 ms |
| Same-site to cross-site navigation; delayed input and submit | 20/20 | 20/20 | 498.4 ms | 699.7 ms |
| Disabled button enabled after 120 ms | 20/20 | 20/20 | 199.5 ms | 209.9 ms |
| Overlay removed after 120 ms | 20/20 | 20/20 | 208.5 ms | 198.1 ms |
| Button moving for 120 ms | 20/20 | 20/20 | 273.0 ms | 289.4 ms |
| Wide button clipped by an overflow container | 20/20 | 20/20 | 251.1 ms | 202.5 ms |
| Rotated button clipped by the viewport | 20/20 | 0/20 | 187.7 ms | — |
| HTTP fetch disconnect and healthy navigation | 20/20 | 20/20 | 107.5 ms | 258.6 ms |
| Pending evaluation interrupted by browser closure | 20/20 | 20/20 | 61.8 ms | 149.7 ms |

Successful p95 uses nearest rank across all 20 measured successes. The dash
means no successes. Playwright's rotated/clipped all-attempt p95 was 2712.0 ms;
observed elapsed times that overran the nominal 2000 ms watchdog are retained.
Timing varied across pairs: Rustwright's cross-site iframe median was 390.4 ms
in the first pair and 258.5 ms in the second; Playwright's was 524.3 ms and
437.6 ms. These local observations do not establish a statistically significant
performance advantage or a latency regression against earlier runs.

The run used Chromium 151.0.7922.173, Playwright Core 1.63.0, Node 24.19.0 and
a Rust 1.99.0 release build, with the same container executable wrapper as above.
The working tree remained based on commit
`91e72af388d000266c9b4e7567a3e1fe4007fc33` with uncommitted changes. All recorded
source and release-binary hashes matched the measured files. The raw local
report is `target/cross-site-after.json`; its command was:

```sh
python3 bench/reliability/run.py \
  --chrome /workspace/.rustwright-env/chromium \
  --samples 20 --pairs 2 --require-engine-success rustwright \
  --output target/cross-site-after.json
```

This change validates Chromium frame DOM/evaluation and input behavior. It does
not add OOPIF-specific network interception or change Firefox/BiDi. The tests
use local HTTP; restricted file/data-URL browser suites were not run. The bounded
point search, small-sample, real-site and browser-disconnect limitations from
the preceding comparisons still apply.

## Cross-site network follow-up (2026-10-08)

This section records the implementation before the renderer-return gate fix
documented in the next follow-up.

Before this change, page routes enabled Fetch only in the main CDP session.
An actual OOPIF's inline `fetch('/api/network')` received `network` from the
HTTP server instead of the configured `mocked` response. The new regression
failed before the fix and passed afterward.

Route updates now configure all active page/iframe sessions. A newly attached,
paused iframe inherits interception before its scripts resume, including nested
renderers and response-stage interception. An asynchronous configuration lock
serializes route mutations with child registration. Every mutation reapplies the
configuration, so a later call can repair a cancelled or partially failed update.
Detached children do not fail an otherwise valid update; unrelated errors and
root-session failures propagate. `clear_routes` disables Fetch in all active
sessions, restores HTTP caching and returns cleanup errors. While routes are
active, cache disabling ensures previously cached resources still reach mocks.

Nine new HTTP browser regressions cover:

- Inline-script mock inheritance before the child starts running.
- Existing-child updates, first-match precedence, unmatched requests, clearing
  and re-adding rules, with parent-page checks as controls.
- Blocking in new and existing children, followed by restored server access.
- Response-stage upgrades, inherited response headers and header cleanup.
- Request-header changes that preserve other supplied headers.
- Reloads and interception reapplied after returning to the parent's renderer.
- Nested iframe inheritance and clearing both renderer sessions.
- Concurrent child attachment with route addition and clearing.
- Mocks overriding previously cached responses.

Validation used Chromium 151.0.7922.173, Rust 1.99.0 and local HTTP with site
isolation enabled. All 53 frame/click/network regressions passed with two test
threads in an isolated run. A preceding run overlapped compilation and other
browser suites and had three existing 600 ms geometry-click timeouts; the full
isolated rerun passed. Workspace library tests (44), assertion/startup regressions
(17), formatting, Clippy with warnings denied and core doctests also passed.
Restricted file/data-URL suites were not run. No new performance comparison was
run; the preceding benchmark tables describe the earlier implementation.

### Initial renderer-return limitation (addressed below)

When an OOPIF navigates back into its parent's renderer, Chromium can start the
new document's first fetch before the `Page.frameAttached` configuration update
is handled. That first request can bypass the route. Reapplying Fetch on that
event preserves interception for subsequent requests; this does not guarantee
interception of the earliest inline-script request on the return navigation.
The process-swap regression checks subsequent requests after each navigation,
and inline-script responses on initial OOPIF attachment and a remote reload.

The independent diagnostic
[`network-swap-probe.mjs`](network-swap-probe.mjs) reproduces the initial-fetch
gap using Playwright Core 1.63.0's public APIs with the same Chromium executable:

```sh
node bench/reliability/network-swap-probe.mjs /workspace/.rustwright-env/chromium
```

| Navigation | Initial inline-script response with an active mock |
| --- | --- |
| Parent `127.0.0.1` → iframe `localhost` | `mocked` |
| Iframe `localhost` → `127.0.0.1` | `network` (route bypassed) |

This is a correctness diagnostic, not a timed benchmark. The change covers the
Chrome/CDP route APIs; it does not extend OOPIF network diagnostics/body retrieval
or change Firefox/BiDi. Workers are outside the iframe-only attachment filter.

## Renderer-return route gate follow-up (2026-10-08)

The previously documented initial-request gap is now addressed in Rustwright's
Chrome backend. Active routes register a document-start `debugger` statement
through CDP and enable the debugger in each owning page/iframe session. When that
startup statement pauses execution, the event pump restores Fetch interception,
waits for the browser's frame-tree response and resumes the renderer. This happens
before the document's inline scripts issue fetch/XHR requests. The earlier
`Page.frameAttached` update is no longer needed. No JavaScript request API is
replaced, and site isolation remains enabled.

Startup scripts are registered only while routes are active. Their CDP
identifiers are retained per session; clearing routes removes the scripts,
disables the driver's debugger and Fetch, and restores caching. Detached-session
identifiers are discarded. Route changes and cleanup hold an owned configuration
lock in a task that completes even when the caller drops its future. Later
cleanup therefore waits for an interrupted caller's registration to finish and
can remove the successfully registered scripts.

The existing process-swap regression now asserts the **first inline-script
response** after returning to the parent renderer, as well as subsequent
requests. Seven additional browser regressions cover:

- Ten cycles through both fetch and XHR documents on both sites: 40 navigations
  plus the initial OOPIF request, with every response mocked and zero requests
  reaching the fixture's API endpoint. Native fetch/XHR functions are checked.
- First-request blocking on return, with zero server API requests, followed by
  restored server access after clearing routes.
- First-response header changes on return and after main-frame navigation.
- Nested child return into its outer iframe's renderer.
- Automatic resumption of application `debugger` statements during routing,
  followed by clearing and re-adding routes.
- No startup pause in a separately attached debugger after routes are cleared.
- Dropped route/cleanup futures followed by reconfiguration and script cleanup.

All 60 HTTP frame/click/network regressions passed in an isolated run with one
browser test thread. An earlier two-thread run had four existing 700 ms frame
helper timeouts; the subsequent serial runs passed without relaxing deadlines.
Workspace library tests (44), assertion/startup regressions (17), core doctests
(2), formatting and workspace Clippy with warnings denied also passed.

```sh
cargo test --locked -p rustwright-integration-tests --test http_frames -- --test-threads=1
```

Validation used Chromium 151.0.7922.173 and Rust 1.99.0 on local HTTP fixtures.
The Playwright diagnostic above still describes its default public-API behavior;
Rustwright's gate is a CDP workaround. This is correctness evidence, not a new
latency comparison or a claim about arbitrary sites. Active routes now use the
CDP debugger and automatically resume debugger pauses, so manual breakpoint
pauses should not be expected to remain stopped while routing is active. Pages
without routes do not install the startup scripts or enable the debugger. The
Firefox/BiDi and OOPIF network-diagnostics/body-retrieval scope is unchanged.

## Network timing follow-up (2026-10-08)

Five shared HTTP scenarios now measure native fetch and XHR mocking. Every
mocked response must have status 201 and body `mocked`; after clearing, the first
responses must have status 200 and body `network`. Native function checks are
included. These scenarios do not include the earlier 120 ms fixture delays:

| Scenario | Timed operation | Untimed scenario setup |
| --- | --- | --- |
| `mock_fetch` | Register the mock, issue fetch/XHR, verify both | Initial page navigation |
| `mock_navigation` | Navigate the main document, verify its first fetch/XHR | Register the mock |
| `mock_frame` | Create a cross-site iframe, verify first fetch/XHR and its CDP iframe target | Register the mock |
| `mock_return` | Navigate that iframe back to the parent site, verify first fetch/XHR | Register the mock, load and verify the OOPIF |
| `mock_clear` | Clear routes, navigate, verify first fetch/XHR reach the server | Register the mock, load and verify a mocked document |

The new `--cases` option selects these cases without rerunning the older click
and disconnect scenarios. Reports record the selection and validate every sample;
missing or unexpected cases are rejected. Failed observations never contribute
to successful-operation latency, including failures that return quickly.

### Removing one redundant protocol round trip

A route startup pause previously sent `Network.setCacheDisabled`, `Fetch.enable`,
`Page.getFrameTree` and `Debugger.resume`. Cache policy is already configured by
route mutations and child initialization, under the same routing lock, and
persists across navigation. Moving the cache command out of the pause handler
reduces that path from **four CDP commands to three**. The Fetch update and
frame-tree synchronization remain, preserving first-request interception.
This is a 25% reduction in commands on that path, not a 25% latency claim.
Registration and clearing retain their existing command sequences.

An added HTTP regression primes cacheable resources on both origins before
mocking, then checks iframe returns, cross-site navigation, main-document
navigation and reload after clearing. This checks the cache policy continues to
hold when it is no longer resent on startup pauses.

### Measurements

Each stage used 30 measured attempts per engine and case, divided into two
15-sample pairs with engine order reversed for the second pair. Each engine/case
has one excluded warmup per pair. Rustwright succeeded in all **150/150**
measured operations in each stage, without failed warmups. Playwright succeeded
30/30 in four cases; `mock_return` had 0/30 successes in both stages, with two
additional excluded warmup failures. Those requests received status 200/body
`network` instead of the mock. All failures remain in the raw reports.

The following values are milliseconds. Each latency cell is **successful
median / nearest-rank p95**; with 30 attempts the p95 is the 29th sorted value.

| Scenario | Rustwright before | Rustwright after | Rustwright after success | Playwright after success | Playwright after latency |
| --- | ---: | ---: | ---: | ---: | ---: |
| `mock_fetch` | 25.0 / 44.7 | 23.7 / 32.6 | 30/30 | 30/30 | 46.7 / 94.8 |
| `mock_navigation` | 84.4 / 323.5 | 65.9 / 110.9 | 30/30 | 30/30 | 96.3 / 161.8 |
| `mock_frame` | 111.6 / 170.4 | 116.2 / 271.9 | 30/30 | 30/30 | 270.9 / 378.8 |
| `mock_return` | 66.3 / 117.4 | 66.0 / 92.6 | 30/30 | 0/30 | — |
| `mock_clear` | 49.7 / 66.5 | 51.7 / 115.6 | 30/30 | 30/30 | 71.5 / 99.2 |

Playwright's failed-return all-attempt p95 was 128.1 ms after and 323.2 ms before;
these are failed-operation times, not successful latency comparisons. The
unchanged registration/clear paths also varied between stages, and iframe/clear
p95 increased. Return medians per pair were 65.7/67.1 ms before and 69.5/60.6 ms
after. These data confirm correctness and the removed command but do **not**
establish a statistically significant latency improvement or general superiority.

Both stages used Chromium 151.0.7922.173, Playwright Core 1.63.0, Node 24.19.0 and
Rust 1.99.0 release builds in the same cloud environment (CPU quota four cores,
32 GiB memory limit). Benchmarks ran without concurrent builds/browser test
suites. Engine launch flags and page/context implementations differ, as in the
earlier comparisons. Two-second measured-operation watchdogs and ten-second
initial-navigation API timeouts are unchanged. The before/after stages were
sequential, not randomized version crossover trials.

Raw local reports are `target/network-before.json` (created
2026-10-08T11:46:14Z) and `target/network-after.json` (11:53:49Z), based on commit
`91e72af388d000266c9b4e7567a3e1fe4007fc33` with uncommitted changes. All recorded
source and binary hashes were checked. The before page-source and release-binary
snapshots are saved as `target/network-before-page.rs` and
`target/network-before-reliability`; the other measured source files are identical
between stages. The command, with the output filename changed for each stage,
was:

```sh
python3 bench/reliability/run.py --chrome /workspace/.rustwright-env/chromium \
  --samples 30 --pairs 2 \
  --cases mock_fetch mock_navigation mock_frame mock_return mock_clear \
  --require-engine-success rustwright --output target/network-after.json
```

The scope remains local Chromium correctness/performance evidence. Manual
breakpoint pauses while routing is active, Firefox/BiDi, OOPIF body retrieval,
real-site variability and the earlier benchmark limitations remain as documented.


Final validation passed: 61 HTTP browser regressions, 44 workspace library tests,
two core doctests, six reporting tests, Node syntax checks, workspace formatting
and Clippy with warnings denied. The final release example built successfully.
Restricted file/data-URL browser suites were not run.
