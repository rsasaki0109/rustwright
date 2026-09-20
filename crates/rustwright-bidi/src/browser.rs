//! A minimal Firefox/BiDi browser and page on top of [`BidiSession`].

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rustwright_browser::{Firefox, LaunchedFirefox};
use rustwright_common::{Role, Route, RouteAction, Selector, INJECTED_SCRIPT};
use serde_json::{json, Value};

use crate::error::{BidiError, BidiResult};
use crate::frame::{collect_frames, BidiFrame};
use crate::locator::BidiLocator;
use crate::network::{
    spawn_intercept_pump, spawn_network_pump, BidiNetworkRequest, NetworkPumpGuard,
};
use crate::session::{
    BidiCookie, BidiOriginStorage, BidiSession, BidiStorageItem, BidiStorageState,
};

/// A browser driven over WebDriver BiDi.
///
/// Use [`BidiBrowser::launch`] with a [`Firefox`], or [`BidiBrowser::connect`]
/// to attach to any BiDi endpoint (`ws://host:port/session` or
/// `http://host:port`).
pub struct BidiBrowser {
    session: BidiSession,
    process: Option<LaunchedFirefox>,
}

impl std::fmt::Debug for BidiBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiBrowser")
            .field("session_id", &self.session.session_id())
            .field("browser_version", &self.session.browser_version())
            .field("launched", &self.process.is_some())
            .finish()
    }
}

impl BidiBrowser {
    /// Launch Firefox and establish a BiDi session.
    pub async fn launch(firefox: Firefox) -> BidiResult<Self> {
        let launched = LaunchedFirefox::launch(&firefox).await?;
        let ws_url = launched.ws_url().to_string();
        match BidiSession::connect(&ws_url).await {
            Ok(session) => Ok(Self {
                session,
                process: Some(launched),
            }),
            Err(error) => Err(error),
        }
    }

    /// Connect to an existing BiDi endpoint.
    pub async fn connect(endpoint: &str) -> BidiResult<Self> {
        let ws_url = bidi_ws_url(endpoint);
        let session = BidiSession::connect(&ws_url).await?;
        Ok(Self {
            session,
            process: None,
        })
    }

    /// The underlying BiDi session.
    pub fn session(&self) -> &BidiSession {
        &self.session
    }

    /// The browser version reported by BiDi, if any.
    pub fn browser_version(&self) -> Option<&str> {
        self.session.browser_version()
    }

    /// Open a new tab in the default user context.
    pub async fn new_page(&self) -> BidiResult<BidiPage> {
        let context = self.session.create_context().await?;
        let page = BidiPage::from_context(self.session.clone(), context);
        page.install_helper().await?;
        Ok(page)
    }

    /// Create an isolated user context (its own cookies, storage and cache).
    pub async fn new_context(&self) -> BidiResult<BidiContext> {
        let user_context = self.session.create_user_context().await?;
        Ok(BidiContext {
            session: self.session.clone(),
            user_context,
        })
    }

    /// The current top-level pages (tabs).
    pub async fn pages(&self) -> BidiResult<Vec<BidiPage>> {
        let mut pages = Vec::new();
        for context in self.session.get_tree().await? {
            if context.parent.is_none() {
                let page = BidiPage::from_context(self.session.clone(), context.context);
                let _ = page.install_helper().await;
                pages.push(page);
            }
        }
        Ok(pages)
    }

    /// End the session and terminate the launched browser, if any.
    pub async fn close(mut self) -> BidiResult<()> {
        let _ = self.session.end().await;
        if let Some(mut process) = self.process.take() {
            process.kill();
        }
        self.session.connection().close();
        Ok(())
    }
}

/// An isolated user context (its own cookies, storage and cache).
///
/// Created with [`BidiBrowser::new_context`]; it is the BiDi equivalent of a
/// Chromium browser context.
#[derive(Clone)]
pub struct BidiContext {
    session: BidiSession,
    user_context: String,
}

impl std::fmt::Debug for BidiContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiContext")
            .field("user_context", &self.user_context)
            .finish()
    }
}

