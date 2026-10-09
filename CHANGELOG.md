# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

- Add Firefox/BiDi reload, back and forward navigation, with explicit timeout
  variants and shared `AnyPage` operations. Wait for same-document history
  commits or the matching full-document load rather than stale/foreign events.
- Define required Chrome/Firefox HTTP CI jobs for Linux, macOS and Windows and
  verified archive/MSRV/native-consumer jobs. Reject empty, ignored or filtered
  portable suites; explicit browser bindings and the required flag make legacy
  discovery/startup errors fail instead of silently skipping.
- Refresh release summaries on failures, require both native consumer tests and
  the macro callback per backend, and retain the README Firefox screenshot.
  Hash package inputs into staging locations and compare archive source bytes
  to avoid reusing old unpublished same-version dependencies from Cargo's cache.

### Added

- Offline release verification packages the eight distributable crates with
  Cargo verification enabled, audits assets and canonical dependency checksums,
  and compiles an external version-only consumer on stable and Rust 1.85.0.
  All nine Rust README fences are compiled; optional native consumer checks cover
  both backends and the procedural-macro runner without registry publication.
- Native Firefox interception lifecycle checks hold actual successful protocol
  acknowledgments, cancel registration/removal/phase expansion, and verify known
  intercept IDs after shared-handle Drop and retained-handle page/context close.
  Same-/cross-origin child-frame routing is exercised over local HTTP.
- Independent Python/WebSocket Firefox memory attribution with matched raw BiDi
  churn, context-only and single-page evaluation controls, idle-tail samples and
  exact resource IDs. Driver/root/tree RSS/PSS, missing values, confirmed zombie
  exclusions and source/binary identities are preserved separately.
- Native Firefox checks exercise one long-lived page through 1,000 destroyed
  iframe contexts and document/helper replacement, plus monitoring startup
  cancellation followed by observed HTTP response completion.
- Native HTTP shutdown-cancellation tests exercise 50 controlled context/page
  closures across Chrome and Firefox, retaining closed page handles while
  checking exact resource identifiers and subsequent usability. Paired click
  tests cover disabled ancestry, overlays, hover movement and clipped controls.
- Deterministic native creation-cancellation tests for Chrome and Firefox,
  delaying actual protocol responses after remote allocation. Both default and
  isolated pages are checked without closing their owner context to hide leaks.
- Native HTTP process-loss tests restart Chrome and Firefox 25 times each,
  checking pending evaluation/locator failure and event-channel cleanup without
  a graceful protocol shutdown. Missing browsers and startup failures fail.
- Firefox operation ablation with matched direct BiDi/page workloads in
  forward/reverse order. Linux endurance and comparison runners also record PSS
  separately from RSS; comparisons preserve external driver/descendant samples.
- Reproducible Linux HTTP endurance runner for Chrome and Firefox: context/page
  churn, cancelled evaluations, locator timeouts, routing removal and iframe
  renderer changes. It records separate driver/browser RSS, residual targets and
  contexts, pending commands, versions and source/binary hashes.
- Read-only `pending_command_count` diagnostics on CDP/BiDi connections and
  the Chrome browser, covered by cancellation, timeout and response tests.
  BiDi sessions expose `helper_preload_count` for internal registrations awaiting
  acknowledged removal, including asynchronous handle cleanup.

### Fixed

- Every distributable archive now includes a compact README with absolute links
  and both license texts. Required embedded helper assets are audited too.
- `offline_smoke` uses a temporary loopback HTTP fixture and validates Unicode
  input, trusted clicks and PNG output. Introductory Chrome/Firefox examples
  accept headless, URL, profile and screenshot options. README form examples no
  longer assume the public example.com page contains a search form.
