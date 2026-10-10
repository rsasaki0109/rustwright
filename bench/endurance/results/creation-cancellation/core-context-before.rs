//! Browser contexts: isolated browsing sessions.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustwright_cdp::protocol::browser::SetDownloadBehaviorParams;
use rustwright_cdp::protocol::target::{
    AttachToTargetParams, AttachToTargetResult, CreateTargetParams, CreateTargetResult,
    DisposeBrowserContextParams, GetBrowserContextsResult, GetTargetsResult, TargetCreatedParams,
    TargetInfo,
};
use rustwright_cdp::CdpConnection;
use serde_json::json;
use tokio::sync::{broadcast, watch, Mutex as AsyncMutex};

use crate::error::{Error, Result};
use crate::page::{Page, Viewport};

/// An isolated browser session, equivalent to Playwright's `BrowserContext`.
///
/// The default context maps to the browser's own profile. Contexts created with
/// [`crate::Browser::new_context`] are CDP browser contexts (incognito-like):
/// they get their own cookies, storage and cache.
#[derive(Clone)]
pub struct BrowserContext {
    connection: CdpConnection,
    browser_context_id: Option<String>,
    pages: Arc<Mutex<HashMap<String, Page>>>,
    viewport: Arc<Mutex<Option<Viewport>>>,
    closed: Arc<AtomicBool>,
    closure: watch::Sender<()>,
    attachments: Arc<AsyncMutex<()>>,
}

// If page initialization fails or its waiter is cancelled, release the new
// session instead of leaving an untracked debugger attached to the target.
struct AttachmentGuard {
    connection: CdpConnection,
    session_id: Option<String>,
}

