//! The [`Page`] abstraction: navigation, content, interaction and waiting.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use rustwright_cdp::protocol::dom::{
    DescribeNodeParams, DescribeNodeResult, GetBoxModelParams, GetBoxModelResult,
    SetFileInputFilesParams,
};
use rustwright_cdp::protocol::emulation::SetDeviceMetricsOverrideParams;
use rustwright_cdp::protocol::fetch::{
    ContinueRequestParams, EnableParams as FetchEnableParams, FailRequestParams,
    FulfillRequestParams, HeaderEntry, RequestPattern, RequestPausedParams,
};
use rustwright_cdp::protocol::input::{
    DispatchKeyEventParams, DispatchMouseEventParams, InsertTextParams,
};
use rustwright_cdp::protocol::network::{
    Cookie, GetCookiesResult, LoadingFailedParams, RequestWillBeSentParams, ResponseReceivedParams,
    SetCookieParams, SetCookieResult,
};
use rustwright_cdp::protocol::page::{
    CaptureScreenshotParams, CaptureScreenshotResult, FrameAttachedParams, FrameDetachedParams,
    FrameNavigatedParams, GetFrameTreeResult, GetNavigationHistoryResult,
    HandleJavaScriptDialogParams, JavascriptDialogOpeningParams, LifecycleEventParams,
    NavigateParams, NavigateResult, NavigateToHistoryEntryParams, ReloadParams,
};
use rustwright_cdp::protocol::runtime::{
    CallArgument, CallFunctionOnParams, ConsoleAPICalledParams, EvaluateParams, EvaluateResult,
    ExceptionThrownParams, ExecutionContextCreatedParams, ExecutionContextDestroyedParams,
    RemoteObject,
};
use rustwright_cdp::protocol::tracing::{
    DataCollectedParams as TracingDataCollectedParams, StartParams as TracingStartParams,
    TracingCompleteParams,
};
use rustwright_cdp::{CdpConnection, CdpSession};
use rustwright_common::{LoadState, Role, Selector, WaitState, INJECTED_SCRIPT};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::diagnostics::{
    ConsoleMessage, DialogInfo, NavigationEvent, NetworkRequest, OriginStorage, PageError,
    StorageItem, StorageState,
};
use crate::error::{Error, Result};
use crate::frame::{Frame, FrameLocator};
use crate::locator::Locator;
use crate::route::{Route, RouteAction};

/// The default timeout for navigation, waits and actions.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A viewport size used for consistent rendering across environments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Width in CSS pixels.
    pub width: i64,
    /// Height in CSS pixels.
    pub height: i64,
    /// Device pixel ratio.
    pub device_scale_factor: f64,
    /// Whether to emulate a mobile device.
    pub mobile: bool,
}

impl Viewport {
    /// A desktop viewport of `width` x `height`.
    pub fn new(width: i64, height: i64) -> Self {
        Self {
            width,
            height,
            device_scale_factor: 1.0,
            mobile: false,
        }
    }

    /// A mobile viewport with a device pixel ratio.
    pub fn mobile(width: i64, height: i64, device_scale_factor: f64) -> Self {
        Self {
            width,
            height,
            device_scale_factor,
            mobile: true,
        }
    }

    /// The common 1280x720 desktop viewport.
    pub fn hd() -> Self {
        Self::new(1280, 720)
    }

    /// The common 1920x1080 desktop viewport.
    pub fn full_hd() -> Self {
        Self::new(1920, 1080)
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::hd()
    }
}

/// Options for Chrome trace collection.
#[derive(Debug, Clone)]
pub struct TracingOptions {
    /// Comma-separated trace categories.
    pub categories: String,
    /// Include periodic screenshots in the trace.
    pub screenshots: bool,
}

impl Default for TracingOptions {
    fn default() -> Self {
        Self {
            categories: DEFAULT_TRACE_CATEGORIES.to_string(),
            screenshots: true,
        }
    }
}

impl TracingOptions {
    /// The effective category string, including screenshots when enabled.
    pub fn category_string(&self) -> String {
        if self.screenshots
            && !self
                .categories
                .contains("disabled-by-default-devtools.screenshot")
        {
            format!(
                "{},disabled-by-default-devtools.screenshot",
                self.categories
            )
        } else {
            self.categories.clone()
        }
    }
}

const DEFAULT_TRACE_CATEGORIES: &str =
    "-*,devtools.timeline,v8,blink,blink.user_timing,loading,disabled-by-default-devtools.timeline";

#[derive(Default)]
struct TraceState {
    recording: bool,
    chunks: Vec<Vec<Value>>,
    complete_tx: Option<oneshot::Sender<()>>,
}

/// Information tracked for a frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct FrameInfo {
    pub(crate) parent_id: Option<String>,
    pub(crate) url: String,
    pub(crate) name: String,
    pub(crate) execution_context_id: Option<i64>,
}

/// Mutable page state shared between clones and the event pump.
pub(crate) struct PageState {
    lifecycle: Mutex<HashSet<String>>,
    lifecycle_tx: broadcast::Sender<String>,
    url_tx: broadcast::Sender<String>,
    console: Mutex<Vec<ConsoleMessage>>,
    errors: Mutex<Vec<PageError>>,
    requests: Mutex<Vec<NetworkRequest>>,
    navigations: Mutex<Vec<NavigationEvent>>,
    dialogs: Mutex<Vec<DialogInfo>>,
    frames: Mutex<HashMap<String, FrameInfo>>,
    main_frame_id: Mutex<Option<String>>,
    routes: Mutex<Vec<Route>>,
    trace: Mutex<TraceState>,
    url: Mutex<String>,
    closed: AtomicBool,
}

impl PageState {
    fn has_lifecycle(&self, name: &str) -> bool {
        self.lifecycle
            .lock()
            .expect("lifecycle mutex poisoned")
            .contains(name)
    }

    fn reset_lifecycle(&self) {
        self.lifecycle
            .lock()
            .expect("lifecycle mutex poisoned")
            .clear();
    }

    fn mark_lifecycle(&self, name: &str) {
        self.lifecycle
            .lock()
            .expect("lifecycle mutex poisoned")
            .insert(name.to_string());
        let _ = self.lifecycle_tx.send(name.to_string());
    }

    fn set_url(&self, url: &str) {
        *self.url.lock().expect("url mutex poisoned") = url.to_string();
        let _ = self.url_tx.send(url.to_string());
    }

    fn url(&self) -> String {
        self.url.lock().expect("url mutex poisoned").clone()
    }
}

/// A single browser page (tab).
///
/// Cloning a `Page` is cheap; all clones share the same CDP session and state.
#[derive(Clone)]
pub struct Page {
    session: CdpSession,
    connection: CdpConnection,
    state: Arc<PageState>,
    _pump: Arc<PumpGuard>,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("target_id", &self.session.target_id())
            .field("url", &self.state.url())
            .field("closed", &self.is_closed())
            .finish()
    }
}

struct PumpGuard {
    handle: JoinHandle<()>,
}

