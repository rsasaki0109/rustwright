# Rustwright — Architecture & Design

> Real-browser-first browser automation for Rust.

This document is the output of the pre-implementation phase requested for Rustwright:
prior-art survey, architecture investigation, required CDP domains, crate layout,
public API proposal and the MVP plan.

## 1. Prior art survey (existing Rust browser automation)

| Project | Transport | Async | Installed Chrome | Persistent profile / existing session | Notes |
|---|---|---|---|---|---|
| `headless_chrome` | CDP over WebSocket | sync (blocking) | yes, can download | partial | Puppeteer-like, sync API, large generated protocol |
| `chromiumoxide` | CDP over WebSocket | async (tokio/async-std) | yes, can download | yes (`connect`) | Generated protocol from PDL, one big `Handler` event stream |
| `fantoccini` | WebDriver (HTTP) | async | needs driver | via capabilities | WebDriver, not CDP |
| `thirtyfour` | WebDriver | async | needs driver | via capabilities | Selenium-like |
| `rust-headless-chrome` | CDP | sync | yes | partial | predecessor of `headless_chrome` |
| `puppeteer-rs` | CDP | async | yes | ? | higher level, less active |

Common issues Rustwright aims to improve on:

- Sync APIs (`headless_chrome`) block the runtime.
- Event handling is either a single monolithic stream (`chromiumoxide Handler`) or
  awkward listeners; there is no first-class, typed `wait_for` primitive.
- Protocol is fully code-generated (large compile times) and the generated types
  leak into the user-facing API.
- Installed-Chrome discovery is treated as a fallback rather than the primary path.

Playwright / Puppeteer / Selenium architectural lessons:

- **Playwright**: process-per-context, CDP (Chromium) + Juggler (Firefox) + WebDriver
  (WebKit); strong `Browser / BrowserContext / Page / Locator` separation; every
  action is auto-waiting; locators are lazy (re-resolved on every action).
- **Puppeteer**: thin CDP client, `Browser` + `Page`, explicit `waitForSelector`.
- **Selenium/WebDriver**: wire protocol, driver process, less control over the
  browser, but best cross-browser story.

Rustwright takes: Playwright's object model and auto-waiting, Puppeteer's thin CDP
flexibility, plus a Rust-native typed API and a diagnostics-first mindset.

## 2. Required CDP domains (MVP)

| Domain | Commands / events used | Purpose |
|---|---|---|
| `Target` | `createTarget`, `attachToTarget(flatten)`, `closeTarget`, `getTargets`, `setDiscoverTargets`, `createBrowserContext`, `disposeBrowserContext`, `attachedToTarget`, `detachedFromTarget`, `targetCreated` | tab & context lifecycle |
| `Browser` | `close`, `getVersion` | shutdown, diagnostics |
| `Page` | `enable`, `navigate`, `reload`, `captureScreenshot`, `getFrameTree`, `setLifecycleEventsEnabled`, `addScriptToEvaluateOnNewDocument`, `lifecycleEvent`, `frameNavigated`, `loadEventFired`, `domContentEventFired`, `javascriptDialog*` | navigation, lifecycle, screenshot, dialog diagnostics |
| `Runtime` | `enable`, `evaluate`, `callFunctionOn`, `consoleAPICalled`, `exceptionThrown` | DOM querying via JS, semantic locators, console/error diagnostics |
| `DOM` | `enable`, `getDocument`, `getBoxModel`, `focus`, `getOuterHTML` | geometry for real input events, HTML serialization |
| `Network` | `enable`, `requestWillBeSent`, `responseReceived`, `loadingFinished`, `loadingFailed` | network diagnostics + `networkIdle` corroboration |
| `Input` | `dispatchMouseEvent`, `dispatchKeyEvent`, `insertText` | real-user-like interaction |

Future domains: `Fetch`/`Network.setRequestInterception` (interception), `Page.setDownloadBehavior`
(downloads), `Page.setInterceptFileChooserDialog` (uploads), `Target.*` for frames/workers,
`Tracing`, and WebDriver BiDi.

## 3. Real-browser-first decisions

- **Installed Chrome is the primary path.** `Chrome::installed()` searches OS-specific
  install locations and `PATH`; `Chrome::at(path)` pins an explicit binary. No bundled
  browser, no downloader in the MVP.