- Firefox page discovery shares routing ownership. Protected serialized workers
  commit rules after registration acknowledgment, roll back unaccepted candidates,
  retain IDs through typed removal failures, and retire owned registrations after
  clear, page/context close or final-handle Drop. Pumps receive events before
  allocation, match owned IDs across descendants and prune acknowledged retired
  IDs after draining earlier events. Persistent cleanup refusal and lost remote
  acknowledgments remain documented limits.
- Firefox helpers no longer retain every historical frame context ID. Helper
  presence is checked in the current document, allowing reinjection after
  replacement and preventing late acknowledgments from populating a stale cache.
  This adds a compact evaluation roundtrip per locator helper check.
- Firefox network subscription and monitoring readiness wait for acknowledgment
  and pump installation. Protected serialized setup survives caller cancellation,
  reports failures and has a five-second local bound. The monitoring receiver is
  acquired before its task starts, avoiding loss of immediately arriving events.
- Chrome browser/context cleanup continues after its waiting close future is
  cancelled; concurrent callers await one completion and failed results are
  replayed. Context cleanup attempts all tracked pages and isolated disposal,
  with bounded phases. Firefox browser/page/context cleanup also survives
  cancellation with bounded workers; helper failures are reported and can be
  retried by explicit closure. External Chrome connections retain no process
  ownership.
- Firefox clicks wait for enabled, stable, unobstructed elements and recheck
  after trusted pointer movement, without repeating movement before dispatch.
  A single click deadline includes protocol waiting. Both backends select
  candidates inside conservative ordinary ancestor overflow bounds, correcting
  clicks on oversized controls whose visible portion is a narrow interior strip.
  Native hit testing and pre-input geometry checks remain required.
- Cancelling page/context creation retains allocation acknowledgments long
  enough to reclaim their identifiers, including results queued before handoff.
  New pages are guarded through initialization; failure or cancellation closes
  the created page. CDP discovery cancellation detaches its session without
  closing the existing target. Abandoned allocation replies and guard cleanup
  commands have 30-second limits; absent identifiers remain a remote-cleanup
  limitation. Guards and Firefox helper Drop use their origin Tokio runtime,
  allowing cleanup when a pending future is dropped on another thread.
- CDP/BiDi socket tasks no longer retain their own connection indefinitely.
  Remote loss, writer failure, explicit close and dropping the last connection
  handle stop both tasks. Shutdown interrupts stalled writes and bounds close
  flushing to 250ms; pending commands fail promptly. Repeated CDP closure emits
  one disconnection marker.
- Chrome contexts release closed page handles immediately, including externally
  detached pages and browser disconnection, without requiring page discovery.
  Page state holds a weak reference to its owning registry, preventing an
  ownership cycle. Closed root-session event pumps stop processing browser events.
- Firefox page discovery shares one internal helper preload per browsing context
  instead of registering another script per discovered handle. Closing a page or
  isolated context removes its helper; dropping the last handle schedules
  best-effort cleanup on a live Tokio runtime. Started helper registration keeps
  the returned script identifier even if its caller is cancelled, so it can be
  removed. Caller-managed raw init scripts retain their existing ownership.
- Chrome iframe hover waits for two animation frames after scrolling before
  computing its point and moving the mouse, allowing a newly loaded remote
  frame's painted surface to receive the event.
- Firefox/BiDi JavaScript exceptions and rejected promises now return
  `BidiError::JavaScript` with the browser's message instead of `Ok(null)`.
  Successful `null` and `undefined` values still return JSON null.
- Firefox frame locators send pointer and keyboard actions to their own browsing
  context. Cross-origin clicks, hover, key presses and typing no longer use the
  parent page's viewport or input focus.
- Chrome main-page and Firefox locator wait deadlines include the complete
  protocol evaluation, including a stalled page-side promise. Timing out or
  cancelling releases the local response waiter without blocking later commands.
- CI now installs Firefox and runs the common HTTP scenarios against both
  browsers without skipping discovery or startup failures, with serial browser
  tests to avoid overlapping Firefox profiles and timing-sensitive work.