impl Drop for PumpGuard {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl Page {
    /// Attach to an already-created target and enable the required domains.
    pub(crate) async fn attach(
        connection: CdpConnection,
        session_id: String,
        target_id: String,
    ) -> Result<Self> {
        let session = CdpSession::new(connection.clone(), session_id, target_id);
        let (lifecycle_tx, _) = broadcast::channel(512);
        let (url_tx, _) = broadcast::channel(512);
        let state = Arc::new(PageState {
            lifecycle: Mutex::new(HashSet::new()),
            lifecycle_tx,
            url_tx,
            console: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            navigations: Mutex::new(Vec::new()),
            dialogs: Mutex::new(Vec::new()),
            frames: Mutex::new(HashMap::new()),
            main_frame_id: Mutex::new(None),
            routes: Mutex::new(Vec::new()),
            trace: Mutex::new(TraceState::default()),
            url: Mutex::new("about:blank".to_string()),
            closed: AtomicBool::new(false),
        });

        let pump = Arc::new(PumpGuard {
            handle: spawn_event_pump(connection.clone(), session.clone(), state.clone()),
        });

        let page = Self {
            session,
            connection,
            state,
            _pump: pump,
        };

        // Enable Page first and seed the frame tree before Runtime, so the
        // execution-context events that Runtime.enable emits are not lost.
        page.session.send("Page.enable", json!({})).await?;
        page.seed_frames().await?;
        for method in ["Runtime.enable", "DOM.enable", "Network.enable"] {
            page.session.send(method, json!({})).await?;
        }
        page.session
            .send("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
            .await?;
        page.session
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": INJECTED_SCRIPT }),
            )
            .await?;
        let _ = page
            .session
            .send("Runtime.evaluate", json!({ "expression": INJECTED_SCRIPT }))
            .await;
        Ok(page)
    }

    /// The CDP target id of this page.
    pub fn target_id(&self) -> &str {
        self.session.target_id()
    }

    /// Whether this page has been closed.
    pub fn is_closed(&self) -> bool {
        self.state.closed.load(Ordering::SeqCst) || self.connection.is_closed()
    }

    // -- Navigation ---------------------------------------------------------

    /// Navigate to `url` and wait for the `load` lifecycle state.
    ///
    /// A missing scheme defaults to `https://`. `data:`, `about:`, `file:` and
    /// other explicit schemes are used as-is.
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.goto_with_load_state(url, LoadState::Load).await
    }

    /// Navigate and wait for an explicit load state.
    pub async fn goto_with_load_state(&self, url: &str, state: LoadState) -> Result<()> {
        self.ensure_open()?;
        let normalized = normalize_url(url);
        self.state.reset_lifecycle();
        let receiver = self.state.lifecycle_tx.subscribe();
        let params = NavigateParams {
            url: normalized.clone(),
            referrer: None,
            frame_id: None,
        };
        let result: NavigateResult = self.call("Page.navigate", json!(params)).await?;
        if let Some(error) = result.error_text {
            if !error.is_empty() {
                return Err(Error::Navigation(error));
            }
        }
        self.state.set_url(&normalized);
        self.wait_lifecycle(receiver, state, DEFAULT_TIMEOUT).await
    }

    /// Navigate with an explicit timeout.
    pub async fn goto_with_timeout(&self, url: &str, timeout: Duration) -> Result<()> {
        self.ensure_open()?;
        let normalized = normalize_url(url);
        self.state.reset_lifecycle();
        let receiver = self.state.lifecycle_tx.subscribe();
        let params = NavigateParams {
            url: normalized.clone(),
            referrer: None,
            frame_id: None,
        };
        let result: NavigateResult = self
            .call_with_timeout("Page.navigate", json!(params), timeout)
            .await?;
        if let Some(error) = result.error_text {
            if !error.is_empty() {
                return Err(Error::Navigation(error));
            }
        }
        self.state.set_url(&normalized);
        self.wait_lifecycle(receiver, LoadState::Load, timeout)
            .await
    }

    /// Reload the current page and wait for `load`.
    pub async fn reload(&self) -> Result<()> {
        self.ensure_open()?;
        self.state.reset_lifecycle();
        let receiver = self.state.lifecycle_tx.subscribe();
        self.call::<Value>("Page.reload", json!(ReloadParams::default()))
            .await?;
        self.wait_lifecycle(receiver, LoadState::Load, DEFAULT_TIMEOUT)
            .await
    }

    /// Navigate back in history and wait for `load`.
    pub async fn go_back(&self) -> Result<()> {
        self.navigate_history(-1).await
    }

    /// Navigate forward in history and wait for `load`.
    pub async fn go_forward(&self) -> Result<()> {
        self.navigate_history(1).await
    }

    async fn navigate_history(&self, delta: i64) -> Result<()> {
        self.ensure_open()?;
        let history: GetNavigationHistoryResult =
            self.call("Page.getNavigationHistory", json!({})).await?;
        let target = history.current_index + delta;
        if target < 0 || target as usize >= history.entries.len() {
            return Err(Error::Navigation(format!(
                "no history entry at offset {delta}"
            )));
        }
        let entry_id = history.entries[target as usize].id;
        self.state.reset_lifecycle();
        let receiver = self.state.lifecycle_tx.subscribe();
        self.call::<Value>(
            "Page.navigateToHistoryEntry",
            json!(NavigateToHistoryEntryParams { entry_id }),
        )
        .await?;
        self.wait_lifecycle(receiver, LoadState::Load, DEFAULT_TIMEOUT)
            .await
    }

    /// The current URL.
    pub fn url(&self) -> String {
        self.state.url()
    }

