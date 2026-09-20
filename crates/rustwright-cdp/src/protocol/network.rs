//! `Network` domain.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Parameters of the `Network.requestWillBeSent` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestWillBeSentParams {
    /// Request id.
    pub request_id: String,
    /// The request.
    pub request: Request,
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
    /// Wall-clock time.
    #[serde(default)]
    pub wall_time: f64,
    /// The document this request belongs to.
    #[serde(default)]
    pub document_url: Option<String>,
    /// The loader of the frame.
    #[serde(default)]
    pub loader_id: Option<String>,
    /// Redirect target when this is a redirect.
    #[serde(default)]
    pub redirect_response: Option<Response>,
}

/// An HTTP request as reported by the `Network` domain.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Request URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Request headers.
    #[serde(default)]
    pub headers: serde_json::Map<String, serde_json::Value>,
}

/// An HTTP response as reported by the `Network` domain.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// Response URL.
    #[serde(default)]
    pub url: String,
    /// HTTP status code.
    #[serde(default)]
    pub status: i64,
    /// HTTP status text.
    #[serde(default)]
    pub status_text: String,
    /// Response mime type.
    #[serde(default)]
    pub mime_type: String,
    /// Response headers.
    #[serde(default)]
    pub headers: serde_json::Map<String, serde_json::Value>,
}

/// Parameters of the `Network.responseReceived` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseReceivedParams {
    /// Request id.
    pub request_id: String,
    /// The loader of the frame.
    #[serde(default)]
    pub loader_id: Option<String>,
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
    /// The response.
    pub response: Response,
    /// The resource type (`Document`, `Script`, ...).
    #[serde(rename = "type", default)]
    pub resource_type: String,
}

/// Parameters of the `Network.loadingFinished` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadingFinishedParams {
    /// Request id.
    pub request_id: String,
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
}

/// Parameters of the `Network.loadingFailed` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadingFailedParams {
    /// Request id.
    pub request_id: String,
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
    /// Failure description.
    #[serde(default)]
    pub error_text: String,
    /// Whether the failure was canceled.
    #[serde(default)]
    pub canceled: bool,
}

/// Response of `Network.getCookies`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetCookiesResult {
    /// The cookies visible to the current page.
    pub cookies: Vec<Cookie>,
}

/// A browser cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie domain.
    #[serde(default)]
    pub domain: String,
    /// Cookie path.
    #[serde(default)]
    pub path: String,
    /// Expiry as seconds since the epoch; `-1` for session cookies.
    #[serde(default)]
    pub expires: f64,
    /// Whether the cookie is HTTP-only.
    #[serde(default)]
    pub http_only: bool,
    /// Whether the cookie is secure.
    #[serde(default)]
    pub secure: bool,
    /// Whether this is a session cookie.
    #[serde(default)]
    pub session: bool,
    /// SameSite policy (`Strict`, `Lax`, `None`).
    #[serde(default)]
    pub same_site: Option<String>,
}

/// Parameters of `Network.setCookie`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCookieParams {
    /// Cookie name.
    pub name: String,
    /// Cookie value.
    pub value: String,
    /// Cookie URL; supply this or `domain`/`path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Cookie domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Cookie path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Whether the cookie is HTTP-only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    /// Whether the cookie is secure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    /// SameSite policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
}

/// Response of `Network.setCookie`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCookieResult {
    /// Whether the cookie was set.
    #[serde(default)]
    pub success: bool,
}

/// Response of `Network.clearBrowserCookies`.
pub type ClearBrowserCookiesResult = Value;