impl BidiContext {
    /// The BiDi user context id.
    pub fn user_context_id(&self) -> &str {
        &self.user_context
    }

    /// Open a page (tab) in this isolated context.
    pub async fn new_page(&self) -> BidiResult<BidiPage> {
        let context = self
            .session
            .create_context_in(Some(&self.user_context))
            .await?;
        let page = BidiPage::from_context(self.session.clone(), context);
        page.install_helper().await?;
        Ok(page)
    }

    /// Pages (tabs) in this context.
    pub async fn pages(&self) -> BidiResult<Vec<BidiPage>> {
        let mut pages = Vec::new();
        for info in self.session.get_tree().await? {
            if info.parent.is_none()
                && info.user_context.as_deref() == Some(self.user_context.as_str())
            {
                let page = BidiPage::from_context(self.session.clone(), info.context);
                let _ = page.install_helper().await;
                pages.push(page);
            }
        }
        Ok(pages)
    }

    /// Remove this context and its pages.
    pub async fn close(&self) -> BidiResult<()> {
        self.session.remove_user_context(&self.user_context).await
    }
}

/// A page (browsing context) driven over WebDriver BiDi.
#[derive(Clone)]
pub struct BidiPage {
    session: BidiSession,
    context: String,
    helper_contexts: Arc<Mutex<HashSet<String>>>,
    network: Arc<Mutex<Vec<BidiNetworkRequest>>>,
    network_started: Arc<AtomicBool>,
    network_pump: Arc<Mutex<Option<NetworkPumpGuard>>>,
    routes: Arc<Mutex<Vec<Route>>>,
    intercept: Arc<Mutex<Option<String>>>,
    intercept_pump: Arc<Mutex<Option<NetworkPumpGuard>>>,
}

impl std::fmt::Debug for BidiPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiPage")
            .field("context", &self.context)
            .finish()
    }
}

impl BidiPage {
    fn from_context(session: BidiSession, context: String) -> Self {
        Self {
            session,
            context,
            helper_contexts: Arc::new(Mutex::new(HashSet::new())),
            network: Arc::new(Mutex::new(Vec::new())),
            network_started: Arc::new(AtomicBool::new(false)),
            network_pump: Arc::new(Mutex::new(None)),
            routes: Arc::new(Mutex::new(Vec::new())),
            intercept: Arc::new(Mutex::new(None)),
            intercept_pump: Arc::new(Mutex::new(None)),
        }
    }

    /// The BiDi browsing context id.
    pub fn context_id(&self) -> &str {
        &self.context
    }

    /// Navigate to `url` and wait for the load to complete.
    pub async fn goto(&self, url: &str) -> BidiResult<()> {
        self.helper_contexts
            .lock()
            .expect("bidi helper mutex poisoned")
            .clear();
        self.session.navigate(&self.context, url).await?;
        self.install_helper().await?;
        Ok(())
    }

    /// The current URL.
    pub async fn url(&self) -> BidiResult<String> {
        Ok(self
            .session
            .get_context(&self.context)
            .await?
            .map(|info| info.url)
            .unwrap_or_default())
    }