- **Ephemeral profile by default, persistent on request.** Chrome 136+ ignores
  `--remote-debugging-port` when started with the *default* profile directory, so
  Rustwright always passes an explicit `--user-data-dir`: a temp dir for ephemeral
  sessions, or the user-supplied directory for `.profile(path)`.
- **Empty launch flags.** We only pass flags needed to make automation reliable
  (`--remote-debugging-port=0`, `--user-data-dir`, `--no-first-run`,
  `--no-default-browser-check`, `--headless=new` when headless). No fingerprint or
  detection-evasion flags. Users can add flags via `.arg()`.
- **Connect to a running browser** with `Browser::connect(endpoint)` for an existing
  Chrome started with `--remote-debugging-port`.
- **No core hacks for specific sites.** Site-specific behaviour belongs in user code.
- **Diagnostics as a feature.** Console messages, JS errors, navigations, network
  requests, browser version, executable path and launch args are observable.

### Non-goals (explicit)

CAPTCHA solving, bot-protection bypass, fingerprint spoofing, hiding
`navigator.webdriver`, proxy rotation, rate-limit evasion, site-specific bypasses.
Rustwright is *not* an anti-bot framework; it minimizes the environment delta versus a
normal browser through honest, standard automation.

## 4. Crate layout

```
rustwright/
├── Cargo.toml                  # workspace
├── crates/
│   ├── rustwright/             # user-facing facade + prelude (re-exports)
│   ├── rustwright-core/        # Browser / BrowserContext / Page / Locator / wait
│   ├── rustwright-common/      # backend-agnostic Selector/WaitState + injected helper
│   ├── rustwright-cdp/         # CDP transport, routing, sessions, typed protocol
│   ├── rustwright-browser/     # Chrome + Firefox discovery, launch, profile, lifecycle
│   ├── rustwright-bidi/        # WebDriver BiDi client + Firefox driver
│   ├── rustwright-test/        # independent browser test runner
│   └── rustwright-test-macros/ # #[rustwright_test] proc macro
├── examples/
├── tests/
└── docs/DESIGN.md
```

Dependency direction (no cycles):

```
rustwright -> rustwright-core -> rustwright-cdp
                             \-> rustwright-browser -> rustwright-cdp
rustwright -> rustwright-bidi -> rustwright-browser
rustwright-test -> rustwright
rustwright-test -> rustwright-test-macros
```

The test runner lives in its own crate so it can evolve independently of core,
as required by the original brief.

## 5. Transport & session model

- One WebSocket per browser, to the **browser** endpoint (`/devtools/browser/<id>`).
- Flat session multiplexing: `Target.attachToTarget { flatten: true }` returns a
  `sessionId`; all traffic carries it. One socket, many logical sessions.
- A reader task routes responses to per-request `oneshot` channels by `id`, and
  broadcasts events on a `tokio::sync::broadcast` channel as
  `CdpEvent { session_id, method, params }`.
- A writer task serializes outgoing frames. No `unsafe`, no blocking.
- HTTP discovery (`/json/version`, `/json/list`) uses a small hand-written HTTP/1.1
  GET over `TcpStream` so the dependency graph stays tiny and TLS-free (localhost only).

## 6. Object model

- `Browser` — owns the CDP connection, the launched process (if any), browser version
  and the default `BrowserContext`. `launch` / `connect` / `new_page` / `new_context`
  `pages` / `version` / `close`.
- `BrowserContext` — a CDP browser context (`Target.createBrowserContext`, i.e. an
  isolated incognito-like profile) or the default context. Owns its pages.
- `Page` — a CDP target session. Navigation, content, screenshot, evaluation,
  locators, waiting, console/error/network diagnostics.
- `Locator` — lazy, re-resolved on every action (Playwright semantics). CSS, text and
  ARIA-role strategies. Auto-waits before acting.
- `Role` — semantic locator roles.

## 7. Waiting model

No fixed sleeps in the public API.

- `page.wait_for_load_state(LoadState::{Load, DomContentLoaded, NetworkIdle})` listens
  for CDP `Page.lifecycleEvent` (Chrome emits `init`, `DOMContentLoaded`, `load`,
  `networkIdle`). `goto`/`reload` arm the subscription *before* navigating to avoid
  races.
- `locator.wait_for(WaitState::{Attached, Detached, Visible, Hidden})` evaluates a JS
  promise injected into the page that resolves via `MutationObserver` + `rAF`, so the
  wait lives next to the DOM it observes instead of poll-looping from Rust.
