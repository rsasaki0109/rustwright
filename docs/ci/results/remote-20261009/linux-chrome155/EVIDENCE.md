# Chrome 155 cross-platform navigation investigation

Authoritative Chrome-for-Testing 155.0.8059.39 Linux was downloaded through the configured proxy with default CA validation. `selected-version.json` records the published download URL/revision; `download.json` records the archive digest. The browser engine is unmodified. The wrapper only applies this container's existing `--no-sandbox --disable-dev-shm-usage` launch adjustment. No network policy was bypassed.

Unchanged Rustwright and HTTP fixture code passes all 23 Linux Chrome cases: compatibility 12, actionability 6, clipped controls 5. The separate exact navigation/forms test also passes and is not counted twice. Commands are Cargo `test --offline --locked -p rustwright-integration-tests --test <target> chrome_ -- --test-threads=1 --nocapture`, with `RUSTWRIGHT_CHROME=/workspace/.rustwright-env/chrome155/chromium`.

The initial remote Windows CI report separately failed 11 compatibility cases with `Navigation("net::ERR_ABORTED")`; Firefox passed 12 cases and Chrome disconnect passed one. The Linux results establish that this is not a universal Chrome 155 navigation failure. Windows cause remains undetermined, pending original network request snapshots and full raw CDP/navigation/browser stderr diagnostics. No retry, version pinning or error suppression has been added.

## Failure-only diagnostic instrumentation

Added only `tests/http_compat.rs` logging/error invocation and `tests/support/chrome_navigation_probe.rs`. All diagnostic async work, including endpoint discovery and WebSocket connect, is bounded by five seconds. The first error still panics; later raw navigation is diagnostic and cannot make the test pass. Session-scoped events, original request snapshot, full raw navigation response, browser stderr (16 KiB cap), and HTTP fixture request/write observations are logged.

Instrumented Linux Chrome155 compatibility remains 12/12. Scoped Clippy with warnings denied and Rust 1.85 compilation pass. An isolated HTTP attachment fixture deliberately causes `ERR_ABORTED`, demonstrates `isDownload=true` capture plus Network.loadingFailed, and verifies that the original result remains an error. This probe validates diagnostics, not the as-yet unknown Windows cause. Exact instrumentation source copies and digests are recorded separately from unchanged baseline source digests.