    /// The current document title.
    pub async fn title(&self) -> Result<String> {
        let value = self.evaluate("document.title").await?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    /// The serialized HTML of the page, including the doctype.
    pub async fn content(&self) -> Result<String> {
        let expression = "(() => { const dt = document.doctype; \
             const prefix = dt ? '<!DOCTYPE ' + dt.name + '>' : ''; \
             const root = document.documentElement; \
             return root ? prefix + root.outerHTML : ''; })()";
        let value = self.evaluate(expression).await?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    // -- Viewport -----------------------------------------------------------

    /// Override the viewport for consistent rendering.
    pub async fn set_viewport(&self, viewport: &Viewport) -> Result<()> {
        let params = SetDeviceMetricsOverrideParams {
            width: viewport.width,
            height: viewport.height,
            device_scale_factor: viewport.device_scale_factor,
            mobile: viewport.mobile,
        };
        self.call::<Value>("Emulation.setDeviceMetricsOverride", json!(params))
            .await?;
        Ok(())
    }

    /// Clear a previously set viewport override.
    pub async fn clear_viewport(&self) -> Result<()> {
        self.call::<Value>("Emulation.clearDeviceMetricsOverride", json!({}))
            .await?;
        Ok(())
    }

    // -- Screenshots --------------------------------------------------------

    /// Take a screenshot and write it to `path`.
    ///
    /// The image format is inferred from the file extension and defaults to PNG.
    pub async fn screenshot(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let format = screenshot_format(path);
        let bytes = self.screenshot_bytes(format).await?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    /// Take a screenshot and return the encoded bytes.
    pub async fn screenshot_bytes(&self, format: &str) -> Result<Vec<u8>> {
        let params = CaptureScreenshotParams {
            format: Some(format.to_string()),
            ..Default::default()
        };
        let result: CaptureScreenshotResult =
            self.call("Page.captureScreenshot", json!(params)).await?;
        decode_base64(&result.data)
    }

    /// Take a full-page screenshot and return the encoded bytes.
    pub async fn full_page_screenshot_bytes(&self, format: &str) -> Result<Vec<u8>> {
        let params = CaptureScreenshotParams {
            format: Some(format.to_string()),
            capture_beyond_viewport: Some(true),
            ..Default::default()
        };
        let result: CaptureScreenshotResult =
            self.call("Page.captureScreenshot", json!(params)).await?;
        decode_base64(&result.data)
    }

    /// Close this page (tab).
    pub async fn close(&self) -> Result<()> {
        if self.state.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self
            .connection
            .send_raw(
                None,
                "Target.closeTarget",
                json!({ "targetId": self.session.target_id() }),
            )
            .await;
        Ok(())
    }

    // -- Waiting ------------------------------------------------------------

    /// Wait for a page load state.
    pub async fn wait_for_load_state(&self, state: LoadState) -> Result<()> {
        self.wait_for_load_state_with_timeout(state, DEFAULT_TIMEOUT)
            .await
    }

    /// Wait for a page load state with an explicit timeout.
    pub async fn wait_for_load_state_with_timeout(
        &self,
        state: LoadState,
        timeout: Duration,
    ) -> Result<()> {
        self.ensure_open()?;
        let receiver = self.state.lifecycle_tx.subscribe();
        self.wait_lifecycle(receiver, state, timeout).await
    }

    /// Wait until the URL contains `pattern`.
    pub async fn wait_for_url(&self, pattern: &str) -> Result<()> {
        self.wait_for_url_with_timeout(pattern, DEFAULT_TIMEOUT)
            .await
    }

    /// Wait until the URL contains `pattern`, with an explicit timeout.
    pub async fn wait_for_url_with_timeout(&self, pattern: &str, timeout: Duration) -> Result<()> {
        self.ensure_open()?;
        if self.url().contains(pattern) {
            return Ok(());
        }
        let mut receiver = self.state.url_tx.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error(
                    format!("url to contain {pattern:?}"),
                    timeout,
                ));
            }
            match tokio::time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(url)) => {
                    if url.contains(pattern) {
                        return Ok(());
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    if self.url().contains(pattern) {
                        return Ok(());
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(Error::PageClosed),
                Err(_) => {
                    return Err(timeout_error(
                        format!("url to contain {pattern:?}"),
                        timeout,
                    ))
                }
            }
        }
    }

    /// Wait for a fixed duration. Prefer event-driven waits; this is an escape
    /// hatch for the rare case where no observable signal exists.
    pub async fn wait_for_timeout(&self, timeout: Duration) {
        tokio::time::sleep(timeout).await;
    }

    async fn wait_lifecycle(
        &self,
        mut receiver: broadcast::Receiver<String>,
        state: LoadState,
        timeout: Duration,
    ) -> Result<()> {
        let target = state.lifecycle_name();
        if self.state.has_lifecycle(target) {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error(
                    format!("load state `{}`", state.as_str()),
                    timeout,
                ));
            }
            match tokio::time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(name)) => {
                    if name == target {
                        return Ok(());
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    if self.state.has_lifecycle(target) {
                        return Ok(());
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(Error::PageClosed),
                Err(_) => {
                    return Err(timeout_error(
                        format!("load state `{}`", state.as_str()),
                        timeout,
                    ))
                }
            }
        }
    }

    // -- Input --------------------------------------------------------------

    /// Move the mouse to a point.
    pub async fn mouse_move(&self, x: f64, y: f64) -> Result<()> {
        self.dispatch_mouse("mouseMoved", x, y, "none", 0).await
    }

    /// Click at a point with the left button.
    pub async fn mouse_click(&self, x: f64, y: f64) -> Result<()> {
        self.mouse_move(x, y).await?;
        self.dispatch_mouse("mousePressed", x, y, "left", 1).await?;
        self.dispatch_mouse("mouseReleased", x, y, "left", 0).await
    }

    /// Dispatch a mouse wheel event at a point.
    ///
    /// Positive `delta_y` scrolls the page down, which is what infinite-scroll
    /// feeds need.
    pub async fn mouse_wheel(&self, x: f64, y: f64, delta_x: f64, delta_y: f64) -> Result<()> {
        let params = DispatchMouseEventParams {
            event_type: "mouseWheel".to_string(),
            x,
            y,
            button: Some("none".to_string()),
            buttons: Some(0),
            click_count: None,
            delta_x: Some(delta_x),
            delta_y: Some(delta_y),
        };
        self.call::<Value>("Input.dispatchMouseEvent", json!(params))
            .await?;
        Ok(())
    }

    /// Scroll the page by the given deltas.
    ///
    /// This is a deterministic convenience built on `window.scrollBy`; for real
    /// wheel input use [`Page::mouse_wheel`].
    pub async fn scroll_by(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let expression =
            format!("window.scrollBy({{ left: {delta_x}, top: {delta_y}, behavior: 'instant' }})");
        self.evaluate(&expression).await?;
        Ok(())
    }

    /// Type `text` into the page as if it were entered on the keyboard.
    pub async fn keyboard_insert_text(&self, text: &str) -> Result<()> {
        self.call::<Value>(
            "Input.insertText",
            json!(InsertTextParams {
                text: text.to_string()
            }),
        )
        .await?;
        Ok(())
    }

    /// Press a single key on the focused element.
    pub async fn press_key(&self, key: &str) -> Result<()> {
        let definition = key_definition(key);
        let event_type = if definition.text.is_some() {
            "keyDown"
        } else {
            "rawKeyDown"
        };
        self.dispatch_key(event_type, &definition, definition.text.as_deref())
            .await?;
        self.dispatch_key("keyUp", &definition, None).await
    }

    async fn dispatch_key(
        &self,
        event_type: &str,
        definition: &KeyDefinition,
        text: Option<&str>,
    ) -> Result<()> {
        let params = DispatchKeyEventParams {
            event_type: event_type.to_string(),
            key: Some(definition.key.clone()),
            code: Some(definition.code.clone()),
            text: text.map(str::to_string),
            windows_virtual_key_code: Some(definition.virtual_key_code),
            native_virtual_key_code: Some(definition.virtual_key_code),
            modifiers: None,
        };
        self.call::<Value>("Input.dispatchKeyEvent", json!(params))
            .await?;
        Ok(())
    }

    // -- Evaluation ---------------------------------------------------------

    /// Evaluate a JavaScript expression in the page and return its value.
    pub async fn evaluate(&self, expression: &str) -> Result<Value> {
        self.ensure_open()?;
        let params = EvaluateParams {
            expression: expression.to_string(),
            return_by_value: Some(true),
            await_promise: Some(true),
            context_id: None,
            user_gesture: Some(true),
        };
        let result: EvaluateResult = self.call("Runtime.evaluate", json!(params)).await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        Ok(result.result.value.unwrap_or(Value::Null))
    }

