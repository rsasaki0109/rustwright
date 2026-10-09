//! The [`Page`] abstraction: navigation, content, interaction and waiting.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use base64::Engine as _;
use rustwright_cdp::protocol::dom::{
    DescribeNodeParams, DescribeNodeResult, GetBoxModelParams, GetBoxModelResult,
    SetFileInputFilesParams,
};
use rustwright_cdp::protocol::emulation::SetDeviceMetricsOverrideParams;
use rustwright_cdp::protocol::fetch::{
    ContinueRequestParams, ContinueResponseParams, EnableParams as FetchEnableParams,
    FailRequestParams, FulfillRequestParams, HeaderEntry, RequestPattern, RequestPausedParams,
};
use rustwright_cdp::protocol::input::{
    DispatchKeyEventParams, DispatchMouseEventParams, InsertTextParams,
};
use rustwright_cdp::protocol::network::{
    Cookie, GetCookiesResult, GetResponseBodyParams, GetResponseBodyResult, LoadingFailedParams,
    LoadingFinishedParams, RequestWillBeSentParams, ResponseReceivedParams, SetCookieParams,
    SetCookieResult,
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
use rustwright_common::{
    LoadState, Role, Route, RouteAction, Selector, WaitState, INJECTED_SCRIPT,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot, watch, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

use crate::diagnostics::{
    ConsoleMessage, DialogInfo, NavigationEvent, NetworkRequest, OriginStorage, PageError,
    StorageItem, StorageState,
};
use crate::error::{Error, Result};
use crate::frame::{Frame, FrameLocator};
use crate::geometry::{click_candidates, click_candidates_in_rect, Point, Projection};
use crate::locator::Locator;
use crate::network_idle::{NetworkActivity, RequestOwner};

/// The default timeout for navigation, waits and actions.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// Registered only while routes are active. No JavaScript request API is replaced.
const ROUTE_GATE: &str = "debugger;\n//# sourceURL=__rustwright_route_gate.js";

#[derive(Deserialize)]
struct ClickGeometry {
    rect: [f64; 4],
    #[serde(default)]
    clip: Option<[f64; 4]>,
}

#[derive(Deserialize)]
struct ContentQuads {
    quads: Vec<[f64; 8]>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HitNode {
    backend_node_id: i64,
    frame_id: String,
}

// Remote object ids and execution context ids are scoped to their CDP session.
struct RemoteNode {
    object_id: String,
    session: CdpSession,
    context_id: Option<i64>,
}

struct FrameBoundary {
    parent: CdpSession,
    owner: i64,
    projection: Projection,
}

impl FrameBoundary {
    fn same_geometry(&self, other: &Self) -> bool {
        self.parent.session_id() == other.parent.session_id()
            && self.owner == other.owner
            && self.projection == other.projection
    }
}

fn map_quad(quad: &[f64; 8], boundaries: &[FrameBoundary]) -> Option<[f64; 8]> {
    let mut mapped = *quad;
    for coords in mapped.chunks_exact_mut(2) {
        let mut point = Point {
            x: coords[0],
            y: coords[1],
        };
        for boundary in boundaries {
            point = boundary.projection.forward(point)?;
        }
        coords[0] = point.x;
        coords[1] = point.y;
    }
    Some(mapped)
}

impl RemoteNode {
    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        Ok(serde_json::from_value(
            self.session.send(method, params).await?,
        )?)
    }

    async fn call_on(
        &self,
        function: &str,
        arguments: Vec<CallArgument>,
        return_by_value: bool,
    ) -> Result<RemoteObject> {
        let params = CallFunctionOnParams {
            function_declaration: function.to_string(),
            object_id: Some(self.object_id.clone()),
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
}

impl Drop for RemoteNode {
    fn drop(&mut self) {
        let session = self.session.clone();
        let object_id = self.object_id.clone();
        tokio::spawn(async move {
            let _ = session
                .send_with_timeout(
                    "Runtime.releaseObject",
                    json!({"objectId": object_id}),
                    Duration::from_secs(1),
                )
                .await;
        });
    }
}

// Click retries create short-lived node handles. Release them even if the
// caller's deadline cancels a probe before it can perform normal cleanup.
struct ClickObjects {
    session: CdpSession,
    ids: Vec<String>,
}

impl Drop for ClickObjects {
    fn drop(&mut self) {
        let session = self.session.clone();
        let ids = std::mem::take(&mut self.ids);
        tokio::spawn(async move {
            for id in ids {
                let _ = session
                    .send_with_timeout(
                        "Runtime.releaseObject",
                        json!({"objectId": id}),
                        Duration::from_secs(1),
                    )
                    .await;
            }
        });
    }
}

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

#[derive(Default)]
struct DocumentLifecycle {
    loader_id: Option<String>,
    states: HashSet<String>,
    revision: u64,
    recent_commits: VecDeque<(u64, String)>,
}

impl DocumentLifecycle {
    fn set_loader(&mut self, loader_id: &str) {
        if self.loader_id.as_deref() != Some(loader_id) {
            self.loader_id = Some(loader_id.to_string());
            self.states.clear();
        }
    }

    fn mark(&mut self, event: &LifecycleEventParams) {
        if event.name == "init" {
            self.set_loader(&event.loader_id);
        }
        if self.loader_id.as_deref() == Some(&event.loader_id) {
            self.states.insert(event.name.clone());
        }
    }

    fn commit(&mut self, loader_id: Option<&str>, restored: bool) {
        if let Some(loader_id) = loader_id {
            self.set_loader(loader_id);
        }
        self.revision += 1;
        if let Some(loader_id) = &self.loader_id {
            self.recent_commits
                .push_back((self.revision, loader_id.clone()));
            if self.recent_commits.len() > 64 {
                self.recent_commits.pop_front();
            }
        }
        // A BFCache restore has no new DOM/load events: the document was
        // already loaded before it entered the cache. Do not invent idle state.
        if restored {
            self.states
                .extend(["DOMContentLoaded".to_string(), "load".to_string()]);
        }
    }
}

enum LoadExpectation {
    Current,
    Loader { id: String, before: u64 },
    AfterCommit(u64),
}

#[derive(Clone)]
struct ObservedRequest {
    session_id: String,
    request: NetworkRequest,
}

#[derive(Default)]
struct NetworkLog {
    requests: Vec<ObservedRequest>,
    activity: NetworkActivity,
}

impl NetworkLog {
    fn find_mut(&mut self, session_id: &str, request_id: &str) -> Option<&mut NetworkRequest> {
        let index = self
            .requests
            .iter()
            .position(|entry| {
                entry.session_id == session_id && entry.request.request_id == request_id
            })
            .or_else(|| {
                // A document navigation can begin in the parent session and finish
                // in its new OOPIF session without another requestWillBeSent event.
                // Only transfer a unique, unfinished document; subresource ids are
                // scoped to their sessions and must never merge.
                let mut candidates = self.requests.iter().enumerate().filter(|(_, entry)| {
                    entry.request.request_id == request_id
                        && entry.request.resource_type == "Document"
                        && entry.request.finished.is_none()
                });
                let (index, _) = candidates.next()?;
                candidates.next().is_none().then_some(index)
            })?;
        let entry = &mut self.requests[index];
        entry.session_id = session_id.to_string();
        Some(&mut entry.request)
    }

    fn dispatch(&mut self, session_id: &str, method: &str, params: &Value) {
        match method {
            "Network.requestWillBeSent" => {
                if let Ok(event) = serde_json::from_value::<RequestWillBeSentParams>(params.clone())
                {
                    self.activity.start(
                        session_id,
                        &event.request_id,
                        RequestOwner {
                            frame: params["frameId"].as_str().map(str::to_string),
                            loader: event.loader_id.clone(),
                            document: params["type"] == "Document",
                        },
                    );
                    let mut request = NetworkRequest::from_request(&event);
                    request.resource_type = params["type"].as_str().unwrap_or_default().to_string();
                    if let Some(entry) = self.requests.iter_mut().find(|entry| {
                        entry.session_id == session_id
                            && entry.request.request_id == event.request_id
                    }) {
                        entry.request = request;
                    } else {
                        self.requests.push(ObservedRequest {
                            session_id: session_id.to_string(),
                            request,
                        });
                    }
                }
            }
            "Network.responseReceived" => {
                if let Ok(event) = serde_json::from_value::<ResponseReceivedParams>(params.clone())
                {
                    self.activity
                        .transfer_document(session_id, &event.request_id);
                    if let Some(request) = self.find_mut(session_id, &event.request_id) {
                        request.apply_response(&event.response, event.timestamp);
                        request.resource_type = event.resource_type;
                    }
                }
            }
            "Network.loadingFinished" => {
                if let Ok(event) = serde_json::from_value::<LoadingFinishedParams>(params.clone()) {
                    self.activity.finish(
                        session_id,
                        &event.request_id,
                        tokio::time::Instant::now(),
                    );
                    if let Some(request) = self.find_mut(session_id, &event.request_id) {
                        request.finish(event.timestamp);
                    }
                }
            }
            "Network.loadingFailed" => {
                if let Ok(event) = serde_json::from_value::<LoadingFailedParams>(params.clone()) {
                    self.activity.finish(
                        session_id,
                        &event.request_id,
                        tokio::time::Instant::now(),
                    );
                    if let Some(request) = self.find_mut(session_id, &event.request_id) {
                        request.failure = Some(event.error_text);
                        request.finish(event.timestamp);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Information tracked for a frame.
#[derive(Debug, Clone, Default)]
pub(crate) struct FrameInfo {
    pub(crate) parent_id: Option<String>,
    pub(crate) url: String,
    pub(crate) name: String,
    pub(crate) execution_context_id: Option<i64>,
    session_id: Option<String>,
    initialization_error: Option<(String, String)>,
}

pub(crate) type PageRegistry = Mutex<HashMap<String, Page>>;

/// Mutable page state shared between clones and the event pump.
pub(crate) struct PageState {
    lifecycle: Mutex<DocumentLifecycle>,
    changes: watch::Sender<()>,
    url_tx: broadcast::Sender<String>,
    console: Mutex<Vec<ConsoleMessage>>,
    errors: Mutex<Vec<PageError>>,
    requests: Mutex<NetworkLog>,
    navigations: Mutex<Vec<NavigationEvent>>,
    dialogs: Mutex<Vec<DialogInfo>>,
    frames: Mutex<HashMap<String, FrameInfo>>,
    main_frame_id: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, CdpSession>>,
    session_parents: Mutex<HashMap<String, String>>,
    routes: Mutex<Vec<Route>>,
    routing: Arc<AsyncMutex<()>>,
    route_gates: Mutex<HashMap<String, String>>,
    trace: Mutex<TraceState>,
    url: Mutex<String>,
    closed: AtomicBool,
    target_id: String,
    registry: Mutex<Weak<PageRegistry>>,
}

impl PageState {
    fn mark_lifecycle(&self, event: &LifecycleEventParams) {
        self.lifecycle
            .lock()
            .expect("lifecycle mutex poisoned")
            .mark(event);
        self.changes.send_replace(());
    }

    fn navigation_revision(&self) -> u64 {
        self.lifecycle
            .lock()
            .expect("lifecycle mutex poisoned")
            .revision
    }

    fn commit_document(&self, loader_id: Option<&str>, restored: bool, url: &str) {
        {
            let mut lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
            lifecycle.commit(loader_id, restored);
            *self.url.lock().expect("url mutex poisoned") = url.to_string();
        }
        let _ = self.url_tx.send(url.to_string());
        self.changes.send_replace(());
    }

    fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.changes.send_replace(());
        // A context owns tracked handles, but a page must not own its context.
        // Release the weak-reference lock before locking the registry: page
        // registration takes these locks in the opposite order.
        let registry = self
            .registry
            .lock()
            .expect("registry mutex poisoned")
            .upgrade();
        if let Some(registry) = registry {
            registry
                .lock()
                .expect("pages mutex poisoned")
                .remove(&self.target_id);
        }
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
        let (changes, _) = watch::channel(());
        let (url_tx, _) = broadcast::channel(512);
        let state = Arc::new(PageState {
            lifecycle: Mutex::new(DocumentLifecycle::default()),
            changes,
            url_tx,
            console: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            requests: Mutex::new(NetworkLog::default()),
            navigations: Mutex::new(Vec::new()),
            dialogs: Mutex::new(Vec::new()),
            frames: Mutex::new(HashMap::new()),
            main_frame_id: Mutex::new(None),
            session_parents: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::from([(
                session.session_id().to_string(),
                session.clone(),
            )])),
            routes: Mutex::new(Vec::new()),
            routing: Arc::new(AsyncMutex::new(())),
            route_gates: Mutex::new(HashMap::new()),
            trace: Mutex::new(TraceState::default()),
            url: Mutex::new("about:blank".to_string()),
            closed: AtomicBool::new(false),
            target_id: session.target_id().to_owned(),
            registry: Mutex::new(Weak::new()),
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
        enable_frame_auto_attach(&page.session).await?;
        Ok(page)
    }

    pub(crate) fn register_in(&self, registry: &Arc<PageRegistry>) -> Result<()> {
        let mut pages = registry.lock().expect("pages mutex poisoned");
        *self.state.registry.lock().expect("registry mutex poisoned") = Arc::downgrade(registry);
        // Closure can arrive during initialization. Publishing the owner and
        // checking closure under the registry lock prevents a stale insertion.
        self.ensure_open()?;
        pages.insert(self.target_id().to_owned(), self.clone());
        Ok(())
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
        self.navigate(url, state, DEFAULT_TIMEOUT).await
    }

    /// Navigate with one timeout covering the command and document load.
    pub async fn goto_with_timeout(&self, url: &str, timeout: Duration) -> Result<()> {
        self.navigate(url, LoadState::Load, timeout).await
    }

    async fn navigate(&self, url: &str, state: LoadState, timeout: Duration) -> Result<()> {
        self.ensure_open()?;
        let normalized = normalize_url(url);
        let receiver = self.state.changes.subscribe();
        let before = self.state.navigation_revision();
        with_deadline(timeout, format!("navigation to {normalized:?}"), async {
            let params = NavigateParams {
                url: normalized.clone(),
                referrer: None,
                frame_id: None,
            };
            let result: NavigateResult = self.call("Page.navigate", json!(params)).await?;
            if let Some(error) = result.error_text.filter(|error| !error.is_empty()) {
                return Err(Error::Navigation(error));
            }
            let expectation = if let Some(id) = result.loader_id {
                LoadExpectation::Loader { id, before }
            } else {
                // A same-document commit preserves load states, but its URL
                // event can reach the pump after the command response. Wait for
                // it unless the requested URL is already current.
                if self.url() == normalized {
                    LoadExpectation::Current
                } else {
                    LoadExpectation::AfterCommit(before)
                }
            };
            self.wait_lifecycle(receiver, state, expectation).await
        })
        .await
    }

    /// Reload the current page and wait for `load`.
    pub async fn reload(&self) -> Result<()> {
        self.ensure_open()?;
        let receiver = self.state.changes.subscribe();
        let before = self.state.navigation_revision();
        with_deadline(DEFAULT_TIMEOUT, "page reload".to_string(), async {
            self.call::<Value>("Page.reload", json!(ReloadParams::default()))
                .await?;
            self.wait_lifecycle(
                receiver,
                LoadState::Load,
                LoadExpectation::AfterCommit(before),
            )
            .await
        })
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
        with_deadline(
            DEFAULT_TIMEOUT,
            format!("history navigation at offset {delta}"),
            async {
                let history: GetNavigationHistoryResult =
                    self.call("Page.getNavigationHistory", json!({})).await?;
                let target = history.current_index + delta;
                if target < 0 || target as usize >= history.entries.len() {
                    return Err(Error::Navigation(format!(
                        "no history entry at offset {delta}"
                    )));
                }
                let entry_id = history.entries[target as usize].id;
                let receiver = self.state.changes.subscribe();
                let before = self.state.navigation_revision();
                self.call::<Value>(
                    "Page.navigateToHistoryEntry",
                    json!(NavigateToHistoryEntryParams { entry_id }),
                )
                .await?;
                self.wait_lifecycle(
                    receiver,
                    LoadState::Load,
                    LoadExpectation::AfterCommit(before),
                )
                .await
            },
        )
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
        let already_closed = self.state.closed.swap(true, Ordering::SeqCst);
        self.state.mark_closed();
        if already_closed {
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
    ///
    /// `NetworkIdle` waits for 500 ms with no unfinished observed HTTP requests
    /// in the page or its frames, including separate iframe renderers. It tracks
    /// response-body completion and restarts when new requests begin. Streaming
    /// requests remain pending until they finish or are aborted.
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
        let receiver = self.state.changes.subscribe();
        with_deadline(
            timeout,
            format!("load state `{}`", state.as_str()),
            self.wait_lifecycle(receiver, state, LoadExpectation::Current),
        )
        .await
    }

    /// Wait until the URL contains `pattern`.
    pub async fn wait_for_url(&self, pattern: &str) -> Result<()> {
        self.wait_for_url_with_timeout(pattern, DEFAULT_TIMEOUT)
            .await
    }

    /// Wait until the URL contains `pattern`, with an explicit timeout.
    pub async fn wait_for_url_with_timeout(&self, pattern: &str, timeout: Duration) -> Result<()> {
        // Subscribe before inspecting state. Keep individual URL events so a
        // short-lived matching URL is observed even if the document changes it
        // again before the waiter runs; state notifications also wake on close.
        let mut urls = self.state.url_tx.subscribe();
        let mut changes = self.state.changes.subscribe();
        with_deadline(timeout, format!("url to contain {pattern:?}"), async {
            loop {
                self.ensure_open()?;
                if self.url().contains(pattern) {
                    return Ok(());
                }
                tokio::select! {
                    event = urls.recv() => match event {
                        Ok(url) if url.contains(pattern) => { self.ensure_open()?; return Ok(()); }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return Err(Error::PageClosed),
                    },
                    changed = changes.changed() => { changed.map_err(|_| Error::PageClosed)?; }
                }
            }
        })
        .await
    }

    /// Wait for a fixed duration. Prefer event-driven waits; this is an escape
    /// hatch for the rare case where no observable signal exists.
    pub async fn wait_for_timeout(&self, timeout: Duration) {
        tokio::time::sleep(timeout).await;
    }

    async fn wait_lifecycle(
        &self,
        mut receiver: watch::Receiver<()>,
        state: LoadState,
        expectation: LoadExpectation,
    ) -> Result<()> {
        let target = state.lifecycle_name();
        loop {
            self.ensure_open()?;
            let idle_deadline = {
                let lifecycle = self
                    .state
                    .lifecycle
                    .lock()
                    .expect("lifecycle mutex poisoned");
                let eligible = match &expectation {
                    LoadExpectation::Current => true,
                    LoadExpectation::AfterCommit(before) => lifecycle.revision > *before,
                    LoadExpectation::Loader { id, before } => {
                        // A previous in-flight commit may arrive after this
                        // navigation started. It is not an interruption until
                        // the requested document itself has committed. Retain
                        // recent commits to detect replacement even if watch
                        // notifications coalesced before the waiter ran.
                        if lifecycle.loader_id.as_ref() != Some(id)
                            && lifecycle
                                .recent_commits
                                .iter()
                                .any(|(revision, loader)| revision > before && loader == id)
                        {
                            return Err(Error::Navigation(
                                "navigation interrupted by another document".to_string(),
                            ));
                        }
                        lifecycle.loader_id.as_ref() == Some(id)
                    }
                };
                if eligible && state == LoadState::NetworkIdle {
                    let deadline = self
                        .state
                        .requests
                        .lock()
                        .expect("requests mutex poisoned")
                        .activity
                        .idle_deadline();
                    if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
                        return Ok(());
                    }
                    deadline
                } else if eligible && lifecycle.states.contains(target) {
                    return Ok(());
                } else {
                    None
                }
            };
            if let Some(deadline) = idle_deadline {
                tokio::select! {
                    changed = receiver.changed() => { changed.map_err(|_| Error::PageClosed)?; }
                    _ = tokio::time::sleep_until(deadline) => {}
                }
            } else {
                receiver.changed().await.map_err(|_| Error::PageClosed)?;
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

    /// Network requests observed since the page was created, including requests
    /// from nested and out-of-process frames. CDP request ids are session-scoped
    /// and can repeat across frames.
    pub fn network_requests(&self) -> Vec<NetworkRequest> {
        self.state
            .requests
            .lock()
            .expect("requests mutex poisoned")
            .requests
            .iter()
            .map(|entry| entry.request.clone())
            .collect()
    }

    /// Build a HAR 1.2 document from the requests observed so far.
    ///
    /// Response bodies are not included; use [`Page::har_with_bodies`] for a
    /// HAR with content.
    pub fn har(&self) -> Value {
        crate::diagnostics::build_har(&self.network_requests(), None)
    }

    /// Build a HAR 1.2 document including response bodies where available.
    ///
    /// Bodies are fetched from the session that observed each request. Entries
    /// remain in the log after a frame detaches, but bodies from detached sessions
    /// or evicted by the browser are omitted.
    pub async fn har_with_bodies(&self) -> Result<Value> {
        let observed = self
            .state
            .requests
            .lock()
            .expect("requests mutex poisoned")
            .requests
            .clone();
        let sessions = self
            .state
            .sessions
            .lock()
            .expect("sessions mutex poisoned")
            .clone();
        let mut bodies = std::collections::HashMap::new();
        for (index, entry) in observed.iter().enumerate() {
            let Some(session) = sessions.get(&entry.session_id) else {
                continue;
            };
            let params = GetResponseBodyParams {
                request_id: entry.request.request_id.clone(),
            };
            if let Ok(value) = session.send("Network.getResponseBody", json!(params)).await {
                let Ok(result) = serde_json::from_value::<GetResponseBodyResult>(value) else {
                    continue;
                };
                bodies.insert(index, (result.body, result.base64_encoded));
            }
        }
        let requests: Vec<_> = observed.into_iter().map(|entry| entry.request).collect();
        Ok(crate::diagnostics::build_har(&requests, Some(&bodies)))
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
    /// Rules apply to the page and its attached iframe sessions. HTTP caching
    /// is disabled until [`Self::clear_routes`] removes all rules. A CDP debugger
    /// gate restores interception before new-document scripts execute, including
    /// when an iframe returns to its parent's renderer.
    /// Debugger pauses are resumed automatically while routes are active. Once
    /// configuration starts, dropping this future does not cancel its updates.
    pub async fn route(&self, pattern: impl Into<String>, action: RouteAction) -> Result<()> {
        self.ensure_open()?;
        let routing = self.state.routing.clone().lock_owned().await;
        let page = self.clone();
        let pattern = pattern.into();
        // Complete configuration even if the caller drops its future. This
        // preserves script identifiers so clear_routes can remove every gate.
        tokio::spawn(async move {
            let _routing = routing;
            page.state
                .routes
                .lock()
                .expect("routes mutex poisoned")
                .push(Route::new(pattern, action));
            page.sync_fetch().await
        })
        .await
        .map_err(|error| Error::Io(std::io::Error::other(error)))?
    }

    // Called with the routing lock held. Reapply on every mutation so a later
    // call also repairs configuration interrupted by cancellation or an error.
    async fn sync_fetch(&self) -> Result<()> {
        let sessions: Vec<_> = self
            .state
            .sessions
            .lock()
            .expect("sessions mutex poisoned")
            .values()
            .cloned()
            .collect();
        let mut first_error = None;
        for session in sessions {
            if let Err(error) = configure_fetch(&session, &self.state).await {
                // A child may disappear while commands are in flight. The root
                // session and unrelated protocol/transport errors still fail.
                let detached = matches!(
                    &error,
                    Error::Cdp(rustwright_cdp::CdpError::SessionDetached { .. })
                ) || matches!(&error, Error::Cdp(rustwright_cdp::CdpError::Protocol {code: -32001, message, ..}) if message.contains("Session with given id not found"));
                if !(detached && session.session_id() != self.session.session_id())
                    && first_error.is_none()
                {
                    first_error = Some(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
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
    ///
    /// Removes startup gates, disables the driver's debugger/interception and
    /// restores HTTP caching in every active frame session. Configuration errors
    /// are returned; calling again retries cleanup. Once cleanup starts, dropping
    /// this future does not cancel its updates.
    pub async fn clear_routes(&self) -> Result<()> {
        self.ensure_open()?;
        let routing = self.state.routing.clone().lock_owned().await;
        let page = self.clone();
        tokio::spawn(async move {
            let _routing = routing;
            page.state
                .routes
                .lock()
                .expect("routes mutex poisoned")
                .clear();
            page.sync_fetch().await
        })
        .await
        .map_err(|error| Error::Io(std::io::Error::other(error)))?
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
        self.state.commit_document(
            tree.frame_tree.frame.loader_id.as_deref(),
            false,
            &tree.frame_tree.frame.full_url(),
        );
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
                    url: node.frame.full_url(),
                    name: node.frame.name.clone(),
                    execution_context_id: None,
                    ..Default::default()
                },
            );
            for child in node.child_frames {
                stack.push((child, false));
            }
        }
        Ok(())
    }

    fn frame_context(&self, frame_id: Option<&str>) -> Result<(CdpSession, Option<i64>)> {
        let Some(id) = frame_id else {
            return Ok((self.session.clone(), None));
        };
        let frames = self.state.frames.lock().expect("frames mutex poisoned");
        let frame = frames.get(id).ok_or_else(|| Error::FrameNotReady {
            frame_id: id.to_string(),
        })?;
        if let Some((failed_session, error)) = &frame.initialization_error {
            if frame
                .session_id
                .as_ref()
                .is_none_or(|current| current == failed_session)
            {
                return Err(Error::JavaScript(format!(
                    "could not initialize frame {id}: {error}"
                )));
            }
        }
        let context_id = frame
            .execution_context_id
            .ok_or_else(|| Error::FrameNotReady {
                frame_id: id.to_string(),
            })?;
        let session = frame
            .session_id
            .as_ref()
            .and_then(|id| {
                self.state
                    .sessions
                    .lock()
                    .expect("sessions mutex poisoned")
                    .get(id)
                    .cloned()
            })
            .ok_or_else(|| Error::FrameNotReady {
                frame_id: id.to_string(),
            })?;
        Ok((session, Some(context_id)))
    }

    pub(crate) async fn frame_id_for_element(&self, selector: &Selector) -> Result<String> {
        let spec = selector.to_spec();
        let object = self
            .resolve_object_spec(&spec, &selector.describe(), None)
            .await?;
        let object_id = object.object_id.clone();
        let described: DescribeNodeResult = object
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
        let object_id = object.object_id.clone();
        let params = SetFileInputFilesParams {
            files: files
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            object_id: Some(object_id),
            ..Default::default()
        };
        object
            .call::<Value>("DOM.setFileInputFiles", json!(params))
            .await?;
        Ok(())
    }

    // -- Internal helpers ---------------------------------------------------

    pub(crate) async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let value = self.session.send(method, params).await?;
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

    async fn resolve_object_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
    ) -> Result<RemoteNode> {
        let serialized = serde_json::to_string(spec)?;
        let expression =
            format!("(window.__rustwright ? window.__rustwright.resolve({serialized}) : null)");
        let (object, session, context_id) = self.evaluate_handle_in(&expression, frame_id).await?;
        if object.is_nullish() || object.object_id.is_none() {
            return Err(Error::ElementNotFound {
                selector: describe.to_string(),
            });
        }
        Ok(RemoteNode {
            object_id: object.object_id.expect("resolved object has an id"),
            session,
            context_id,
        })
    }

    /// Evaluate `expression` in the given frame (or the main frame).
    pub(crate) async fn evaluate_in(
        &self,
        expression: &str,
        frame_id: Option<&str>,
    ) -> Result<Value> {
        self.ensure_open()?;
        let (session, context_id) = self.frame_context(frame_id)?;
        let params = EvaluateParams {
            expression: expression.to_string(),
            return_by_value: Some(true),
            await_promise: Some(true),
            context_id,
            user_gesture: Some(true),
        };
        let result: EvaluateResult =
            serde_json::from_value(session.send("Runtime.evaluate", json!(params)).await?)?;
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
    ) -> Result<(RemoteObject, CdpSession, Option<i64>)> {
        self.ensure_open()?;
        let (session, context_id) = self.frame_context(frame_id)?;
        let params = EvaluateParams {
            expression: expression.to_string(),
            return_by_value: Some(false),
            await_promise: Some(false),
            context_id,
            user_gesture: Some(true),
        };
        let result: EvaluateResult =
            serde_json::from_value(session.send("Runtime.evaluate", json!(params)).await?)?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        Ok((result.result, session, context_id))
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
        let (session, context_id) = self.frame_context(frame_id)?;
        let params = EvaluateParams {
            expression,
            return_by_value: Some(true),
            await_promise: Some(true),
            context_id,
            user_gesture: Some(true),
        };
        let result: EvaluateResult = serde_json::from_value(
            session
                .send_with_timeout(
                    "Runtime.evaluate",
                    json!(params),
                    timeout + Duration::from_secs(5),
                )
                .await?,
        )?;
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

    // Quads in an OOPIF are relative to that renderer's viewport. Each remote
    // boundary maps that viewport into its owner's painted content quad.
    async fn frame_boundaries(
        &self,
        child: &CdpSession,
        scroll: bool,
    ) -> Result<Vec<FrameBoundary>> {
        let mut session = child.clone();
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        while session.session_id() != self.session.session_id() {
            let id = session.target_id();
            if !visited.insert(id.to_string()) {
                return Err(Error::JavaScript("cyclic frame ancestry".to_string()));
            }
            let parent_id = self
                .frame_info(id)
                .and_then(|info| info.parent_id)
                .ok_or_else(|| Error::FrameNotReady {
                    frame_id: id.to_string(),
                })?;
            let (parent, _) = self.frame_context(Some(&parent_id))?;
            let owner: Value = parent
                .send("DOM.getFrameOwner", json!({"frameId": id}))
                .await?;
            let owner_id = owner["backendNodeId"]
                .as_i64()
                .ok_or_else(|| Error::FrameNotReady {
                    frame_id: id.to_string(),
                })?;
            if scroll {
                let resolved = parent
                    .send("DOM.resolveNode", json!({"backendNodeId": owner_id}))
                    .await?;
                let object: RemoteObject = serde_json::from_value(resolved["object"].clone())?;
                let owner_node = RemoteNode {
                    object_id: object.object_id.ok_or_else(|| Error::FrameNotReady {
                        frame_id: id.to_string(),
                    })?,
                    session: parent.clone(),
                    context_id: None,
                };
                owner_node.call_on("function () { const r = this.getBoundingClientRect(); const v = this.ownerDocument.defaultView; if (r.bottom <= 0 || r.right <= 0 || r.top >= v.innerHeight || r.left >= v.innerWidth) this.scrollIntoView({block:'nearest',inline:'nearest',behavior:'instant'}); }", Vec::new(), true).await?;
            }
            let model: GetBoxModelResult = serde_json::from_value(
                parent
                    .send("DOM.getBoxModel", json!({"backendNodeId": owner_id}))
                    .await?,
            )?;
            let quad: [f64; 8] = model
                .model
                .content
                .0
                .try_into()
                .map_err(|_| Error::JavaScript("invalid iframe content quad".to_string()))?;
            let viewport: [f64; 2] = serde_json::from_value(
                evaluate_session(&session, "[innerWidth, innerHeight]").await?,
            )?;
            let projection = Projection::new(quad, viewport[0], viewport[1]).ok_or_else(|| {
                Error::FrameNotReady {
                    frame_id: id.to_string(),
                }
            })?;
            result.push(FrameBoundary {
                parent: parent.clone(),
                owner: owner_id,
                projection,
            });
            session = parent;
        }
        Ok(result)
    }

    /// Try one actionable click; return the reason to retry if it is not ready.
    pub(crate) async fn click_spec(
        &self,
        spec: &Value,
        describe: &str,
        frame_id: Option<&str>,
        observation: &mut String,
    ) -> Result<Option<String>> {
        self.ensure_open()?;
        let object = self.resolve_object_spec(spec, describe, frame_id).await?;
        let object_id = object.object_id.clone();
        let mut objects = ClickObjects {
            session: object.session.clone(),
            ids: Vec::new(),
        };
        let initial_boundaries = self.frame_boundaries(&object.session, true).await?;
        let initial: ContentQuads = object
            .call("DOM.getContentQuads", json!({"objectId": object_id}))
            .await?;
        let params = CallFunctionOnParams {
            function_declaration: "function () { return window.__rustwright.prepareClick(this); }"
                .to_string(),
            object_id: Some(object_id.clone()),
            arguments: Vec::new(),
            return_by_value: Some(true),
            await_promise: Some(true),
            user_gesture: Some(true),
        };
        let result: EvaluateResult = object.call("Runtime.callFunctionOn", json!(params)).await?;
        if let Some(details) = result.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        let prepared = result.result.value.unwrap_or(Value::Null);
        if let Some(reason) = prepared.get("reason").and_then(Value::as_str) {
            return Ok(Some(reason.to_string()));
        }
        let geometry: ClickGeometry = serde_json::from_value(prepared.clone())?;
        if geometry.rect[2] <= 0.0 || geometry.rect[3] <= 0.0 {
            return Ok(Some("element has no clickable box".to_string()));
        }
        let quads: ContentQuads = object
            .call("DOM.getContentQuads", json!({"objectId": object_id}))
            .await?;
        let boundaries = self.frame_boundaries(&object.session, false).await?;
        if quads.quads != initial.quads
            || boundaries.len() != initial_boundaries.len()
            || boundaries
                .iter()
                .zip(&initial_boundaries)
                .any(|(a, b)| !a.same_geometry(b))
        {
            return Ok(Some("element or frame is moving".to_string()));
        }
        // Content quads are painted fragments in root-viewport coordinates.
        // Clip their polygons, not their axis-aligned bounding rectangles.
        let viewport: [f64; 4] = serde_json::from_value(
            self.evaluate("[innerWidth, innerHeight, scrollX, scrollY]")
                .await?,
        )?;
        let root_quads: Vec<_> = quads
            .quads
            .iter()
            .filter_map(|quad| map_quad(quad, &boundaries))
            .collect();
        // The main document's helper and native quads share viewport coordinates.
        // An oversized control may be clipped to an interior strip by an overflow
        // ancestor; clipping only to the viewport can miss that strip entirely.
        // Frame quads retain their existing renderer/projective coordinate path.
        let candidates = if frame_id.is_none() {
            if let Some([left, top, right, bottom]) = geometry.clip {
                click_candidates_in_rect(
                    &root_quads,
                    [
                        left.max(0.0),
                        top.max(0.0),
                        right.min(viewport[0]),
                        bottom.min(viewport[1]),
                    ],
                )
            } else {
                click_candidates(&root_quads, viewport[0], viewport[1])
            }
        } else {
            click_candidates(&root_quads, viewport[0], viewport[1])
        };
        if candidates.is_empty() {
            return Ok(Some("element has no visible clickable area".to_string()));
        }
        for point in candidates {
            // Hit testing accepts integer document coordinates. Dispatch to the
            // same physical point instead of testing one point and clicking a
            // nearby fractional point outside a thin or transformed fragment.
            let x = (point.x + viewport[2]).floor() - viewport[2];
            let y = (point.y + viewport[3]).floor() - viewport[3];
            if !self
                .click_hits_target(&object, frame_id, Point { x, y }, &boundaries, &mut objects)
                .await?
            {
                *observation = "another element intercepts pointer events".to_string();
                continue;
            }
            self.mouse_move(x, y).await?;
            // Hover handlers may disable, replace or cover the target.
            let checked = object.call_on(
                "function (spec, geometry) { return window.__rustwright.checkClick(this, spec, geometry); }",
                vec![CallArgument::value(spec.clone()), CallArgument::value(prepared.clone())], true,
            ).await?;
            if let Some(reason) = checked.value.as_ref().and_then(Value::as_str) {
                return Ok(Some(reason.to_string()));
            }
            let current: ContentQuads = object
                .call("DOM.getContentQuads", json!({"objectId": object_id}))
                .await?;
            let current_boundaries = self.frame_boundaries(&object.session, false).await?;
            if current.quads != quads.quads
                || current_boundaries.len() != boundaries.len()
                || current_boundaries
                    .iter()
                    .zip(&boundaries)
                    .any(|(a, b)| !a.same_geometry(b))
            {
                return Ok(Some("element or frame is moving".to_string()));
            }
            if !self
                .click_hits_target(&object, frame_id, Point { x, y }, &boundaries, &mut objects)
                .await?
            {
                return Ok(Some(
                    "another element intercepts pointer events".to_string(),
                ));
            }
            self.dispatch_mouse("mousePressed", x, y, "left", 1).await?;
            self.dispatch_mouse("mouseReleased", x, y, "left", 0)
                .await?;
            return Ok(None);
        }
        Ok(Some(
            "another element intercepts pointer events at candidate points".to_string(),
        ))
    }

    async fn click_hits_target(
        &self,
        object: &RemoteNode,
        frame_id: Option<&str>,
        point: Point,
        boundaries: &[FrameBoundary],
        objects: &mut ClickObjects,
    ) -> Result<bool> {
        let mut local = point;
        // Verify every ancestor renderer. A child-only hit test cannot see a
        // parent overlay covering its iframe, so it is insufficient for input.
        for boundary in boundaries.iter().rev() {
            let hit = hit_node(&boundary.parent, local).await?;
            if hit.backend_node_id != boundary.owner {
                return Ok(false);
            }
            let Some(mapped) = boundary.projection.backward(local) else {
                return Ok(false);
            };
            local = mapped;
        }
        let hit = hit_node(&object.session, local).await?;
        let main = self.main_frame();
        if hit.frame_id != frame_id.unwrap_or(main.frame_id()) {
            return Ok(false);
        }
        let mut params = json!({"backendNodeId": hit.backend_node_id});
        if let Some(context_id) = object.context_id {
            params["executionContextId"] = json!(context_id);
        }
        // Native hit testing takes integer coordinates; for an OOPIF's own
        // document also verify the actual fractional point after projection.
        if !boundaries.is_empty() && frame_id == Some(object.session.target_id()) {
            let received = object.call_on("function (x,y) { return window.__rustwright.includesHit(this, document.elementFromPoint(x,y)); }",
                vec![CallArgument::value(json!(local.x)), CallArgument::value(json!(local.y))], true).await?;
            if received.value != Some(json!(true)) {
                return Ok(false);
            }
        }
        let resolved: Value = object.call("DOM.resolveNode", params).await?;
        let node: RemoteObject = serde_json::from_value(resolved["object"].clone())?;
        let hit_id = match node.object_id {
            Some(id) => id,
            None => return Ok(false),
        };
        objects.ids.push(hit_id.clone());
        let result = object
            .call_on(
                "function (hit) { return window.__rustwright.includesHit(this, hit); }",
                vec![CallArgument::object_id(hit_id)],
                true,
            )
            .await?;
        Ok(result
            .value
            .and_then(|value| value.as_bool())
            .unwrap_or(false))
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
        object
            .call_on(
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
        let object_id = object.object_id.clone();
        // Newly loaded remote frames need a rendering opportunity before the
        // browser can route mouse movement to their painted surface.
        let params = CallFunctionOnParams {
            function_declaration: "function () { this.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); return new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); }".to_string(),
            object_id: Some(object_id.clone()),
            arguments: Vec::new(),
            return_by_value: Some(true),
            await_promise: Some(true),
            user_gesture: Some(true),
        };
        let painted: EvaluateResult = object.call("Runtime.callFunctionOn", json!(params)).await?;
        if let Some(details) = painted.exception_details {
            return Err(Error::JavaScript(details.message()));
        }
        let model: GetBoxModelResult = object
            .call(
                "DOM.getBoxModel",
                json!(GetBoxModelParams {
                    object_id: Some(object_id),
                    ..Default::default()
                }),
            )
            .await?;
        let (x, y) = model.model.content_center();
        let mut point = Point { x, y };
        for boundary in self.frame_boundaries(&object.session, true).await? {
            point = boundary
                .projection
                .forward(point)
                .ok_or_else(|| Error::FrameNotReady {
                    frame_id: object.session.target_id().to_string(),
                })?;
        }
        Ok((point.x, point.y))
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
        object
            .call_on(
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
        object
            .call_on(function, vec![CallArgument::value(json!(text))], true)
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
        let function = if inner {
            "function () { return this.innerText != null ? this.innerText : this.textContent; }"
        } else {
            "function () { return this.textContent; }"
        };
        let result = object.call_on(function, Vec::new(), true).await?;
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
        let result = object
            .call_on(
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
        let result = object
            .call_on(
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
        let result = object
            .call_on(
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

async fn evaluate_session(session: &CdpSession, expression: &str) -> Result<Value> {
    let result: EvaluateResult = serde_json::from_value(
        session
            .send(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
            )
            .await?,
    )?;
    if let Some(details) = result.exception_details {
        return Err(Error::JavaScript(details.message()));
    }
    Ok(result.result.value.unwrap_or(Value::Null))
}

async fn hit_node(session: &CdpSession, point: Point) -> Result<HitNode> {
    // getNodeForLocation uses document coordinates within this renderer.
    let scroll: [f64; 2] =
        serde_json::from_value(evaluate_session(session, "[scrollX, scrollY]").await?)?;
    Ok(serde_json::from_value(session.send("DOM.getNodeForLocation", json!({
        "x": (point.x + scroll[0]).round() as i64, "y": (point.y + scroll[1]).round() as i64,
        "includeUserAgentShadowDOM": true,
    })).await?)?)
}

async fn enable_frame_auto_attach(session: &CdpSession) -> Result<()> {
    session
        .send(
            "Target.setAutoAttach",
            json!({
                "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true,
                "filter": [{"type": "iframe", "exclude": false}, {"exclude": true}],
            }),
        )
        .await?;
    Ok(())
}

async fn configure_fetch(session: &CdpSession, state: &PageState) -> Result<()> {
    let enabled = !state
        .routes
        .lock()
        .expect("routes mutex poisoned")
        .is_empty();
    let gate = state
        .route_gates
        .lock()
        .expect("route gates mutex poisoned")
        .get(session.session_id())
        .cloned();
    if enabled && gate.is_none() {
        session.send("Debugger.enable", json!({})).await?;
        let added = session
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source":ROUTE_GATE}),
            )
            .await?;
        #[derive(Deserialize)]
        struct InstalledScript {
            identifier: String,
        }
        let added: InstalledScript = serde_json::from_value(added)?;
        let sessions = state.sessions.lock().expect("sessions mutex poisoned");
        if sessions.contains_key(session.session_id()) {
            state
                .route_gates
                .lock()
                .expect("route gates mutex poisoned")
                .insert(session.session_id().to_string(), added.identifier);
        }
    } else if !enabled {
        if let Some(identifier) = gate {
            session
                .send(
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({"identifier":identifier}),
                )
                .await?;
            state
                .route_gates
                .lock()
                .expect("route gates mutex poisoned")
                .remove(session.session_id());
        }
        session.send("Debugger.disable", json!({})).await?;
    }
    // Route mutations and child initialization own the routing lock. Cache
    // policy persists across navigation, so startup pauses only restore Fetch.
    session
        .send("Network.setCacheDisabled", json!({"cacheDisabled":enabled}))
        .await?;
    apply_fetch(session, state).await
}

async fn apply_fetch(session: &CdpSession, state: &PageState) -> Result<()> {
    let (enabled, with_response) = {
        let routes = state.routes.lock().expect("routes mutex poisoned");
        (
            !routes.is_empty(),
            routes
                .iter()
                .any(|route| matches!(route.action, RouteAction::SetResponseHeaders(_))),
        )
    };
    if !enabled {
        session.send("Fetch.disable", json!({})).await?;
        return Ok(());
    }
    let mut patterns = vec![RequestPattern {
        url_pattern: Some("*".to_string()),
        request_stage: Some("Request".to_string()),
        ..Default::default()
    }];
    if with_response {
        patterns.push(RequestPattern {
            url_pattern: Some("*".to_string()),
            request_stage: Some("Response".to_string()),
            ..Default::default()
        });
    }
    session
        .send(
            "Fetch.enable",
            json!(FetchEnableParams {
                patterns,
                handle_auth_requests: false
            }),
        )
        .await?;
    Ok(())
}

async fn attach_child_frame(
    connection: &CdpConnection,
    state: &PageState,
    parent_session: &str,
    params: &Value,
) {
    let (Some(id), Some(target)) = (
        params["sessionId"].as_str(),
        params["targetInfo"]["targetId"].as_str(),
    ) else {
        return;
    };
    let session = CdpSession::new(connection.clone(), id.to_string(), target.to_string());
    // The iframe-only auto-attach filter avoids pausing unrelated workers.
    if params["targetInfo"]["type"] != "iframe" {
        return;
    }
    // Serialize registration/configuration with route changes. An attached
    // renderer inherits interception before any of its scripts can run.
    let routing = state.routing.lock().await;
    state
        .sessions
        .lock()
        .expect("sessions mutex poisoned")
        .insert(id.to_string(), session.clone());
    state
        .session_parents
        .lock()
        .expect("session parents mutex poisoned")
        .insert(id.to_string(), parent_session.to_string());
    {
        let mut frames = state.frames.lock().expect("frames mutex poisoned");
        let frame = frames.entry(target.to_string()).or_default();
        if let Some(parent) = params["targetInfo"]["parentFrameId"].as_str() {
            frame.parent_id = Some(parent.to_string());
        }
        frame.initialization_error = None;
    }
    let initialized = async {
        if !state
            .routes
            .lock()
            .expect("routes mutex poisoned")
            .is_empty()
        {
            configure_fetch(&session, state).await?;
        }
        drop(routing);
        session.send("Page.enable", json!({})).await?;
        let tree: GetFrameTreeResult =
            serde_json::from_value(session.send("Page.getFrameTree", json!({})).await?)?;
        {
            let mut frames = state.frames.lock().expect("frames mutex poisoned");
            let mut stack = vec![tree.frame_tree];
            while let Some(node) = stack.pop() {
                let url = node.frame.full_url();
                let info = frames.entry(node.frame.id).or_default();
                info.url = url;
                info.name = node.frame.name;
                if node.frame.parent_id.is_some() {
                    info.parent_id = node.frame.parent_id;
                }
                stack.extend(node.child_frames);
            }
        }
        session
            .send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": INJECTED_SCRIPT, "runImmediately": true}),
            )
            .await?;
        session.send("DOM.enable", json!({})).await?;
        session.send("Runtime.enable", json!({})).await?;
        session.send("Network.enable", json!({})).await?;
        session
            .send("Page.setLifecycleEventsEnabled", json!({"enabled": true}))
            .await?;
        enable_frame_auto_attach(&session).await?;
        Result::Ok(())
    }
    .await;
    if let Err(error) = initialized {
        state
            .frames
            .lock()
            .expect("frames mutex poisoned")
            .entry(target.to_string())
            .or_default()
            .initialization_error = Some((id.to_string(), error.to_string()));
    }
    // Resume even when initialization failed; never leave a renderer paused.
    let _ = session
        .send("Runtime.runIfWaitingForDebugger", json!({}))
        .await;
}

fn remove_session_subtree(parents: &mut HashMap<String, String>, id: &str) -> HashSet<String> {
    let mut removed = HashSet::from([id.to_string()]);
    loop {
        let children: Vec<_> = parents
            .iter()
            .filter(|(_, parent)| removed.contains(*parent))
            .map(|(child, _)| child.clone())
            .collect();
        let before = removed.len();
        removed.extend(children);
        if before == removed.len() {
            break;
        }
    }
    parents.retain(|child, _| !removed.contains(child));
    removed
}

fn detach_child_frame(state: &PageState, params: &Value) {
    let Some(id) = params["sessionId"].as_str() else {
        return;
    };
    let removed = remove_session_subtree(
        &mut state
            .session_parents
            .lock()
            .expect("session parents mutex poisoned"),
        id,
    );
    state
        .sessions
        .lock()
        .expect("sessions mutex poisoned")
        .retain(|id, _| !removed.contains(id));
    state
        .route_gates
        .lock()
        .expect("route gates mutex poisoned")
        .retain(|id, _| !removed.contains(id));
    state
        .requests
        .lock()
        .expect("requests mutex poisoned")
        .activity
        .remove_sessions(&removed, tokio::time::Instant::now());
    state.changes.send_replace(());
    for frame in state
        .frames
        .lock()
        .expect("frames mutex poisoned")
        .values_mut()
    {
        if frame
            .session_id
            .as_ref()
            .is_some_and(|id| removed.contains(id))
        {
            frame.execution_context_id = None;
            frame.session_id = None;
        }
    }
}

fn spawn_event_pump(
    connection: CdpConnection,
    session: CdpSession,
    state: Arc<PageState>,
) -> JoinHandle<()> {
    let root_id = session.session_id().to_string();
    let mut receiver = connection.subscribe();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.is_disconnected() {
                        break;
                    }
                    if event.method == "Target.detachedFromTarget"
                        && event.params["sessionId"] == root_id
                    {
                        state.mark_closed();
                        detach_child_frame(&state, &event.params);
                        break;
                    }
                    let Some(id) = event.session_id.as_deref() else {
                        continue;
                    };
                    let event_session = state
                        .sessions
                        .lock()
                        .expect("sessions mutex poisoned")
                        .get(id)
                        .cloned();
                    let Some(event_session) = event_session else {
                        continue;
                    };
                    if event.method == "Target.attachedToTarget" {
                        attach_child_frame(&connection, &state, id, &event.params).await;
                        continue;
                    }
                    if event.method == "Target.detachedFromTarget" {
                        detach_child_frame(&state, &event.params);
                        continue;
                    }
                    if event.method == "Debugger.paused" {
                        let _routing = state.routing.lock().await;
                        let restored = async {
                            apply_fetch(&event_session, &state).await?;
                            // Wait for the browser's frame-tree response before
                            // resuming the renderer after its Fetch update.
                            event_session.send("Page.getFrameTree", json!({})).await?;
                            Result::Ok(())
                        }
                        .await;
                        if let Err(error) = restored {
                            for frame in state
                                .frames
                                .lock()
                                .expect("frames mutex poisoned")
                                .values_mut()
                            {
                                if frame.session_id.as_deref() == Some(id) {
                                    frame.initialization_error =
                                        Some((id.to_string(), error.to_string()));
                                }
                            }
                        }
                        // Includes application debugger statements while routes
                        // are active. Never leave a document paused on failure.
                        let _ = event_session.send("Debugger.resume", json!({})).await;
                        continue;
                    }
                    if event.method == "Page.javascriptDialogOpening" {
                        handle_dialog(&event_session, &state, &event.params).await;
                        continue;
                    }
                    if event.method == "Fetch.requestPaused" {
                        handle_request_paused(&event_session, &state, &event.params).await;
                        continue;
                    }
                    dispatch_event(&state, id, id == root_id, &event.method, &event.params);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        state.mark_closed();
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

    // `Fetch.requestPaused` also fires at the response stage; response fields
    // are present when the request had already been sent.
    let is_response = paused.response_status_code.is_some()
        || !paused.response_headers.is_empty()
        || paused.response_error_reason.is_some();

    if is_response {
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
            Some(RouteAction::SetResponseHeaders(overrides)) => {
                let headers = merge_response_headers(&paused.response_headers, &overrides);
                let params = ContinueResponseParams {
                    request_id,
                    response_code: paused.response_status_code,
                    response_phrase: None,
                    response_headers: Some(headers),
                };
                let _ = session.send("Fetch.continueResponse", json!(params)).await;
            }
            _ => {
                let params = ContinueResponseParams {
                    request_id,
                    response_code: None,
                    response_phrase: None,
                    response_headers: None,
                };
                let _ = session.send("Fetch.continueResponse", json!(params)).await;
            }
        }
        return;
    }

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
        Some(RouteAction::SetRequestHeaders(overrides)) => {
            let headers = merge_request_headers(&paused.request.headers, &overrides);
            let params = ContinueRequestParams {
                request_id,
                headers: Some(headers),
            };
            let _ = session.send("Fetch.continueRequest", json!(params)).await;
        }
        _ => {
            let params = ContinueRequestParams {
                request_id,
                headers: None,
            };
            let _ = session.send("Fetch.continueRequest", json!(params)).await;
        }
    }
}

fn merge_request_headers(
    original: &serde_json::Map<String, Value>,
    overrides: &[(String, String)],
) -> Vec<HeaderEntry> {
    let mut merged: BTreeMap<String, String> = original
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            (name.clone(), value)
        })
        .collect();
    for (name, value) in overrides {
        merged.insert(name.clone(), value.clone());
    }
    merged
        .into_iter()
        .map(|(name, value)| HeaderEntry { name, value })
        .collect()
}

fn merge_response_headers(
    original: &[HeaderEntry],
    overrides: &[(String, String)],
) -> Vec<HeaderEntry> {
    let mut merged: BTreeMap<String, String> = original
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();
    for (name, value) in overrides {
        merged.insert(name.clone(), value.clone());
    }
    merged
        .into_iter()
        .map(|(name, value)| HeaderEntry { name, value })
        .collect()
}

fn dispatch_event(
    state: &PageState,
    session_id: &str,
    root_session: bool,
    method: &str,
    params: &Value,
) {
    match method {
        "Page.lifecycleEvent" => {
            if let Ok(event) = serde_json::from_value::<LifecycleEventParams>(params.clone()) {
                if root_session
                    && state
                        .main_frame_id
                        .lock()
                        .expect("main frame mutex poisoned")
                        .as_deref()
                        == Some(event.frame_id.as_str())
                {
                    state.mark_lifecycle(&event);
                }
            }
        }
        "Page.frameNavigated" => {
            if let Ok(event) = serde_json::from_value::<FrameNavigatedParams>(params.clone()) {
                let is_main = root_session && event.frame.parent_id.is_none();
                if let Some(loader) = event.frame.loader_id.as_deref() {
                    state
                        .requests
                        .lock()
                        .expect("requests mutex poisoned")
                        .activity
                        .commit_frame(
                            &event.frame.id,
                            loader,
                            is_main,
                            tokio::time::Instant::now(),
                        );
                    state.changes.send_replace(());
                }
                {
                    let mut frames = state.frames.lock().expect("frames mutex poisoned");
                    let info = frames.entry(event.frame.id.clone()).or_default();
                    info.url = event.frame.full_url();
                    if event.frame.parent_id.is_some() || is_main {
                        info.parent_id = event.frame.parent_id.clone();
                    }
                    info.name = event.frame.name.clone();
                    if info.session_id.as_deref().is_none_or(|id| id == session_id) {
                        info.execution_context_id = None;
                    }
                }
                if is_main {
                    *state
                        .main_frame_id
                        .lock()
                        .expect("main frame mutex poisoned") = Some(event.frame.id.clone());
                    state.commit_document(
                        event.frame.loader_id.as_deref(),
                        params["type"] == "BackForwardCacheRestore",
                        &event.frame.full_url(),
                    );
                    state
                        .navigations
                        .lock()
                        .expect("navigations mutex poisoned")
                        .push(NavigationEvent {
                            url: event.frame.full_url(),
                            frame_id: event.frame.id.clone(),
                            loader_id: event.frame.loader_id.clone(),
                            mime_type: event.frame.mime_type.clone(),
                        });
                }
            }
        }
        "Page.navigatedWithinDocument" => {
            if let (Some(frame_id), Some(url)) =
                (params["frameId"].as_str(), params["url"].as_str())
            {
                if let Some(frame) = state
                    .frames
                    .lock()
                    .expect("frames mutex poisoned")
                    .get_mut(frame_id)
                {
                    frame.url = url.to_string();
                }
                if root_session
                    && state
                        .main_frame_id
                        .lock()
                        .expect("main frame mutex poisoned")
                        .as_deref()
                        == Some(frame_id)
                {
                    state.commit_document(None, false, url);
                }
            }
        }
        "Page.frameAttached" => {
            if let Ok(event) = serde_json::from_value::<FrameAttachedParams>(params.clone()) {
                let mut frames = state.frames.lock().expect("frames mutex poisoned");
                let info = frames.entry(event.frame_id).or_default();
                info.parent_id = non_empty(&event.parent_frame_id);
            }
        }
        "Page.frameDetached" => {
            if let Ok(event) = serde_json::from_value::<FrameDetachedParams>(params.clone()) {
                let mut frames = state.frames.lock().expect("frames mutex poisoned");
                if event.reason == "swap" {
                    if let Some(frame) = frames.get_mut(&event.frame_id) {
                        if frame.session_id.as_deref() == Some(session_id) {
                            frame.execution_context_id = None;
                        }
                    }
                } else {
                    let mut removed = HashSet::from([event.frame_id]);
                    loop {
                        let children: Vec<_> = frames
                            .iter()
                            .filter(|(_, info)| {
                                info.parent_id
                                    .as_ref()
                                    .is_some_and(|id| removed.contains(id))
                            })
                            .map(|(id, _)| id.clone())
                            .collect();
                        let before = removed.len();
                        removed.extend(children);
                        if before == removed.len() {
                            break;
                        }
                    }
                    frames.retain(|id, _| !removed.contains(id));
                    state
                        .requests
                        .lock()
                        .expect("requests mutex poisoned")
                        .activity
                        .remove_frames(&removed, tokio::time::Instant::now());
                    state.changes.send_replace(());
                }
            }
        }
        "Runtime.executionContextCreated" => {
            if let Ok(event) =
                serde_json::from_value::<ExecutionContextCreatedParams>(params.clone())
            {
                if let Some(aux) = event.context.aux_data {
                    if aux.is_default {
                        if let Some(frame_id) = aux.frame_id {
                            let mut frames = state.frames.lock().expect("frames mutex poisoned");
                            let frame = frames.entry(frame_id).or_default();
                            frame.execution_context_id = Some(event.context.id);
                            frame.session_id = Some(session_id.to_string());
                        }
                    }
                }
            }
        }
        "Runtime.executionContextDestroyed" => {
            if let Ok(event) =
                serde_json::from_value::<ExecutionContextDestroyedParams>(params.clone())
            {
                for frame in state
                    .frames
                    .lock()
                    .expect("frames mutex poisoned")
                    .values_mut()
                {
                    if frame.session_id.as_deref() == Some(session_id)
                        && frame.execution_context_id == Some(event.execution_context_id)
                    {
                        frame.execution_context_id = None;
                    }
                }
            }
        }
        "Runtime.executionContextsCleared" => {
            for frame in state
                .frames
                .lock()
                .expect("frames mutex poisoned")
                .values_mut()
            {
                if frame.session_id.as_deref() == Some(session_id) {
                    frame.execution_context_id = None;
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
        "Network.requestWillBeSent"
        | "Network.responseReceived"
        | "Network.loadingFinished"
        | "Network.loadingFailed" => {
            state
                .requests
                .lock()
                .expect("requests mutex poisoned")
                .dispatch(session_id, method, params);
            state.changes.send_replace(());
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

async fn with_deadline<T>(
    timeout: Duration,
    what: String,
    operation: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(timeout, operation)
        .await
        .map_err(|_| timeout_error(what, timeout))?
}

fn timeout_error(what: String, timeout: Duration) -> Error {
    Error::Timeout { what, timeout }
}

#[cfg(test)]
mod tests {
    use super::{
        key_definition, normalize_url, remove_session_subtree, DocumentLifecycle, NetworkLog,
    };
    use rustwright_cdp::protocol::page::LifecycleEventParams;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};

    fn mark_load(lifecycle: &mut DocumentLifecycle, loader: &str, name: &str) {
        lifecycle.mark(&LifecycleEventParams {
            frame_id: "main".to_string(),
            loader_id: loader.to_string(),
            name: name.to_string(),
            timestamp: 0.0,
        });
    }

    #[test]
    fn new_documents_discard_old_load_states_and_ignore_late_old_events() {
        let mut lifecycle = DocumentLifecycle::default();
        lifecycle.commit(Some("old"), false);
        mark_load(&mut lifecycle, "old", "load");
        mark_load(&mut lifecycle, "old", "networkIdle");
        mark_load(&mut lifecycle, "new", "init");
        mark_load(&mut lifecycle, "old", "load");
        mark_load(&mut lifecycle, "old", "networkIdle");
        assert!(!lifecycle.states.contains("load"));
        assert!(!lifecycle.states.contains("networkIdle"));
        mark_load(&mut lifecycle, "new", "DOMContentLoaded");
        lifecycle.commit(Some("new"), false);
        assert!(lifecycle.states.contains("DOMContentLoaded"));
        assert!(!lifecycle.states.contains("load"));
        mark_load(&mut lifecycle, "new", "load");
        assert!(lifecycle.states.contains("load"));
    }

    #[test]
    fn same_document_and_bfcache_commits_do_not_wait_for_nonexistent_load_events() {
        let mut lifecycle = DocumentLifecycle::default();
        lifecycle.commit(Some("first"), false);
        mark_load(&mut lifecycle, "first", "DOMContentLoaded");
        mark_load(&mut lifecycle, "first", "load");
        mark_load(&mut lifecycle, "first", "networkIdle");
        lifecycle.commit(None, false);
        assert!(lifecycle.states.contains("load"));
        lifecycle.commit(Some("second"), false);
        assert!(!lifecycle.states.contains("load"));
        lifecycle.commit(Some("first"), true);
        assert!(lifecycle.states.contains("DOMContentLoaded"));
        assert!(lifecycle.states.contains("load"));
        assert!(!lifecycle.states.contains("networkIdle"));
    }

    fn start_request(log: &mut NetworkLog, session: &str, kind: &str) {
        log.dispatch(
            session,
            "Network.requestWillBeSent",
            &json!({
                "requestId": "shared", "type": kind, "timestamp": 1,
                "request": {"url": format!("https://{session}.example/"), "method": "GET"}
            }),
        );
    }

    #[test]
    fn network_ids_and_har_bodies_are_isolated_between_sessions() {
        let mut log = NetworkLog::default();
        start_request(&mut log, "parent", "Fetch");
        start_request(&mut log, "child", "Fetch");
        // A redirect replaces only the matching session's current request.
        start_request(&mut log, "parent", "Fetch");
        for (session, status) in [("parent", 200), ("child", 201)] {
            log.dispatch(
                session,
                "Network.responseReceived",
                &json!({
                    "requestId": "shared", "type": "Fetch", "timestamp": 2,
                    "response": {"status": status}
                }),
            );
        }
        log.dispatch(
            "unrelated",
            "Network.loadingFailed",
            &json!({"requestId": "shared", "errorText": "wrong session", "timestamp": 3}),
        );
        assert!(log
            .requests
            .iter()
            .all(|entry| entry.request.finished.is_none()));
        log.dispatch(
            "parent",
            "Network.loadingFinished",
            &json!({"requestId": "shared", "timestamp": 4}),
        );
        log.dispatch(
            "child",
            "Network.loadingFailed",
            &json!({"requestId": "shared", "errorText": "child failure", "timestamp": 5}),
        );
        assert_eq!(log.requests.len(), 2);
        assert_eq!(log.requests[0].request.status, Some(200));
        assert_eq!(log.requests[0].request.finished, Some(4.0));
        assert!(log.requests[0].request.failure.is_none());
        assert_eq!(log.requests[1].request.status, Some(201));
        assert_eq!(log.requests[1].request.finished, Some(5.0));
        assert_eq!(
            log.requests[1].request.failure.as_deref(),
            Some("child failure")
        );
        let requests: Vec<_> = log
            .requests
            .into_iter()
            .map(|entry| entry.request)
            .collect();
        let bodies = HashMap::from([
            (0, ("parent body".to_string(), false)),
            (1, ("AAEC".to_string(), true)),
        ]);
        let har = crate::diagnostics::build_har(&requests, Some(&bodies));
        assert_eq!(
            har["log"]["entries"][0]["response"]["content"]["text"],
            "parent body"
        );
        assert_eq!(
            har["log"]["entries"][1]["response"]["content"]["text"],
            "AAEC"
        );
        assert_eq!(
            har["log"]["entries"][1]["response"]["content"]["encoding"],
            "base64"
        );
    }

    #[test]
    fn document_completion_transfers_to_its_new_session_without_duplicates() {
        let mut log = NetworkLog::default();
        start_request(&mut log, "parent", "Document");
        log.dispatch(
            "parent",
            "Network.responseReceived",
            &json!({"requestId": "shared", "type": "Document", "response": {"status": 200}}),
        );
        log.dispatch(
            "child",
            "Network.loadingFinished",
            &json!({"requestId": "shared", "timestamp": 3}),
        );
        assert_eq!(log.requests.len(), 1);
        assert_eq!(log.requests[0].session_id, "child");
        assert_eq!(log.requests[0].request.finished, Some(3.0));
        assert_eq!(log.requests[0].request.status, Some(200));
        log.dispatch(
            "unrelated",
            "Network.loadingFailed",
            &json!({"requestId": "shared", "errorText": "late", "timestamp": 4}),
        );
        assert!(log.requests[0].request.failure.is_none());

        let mut ambiguous = NetworkLog::default();
        start_request(&mut ambiguous, "parent", "Document");
        start_request(&mut ambiguous, "sibling", "Document");
        ambiguous.dispatch(
            "child",
            "Network.loadingFinished",
            &json!({"requestId": "shared", "timestamp": 3}),
        );
        assert!(ambiguous
            .requests
            .iter()
            .all(|entry| entry.request.finished.is_none()));
    }

    #[test]
    fn detaching_a_session_removes_descendants_and_preserves_siblings() {
        let mut parents = HashMap::from([
            ("child".to_string(), "root".to_string()),
            ("nested".to_string(), "child".to_string()),
            ("leaf".to_string(), "nested".to_string()),
            ("sibling".to_string(), "root".to_string()),
        ]);
        let removed = remove_session_subtree(&mut parents, "child");
        assert_eq!(
            removed,
            HashSet::from([
                "child".to_string(),
                "nested".to_string(),
                "leaf".to_string()
            ])
        );
        assert_eq!(
            parents,
            HashMap::from([("sibling".to_string(), "root".to_string())])
        );
        assert_eq!(
            remove_session_subtree(&mut parents, "root"),
            HashSet::from(["root".to_string(), "sibling".to_string()])
        );
        assert!(parents.is_empty());
    }

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