- Every operation has a caller-overridable timeout.

## 8. Public API (MVP)

```rust
use rustwright::prelude::*;

let browser = Browser::launch(Chrome::installed().headless(false)).await?;
let page = browser.new_page().await?;
page.goto("https://example.com").await?;
println!("{}", page.title().await?);

let q = page.locator("input[name=q]");
q.fill("hello").await?;
q.wait_for(WaitState::Visible).await?;
q.click().await?;

page.get_by_text("Login").click().await?;
page.get_by_role(Role::Button, Some("Submit")).click().await?;

page.screenshot("page.png").await?;
page.wait_for_load_state(LoadState::NetworkIdle).await?;

browser.close().await?;
```

Persistent profile and existing session:

```rust
let browser = Browser::launch(Chrome::installed().profile("./profile").headless(false)).await?;
let browser = Browser::connect("http://127.0.0.1:9222").await?;
```

## 9. MVP vertical slice

`Chrome launch -> CDP connect -> new page -> goto -> DOM query -> locator ->
click/fill -> wait -> screenshot`, testable against the locally installed Chrome.

Implementation order:

1. `rustwright-cdp`: WebSocket transport, request routing, sessions, events, typed protocol.
2. `rustwright-browser`: discovery, launch, profile, `DevToolsActivePort`, lifecycle,
   version diagnostics.
3. `rustwright-core`: Browser/Context/Page/Locator/wait/diagnostics.
4. `rustwright`: facade + prelude.
5. Examples, integration tests, quality gates (`fmt`, `clippy`, `test`).

Post-MVP roadmap: BrowserContext isolation polish → network interception → downloads/
uploads → dialogs → frames → tabs/windows → tracing → WebDriver BiDi → Firefox → test runner.

## 11. Round 2: real-site compatibility capabilities

To browse real SPA/EC/SNS sites without site-specific hacks, the following generic
capabilities were added:

- **Viewport / device metrics** (`Emulation.setDeviceMetricsOverride`) so SPA layout is
  deterministic across machines.
- **Locator collections** (`first` / `last` / `nth` / `all`) and extra strategies
  (`placeholder`, `label`, `alt`, `testid`), resolved by the injected script so they work
  in the page's own realm.
- **Input**: hover, real wheel events (`mouse_wheel`), deterministic `scroll_by`,
  `check` / `uncheck` / `select_option`, and a key map for `press`.
