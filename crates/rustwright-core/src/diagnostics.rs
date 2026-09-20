//! Observability primitives: console messages, page errors and network traffic.
//!
//! These exist so that when a page behaves differently under Rustwright than in
//! a normal browser, the environment delta can be identified instead of guessed.

use std::path::PathBuf;

use rustwright_cdp::protocol::network::{Cookie, RequestWillBeSentParams, Response};
use serde::{Deserialize, Serialize};

/// A console message emitted by the page.
#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    /// The console method (`log`, `warn`, `error`, ...).
    pub level: String,
    /// The flattened message text.
    pub text: String,
    /// Monotonic timestamp in milliseconds.
    pub timestamp: f64,
}

/// An uncaught JavaScript error in the page.
#[derive(Debug, Clone)]
pub struct PageError {
    /// The error message.
    pub message: String,
    /// Monotonic timestamp in milliseconds.
    pub timestamp: f64,
}

/// A network request observed by the `Network` domain.
#[derive(Debug, Clone)]
pub struct NetworkRequest {
    /// CDP request id.
    pub request_id: String,
    /// Request URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Resource type (`Document`, `Script`, ...).
    pub resource_type: String,
    /// Request headers.
    pub request_headers: serde_json::Map<String, serde_json::Value>,
    /// Response headers once known.
    pub response_headers: serde_json::Map<String, serde_json::Value>,
    /// Response status once known.
    pub status: Option<i64>,
    /// Response status text once known.
    pub status_text: Option<String>,
    /// Response MIME type once known.
    pub mime_type: Option<String>,
    /// Failure text if the request failed.
    pub failure: Option<String>,
}

impl NetworkRequest {
    pub(crate) fn from_request(params: &RequestWillBeSentParams) -> Self {
        Self {
            request_id: params.request_id.clone(),
            url: params.request.url.clone(),
            method: params.request.method.clone(),
            resource_type: String::new(),
            request_headers: params.request.headers.clone(),
            response_headers: serde_json::Map::new(),
            status: None,
            status_text: None,
            mime_type: None,
            failure: None,
        }
    }

    pub(crate) fn apply_response(&mut self, response: &Response) {
        self.status = Some(response.status);
        self.status_text = Some(response.status_text.clone());
        self.mime_type = Some(response.mime_type.clone());
        self.response_headers = response.headers.clone();
    }
}

/// A committed main-frame navigation.
#[derive(Debug, Clone)]
pub struct NavigationEvent {
    /// The URL that was navigated to.
    pub url: String,
    /// The frame id.
    pub frame_id: String,
    /// The loader id of the committed document.
    pub loader_id: Option<String>,
    /// The document MIME type.
    pub mime_type: String,
}

/// A JavaScript dialog (alert/confirm/prompt/beforeunload) observed on a page.
#[derive(Debug, Clone)]
pub struct DialogInfo {
    /// The dialog type.
    pub dialog_type: String,
    /// The dialog message.
    pub message: String,
    /// Default prompt text, if any.
    pub default_prompt: Option<String>,
}

/// A serialized `localStorage` item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageItem {
    /// Item name.
    pub name: String,
    /// Item value.
    pub value: String,
}

/// `localStorage` for a single origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OriginStorage {
    /// The origin, e.g. `https://example.com`.
    pub origin: String,
    /// The stored items.
    #[serde(default)]
    pub local_storage: Vec<StorageItem>,
}

/// A portable snapshot of cookies and local storage.
///
/// Used to reuse an authenticated session without copying an entire browser
/// profile. Local storage is captured for the page's current origin.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageState {
    /// Cookies visible to the page.
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    /// Per-origin local storage.
    #[serde(default)]
    pub origins: Vec<OriginStorage>,
}

/// A snapshot of how the browser was launched and what it is running.
#[derive(Debug, Clone)]
pub struct BrowserDiagnostics {
    /// The browser product and version string.
    pub product: String,
    /// The CDP protocol version.
    pub protocol_version: String,
    /// The browser user agent.
    pub user_agent: String,
    /// Path to the browser executable, if Rustwright launched it.
    pub executable: Option<PathBuf>,
    /// Launch flags used, if Rustwright launched the browser.
    pub launch_args: Vec<String>,
    /// The user data directory in use.
    pub user_data_dir: Option<PathBuf>,
    /// The DevTools HTTP endpoint.
    pub endpoint: Option<String>,
    /// The OS process id, if Rustwright launched the browser.
    pub pid: Option<u32>,
    /// Whether the profile is temporary.
    pub ephemeral_profile: bool,
    /// Path to the captured browser log, if any.
    pub browser_log: Option<PathBuf>,
}