    /// The document title.
    pub async fn title(&self) -> BidiResult<String> {
        Ok(self
            .session
            .evaluate(&self.context, "document.title")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    /// The serialized HTML of the page, including the doctype.
    pub async fn content(&self) -> BidiResult<String> {
        let expression = "(() => { const dt = document.doctype; \
             const prefix = dt ? '<!DOCTYPE ' + dt.name + '>' : ''; \
             const root = document.documentElement; \
             return root ? prefix + root.outerHTML : ''; })()";
        Ok(self
            .session
            .evaluate(&self.context, expression)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    /// Evaluate an expression in the page.
    pub async fn evaluate(&self, expression: &str) -> BidiResult<Value> {
        self.evaluate_in(&self.context, expression).await
    }

    /// Evaluate an expression in a specific browsing context (for example a frame).
    pub async fn evaluate_in(&self, context: &str, expression: &str) -> BidiResult<Value> {
        self.session.evaluate(context, expression).await
    }

    // -- Frames -------------------------------------------------------------

    /// The page's main frame.
    pub fn main_frame(&self) -> BidiFrame {
        BidiFrame::new(self.clone(), self.context.clone(), String::new())
    }

    /// Every child frame of the page, depth-first.
    pub async fn frames(&self) -> BidiResult<Vec<BidiFrame>> {
        let roots = self.session.get_tree_from(&self.context).await?;
        let mut frames = Vec::new();
        for root in &roots {
            collect_frames(self, root, &mut frames);
        }
        Ok(frames)
    }

    /// Resolve an `<iframe>` element to its current frame.
    ///
    /// BiDi does not expose a direct element-to-frame mapping, so this matches
    /// the element's resolved `src` against the frame tree.
    pub async fn frame_locator(&self, selector: impl AsRef<str>) -> BidiResult<BidiFrame> {
        let selector = selector.as_ref();
        let expression = format!(
            "(() => {{ const el = document.querySelector({}); \
             if (!el) return null; return {{ src: el.src || '', name: el.name || '' }}; }})()",
            serde_json::to_string(selector)?
        );
        let value = self.evaluate(&expression).await?;
        if value.is_null() {
            return Err(BidiError::ElementNotFound(format!("iframe {selector:?}")));
        }
        let src = value
            .get("src")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let frames = self.frames().await?;
        frames
            .into_iter()
            .find(|frame| {
                !src.is_empty()
                    && (frame.url() == src.as_str() || frame.url().contains(src.as_str()))
            })
            .ok_or_else(|| BidiError::ElementNotFound(format!("frame for {selector:?}")))
    }

    // -- Network diagnostics ------------------------------------------------

    /// Start collecting network events for this page.
    ///
    /// This is opt-in because it subscribes to `network.*` events for the whole
    /// session. Calling it more than once is a no-op.
    pub async fn start_network_monitoring(&self) -> BidiResult<()> {
        if self.network_started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Err(error) = self.session.ensure_network_subscription().await {
            self.network_started.store(false, Ordering::SeqCst);
            return Err(error);
        }
        let guard = spawn_network_pump(
            self.session.clone(),
            self.context.clone(),
            self.network.clone(),
        );
        *self
            .network_pump
            .lock()
            .expect("bidi network pump mutex poisoned") = Some(guard);
        Ok(())
    }

    /// Network requests observed since monitoring started.
    pub fn network_requests(&self) -> Vec<BidiNetworkRequest> {
        self.network
            .lock()
            .expect("bidi network mutex poisoned")
            .clone()
    }

    // -- Cookies and storage ------------------------------------------------

    /// Cookies visible to the page.
    pub async fn cookies(&self) -> BidiResult<Vec<BidiCookie>> {
        self.session.get_cookies(&self.context).await
    }

    /// Add a cookie for `domain`.
    pub async fn add_cookie(
        &self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
        domain: impl AsRef<str>,
    ) -> BidiResult<()> {
        self.session
            .set_cookie(
                &self.context,
                name.as_ref(),
                value.as_ref(),
                domain.as_ref(),
            )
            .await
    }

    /// Remove cookies for this context.
    pub async fn clear_cookies(&self) -> BidiResult<()> {
        self.session.delete_cookies(&self.context).await
    }

    /// Capture cookies and the current origin's local storage.
    pub async fn storage_state(&self) -> BidiResult<BidiStorageState> {
        let cookies = self.cookies().await?;
        let origin = self
            .evaluate("location.origin")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string();
        let raw = self
            .evaluate(
                "(() => { try { return JSON.stringify(Object.entries(localStorage)); } \
                 catch (error) { return '[]'; } })()",
            )
            .await?;
        let entries: Vec<(String, String)> = serde_json::from_str(raw.as_str().unwrap_or("[]"))?;
        let local_storage = entries
            .into_iter()
            .map(|(name, value)| BidiStorageItem { name, value })
            .collect();
        Ok(BidiStorageState {
            cookies,
            origins: vec![BidiOriginStorage {
                origin,
                local_storage,
            }],
        })
    }

    /// Restore cookies and local storage from a [`BidiStorageState`].
    pub async fn restore_storage_state(&self, state: &BidiStorageState) -> BidiResult<()> {
        for cookie in &state.cookies {
            let _ = self
                .session
                .set_cookie(&self.context, &cookie.name, &cookie.value, &cookie.domain)
                .await;
        }
        let origin = self
            .evaluate("location.origin")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string();
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

    // -- Request interception ----------------------------------------------

    /// Add a request interception rule (wildcards via `*` / `?`).
    ///
    /// The first matching rule wins; unmatched requests continue unchanged.
    /// Interception is generic and contains no site-specific behaviour.
    pub async fn route(&self, pattern: impl Into<String>, action: RouteAction) -> BidiResult<()> {
        let first = {
            let mut routes = self.routes.lock().expect("bidi routes mutex poisoned");
            let first = routes.is_empty();
            routes.push(Route::new(pattern, action));
            first
        };
        if first {
            self.session.ensure_network_subscription().await?;
            let intercept = self.session.add_intercept(&self.context).await?;
            *self
                .intercept
                .lock()
                .expect("bidi intercept mutex poisoned") = Some(intercept);
            let guard = spawn_intercept_pump(
                self.session.clone(),
                self.context.clone(),
                self.routes.clone(),
            );
            *self
                .intercept_pump
                .lock()
                .expect("bidi intercept pump mutex poisoned") = Some(guard);
        }
        Ok(())
    }

    /// Abort requests matching `pattern`.
    pub async fn block(&self, pattern: impl Into<String>) -> BidiResult<()> {
        self.route(pattern, RouteAction::Abort).await
    }

    /// Answer requests matching `pattern` with a synthetic response.
    pub async fn mock(
        &self,
        pattern: impl Into<String>,
        status: i64,
        content_type: impl Into<String>,
        body: impl Into<Vec<u8>>,
    ) -> BidiResult<()> {
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
    pub async fn clear_routes(&self) -> BidiResult<()> {
        let had_routes = {
            let mut routes = self.routes.lock().expect("bidi routes mutex poisoned");
            let had = !routes.is_empty();
            routes.clear();
            had
        };
        if had_routes {
            let intercept = self
                .intercept
                .lock()
                .expect("bidi intercept mutex poisoned")
                .take();
            if let Some(intercept) = intercept {
                let _ = self.session.remove_intercept(&intercept).await;
            }
            *self
                .intercept_pump
                .lock()
                .expect("bidi intercept pump mutex poisoned") = None;
        }
        Ok(())
    }

    // -- Locators -----------------------------------------------------------

    /// Create a locator for a CSS selector.
    pub fn locator(&self, selector: impl Into<Selector>) -> BidiLocator {
        BidiLocator::new(self.clone(), selector.into())
    }

    /// Create a locator that matches elements by their text.
    pub fn get_by_text(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::text(text, false))
    }

    /// Create a locator that matches elements by their exact text.
    pub fn get_by_text_exact(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::text(text, true))
    }

    /// Create a semantic locator by ARIA role and optional accessible name.
    pub fn get_by_role(&self, role: Role, name: Option<&str>) -> BidiLocator {
        self.locator(Selector::role(role, name))
    }

    /// Create a locator by `placeholder` attribute.
    pub fn get_by_placeholder(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::placeholder(text, false))
    }

    /// Create a locator by associated `<label>`.
    pub fn get_by_label(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::label(text, false))
    }

    /// Create a locator by image `alt` text.
    pub fn get_by_alt_text(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::alt_text(text, false))
    }

    /// Create a locator by test id (`data-testid`, `data-test-id`, `data-test`).
    pub fn get_by_test_id(&self, id: impl Into<String>) -> BidiLocator {
        self.locator(Selector::test_id(id))
    }

    // -- Internal helpers ---------------------------------------------------

    pub(crate) async fn install_helper(&self) -> BidiResult<()> {
        self.ensure_helper_in(&self.context).await
    }

    pub(crate) async fn ensure_helper_in(&self, context: &str) -> BidiResult<()> {
        if self
            .helper_contexts
            .lock()
            .expect("bidi helper mutex poisoned")
            .contains(context)
        {
            return Ok(());
        }
        self.session.evaluate(context, INJECTED_SCRIPT).await?;
        self.helper_contexts
            .lock()
            .expect("bidi helper mutex poisoned")
            .insert(context.to_string());
        Ok(())
    }

    pub(crate) async fn pointer_click(&self, x: f64, y: f64) -> BidiResult<()> {
        self.session
            .perform_actions(
                &self.context,
                json!([{
                    "type": "pointer",
                    "id": "rustwright-mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        { "type": "pointerMove", "x": x, "y": y, "origin": "viewport" },
                        { "type": "pointerDown", "button": 0 },
                        { "type": "pointerUp", "button": 0 }
                    ]
                }]),
            )
            .await
    }

