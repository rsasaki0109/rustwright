//! Browser contexts: isolated browsing sessions.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustwright_cdp::protocol::browser::SetDownloadBehaviorParams;
use rustwright_cdp::protocol::target::{
    AttachToTargetParams, AttachToTargetResult, CreateTargetParams, CreateTargetResult,
    DisposeBrowserContextParams, GetTargetsResult, TargetCreatedParams, TargetInfo,
};
use rustwright_cdp::CdpConnection;
use serde_json::json;
use tokio::sync::broadcast;

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
        Self {
            connection,
            browser_context_id,
            pages: Arc::new(Mutex::new(HashMap::new())),
            viewport: Arc::new(Mutex::new(None)),
            closed: Arc::new(AtomicBool::new(false)),
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
        self.attach_target(&created.target_id).await
    }

    /// Pages currently tracked by this context.
    pub fn pages(&self) -> Vec<Page> {
        self.pages
            .lock()
            .expect("pages mutex poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Discover and attach pages that already exist in this context.
    ///
    /// This is useful with [`crate::Browser::connect`], when the browser was
    /// already running and has tabs open.
    pub async fn refresh_pages(&self) -> Result<Vec<Page>> {
        self.ensure_open()?;
        for target in self.page_targets().await? {
            if self
                .pages
                .lock()
                .expect("pages mutex poisoned")
                .contains_key(&target.target_id)
            {
                continue;
            }
            let _ = self.attach_target(&target.target_id).await;
        }
        Ok(self.pages())
    }

    /// Wait for a new page to open (a popup, `window.open`, or new tab).
    ///
    /// This relies on `Target.setDiscoverTargets`, which
    /// [`crate::Browser::launch`] and [`crate::Browser::connect`] enable.
    pub async fn wait_for_page(&self, timeout: Duration) -> Result<Page> {
        self.ensure_open()?;
        // Attach first so we do not miss an already-created target.
        for target in self.page_targets().await? {
            if self
                .pages
                .lock()
                .expect("pages mutex poisoned")
                .contains_key(&target.target_id)
            {
                continue;
            }
            if let Ok(page) = self.attach_target(&target.target_id).await {
                return Ok(page);
            }
        }

        let mut receiver = self.connection.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout {
                    what: "a new page".to_string(),
                    timeout,
                });
            }
            match tokio::time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(event)) => {
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
                        || !self.matches_context(&target)
                        || self
                            .pages
                            .lock()
                            .expect("pages mutex poisoned")
                            .contains_key(&target.target_id)
                    {
                        continue;
                    }
                    if let Ok(page) = self.attach_target(&target.target_id).await {
                        return Ok(page);
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => return Err(Error::BrowserClosed),
                Err(_) => {
                    return Err(Error::Timeout {
                        what: "a new page".to_string(),
                        timeout,
                    })
                }
            }
        }
    }

    /// Close every page in this context and dispose the context.
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
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
        let page = Page::attach(
            self.connection.clone(),
            attached.session_id,
            target_id.to_string(),
        )
        .await?;
        if let Some(viewport) = self.viewport() {
            let _ = page.set_viewport(&viewport).await;
        }
        self.pages
            .lock()
            .expect("pages mutex poisoned")
            .insert(target_id.to_string(), page.clone());
        Ok(page)
    }

    async fn page_targets(&self) -> Result<Vec<TargetInfo>> {
        let targets: GetTargetsResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.getTargets", json!({}))
                .await?,
        )?;
        Ok(targets
            .target_infos
            .into_iter()
            .filter(|target| target.target_type == "page" && self.matches_context(target))
            .collect())
    }

    fn matches_context(&self, target: &TargetInfo) -> bool {
        match &self.browser_context_id {
            // The default context has no known id up front, so it adopts every
            // page target. An isolated context matches exactly.
            None => true,
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