- **Dialogs**: `Page.javascriptDialogOpening` is auto-dismissed (Playwright's default) and
  recorded, so a stray `alert()` can never hang automation.
- **Navigation**: `wait_for_url`, `go_back` / `go_forward` via `Page.getNavigationHistory`.
- **Popups / tabs**: `BrowserContext::wait_for_page` observes `Target.targetCreated`
  (discovery enabled at launch/connect) and attaches the new page.
- **Reusable login state**: `Page::storage_state` / `restore_storage_state` for cookies and
  the current origin's `localStorage`.
- **Persistent-profile correctness**: stale `DevToolsActivePort` files are removed before
  launch, so re-launching against an existing profile connects to the new process.

Compatibility harness (`examples/compat_report.rs`) loads real sites and prints final URL,
navigation chain, console/JS errors, failed requests, non-2xx document responses and a
screenshot. Observed with installed Chrome 150, without any bypass logic:

| Site | Headless (fresh) | Headed + persistent |
|---|---|---|
| Mercari | loads | loads |
| Rakuma | loads | loads |
| X | 403 | loads |
| YouTube | loads | loads |
| Instagram | loads | loads |
| TikTok | 403 | 403 (server-side challenge) |

X succeeding only in a headed, persistent browser validates the real-browser-first
premise. TikTok's 403 is a server-side anti-bot response and is deliberately **not**
circumvented; sites that gate anonymous access are handled by logging in by hand once via
`examples/browse_session.rs` and reusing the profile.

Deliberately still deferred: tracing, WebDriver BiDi, Firefox and the independent
test runner.

## 12. Round 3: interception, files and frames

- **Network interception** (`Fetch` domain): declarative `Route` rules with `*`/`?`
  wildcards (`Page::route` / `mock` / `block` / `clear_routes`). `Fetch.enable` is
  toggled on the first rule, and the page event pump answers `Fetch.requestPaused`
  with continue/fulfill/fail. Rules are data, so the interceptor stays `Send + Sync`
  and contains no callbacks or site-specific logic.
- **Uploads** (`DOM.setFileInputFiles`): `Locator::set_input_files` assigns real file
  paths to an `<input type=file>`.
- **Downloads** (`Browser.setDownloadBehavior`):
  `BrowserContext::set_download_path` redirects downloads to a directory with progress
  events enabled.
- **Frames**: the page tracks `Page.getFrameTree`, `Page.frameAttached/Detached`,
  `Page.frameNavigated` and `Runtime.executionContextCreated/Destroyed`. Each frame's
  main-world execution context id is recorded, so `Page::frames`, `Page::main_frame`
  and `Page::frame_locator(iframe)` produce frame-scoped `Locator`s that evaluate in
  the right context. This makes **cross-origin iframes** work without `contentDocument`
  access hacks: `DOM.describeNode` yields the element's `frameId`, and the injected
  helper runs in every frame's realm.

Added tests (`tests/advanced.rs`): mocked and aborted requests, file upload, download
to a configured path, and filling/reading a value inside a cross-origin `data:` iframe.

Remaining roadmap: WebDriver BiDi and Firefox.

## 13. Round 4: tracing and the test runner

- **Tracing** (`Tracing` domain): `Page::start_tracing` / `start_tracing_with` /
  `stop_tracing`. Trace data arrives over `Tracing.dataCollected`, completion is
  signalled by `Tracing.tracingComplete`, and the assembled `{"traceEvents": [...]}`
  JSON is written to disk. Default categories cover timeline/V8/blink/loading and
  optionally periodic screenshots.
- **Test runner** (`rustwright-test` + `rustwright-test-macros`): a `#[rustwright_test]`
  attribute turns an `async fn(TestContext) -> Result<()>` into a normal `#[test]`.
  Each test gets a fresh browser (headless by default; `RUSTWRIGHT_HEADLESS=0` for
  headed), an isolated `BrowserContext`, and a `Page`, all torn down afterwards.
  Tests skip cleanly when no browser is installed. The runner depends on the facade,
  never on core internals, so it can grow reporting/retry/sharding independently.
  To support this, `Browser` became cheaply cloneable: the process handle is held in
  an `Arc<Mutex<..>>` shared by all clones, and only the first `close` takes it.

Remaining roadmap: WebDriver BiDi and Firefox support.

## 14. Round 5: WebDriver BiDi and Firefox

BiDi is a different wire protocol from CDP, so it gets its own transport rather
than being shoehorned into `rustwright-core`'s CDP assumptions:

- `rustwright-bidi::BidiConnection` — WebDriver BiDi over one WebSocket, with
  id-based request routing and a broadcast event stream, mirroring the CDP
  transport's design.
- `rustwright-bidi::BidiSession` — typed helpers: `session.new` / `subscribe` /
  `status`, `browsingContext.getTree` / `create` / `close` / `navigate` /
  `captureScreenshot` / `setViewport`, and `script.evaluate` with a
  `RemoteValue` -> JSON converter.
- `rustwright-browser::Firefox` + `LaunchedFirefox` — discovery on Linux/macOS/
  Windows, launch with `--remote-debugging-port=<free port>`, wait for the port,
  process lifecycle.
- `BidiBrowser` / `BidiPage` — launch/connect, tabs, navigation, title/content,
  evaluate, screenshot, viewport, close.

Verification on this machine (Firefox 156, CDP-independent):

- `BidiBrowser::launch(Firefox::installed().headless(true))`, `new_page`, `goto`,
  `title`, `evaluate`, `content`, `screenshot`, `close` all pass
  (`tests/bidi.rs`), and `examples/bidi_firefox.rs` drives example.com headed.

Findings:

- Chrome 150 does not expose a BiDi `/session` endpoint (404), so Chrome remains
  CDP-only; `BidiBrowser::connect` accepts a BiDi endpoint when a build provides
  one.
- Snap Firefox cannot use arbitrary `--profile` paths or `file://` URLs outside
  its sandbox; the driver defaults to the browser's own profile.

All ten roadmap items are now implemented. Natural next steps: a locator and
auto-wait layer over BiDi, richer network events, and a unified `Page` trait
shared by the CDP and BiDi backends.

## 15. Round 6: shared types and BiDi locators

The BiDi backend needed the same selector semantics and page-side helper as the
CDP backend, so protocol-agnostic pieces were extracted:

- **`rustwright-common`** holds `Selector`, `Role`, `LoadState`, `WaitState` and
  the `INJECTED_SCRIPT` page helper. `rustwright-core` re-exports them (public
  API unchanged) and `rustwright-bidi` depends on them directly.
- **`BidiLocator`** mirrors the CDP `Locator`: lazy resolution, auto-wait,
  `first`/`last`/`nth`/`all`, `get_by_text`/`role`/`placeholder`/`label`/
  `alt_text`/`test_id`, and `click`/`fill`/`text`/`is_visible`/`count`/
  `get_attribute`/`select_option`/`check`/`press`.
- Interaction uses **real input**: clicks and hovers go through
  `input.performActions` pointer actions at the element's box center (matching
  the CDP backend's `Input.dispatchMouseEvent`), and keys use BiDi key actions
  with Unicode PUA code points for special keys.
- The helper script is installed after every navigation (and lazily on first
  use) because a new document discards it.

Verified on Firefox 156 (`tests/bidi.rs`): fill an input, click a role locator,
`wait_for` visible text, `count`, and hidden-element checks all pass over BiDi.

The shared crate is the foundation for a future unified `Page` trait; the two
backends now agree on selectors, wait states and the page helper, leaving the
transport as the only thing that differs.

## 16. Round 7: a unified Page/Locator API

`rustwright-common` now defines `PageApi` and `LocatorApi`:

- `PageApi` — `goto`, `url`, `title`, `content`, `evaluate`, `screenshot`,
  `set_viewport`, `close`, `locator`, plus default `get_by_*` helpers.
- `LocatorApi` — `click`, `fill`, `text`, `text_content`, `is_visible`,
  `is_hidden`, `is_enabled`, `count`, `get_attribute`, `hover`,
  `scroll_into_view_if_needed`, `wait_for(_with_timeout)`, `first`/`last`/`nth`.

Design choices:

- **Generic over an associated `Error`** (`type Error: Error + Send + Sync`), so
  each backend keeps its own error type and no lossy conversion layer is needed.
  A consequence is static dispatch: use `P: PageApi` bounds rather than
  `Box<dyn PageApi>`.
- **No mapping/adaptor crate**: the impls live in `rustwright-core` (CDP) and
  `rustwright-bidi` (BiDi); `rustwright-common` stays dependency-light.

Proof (`tests/unified.rs`): the same generic helpers `fill_and_read` and
`click_and_wait` pass against Chrome (CDP) and Firefox (BiDi) unchanged. This is
the milestone the whole layered design was aiming for — one Playwright-like API,
two real-browser transports.

Possible future work: object-safe wrappers for dynamic backends, BiDi `network.*`
events, and a Firefox-native profile strategy that respects Snap confinement.

## 17. Round 8: dynamic dispatch, BiDi network, sandboxed Firefox

The three items left after the unified API:

- **Dynamic dispatch**: `AnyPage` / `AnyLocator` in the facade wrap either
  backend in an enum and implement `PageApi` / `LocatorApi`, with `AnyError`
  unifying the two error types. This gives a single concrete type (usable in a
  `Vec`, no generics) while keeping the static-dispatch traits. Verified by
  `dynamic_backends_in_one_vec`, which runs one generic helper over Chrome and
  Firefox pages stored together.
- **BiDi network diagnostics**: `BidiPage::start_network_monitoring` subscribes
  once per session to `network.beforeRequestSent` / `responseCompleted` /
  `fetchError` and spawns a context-filtered pump feeding
  `BidiPage::network_requests`, mirroring the CDP backend's request list.
  Verified against a local HTTP server in `tests/bidi.rs`.
- **Sandboxed Firefox**: `is_sandboxed_launcher` detects Snap-style wrapper
  scripts (and `$SNAP`). When a custom `--profile` is requested on such a build,
  launch logs a warning and falls back to the browser's default profile instead
  of failing; `Firefox::is_sandboxed()` exposes the detection. Covered by a unit
  test with a synthetic wrapper script.

## 18. Quality bar

- No `unsafe` (forbid in every crate).
- rustdoc on all public items.
- Typed, non-swallowed errors that preserve the causal chain
  (`Error` -> `CdpError`/`BrowserError` with protocol `code`/`message`/`data`).
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`.
- Runnable examples and integration tests against real installed Chrome.