    pub(crate) async fn pointer_move(&self, x: f64, y: f64) -> BidiResult<()> {
        self.session
            .perform_actions(
                &self.context,
                json!([{
                    "type": "pointer",
                    "id": "rustwright-mouse",
                    "parameters": { "pointerType": "mouse" },
                    "actions": [
                        { "type": "pointerMove", "x": x, "y": y, "origin": "viewport" }
                    ]
                }]),
            )
            .await
    }

    pub(crate) async fn press_key(&self, key: &str) -> BidiResult<()> {
        let value = key_value(key);
        self.session
            .perform_actions(
                &self.context,
                json!([{
                    "type": "key",
                    "id": "rustwright-keyboard",
                    "actions": [
                        { "type": "keyDown", "value": value },
                        { "type": "keyUp", "value": value }
                    ]
                }]),
            )
            .await
    }

    pub(crate) async fn type_text(&self, text: &str) -> BidiResult<()> {
        let mut actions = Vec::new();
        for character in text.chars() {
            let value = character.to_string();
            actions.push(json!({ "type": "keyDown", "value": value }));
            actions.push(json!({ "type": "keyUp", "value": value }));
        }
        if actions.is_empty() {
            return Ok(());
        }
        self.session
            .perform_actions(
                &self.context,
                json!([{
                    "type": "key",
                    "id": "rustwright-keyboard",
                    "actions": actions
                }]),
            )
            .await
    }