- Chrome context discovery no longer adopts another context's pages, including
  contexts created by other clients. Closed pages are pruned from tracked lists,
  and concurrent discovery shares one session per target. Popup waits subscribe
  before discovery, cover attachment with the same deadline, and wake on context
  closure or browser disconnection. Failed or cancelled initialization detaches
  temporary sessions, and discovery reports initialization errors.
- The declared minimum Rust version and README now require Rust 1.85, matching
  the existing WebSocket dependencies. CI checks all locked workspace targets on
  Rust 1.85.0 to keep that support claim verifiable.
- Chrome `NetworkIdle` no longer reuses a past lifecycle event after traffic
  resumes. It tracks session-scoped HTTP requests across the page and its frames,
  including response bodies and separate renderers, and requires a fresh 500 ms
  quiet window after the last request finishes or fails. Redirects and renderer
  transfers retain one active request; frame/session removal and document changes
  clear obsolete pending requests. Timed-out or cancelled waits do not stop the
  shared activity tracking, and closing the page wakes an idle timer.
- Chrome navigation waits track document loader ids and discard stale load events.
  Fragment navigation, `pushState` and same-document history preserve load states;
  frame URLs include CDP's separately reported fragment. URL waits subscribe before
  checking state and retain short-lived matching URL events. Page closure and
  browser disconnection wake pending waits. Navigation timeouts cover the command
  and load wait together, and cancelled or superseded navigation waits cannot
  reuse another document's load. Reload and history waits require a new commit;
  BFCache restores reuse the document's DOM/load completion.
- Chrome network diagnostics now include requests from nested out-of-process
  iframes. Response bodies for HAR export are fetched from the observing session;
  session-scoped request ids cannot overwrite another frame's metadata or body.
  Document requests that transfer to a child renderer retain their completion
  and body ownership. Detached frames retain entries, with unavailable bodies omitted.
- Chrome request routes now apply to existing and newly attached out-of-process
  iframe sessions, including nested frames. Mocking, blocking and header changes
  inherit before paused child scripts resume; clearing routes disables Fetch in
  every active session and restores HTTP caching. Active routes install a document
  startup debugger gate so an iframe returning to its parent's renderer also
  intercepts its first inline-script fetch/XHR. Clearing removes these gates and
  disables the driver's debugger. Route changes and child registration are
  serialized; started configuration finishes if its caller is cancelled, preserving
  the identifiers needed for later cleanup. Startup pauses reuse the existing
  cache policy instead of resending `Network.setCacheDisabled`; cache updates
  remain serialized with route changes and child initialization.
- Chrome frame evaluation and locator actions attach to out-of-process iframe
  targets and keep execution contexts and remote objects in their owning CDP
  session. Clicks map coordinates across nested renderer boundaries and check
  parent overlays. Frame reloads and process swaps retain ancestry; isolated
  worlds cannot replace the default context. Existing-page attachment initializes
  loaded iframe documents and the page URL.
- CDP session detachment releases that session's pending response waiters with
  `CdpError::SessionDetached`, without closing the browser connection or affecting
  other sessions. Clicks retry detached probes before input; dispatched input is
  never retried.
- Destruction of an explicitly selected execution context releases pending
  `Runtime.evaluate` waiters with `CdpError::ContextDestroyed`, scoped to the
  session and context. Frame waits can retry after a process swap without waiting
  for a response from the old document.
- Chrome clicks choose verified points from viewport-clipped content quads,
  including alternate points and inline fragments when the centre is occluded.
  This handles partially clipped rotation, perspective and transformed iframes
  without mapping an axis-aligned bounding box into a transformed quad. Geometry
  stability checks include active transforms as well as bounding rectangles.
- Chrome locator clicks wait for enabled state, two stable animation frames and
  a hit target that receives pointer events. They recheck after hover handlers,
  include iframe/scroll offsets and share one `click_with_timeout` deadline.
  Timeout errors explain the last blocked state. Checkbox clicks use the same
  checks, and enabled queries respect disabled fieldsets and inherited ARIA state.
