//! Browser version diagnostics.

use serde::Deserialize;

use crate::error::CdpResult;
use crate::http::{http_get_json, HttpEndpoint};

/// Information returned by the DevTools `/json/version` endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserVersion {
    /// The browser product, for example `Chrome/150.0.7871.186`.
    #[serde(default)]
    pub browser: String,
    /// The CDP protocol version, for example `1.3`.
    #[serde(default, rename = "Protocol-Version")]
    pub protocol_version: String,
    /// Default user agent.
    #[serde(default, rename = "User-Agent")]
    pub user_agent: String,
    /// V8 version.
    #[serde(default, rename = "V8-Version")]
    pub v8_version: String,
    /// WebKit version.
    #[serde(default, rename = "WebKit-Version")]
    pub webkit_version: String,
    /// Browser-level WebSocket debugger URL.
    #[serde(default, rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: String,
}

impl BrowserVersion {
    /// Fetch version information from a DevTools HTTP endpoint.
    pub async fn fetch(endpoint: &HttpEndpoint) -> CdpResult<Self> {
        http_get_json(endpoint, "/json/version").await
    }
}
