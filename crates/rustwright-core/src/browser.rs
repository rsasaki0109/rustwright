//! The [`Browser`] entry point.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustwright_browser::{Chrome, LaunchedBrowser};
use rustwright_cdp::protocol::target::{CreateBrowserContextResult, GetVersionResult};
use rustwright_cdp::protocol::version::BrowserVersion;
use rustwright_cdp::{discover_ws_url, CdpConnection};
use serde_json::json;

use crate::context::BrowserContext;
use crate::diagnostics::BrowserDiagnostics;
use crate::error::{Error, Result};
use crate::page::Page;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// A running browser, either launched by Rustwright or connected to over CDP.
///
/// A `Browser` owns the default [`BrowserContext`] and, when it launched the
/// process, the browser process itself (killed on [`Browser::close`] or drop).
/// Cloning shares the same process and connection.
#[derive(Clone)]
pub struct Browser {
    connection: CdpConnection,
    version: BrowserVersion,
    process: Arc<Mutex<Option<LaunchedBrowser>>>,
    default_context: BrowserContext,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("product", &self.version.browser)
            .field("protocol_version", &self.version.protocol_version)
            .field("connected", &self.is_connected())
            .finish()
    }
}

impl Browser {
    /// Launch an installed Chrome/Chromium and connect to it.
    ///
    /// ```no_run
    /// # use rustwright_core::{Browser, Chrome};
    /// # async fn run() -> rustwright_core::Result<()> {
    /// let browser = Browser::launch(Chrome::installed().headless(true)).await?;
    /// # browser.close().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn launch(chrome: Chrome) -> Result<Self> {
        let launched = LaunchedBrowser::launch(&chrome).await?;
        let ws_url = launched.ws_url().to_string();
        let connection = CdpConnection::connect(&ws_url).await?;
        let version = fetch_version(&connection, &ws_url).await?;
        let _ = connection
            .send_raw(
                None,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
            )
            .await;
        let default_context = BrowserContext::new(connection.clone(), None);
        Ok(Self {
            connection,
            version,
            process: Arc::new(Mutex::new(Some(launched))),
            default_context,
        })
    }

    /// Connect to an already-running browser.
    ///
    /// `endpoint` may be an `http://host:port` DevTools endpoint or a browser
    /// `ws://` debugger URL. The browser must have been started with remote
    /// debugging enabled, for example
    /// `chrome --remote-debugging-port=9222 --user-data-dir=/tmp/profile`.
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let ws_url = discover_ws_url(endpoint).await?;
        let connection = CdpConnection::connect(&ws_url).await?;
        let version = fetch_version(&connection, &ws_url).await?;
        let _ = connection
            .send_raw(
                None,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
            )
            .await;
        let default_context = BrowserContext::new(connection.clone(), None);
        Ok(Self {
            connection,
            version,
            process: Arc::new(Mutex::new(None)),
            default_context,
        })
    }

    /// Browser version and protocol information.
    pub fn version(&self) -> &BrowserVersion {
        &self.version
    }

    /// Whether the connection to the browser is still open.
    pub fn is_connected(&self) -> bool {
        !self.connection.is_closed()
    }

    /// The browser's default context.
    pub fn default_context(&self) -> BrowserContext {
        self.default_context.clone()
    }

    /// Open a new page in the default context.
    pub async fn new_page(&self) -> Result<Page> {
        self.default_context.new_page().await
    }

    /// Create a new isolated context (its own cookies, storage and cache).
    pub async fn new_context(&self) -> Result<BrowserContext> {
        if !self.is_connected() {
            return Err(Error::BrowserClosed);
        }
        let created: CreateBrowserContextResult = serde_json::from_value(
            self.connection
                .send_raw(None, "Target.createBrowserContext", json!({}))
                .await?,
        )?;
        Ok(BrowserContext::new(
            self.connection.clone(),
            Some(created.browser_context_id),
        ))
    }

    /// Pages in the default context, discovering existing tabs on first call.
    pub async fn pages(&self) -> Result<Vec<Page>> {
        self.default_context.refresh_pages().await
    }

    /// A snapshot of how the browser was launched and what it is running.
    pub fn diagnostics(&self) -> BrowserDiagnostics {
        let process = self.process.lock().expect("process mutex poisoned");
        BrowserDiagnostics {
            product: self.version.browser.clone(),
            protocol_version: self.version.protocol_version.clone(),
            user_agent: self.version.user_agent.clone(),
            executable: process
                .as_ref()
                .map(|launched| launched.executable().to_path_buf()),
            launch_args: process
                .as_ref()
                .map(|launched| launched.launch_args().to_vec())
                .unwrap_or_default(),
            user_data_dir: process
                .as_ref()
                .map(|launched| launched.user_data_dir().to_path_buf()),
            endpoint: process
                .as_ref()
                .map(|launched| launched.endpoint().base_url()),
            pid: process.as_ref().map(LaunchedBrowser::pid),
            ephemeral_profile: process
                .as_ref()
                .map(LaunchedBrowser::is_ephemeral)
                .unwrap_or(false),
            browser_log: process
                .as_ref()
                .and_then(|launched| launched.stderr_log().map(Path::to_path_buf)),
        }
    }

    /// Shut the browser down.
    ///
    /// For a browser launched by Rustwright this closes it gracefully (allowing
    /// the profile to flush) and then terminates the process. For a connected
    /// browser it only disconnects, leaving the user's browser running.
    pub async fn close(&self) -> Result<()> {
        let launched = self.process.lock().expect("process mutex poisoned").take();
        if let Some(mut launched) = launched {
            let _ = tokio::time::timeout(
                SHUTDOWN_TIMEOUT,
                self.connection.send_raw(None, "Browser.close", json!({})),
            )
            .await;
            launched.kill();
        }
        self.connection.close();
        Ok(())
    }
}

async fn fetch_version(connection: &CdpConnection, ws_url: &str) -> Result<BrowserVersion> {
    let result: GetVersionResult = serde_json::from_value(
        connection
            .send_raw(None, "Browser.getVersion", json!({}))
            .await?,
    )?;
    Ok(BrowserVersion {
        browser: result.product,
        protocol_version: result.protocol_version,
        user_agent: result.user_agent,
        v8_version: result.js_version,
        webkit_version: String::new(),
        web_socket_debugger_url: ws_url.to_string(),
    })
}
