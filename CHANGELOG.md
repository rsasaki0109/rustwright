# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
  response-header modification (CDP), network diagnostics.
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