    /// Take a screenshot and write it to `path` (PNG).
    pub async fn screenshot(&self, path: impl AsRef<Path>) -> BidiResult<()> {
        let bytes = self.session.screenshot(&self.context).await?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    /// Override the viewport size.
    pub async fn set_viewport(
        &self,
        width: i64,
        height: i64,
        device_pixel_ratio: f64,
    ) -> BidiResult<()> {
        self.session
            .set_viewport(&self.context, width, height, device_pixel_ratio)
            .await
    }

    /// Close this page.
    pub async fn close(&self) -> BidiResult<()> {
        self.session.close_context(&self.context).await
    }
}

fn bidi_ws_url(endpoint: &str) -> String {
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return endpoint.to_string();
    }
    let trimmed = endpoint
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    format!("ws://{trimmed}/session")
}

/// Map a friendly key name to the BiDi key value (a character or a Unicode PUA
/// code point for special keys).
fn key_value(key: &str) -> String {
    match key {
        "Enter" | "\n" => "\u{E007}".to_string(),
        "Tab" => "\u{E004}".to_string(),
        "Escape" | "Esc" => "\u{E00C}".to_string(),
        "Backspace" => "\u{E003}".to_string(),
        "Delete" => "\u{E017}".to_string(),
        "ArrowUp" | "Up" => "\u{E013}".to_string(),
        "ArrowDown" | "Down" => "\u{E015}".to_string(),
        "ArrowLeft" | "Left" => "\u{E012}".to_string(),
        "ArrowRight" | "Right" => "\u{E014}".to_string(),
        " " | "Space" => " ".to_string(),
        other => other.to_string(),
    }
}