    pub(crate) async fn call_on(
        &self,
        object_id: &str,
        function: &str,
        arguments: Vec<CallArgument>,
        return_by_value: bool,
    ) -> Result<RemoteObject> {
        let params = CallFunctionOnParams {
            function_declaration: function.to_string(),
            object_id: Some(object_id.to_string()),
            arguments,
            return_by_value: Some(return_by_value),
            await_promise: Some(false),
            user_gesture: Some(true),
        };
        let result: EvaluateResult = self.call("Runtime.callFunctionOn", json!(params)).await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        Ok(result.result)
    }

    // -- Locators -----------------------------------------------------------

    /// Create a locator for a CSS selector.
    pub fn locator(&self, selector: impl Into<Selector>) -> Locator {
        Locator::new(self.clone(), selector.into())
    }

    /// Create a locator that matches elements by their text.
    pub fn get_by_text(&self, text: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::text(text, false))
    }

    /// Create a locator that matches elements by their exact text.
    pub fn get_by_text_exact(&self, text: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::text(text, true))
    }

    /// Create a semantic locator by ARIA role and optional accessible name.
    pub fn get_by_role(&self, role: Role, name: Option<&str>) -> Locator {
        Locator::new(self.clone(), Selector::role(role, name))
    }

    /// Create a locator by `placeholder` attribute.
    pub fn get_by_placeholder(&self, text: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::placeholder(text, false))
    }

    /// Create a locator by associated `<label>`.
    pub fn get_by_label(&self, text: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::label(text, false))
    }

    /// Create a locator by image `alt` text.
    pub fn get_by_alt_text(&self, text: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::alt_text(text, false))
    }

    /// Create a locator by test id (`data-testid`, `data-test-id`, `data-test`).
    pub fn get_by_test_id(&self, id: impl Into<String>) -> Locator {
        Locator::new(self.clone(), Selector::test_id(id))
    }

    // -- Diagnostics --------------------------------------------------------

    /// Console messages observed since the page was created.
    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.state
            .console
            .lock()
            .expect("console mutex poisoned")
            .clone()
    }

    /// Uncaught JavaScript errors observed since the page was created.
    pub fn errors(&self) -> Vec<PageError> {
        self.state
            .errors
            .lock()
            .expect("errors mutex poisoned")
            .clone()
    }

    /// Network requests observed since the page was created.
    pub fn network_requests(&self) -> Vec<NetworkRequest> {
        self.state
            .requests
            .lock()
            .expect("requests mutex poisoned")
            .clone()
    }

    /// Main-frame navigations observed since the page was created.
    pub fn navigations(&self) -> Vec<NavigationEvent> {
        self.state
            .navigations
            .lock()
            .expect("navigations mutex poisoned")
            .clone()
    }

    /// JavaScript dialogs observed since the page was created.
    ///
    /// Dialogs are auto-dismissed so that automation never hangs on an alert.
    pub fn dialogs(&self) -> Vec<DialogInfo> {
        self.state
            .dialogs
            .lock()
            .expect("dialogs mutex poisoned")
            .clone()
    }

    /// Cookies visible to the page.
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        self.ensure_open()?;
        let result: GetCookiesResult = self.call("Network.getCookies", json!({})).await?;
        Ok(result.cookies)
    }

    /// Set a cookie. `url` scopes the cookie to a page.
    pub async fn add_cookie(
        &self,
        name: impl Into<String>,
        value: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<bool> {
        self.set_cookie(SetCookieParams {
            name: name.into(),
            value: value.into(),
            url: Some(url.into()),
            domain: None,
            path: None,
            http_only: None,
            secure: None,
            same_site: None,
        })
        .await
    }

    /// Set a cookie with full control over its attributes.
    pub async fn set_cookie(&self, params: SetCookieParams) -> Result<bool> {
        self.ensure_open()?;
        let result: SetCookieResult = self.call("Network.setCookie", json!(params)).await?;
        Ok(result.success)
    }

    /// Remove all browser cookies.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.ensure_open()?;
        self.call::<Value>("Network.clearBrowserCookies", json!({}))
            .await?;
        Ok(())
    }

    // -- Storage state ------------------------------------------------------

    /// Capture cookies and the current origin's local storage.
    ///
    /// This allows an authenticated session to be reused later without copying
    /// a whole browser profile.
    pub async fn storage_state(&self) -> Result<StorageState> {
        self.ensure_open()?;
        let cookies = self.cookies().await?;
        let origin = self.origin().await?;
        let raw = self
            .evaluate(
                "(() => { try { return JSON.stringify(Object.entries(localStorage)); } \
                 catch (error) { return '[]'; } })()",
            )
            .await?;
        let entries: Vec<(String, String)> = serde_json::from_str(raw.as_str().unwrap_or("[]"))?;
        let local_storage = entries
            .into_iter()
            .map(|(name, value)| StorageItem { name, value })
            .collect();
        Ok(StorageState {
            cookies,
            origins: vec![OriginStorage {
                origin,
                local_storage,
            }],
        })
    }

    /// Restore cookies and local storage from a [`StorageState`].
    ///
    /// Local storage is only applied when the origin matches the current page.
    pub async fn restore_storage_state(&self, state: &StorageState) -> Result<()> {
        self.ensure_open()?;
        for cookie in &state.cookies {
            let params = SetCookieParams {
                name: cookie.name.clone(),
                value: cookie.value.clone(),
                url: None,
                domain: non_empty(&cookie.domain),
                path: non_empty(&cookie.path),
                http_only: Some(cookie.http_only),
                secure: Some(cookie.secure),
                same_site: cookie.same_site.clone(),
            };
            let _ = self.set_cookie(params).await;
        }
        let origin = self.origin().await?;
        for stored in &state.origins {
            if stored.origin != origin {
                continue;
            }
            for item in &stored.local_storage {
                let expression = format!(
                    "(() => {{ try {{ localStorage.setItem({}, {}); }} catch (error) {{}} }})()",
                    serde_json::to_string(&item.name)?,
                    serde_json::to_string(&item.value)?,
                );
                let _ = self.evaluate(&expression).await;
            }
        }
        Ok(())
    }

    async fn origin(&self) -> Result<String> {
        Ok(self
            .evaluate("location.origin")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    // -- Request interception ----------------------------------------------

    /// Add a request interception rule.
    ///
    /// `pattern` uses `*`/`?` wildcards; without wildcards it is a substring
    /// match. The first matching rule wins. Interception is generic: it can
    /// abort or fulfill requests, but implements no site-specific behaviour.
    pub async fn route(&self, pattern: impl Into<String>, action: RouteAction) -> Result<()> {
        self.ensure_open()?;
        let first = {
            let mut routes = self.state.routes.lock().expect("routes mutex poisoned");
            let first = routes.is_empty();
            routes.push(Route::new(pattern, action));
            first
        };
        if first {
            let params = FetchEnableParams {
                patterns: vec![RequestPattern {
                    url_pattern: Some("*".to_string()),
                    ..Default::default()
                }],
                handle_auth_requests: false,
            };
            self.session.send("Fetch.enable", json!(params)).await?;
        }
        Ok(())
    }

    /// Abort requests matching `pattern`.
    pub async fn block(&self, pattern: impl Into<String>) -> Result<()> {
        self.route(pattern, RouteAction::Abort).await
    }

    /// Answer requests matching `pattern` with a synthetic response.
    pub async fn mock(
        &self,
        pattern: impl Into<String>,
        status: i64,
        content_type: impl Into<String>,
        body: impl Into<Vec<u8>>,
    ) -> Result<()> {
        self.route(
            pattern,
            RouteAction::Fulfill {
                status,
                content_type: content_type.into(),
                body: body.into(),
            },
        )
        .await
    }

    /// Remove all interception rules.
    pub async fn clear_routes(&self) -> Result<()> {
        let had_routes = {
            let mut routes = self.state.routes.lock().expect("routes mutex poisoned");
            let had = !routes.is_empty();
            routes.clear();
            had
        };
        if had_routes {
            let _ = self.session.send("Fetch.disable", json!({})).await;
        }
        Ok(())
    }

    // -- Tracing ------------------------------------------------------------

    /// Whether a trace is currently being recorded.
    pub fn is_tracing(&self) -> bool {
        self.state
            .trace
            .lock()
            .expect("trace mutex poisoned")
            .recording
    }

    /// Start recording a Chrome trace with default categories.
    pub async fn start_tracing(&self) -> Result<()> {
        self.start_tracing_with(TracingOptions::default()).await
    }

    /// Start recording a Chrome trace.
    ///
    /// Screenshots are captured periodically when
    /// [`TracingOptions::screenshots`] is enabled.
    pub async fn start_tracing_with(&self, options: TracingOptions) -> Result<()> {
        self.ensure_open()?;
        {
            let mut trace = self.state.trace.lock().expect("trace mutex poisoned");
            if trace.recording {
                return Err(Error::JavaScript(
                    "tracing is already active on this page".to_string(),
                ));
            }
            trace.recording = true;
            trace.chunks.clear();
            trace.complete_tx = None;
        }
        let params = TracingStartParams {
            categories: Some(options.category_string()),
            transfer_mode: Some("ReportEvents".to_string()),
            ..Default::default()
        };
        if let Err(error) = self.session.send("Tracing.start", json!(params)).await {
            self.state
                .trace
                .lock()
                .expect("trace mutex poisoned")
                .recording = false;
            return Err(error.into());
        }
        Ok(())
    }

    /// Stop recording and write a Chrome trace to `path`.
    pub async fn stop_tracing(&self, path: impl AsRef<Path>) -> Result<()> {
        self.ensure_open()?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut trace = self.state.trace.lock().expect("trace mutex poisoned");
            if !trace.recording {
                return Err(Error::JavaScript(
                    "tracing is not active on this page".to_string(),
                ));
            }
            trace.complete_tx = Some(sender);
        }
        self.session.send("Tracing.end", json!({})).await?;
        let _ = tokio::time::timeout(Duration::from_secs(30), receiver).await;

        let chunks = {
            let mut trace = self.state.trace.lock().expect("trace mutex poisoned");
            trace.recording = false;
            trace.complete_tx = None;
            std::mem::take(&mut trace.chunks)
        };
        let events: Vec<Value> = chunks.into_iter().flatten().collect();
        let document = json!({ "traceEvents": events });

        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        tokio::fs::write(path, serde_json::to_vec(&document)?).await?;
        Ok(())
    }

    // -- Frames -------------------------------------------------------------

    /// The main frame of the page.
    pub fn main_frame(&self) -> Frame {
        let frame_id = self
            .state
            .main_frame_id
            .lock()
            .expect("main frame mutex poisoned")
            .clone()
            .unwrap_or_default();
        Frame::new(self.clone(), frame_id, true)
    }

    /// Every known frame, main frame first.
    pub fn frames(&self) -> Vec<Frame> {
        let main = self
            .state
            .main_frame_id
            .lock()
            .expect("main frame mutex poisoned")
            .clone();
        let mut frames: Vec<Frame> = self
            .state
            .frames
            .lock()
            .expect("frames mutex poisoned")
            .keys()
            .map(|id| {
                Frame::new(
                    self.clone(),
                    id.clone(),
                    main.as_deref() == Some(id.as_str()),
                )
            })
            .collect();
        frames.sort_by_key(|frame| !frame.is_main_frame());
        frames
    }

    /// A lazy locator scoped to the contents of an `<iframe>`.
    pub fn frame_locator(&self, selector: impl Into<Selector>) -> FrameLocator {
        FrameLocator::new(self.clone(), selector.into())
    }

    pub(crate) fn frame_info(&self, frame_id: &str) -> Option<FrameInfo> {
        self.state
            .frames
            .lock()
            .expect("frames mutex poisoned")
            .get(frame_id)
            .cloned()
    }

    async fn seed_frames(&self) -> Result<()> {
        let tree: GetFrameTreeResult = self.call("Page.getFrameTree", json!({})).await?;
        let mut frames = self.state.frames.lock().expect("frames mutex poisoned");
        frames.clear();
        let mut stack = vec![(tree.frame_tree, true)];
        while let Some((node, is_main)) = stack.pop() {
            if is_main {
                *self
                    .state
                    .main_frame_id
                    .lock()
                    .expect("main frame mutex poisoned") = Some(node.frame.id.clone());
            }
            frames.insert(
                node.frame.id.clone(),
                FrameInfo {
                    parent_id: node.frame.parent_id.clone(),
                    url: node.frame.url.clone(),
                    name: node.frame.name.clone(),
                    execution_context_id: None,
                },
            );
            for child in node.child_frames {
                stack.push((child, false));
            }
        }
        Ok(())
    }

    pub(crate) fn execution_context_id(&self, frame_id: Option<&str>) -> Option<i64> {
        match frame_id {
            None => None,
            Some(id) => self
                .state
                .frames
                .lock()
                .expect("frames mutex poisoned")
                .get(id)
                .and_then(|frame| frame.execution_context_id),
        }
    }

    pub(crate) async fn frame_id_for_element(&self, selector: &Selector) -> Result<String> {
        let spec = selector.to_spec();
        let object = self
            .resolve_object_spec(&spec, &selector.describe(), None)
            .await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let described: DescribeNodeResult = self
            .call(
                "DOM.describeNode",
                json!(DescribeNodeParams {
                    object_id: Some(object_id),
                    ..Default::default()
                }),
            )
            .await?;
        described
            .node
            .frame_id
            .ok_or_else(|| Error::ElementNotFound {
                selector: format!("{} (not a frame element)", selector.describe()),
            })
    }

    // -- Uploads ------------------------------------------------------------

    pub(crate) async fn set_input_files_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
        files: &[PathBuf],
    ) -> Result<()> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let params = SetFileInputFilesParams {
            files: files
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            object_id: Some(object_id),
            ..Default::default()
        };
        self.call::<Value>("DOM.setFileInputFiles", json!(params))
            .await?;
        Ok(())
    }

    // -- Internal helpers ---------------------------------------------------

    pub(crate) async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let value = self.session.send(method, params).await?;
        Ok(serde_json::from_value(value)?)
    }

    async fn call_with_timeout<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<T> {
        let value = self
            .session
            .send_with_timeout(method, params, timeout)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    pub(crate) fn ensure_open(&self) -> Result<()> {
        if self.state.closed.load(Ordering::SeqCst) {
            return Err(Error::PageClosed);
        }
        if self.connection.is_closed() {
            return Err(Error::BrowserClosed);
        }
        Ok(())
    }

    pub(crate) async fn resolve_object_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<RemoteObject> {
        let serialized = serde_json::to_string(spec)?;
        let expression =
            format!("(window.__rustwright ? window.__rustwright.resolve({serialized}) : null)");
        let object = self.evaluate_handle_in(&expression, frame_id).await?;
        if object.is_nullish() || object.object_id.is_none() {
            return Err(Error::ElementNotFound {
                selector: describe.to_string(),
            });
        }
        Ok(object)
    }

    /// Evaluate `expression` in the given frame (or the main frame).
    pub(crate) async fn evaluate_in(
        &self,
        expression: &str,
        frame_id: Option<&str>,
    ) -> Result<Value> {
        self.ensure_open()?;
        let params = EvaluateParams {
            expression: expression.to_string(),
            return_by_value: Some(true),
            await_promise: Some(true),
            context_id: self.expect_context_id(frame_id)?,
            user_gesture: Some(true),
        };
        let result: EvaluateResult = self.call("Runtime.evaluate", json!(params)).await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        Ok(result.result.value.unwrap_or(Value::Null))
    }

    /// Evaluate `expression` in the given frame, returning a handle.
    pub(crate) async fn evaluate_handle_in(
        &self,
        expression: &str,
        frame_id: Option<&str>,
    ) -> Result<RemoteObject> {
        self.ensure_open()?;
        let params = EvaluateParams {
            expression: expression.to_string(),
            return_by_value: Some(false),
            await_promise: Some(false),
            context_id: self.expect_context_id(frame_id)?,
            user_gesture: Some(true),
        };
        let result: EvaluateResult = self.call("Runtime.evaluate", json!(params)).await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        Ok(result.result)
    }

    fn expect_context_id(&self, frame_id: Option<&str>) -> Result<Option<i64>> {
        match frame_id {
            None => Ok(None),
            Some(id) => self
                .execution_context_id(Some(id))
                .map(Some)
                .ok_or_else(|| {
                    Error::JavaScript(format!(
                        "execution context for frame {id} is not available yet"
                    ))
                }),
        }
    }

    pub(crate) async fn wait_for_spec(
        &self,
        spec: &Value,
        describe: &str,
        state: WaitState,
        timeout: Duration,
        frame_id: Option<&str>,
    ) -> Result<()> {
        self.ensure_open()?;
        let serialized = serde_json::to_string(spec)?;
        let millis = timeout.as_millis();
        let expression = format!(
            "(window.__rustwright ? window.__rustwright.waitFor({serialized}, \"{}\", {millis}) : false)",
            state.as_str()
        );
        let params = EvaluateParams {
            expression,
            return_by_value: Some(true),
            await_promise: Some(true),
            context_id: self.expect_context_id(frame_id)?,
            user_gesture: Some(true),
        };
        let result: EvaluateResult = self
            .call_with_timeout(
                "Runtime.evaluate",
                json!(params),
                timeout + Duration::from_secs(5),
            )
            .await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        let satisfied = result
            .result
            .value
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !satisfied {
            return Err(timeout_error(
                format!("element {describe} to be {}", state.as_str()),
                timeout,
            ));
        }
        Ok(())
    }

    pub(crate) async fn click_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let (x, y) = self.center_of(spec, describe, frame_id).await?;
        self.mouse_move(x, y).await?;
        self.dispatch_mouse("mousePressed", x, y, "left", 1).await?;
        self.dispatch_mouse("mouseReleased", x, y, "left", 0)
            .await?;
        Ok(())
    }

    pub(crate) async fn hover_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let (x, y) = self.center_of(spec, describe, frame_id).await?;
        self.mouse_move(x, y).await
    }

    pub(crate) async fn scroll_into_view_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        self.call_on(
            &object_id,
            "function () { this.scrollIntoView({ block: 'center', inline: 'center' }); }",
            Vec::new(),
            true,
        )
        .await?;
        Ok(())
    }

    async fn center_of(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<(f64, f64)> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        self.call_on(
            &object_id,
            "function () { this.scrollIntoView({ block: 'center', inline: 'center' }); }",
            Vec::new(),
            true,
        )
        .await?;
        let model: GetBoxModelResult = self
            .call(
                "DOM.getBoxModel",
                json!(GetBoxModelParams {
                    object_id: Some(object_id),
                    ..Default::default()
                }),
            )
            .await?;
        Ok(model.model.content_center())
    }

    async fn dispatch_mouse(
        &self,
        event_type: &str,
        x: f64,
        y: f64,
        button: &str,
        buttons: i64,
    ) -> Result<()> {
        let params = DispatchMouseEventParams {
            event_type: event_type.to_string(),
            x,
            y,
            button: Some(button.to_string()),
            buttons: Some(buttons),
            click_count: Some(1),
            delta_x: None,
            delta_y: None,
        };
        self.call::<Value>("Input.dispatchMouseEvent", json!(params))
            .await?;
        Ok(())
    }

    pub(crate) async fn focus_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        self.call_on(
            &object_id,
            "function () { if (this.focus) { this.focus(); } }",
            Vec::new(),
            true,
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn fill_spec(
        &self,
        spec: &Value,
        describe: &str,
        text: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let function = "function (text) { \
             this.focus(); \
             const tag = this.tagName; \
             let proto = null; \
             if (tag === 'TEXTAREA') { proto = HTMLTextAreaElement.prototype; } \
             else if (tag === 'INPUT') { proto = HTMLInputElement.prototype; } \
             if (proto) { \
               const desc = Object.getOwnPropertyDescriptor(proto, 'value'); \
               if (desc && desc.set) { desc.set.call(this, text); } else { this.value = text; } \
             } else if (this.isContentEditable) { this.textContent = text; } \
             else { this.value = text; } \
             this.dispatchEvent(new Event('input', { bubbles: true, cancelable: true })); \
             this.dispatchEvent(new Event('change', { bubbles: true })); \
           }";
        self.call_on(
            &object_id,
            function,
            vec![CallArgument::value(json!(text))],
            true,
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn text_spec(
        &self,
        spec: &Value,
        describe: &str,
        inner: bool,
        frame_id: Option<&str>,
    ) -> Result<String> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let function = if inner {
            "function () { return this.innerText != null ? this.innerText : this.textContent; }"
        } else {
            "function () { return this.textContent; }"
        };
        let result = self.call_on(&object_id, function, Vec::new(), true).await?;
        Ok(result
            .value
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string())
    }

    pub(crate) async fn spec_is_visible(
        &self,
        spec: &Value,
        frame_id: Option<&str>,
    ) -> Result<bool> {
        let serialized = serde_json::to_string(spec)?;
        let expression =
            format!("(window.__rustwright ? window.__rustwright.isVisible({serialized}) : false)");
        Ok(self
            .evaluate_in(&expression, frame_id)
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    pub(crate) async fn spec_is_enabled(
        &self,
        spec: &Value,
        frame_id: Option<&str>,
    ) -> Result<bool> {
        let serialized = serde_json::to_string(spec)?;
        let expression =
            format!("(window.__rustwright ? window.__rustwright.isEnabled({serialized}) : false)");
        Ok(self
            .evaluate_in(&expression, frame_id)
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    pub(crate) async fn spec_is_checked(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<bool> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let result = self
            .call_on(
                &object_id,
                "function () { return this.checked === true; }",
                Vec::new(),
                true,
            )
            .await?;
        Ok(result
            .value
            .as_ref()
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    pub(crate) async fn spec_count(&self, spec: &Value, frame_id: Option<&str>) -> Result<usize> {
        let serialized = serde_json::to_string(spec)?;
        let expression =
            format!("(window.__rustwright ? window.__rustwright.count({serialized}) : 0)");
        Ok(self
            .evaluate_in(&expression, frame_id)
            .await?
            .as_u64()
            .unwrap_or(0) as usize)
    }

    pub(crate) async fn get_attribute_spec(
        &self,
        spec: &Value,
        describe: &str,
        name: &str,
        frame_id: Option<&str>,
    ) -> Result<Option<String>> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let result = self
            .call_on(
                &object_id,
                "function (name) { return this.getAttribute(name); }",
                vec![CallArgument::value(json!(name))],
                true,
            )
            .await?;
        Ok(result
            .value
            .as_ref()
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub(crate) async fn select_option_spec(
        &self,
        spec: &Value,
        describe: &str,
        value: &str,
        frame_id: Option<&str>,
    ) -> Result<()> {
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.expect("resolved object has an id");
        let result = self
            .call_on(
                &object_id,
                "function (value) { \
                   if (this.tagName !== 'SELECT') { return false; } \
                   this.value = value; \
                   this.dispatchEvent(new Event('input', { bubbles: true })); \
                   this.dispatchEvent(new Event('change', { bubbles: true })); \
                   return true; \
                 }",
                vec![CallArgument::value(json!(value))],
                true,
            )
            .await?;
        if result.value.as_ref().and_then(Value::as_bool) != Some(true) {
            return Err(Error::ElementNotFound {
                selector: format!("{describe} (not a <select>)"),
            });
        }
        Ok(())
    }
}

struct KeyDefinition {
    key: String,
    code: String,
    virtual_key_code: i64,
    text: Option<String>,
}

fn key_definition(key: &str) -> KeyDefinition {
    let simple = |key: &str, code: &str, virtual_key_code: i64| KeyDefinition {
        key: key.to_string(),
        code: code.to_string(),
        virtual_key_code,
        text: None,
    };
    match key {
        "Enter" | "\n" => KeyDefinition {
            key: "Enter".to_string(),
            code: "Enter".to_string(),
            virtual_key_code: 13,
            text: Some("\r".to_string()),
        },
        "Tab" => simple("Tab", "Tab", 9),
        "Escape" | "Esc" => simple("Escape", "Escape", 27),
        "Backspace" => simple("Backspace", "Backspace", 8),
        "Delete" => simple("Delete", "Delete", 46),
        " " | "Space" => KeyDefinition {
            key: " ".to_string(),
            code: "Space".to_string(),
            virtual_key_code: 32,
            text: Some(" ".to_string()),
        },
        "ArrowUp" | "Up" => simple("ArrowUp", "ArrowUp", 38),
        "ArrowDown" | "Down" => simple("ArrowDown", "ArrowDown", 40),
        "ArrowLeft" | "Left" => simple("ArrowLeft", "ArrowLeft", 37),
        "ArrowRight" | "Right" => simple("ArrowRight", "ArrowRight", 39),
        other => {
            if other.chars().count() == 1 {
                let character = other.chars().next().unwrap_or(' ');
                KeyDefinition {
                    key: other.to_string(),
                    code: code_for_char(character),
                    virtual_key_code: character.to_ascii_uppercase() as i64,
                    text: Some(other.to_string()),
                }
            } else {
                simple(other, other, 0)
            }
        }
    }
}

fn code_for_char(character: char) -> String {
    if character.is_ascii_alphabetic() {
        format!("Key{}", character.to_ascii_uppercase())
    } else if character.is_ascii_digit() {
        format!("Digit{character}")
    } else {
        String::new()
    }
}

fn spawn_event_pump(
    connection: CdpConnection,
    session: CdpSession,
    state: Arc<PageState>,
) -> JoinHandle<()> {
    let session_id = session.session_id().to_string();
    tokio::spawn(async move {
        let mut receiver = connection.subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.session_id.as_deref() != Some(session_id.as_str()) {
                        continue;
                    }
                    if event.method == "Page.javascriptDialogOpening" {
                        handle_dialog(&session, &state, &event.params).await;
                        continue;
                    }
                    if event.method == "Fetch.requestPaused" {
                        handle_request_paused(&session, &state, &event.params).await;
                        continue;
                    }
                    dispatch_event(&state, &event.method, &event.params);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        state.closed.store(true, Ordering::SeqCst);
    })
}

async fn handle_dialog(session: &CdpSession, state: &PageState, params: &Value) {
    if let Ok(opening) = serde_json::from_value::<JavascriptDialogOpeningParams>(params.clone()) {
        state
            .dialogs
            .lock()
            .expect("dialogs mutex poisoned")
            .push(DialogInfo {
                dialog_type: opening.dialog_type,
                message: opening.message,
                default_prompt: opening.default_prompt,
            });
    }
    // Auto-dismiss so a stray alert can never hang automation.
    let _ = session
        .send(
            "Page.handleJavaScriptDialog",
            json!(HandleJavaScriptDialogParams {
                accept: false,
                prompt_text: None
            }),
        )
        .await;
}

async fn handle_request_paused(session: &CdpSession, state: &PageState, params: &Value) {
    let Ok(paused) = serde_json::from_value::<RequestPausedParams>(params.clone()) else {
        return;
    };
    let request_id = paused.request_id.clone();
    let action = {
        let routes = state.routes.lock().expect("routes mutex poisoned");
        routes
            .iter()
            .find(|route| route.matches(&paused.request.url))
            .map(|route| route.action.clone())
    };
    match action {
        Some(RouteAction::Abort) => {
            let _ = session
                .send(
                    "Fetch.failRequest",
                    json!(FailRequestParams {
                        request_id,
                        error_reason: "Aborted".to_string(),
                    }),
                )
                .await;
        }
        Some(RouteAction::Fulfill {
            status,
            content_type,
            body,
        }) => {
            let params = FulfillRequestParams {
                request_id,
                response_code: status,
                response_headers: vec![HeaderEntry {
                    name: "Content-Type".to_string(),
                    value: content_type,
                }],
                body: Some(base64::engine::general_purpose::STANDARD.encode(body)),
            };
            let _ = session.send("Fetch.fulfillRequest", json!(params)).await;
        }
        _ => {
            let _ = session
                .send(
                    "Fetch.continueRequest",
                    json!(ContinueRequestParams { request_id }),
                )
                .await;
        }
    }
}

fn dispatch_event(state: &PageState, method: &str, params: &Value) {
    match method {
        "Page.lifecycleEvent" => {
            if let Ok(event) = serde_json::from_value::<LifecycleEventParams>(params.clone()) {
                state.mark_lifecycle(&event.name);
            }
        }
        "Page.frameNavigated" => {
            if let Ok(event) = serde_json::from_value::<FrameNavigatedParams>(params.clone()) {
                let is_main = event.frame.parent_id.is_none();
                if let Some(info) = state
                    .frames
                    .lock()
                    .expect("frames mutex poisoned")
                    .get_mut(&event.frame.id)
                {
                    info.url = event.frame.url.clone();
                    info.parent_id = event.frame.parent_id.clone();
                    info.name = event.frame.name.clone();
                    info.execution_context_id = None;
                }
                if is_main {
                    *state
                        .main_frame_id
                        .lock()
                        .expect("main frame mutex poisoned") = Some(event.frame.id.clone());
                    state.set_url(&event.frame.url);
                    state
                        .navigations
                        .lock()
                        .expect("navigations mutex poisoned")
                        .push(NavigationEvent {
                            url: event.frame.url.clone(),
                            frame_id: event.frame.id.clone(),
                            loader_id: event.frame.loader_id.clone(),
                            mime_type: event.frame.mime_type.clone(),
                        });
                }
            }
        }
        "Page.frameAttached" => {
            if let Ok(event) = serde_json::from_value::<FrameAttachedParams>(params.clone()) {
                state
                    .frames
                    .lock()
                    .expect("frames mutex poisoned")
                    .entry(event.frame_id)
                    .or_insert_with(|| FrameInfo {
                        parent_id: non_empty(&event.parent_frame_id),
                        ..Default::default()
                    });
            }
        }
        "Page.frameDetached" => {
            if let Ok(event) = serde_json::from_value::<FrameDetachedParams>(params.clone()) {
                state
                    .frames
                    .lock()
                    .expect("frames mutex poisoned")
                    .remove(&event.frame_id);
            }
        }
        "Runtime.executionContextCreated" => {
            if let Ok(event) =
                serde_json::from_value::<ExecutionContextCreatedParams>(params.clone())
            {
                if let Some(aux) = event.context.aux_data {
                    if let Some(frame_id) = aux.frame_id {
                        let mut frames = state.frames.lock().expect("frames mutex poisoned");
                        let frame = frames.entry(frame_id).or_default();
                        frame.execution_context_id = Some(event.context.id);
                    }
                }
            }
        }
        "Runtime.executionContextDestroyed" => {
            if let Ok(event) =
                serde_json::from_value::<ExecutionContextDestroyedParams>(params.clone())
            {
                let mut frames = state.frames.lock().expect("frames mutex poisoned");
                for frame in frames.values_mut() {
                    if frame.execution_context_id == Some(event.execution_context_id) {
                        frame.execution_context_id = None;
                    }
                }
            }
        }
        "Tracing.dataCollected" => {
            if let Ok(event) = serde_json::from_value::<TracingDataCollectedParams>(params.clone())
            {
                state
                    .trace
                    .lock()
                    .expect("trace mutex poisoned")
                    .chunks
                    .push(event.value);
            }
        }
        "Tracing.tracingComplete" => {
            if serde_json::from_value::<TracingCompleteParams>(params.clone()).is_ok() {
                let mut trace = state.trace.lock().expect("trace mutex poisoned");
                trace.recording = false;
                if let Some(sender) = trace.complete_tx.take() {
                    let _ = sender.send(());
                }
            }
        }
        "Runtime.consoleAPICalled" => {
            if let Ok(event) = serde_json::from_value::<ConsoleAPICalledParams>(params.clone()) {
                state
                    .console
                    .lock()
                    .expect("console mutex poisoned")
                    .push(ConsoleMessage {
                        level: event.call_type.clone(),
                        text: event.text(),
                        timestamp: event.timestamp,
                    });
            }
        }
        "Runtime.exceptionThrown" => {
            if let Ok(event) = serde_json::from_value::<ExceptionThrownParams>(params.clone()) {
                state
                    .errors
                    .lock()
                    .expect("errors mutex poisoned")
                    .push(PageError {
                        message: event.exception_details.message(),
                        timestamp: event.timestamp,
                    });
            }
        }
        "Network.requestWillBeSent" => {
            if let Ok(event) = serde_json::from_value::<RequestWillBeSentParams>(params.clone()) {
                let mut requests = state.requests.lock().expect("requests mutex poisoned");
                if let Some(existing) = requests
                    .iter_mut()
                    .find(|request| request.request_id == event.request_id)
                {
                    *existing = NetworkRequest::from_request(&event);
                } else {
                    requests.push(NetworkRequest::from_request(&event));
                }
            }
        }
        "Network.responseReceived" => {
            if let Ok(event) = serde_json::from_value::<ResponseReceivedParams>(params.clone()) {
                let mut requests = state.requests.lock().expect("requests mutex poisoned");
                if let Some(request) = requests
                    .iter_mut()
                    .find(|request| request.request_id == event.request_id)
                {
                    request.apply_response(&event.response);
                    request.resource_type = event.resource_type;
                }
            }
        }
        "Network.loadingFailed" => {
            if let Ok(event) = serde_json::from_value::<LoadingFailedParams>(params.clone()) {
                let mut requests = state.requests.lock().expect("requests mutex poisoned");
                if let Some(request) = requests
                    .iter_mut()
                    .find(|request| request.request_id == event.request_id)
                {
                    request.failure = Some(event.error_text);
                }
            }
        }
        _ => {}
    }
}

/// Prefix a bare host with `https://`, leaving explicit schemes untouched.
pub fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "about:blank".to_string();
    }
    let has_scheme = trimmed
        .find(':')
        .map(|index| {
            let (scheme, rest) = trimmed.split_at(index);
            !rest.is_empty()
                && rest.starts_with(':')
                && (index == 0
                    || scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'))
                && !scheme.chars().next().map(char::is_numeric).unwrap_or(true)
        })
        .unwrap_or(false);
    if has_scheme {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn screenshot_format(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "jpeg",
        Some("webp") => "webp",
        _ => "png",
    }
}

fn decode_base64(data: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| Error::JavaScript(format!("invalid base64 payload: {error}")))
}

fn timeout_error(what: String, timeout: Duration) -> Error {
    Error::Timeout { what, timeout }
}

#[cfg(test)]
mod tests {
    use super::{key_definition, normalize_url};

    #[test]
    fn prefixes_bare_hosts_with_https() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("  example.com  "), "https://example.com");
    }

    #[test]
    fn leaves_explicit_schemes_alone() {
        assert_eq!(normalize_url("http://example.com"), "http://example.com");
        assert_eq!(normalize_url("file:///tmp/a.html"), "file:///tmp/a.html");
        assert_eq!(
            normalize_url("data:text/html,<title>x</title>"),
            "data:text/html,<title>x</title>"
        );
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }

    #[test]
    fn empty_input_becomes_about_blank() {
        assert_eq!(normalize_url(""), "about:blank");
    }

    #[test]
    fn maps_common_keys() {
        let enter = key_definition("Enter");
        assert_eq!(enter.key, "Enter");
        assert_eq!(enter.virtual_key_code, 13);
        assert_eq!(enter.text.as_deref(), Some("\r"));

        let letter = key_definition("a");
        assert_eq!(letter.key, "a");
        assert_eq!(letter.code, "KeyA");
        assert_eq!(letter.virtual_key_code, 65);
        assert_eq!(letter.text.as_deref(), Some("a"));
    }
}