- Frame-scoped locator actions wait for late iframe elements and execution
  contexts, and retry across frame navigation. The explicit wait timeout covers
  frame resolution and the child element together; JavaScript and closed-page
  errors still propagate. Missing frame contexts now use `Error::FrameNotReady`.
- The test runner skips only when automatic discovery finds no browser. Invalid
  executable overrides and startup, connection or context setup errors fail the
  test and follow the configured retry policy instead of silently passing.
- `expect(locator)` matchers retry until the condition matches, with a default
  five-second deadline and a `with_timeout` override. Assertion timeouts include
  locator reads and report the last observation; transport errors still propagate.
- CDP and BiDi connections release pending response waiters when commands time
  out or their futures are dropped, preventing accumulation on long-lived
  connections. Commands already sent to the browser are not cancelled.

## [0.1.0] - 2026-09-20

Initial release.

### Added

- **Workspace**: `rustwright` (facade + prelude), `rustwright-core` (CDP object
  model), `rustwright-cdp` (transport + typed protocol), `rustwright-browser`
  (Chrome/Firefox discovery, launch, profiles), `rustwright-common`
  (backend-agnostic `Selector`, `PageApi`/`LocatorApi`, injected helper),
  `rustwright-bidi` (WebDriver BiDi client + Firefox driver),
  `rustwright-test` + `rustwright-test-macros` (independent test runner).
- **Browser model**: `Browser` / `BrowserContext` / `Page` / `Locator` with
  lazy, auto-waiting locators and `get_by_text|role|placeholder|label|alt_text|test_id`.
- **Navigation & waits**: `goto`, `reload`, `go_back`/`go_forward`,
  `wait_for_load_state`, `wait_for_url`, event-driven locator `wait_for`.
- **Input & forms**: real mouse/keyboard input, `fill`, `check`/`uncheck`,
  `select_option`, `press`, `type_text`, `hover`, `scroll_into_view_if_needed`,
  `mouse_wheel`, `set_input_files`.
- **Frames**: `frames`, `main_frame`, `frame_locator`, cross-origin iframe support.
- **Network**: request interception (`route`/`mock`/`block`), request- and
  response-header modification, network diagnostics.
- **Files & dialogs**: download path, uploads, auto-dismissed JavaScript dialogs.
- **Tracing**: Chrome trace capture (`start_tracing`/`stop_tracing`).
- **Storage**: cookies and `storage_state`/`restore_storage_state`.
- **Diagnostics**: console messages, JS errors, navigations, network requests,
  dialogs, browser version and launch flags, plus HAR 1.2 export
  (`Page::har` / `har_with_bodies`).
- **WebDriver BiDi**: Firefox driver over BiDi with locators, frames,
  interception, network monitoring, cookies/storage, isolated user contexts, and
  init scripts (`add_init_script`). Downloads remain CDP-only because Firefox has
  no BiDi download command.
- **Unified API**: `PageApi`/`LocatorApi` implemented by both backends, with
  `AnyPage`/`AnyLocator` for dynamic dispatch.
- **Test runner**: `#[rustwright_test]` with per-test browser/context/page,
  `RUSTWRIGHT_BROWSER=firefox` to run the same tests on Firefox,
  `RUSTWRIGHT_RETRIES=N`, `RUSTWRIGHT_SHARD=i/N`, and `expect(locator)`
  assertions (`to_be_visible` / `to_have_text` / `to_contain_text` /
  `to_have_count` / ...).
- **CI**: GitHub Actions running `fmt`, `clippy -D warnings` and the full test
  suite against installed Chrome (Firefox best-effort).
- **Benchmarks**: reproducible Rustwright vs Playwright driver-overhead harness.

[0.1.0]: https://github.com/rsasaki0109/rustwright/releases/tag/v0.1.0
