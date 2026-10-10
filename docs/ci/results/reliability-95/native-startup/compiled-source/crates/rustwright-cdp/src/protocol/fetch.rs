//! `Fetch` domain: request interception and fulfillment.

use serde::{Deserialize, Serialize};

use crate::protocol::network::Request;

/// A URL/resource pattern for request interception.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPattern {
    /// Wildcard URL pattern; `*` matches everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_pattern: Option<String>,
    /// Resource type (`Document`, `XHR`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// Interception stage (`Request` or `Response`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_stage: Option<String>,
}

/// Parameters of `Fetch.enable`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableParams {
    /// Patterns to intercept.
    pub patterns: Vec<RequestPattern>,
    /// Whether to intercept auth challenges too.
    #[serde(default)]
    pub handle_auth_requests: bool,
}

/// An HTTP header name/value pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderEntry {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

/// Parameters of the `Fetch.requestPaused` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPausedParams {
    /// CDP request id.
    pub request_id: String,
    /// The paused request.
    pub request: Request,
    /// The frame the request belongs to.
    #[serde(default)]
    pub frame_id: String,
    /// Resource type.
    #[serde(default)]
    pub resource_type: String,
    /// Present when a response stage was paused and failed.
    #[serde(default)]
    pub response_error_reason: Option<String>,
    /// Present when a response stage was paused.
    #[serde(default)]
    pub response_status_code: Option<i64>,
    /// Present when a response stage was paused.
    #[serde(default)]
    pub response_headers: Vec<HeaderEntry>,
}

/// Parameters of `Fetch.continueRequest`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueRequestParams {
    /// CDP request id.
    pub request_id: String,
    /// Replacement request headers (the complete set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<HeaderEntry>>,
}

/// Parameters of `Fetch.continueResponse`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueResponseParams {
    /// CDP request id.
    pub request_id: String,
    /// Replacement status code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_code: Option<i64>,
    /// Replacement reason phrase.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_phrase: Option<String>,
    /// Replacement response headers (the complete set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_headers: Option<Vec<HeaderEntry>>,
}

/// Parameters of `Fetch.fulfillRequest`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FulfillRequestParams {
    /// CDP request id.
    pub request_id: String,
    /// HTTP status code.
    pub response_code: i64,
    /// Response headers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub response_headers: Vec<HeaderEntry>,
    /// Base64-encoded response body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Parameters of `Fetch.failRequest`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailRequestParams {
    /// CDP request id.
    pub request_id: String,
    /// Failure reason, e.g. `Aborted`.
    pub error_reason: String,
}