impl Drop for AttachmentGuard {
    fn drop(&mut self) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let connection = self.connection.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = connection
                    .send_raw(
                        None,
                        "Target.detachFromTarget",
                        json!({
                            "sessionId": session_id,
                        }),
                    )
                    .await;
            });
        }
    }
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("browser_context_id", &self.browser_context_id)
            .field("is_default", &self.is_default())
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl BrowserContext {
    pub(crate) fn new(connection: CdpConnection, browser_context_id: Option<String>) -> Self {
        let (closure, _) = watch::channel(());
        Self {
            connection,
            browser_context_id,
            pages: Arc::new(Mutex::new(HashMap::new())),
            viewport: Arc::new(Mutex::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
            closure,
            attachments: Arc::new(AsyncMutex::new(())),
        }
    }

    /// The CDP browser context id, or `None` for the default context.
    pub fn browser_context_id(&self) -> Option<&str> {
        self.browser_context_id.as_deref()
    }

    /// Whether this is the browser's default context.
    pub fn is_default(&self) -> bool {
        self.browser_context_id.is_none()
    }

    /// Whether the context has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.connection.is_closed()
    }

    /// Set a default viewport applied to existing and future pages.
    ///
    /// A stable viewport makes SPA rendering deterministic across machines.
    pub async fn set_viewport(&self, viewport: Viewport) -> Result<()> {
        self.ensure_open()?;
        *self.viewport.lock().expect("viewport mutex poisoned") = Some(viewport);
        let pages: Vec<Page> = self
            .pages
            .lock()
            .expect("pages mutex poisoned")
            .values()
            .cloned()
            .collect();
        for page in pages {
            let _ = page.set_viewport(&viewport).await;
        }
        Ok(())
    }

    /// The configured default viewport, if any.
    pub fn viewport(&self) -> Option<Viewport> {
        *self.viewport.lock().expect("viewport mutex poisoned")
    }

    /// Allow downloads and write them to `path`.
    ///
    /// The directory is created if it does not exist. Files keep the names the
    /// server suggests; use `events_enabled` progress events for advanced needs.
    pub async fn set_download_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.ensure_open()?;
        let path = path.as_ref();
        tokio::fs::create_dir_all(path).await?;
        let params = SetDownloadBehaviorParams {
            behavior: "allow".to_string(),
            browser_context_id: self.browser_context_id.clone(),
            download_path: Some(path.display().to_string()),
            events_enabled: Some(true),
        };
        self.connection
            .send_raw(None, "Browser.setDownloadBehavior", json!(params))
            .await?;
        Ok(())
    }

    /// Open a new page in this context.
    pub async fn new_page(&self) -> Result<Page> {
        let _attachment = self.attachments.lock().await;
        self.ensure_open()?;
        let params = CreateTargetParams {
            url: "about:blank".to_string(),
            browser_context_id: self.browser_context_id.clone(),
            new_window: None,
            background: None,
        };
        let created: CreateTargetResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.createTarget", json!(params))
                .await?,
        )?;
        if let Err(error) = self.ensure_open() {
            let _ = self
                .connection
                .send_raw(
                    None,
                    "Target.closeTarget",
                    json!({
                        "targetId": created.target_id,
                    }),
                )
                .await;
            return Err(error);
        }
        self.attach_target(&created.target_id).await
    }

    /// Pages currently tracked by this context.
    pub fn pages(&self) -> Vec<Page> {
        let mut pages = self.pages.lock().expect("pages mutex poisoned");
        pages.retain(|_, page| !page.is_closed());
        pages.values().cloned().collect()
    }

    /// Discover and attach pages that already exist in this context.
    ///
    /// This is useful with [`crate::Browser::connect`], when the browser was
    /// already running and has tabs open.
    pub async fn refresh_pages(&self) -> Result<Vec<Page>> {
        self.ensure_open()?;
        for target in self.page_targets().await? {
            self.attach_new_target(&target.target_id).await?;
        }
        self.ensure_open()?;
        Ok(self.pages())
    }

    /// Wait for a new page to open (a popup, `window.open`, or new tab).
    ///
    /// This relies on `Target.setDiscoverTargets`, which
    /// [`crate::Browser::launch`] and [`crate::Browser::connect`] enable.
    /// The timeout covers discovery and attachment as well as event waiting.
    /// Closing the context or disconnecting the browser wakes pending waits.
    pub async fn wait_for_page(&self, timeout: Duration) -> Result<Page> {
        // Subscribe before taking the snapshot: a popup may open while discovery
        // is in flight. The same rule prevents losing a concurrent close.
        let mut receiver = self.connection.subscribe();
        let mut closure = self.closure.subscribe();
        self.ensure_open()?;
        let result = tokio::select! {
            result = tokio::time::timeout(timeout, self.wait_for_page_inner(&mut receiver)) => {
                result.unwrap_or_else(|_| Err(Error::Timeout {
                    what: "a new page".to_string(), timeout,
                }))
            },
            _ = closure.changed() => Err(Error::ContextClosed),
        };
        // A disconnect during discovery/attachment may surface as a CDP error.
        self.ensure_open()?;
        result
    }

    async fn wait_for_page_inner(
        &self,
        receiver: &mut broadcast::Receiver<rustwright_cdp::CdpEvent>,
    ) -> Result<Page> {
        for target in self.page_targets().await? {
            if let Some(page) = self.attach_new_target(&target.target_id).await? {
                return Ok(page);
            }
        }
        loop {
            self.ensure_open()?;
            match receiver.recv().await {
                Ok(event) => {
                    if event.is_disconnected() {
                        return Err(Error::BrowserClosed);
                    }
                    if event.method != "Target.targetCreated" {
                        continue;
                    }
                    let Ok(params) =
                        serde_json::from_value::<TargetCreatedParams>(event.params.clone())
                    else {
                        continue;
                    };
                    let target = params.target_info;
                    if target.target_type != "page"
                        || !self.matches_context(&target, &self.isolated_context_ids().await?)
                    {
                        continue;
                    }
                    if let Some(page) = self.attach_new_target(&target.target_id).await? {
                        return Ok(page);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Recover missed target-created events from a fresh snapshot.
                    for target in self.page_targets().await? {
                        if let Some(page) = self.attach_new_target(&target.target_id).await? {
                            return Ok(page);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return Err(Error::BrowserClosed),
            }
        }
    }

    /// Close every page in this context and dispose the context.
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.closure.send_replace(());
        let _attachment = self.attachments.lock().await;
        let pages: Vec<Page> = self
            .pages
            .lock()
            .expect("pages mutex poisoned")
            .drain()
            .map(|(_, page)| page)
            .collect();
        for page in pages {
            let _ = page.close().await;
        }
        if let Some(context_id) = &self.browser_context_id {
            let params = DisposeBrowserContextParams {
                browser_context_id: context_id.clone(),
            };
            let _ = self
                .connection
                .send_raw(None, "Target.disposeBrowserContext", json!(params))
                .await;
        }
        Ok(())
    }

    async fn attach_new_target(&self, target_id: &str) -> Result<Option<Page>> {
        let _attachment = self.attachments.lock().await;
        self.ensure_open()?;
        if self
            .pages
            .lock()
            .expect("pages mutex poisoned")
            .contains_key(target_id)
        {
            return Ok(None);
        }
        match self.attach_target(target_id).await {
            Ok(page) => Ok(Some(page)),
            // A discovered popup can disappear before it is attached. Other
            // protocol and initialization errors must remain visible.
            Err(Error::Cdp(rustwright_cdp::CdpError::Protocol {
                ref method,
                ref message,
                code: -32602,
                ..
            })) if method == "Target.attachToTarget"
                && message == "No target with given id found" =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    // Caller holds the attachment lock, preventing duplicate sessions/pumps.
    async fn attach_target(&self, target_id: &str) -> Result<Page> {
        let attach_params = AttachToTargetParams {
            target_id: target_id.to_string(),
            flatten: Some(true),
        };
        let attached: AttachToTargetResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.attachToTarget", json!(attach_params))
                .await?,
        )?;
        let mut attachment = AttachmentGuard {
            connection: self.connection.clone(),
            session_id: Some(attached.session_id.clone()),
        };
        let page = Page::attach(
            self.connection.clone(),
            attached.session_id,
            target_id.to_string(),
        )
        .await?;
        if let Err(error) = self.ensure_open() {
            page.close().await?;
            return Err(error);
        }
        if let Some(viewport) = self.viewport() {
            let _ = page.set_viewport(&viewport).await;
        }
        page.register_in(&self.pages)?;
        attachment.session_id = None;
        Ok(page)
    }

    async fn page_targets(&self) -> Result<Vec<TargetInfo>> {
        let targets: GetTargetsResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.getTargets", json!({}))
                .await?,
        )?;
        let isolated_contexts = self.isolated_context_ids().await?;
        Ok(targets
            .target_infos
            .into_iter()
            .filter(|target| {
                target.target_type == "page" && self.matches_context(target, &isolated_contexts)
            })
            .collect())
    }

    async fn isolated_context_ids(&self) -> Result<Vec<String>> {
        if self.browser_context_id.is_some() {
            return Ok(Vec::new());
        }
        let contexts: GetBrowserContextsResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.getBrowserContexts", json!({}))
                .await?,
        )?;
        Ok(contexts.browser_context_ids)
    }

    fn matches_context(&self, target: &TargetInfo, isolated_contexts: &[String]) -> bool {
        match &self.browser_context_id {
            // Chrome may give the default context an opaque id too. Its
            // distinguishing property is absence from getBrowserContexts,
            // which lists isolated contexts, including other clients' contexts.
            None => target
                .browser_context_id
                .as_ref()
                .is_none_or(|id| !isolated_contexts.contains(id)),
            Some(context_id) => target.browser_context_id.as_deref() == Some(context_id.as_str()),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::ContextClosed);
        }
        if self.connection.is_closed() {
            return Err(Error::BrowserClosed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
    use tokio_tungstenite::tungstenite::Message;

    #[derive(Clone, Copy)]
    enum Hold {
        Discovery,
        TargetDuringDiscovery,
        Initialization,
        FailInitialization,
        Live,
    }

    async fn mock_context(
        hold: Hold,
    ) -> (
        BrowserContext,
        mpsc::UnboundedReceiver<Value>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if !message.is_text() {
                    continue;
                }
                let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                tx.send(command.clone()).unwrap();
                let result = match command["method"].as_str().unwrap() {
                    "Target.createTarget" if matches!(hold, Hold::Live) => {
                        json!({"targetId":format!("tab-{}",command["id"])})
                    }
                    "Target.attachToTarget" if matches!(hold, Hold::Live) => {
                        json!({"sessionId":format!("session-{}",command["params"]["targetId"].as_str().unwrap())})
                    }
                    "Page.getFrameTree" if matches!(hold, Hold::Live) => {
                        json!({"frameTree":{"frame":{"id":"main","url":"about:blank","mimeType":"text/html"}}})
                    }
                    "Test.detach" if matches!(hold, Hold::Live) => {
                        socket.send(Message::Text(json!({"method":"Target.detachedFromTarget","params":{"sessionId":command["params"]["sessionId"]}}).to_string().into())).await.unwrap();
                        json!({})
                    }
                    _ if matches!(hold, Hold::Live) => json!({}),
                    "Target.getTargets" if matches!(hold, Hold::Discovery) => continue,
                    "Target.getTargets" if matches!(hold, Hold::TargetDuringDiscovery) => {
                        socket.send(Message::Text(json!({
                            "method": "Target.targetCreated",
                            "params": {"targetInfo": {
                                "targetId": "popup", "type": "page", "browserContextId": "isolated",
                            }},
                        }).to_string().into())).await.unwrap();
                        json!({"targetInfos": []})
                    }
                    "Target.getTargets" => json!({"targetInfos": [{
                        "targetId": "popup", "type": "page", "browserContextId": "isolated",
                    }]}),
                    "Target.attachToTarget" => json!({"sessionId": "attachment"}),
                    "Page.enable" if matches!(hold, Hold::Initialization) => continue,
                    "Page.enable" => {
                        socket
                            .send(Message::Text(
                                json!({"id": command["id"], "error": {
                                    "code": -32000, "message": "initialization failed",
                                }})
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                        continue;
                    }
                    "Target.detachFromTarget" => json!({}),
                    method => panic!("unexpected mock command: {method}"),
                };
                socket
                    .send(Message::Text(
                        json!({"id": command["id"], "result": result})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
        });
        let connection = CdpConnection::connect(&url).await.unwrap();
        (
            BrowserContext::new(connection, Some("isolated".into())),
            rx,
            server,
        )
    }

    async fn observed(rx: &mut mpsc::UnboundedReceiver<Value>, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let command = rx.recv().await.expect("mock connection is live");
                if command["method"] == method {
                    return command;
                }
            }
        })
        .await
        .expect("expected command must arrive")
    }

    #[tokio::test]
    async fn popup_created_during_discovery_is_not_lost() {
        let (context, mut rx, server) = mock_context(Hold::TargetDuringDiscovery).await;
        // The snapshot is empty. Only the event sent before its response can
        // lead to attachment and the deliberate initialization error.
        let result = context.wait_for_page(Duration::from_secs(1)).await;
        assert!(
            matches!(result, Err(Error::Cdp(rustwright_cdp::CdpError::Protocol { ref method, .. })) if method == "Page.enable"),
            "{result:?}"
        );
        observed(&mut rx, "Target.detachFromTarget").await;
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn page_wait_deadline_includes_stalled_discovery() {
        let (context, mut rx, server) = mock_context(Hold::Discovery).await;
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            context.wait_for_page(Duration::from_millis(100)),
        )
        .await
        .expect("whole operation obeys deadline");
        assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
        observed(&mut rx, "Target.getTargets").await;
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn page_wait_timeout_detaches_partial_initialization() {
        let (context, mut rx, server) = mock_context(Hold::Initialization).await;
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            context.wait_for_page(Duration::from_millis(100)),
        )
        .await
        .expect("attachment obeys deadline");
        assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
        let detach = observed(&mut rx, "Target.detachFromTarget").await;
        assert_eq!(detach["params"]["sessionId"], "attachment");
        assert!(context.pages().is_empty());
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn cancelled_page_wait_detaches_partial_initialization() {
        let (context, mut rx, server) = mock_context(Hold::Initialization).await;
        let waiting = context.clone();
        let task = tokio::spawn(async move { waiting.wait_for_page(Duration::from_secs(5)).await });
        observed(&mut rx, "Page.enable").await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let detach = observed(&mut rx, "Target.detachFromTarget").await;
        assert_eq!(detach["params"]["sessionId"], "attachment");
        assert!(context.pages().is_empty());
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn discovery_returns_initialization_errors_and_detaches_the_session() {
        let (context, mut rx, server) = mock_context(Hold::FailInitialization).await;
        let result = context.refresh_pages().await;
        assert!(
            matches!(result, Err(Error::Cdp(rustwright_cdp::CdpError::Protocol { ref method, code: -32000, .. })) if method == "Page.enable"),
            "{result:?}"
        );
        observed(&mut rx, "Target.detachFromTarget").await;
        assert!(context.pages().is_empty());
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn explicit_close_releases_registry_without_discovery() {
        let (context, _rx, server) = mock_context(Hold::Live).await;
        let keep = context.new_page().await.unwrap();
        for _ in 0..20 {
            let page = context.new_page().await.unwrap();
            let retained = page.clone();
            page.close().await.unwrap();
            assert_eq!(
                context.pages.lock().unwrap().len(),
                1,
                "closed pages must be removed without calling pages() or refresh_pages()"
            );
            assert!(retained.is_closed());
            assert!(!keep.is_closed());
        }
        keep.close().await.unwrap();
        assert!(context.pages.lock().unwrap().is_empty());
        context.connection.close();
        server.abort();
    }

    async fn registry_empty(context: &BrowserContext) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if context.pages.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closure must release the registry without discovery");
    }

    #[tokio::test]
    async fn external_detach_releases_registry_with_retained_handles() {
        let (context, _rx, server) = mock_context(Hold::Live).await;
        let page = context.new_page().await.unwrap();
        context
            .connection
            .send_raw(
                None,
                "Test.detach",
                json!({"sessionId":format!("session-{}",page.target_id())}),
            )
            .await
            .unwrap();
        registry_empty(&context).await;
        assert!(page.is_closed());
        page.close().await.unwrap();
        context.connection.close();
        server.abort();
    }

    #[tokio::test]
    async fn disconnect_releases_registry_with_retained_handles() {
        let (context, _rx, server) = mock_context(Hold::Live).await;
        let page = context.new_page().await.unwrap();
        context.connection.close();
        registry_empty(&context).await;
        assert!(page.is_closed());
        server.abort();
    }
}
