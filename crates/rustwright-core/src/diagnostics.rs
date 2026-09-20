//! Observability primitives: console messages, page errors and network traffic.
//!
//! These exist so that when a page behaves differently under Rustwright than in
//! a normal browser, the environment delta can be identified instead of guessed.

use std::collections::HashMap;
use std::path::PathBuf;

use rustwright_cdp::protocol::network::{Cookie, RequestWillBeSentParams, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    /// Wall-clock start time (seconds since the epoch).
    pub wall_time: Option<f64>,
    /// Monotonic start timestamp (seconds).
    pub started: Option<f64>,
    /// Monotonic timestamp when the response headers arrived.
    pub responded: Option<f64>,
    /// Monotonic timestamp when the request finished.
    pub finished: Option<f64>,
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
            wall_time: Some(params.wall_time),
            started: Some(params.timestamp),
            responded: None,
            finished: None,
        }
    }

    pub(crate) fn apply_response(&mut self, response: &Response, timestamp: f64) {
        self.status = Some(response.status);
        self.status_text = Some(response.status_text.clone());
        self.mime_type = Some(response.mime_type.clone());
        self.response_headers = response.headers.clone();
        self.responded = Some(timestamp);
    }

    pub(crate) fn finish(&mut self, timestamp: f64) {
        self.finished = Some(timestamp);
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

/// Build a HAR 1.2 document from the requests observed by a page.
///
/// `bodies` maps a request id to `(body, base64_encoded)` and is optional.
pub fn build_har(
    requests: &[NetworkRequest],
    bodies: Option<&HashMap<String, (String, bool)>>,
) -> Value {
    let entries: Vec<Value> = requests
        .iter()
        .map(|request| har_entry(request, bodies))
        .collect();
    json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "Rustwright", "version": env!("CARGO_PKG_VERSION") },
            "entries": entries,
        }
    })
}

fn har_entry(request: &NetworkRequest, bodies: Option<&HashMap<String, (String, bool)>>) -> Value {
    let time_ms = match (request.started, request.finished) {
        (Some(start), Some(end)) => (end - start).max(0.0) * 1000.0,
        _ => 0.0,
    };
    let wait = match (request.started, request.responded) {
        (Some(start), Some(responded)) => (responded - start).max(0.0) * 1000.0,
        _ => 0.0,
    };
    let receive = (time_ms - wait).max(0.0);
    let status = if request.failure.is_some() {
        0
    } else {
        request.status.unwrap_or(0)
    };

    let mut content = json!({
        "size": 0,
        "mimeType": request.mime_type.clone().unwrap_or_default(),
    });
    if let Some((body, base64)) = bodies.and_then(|bodies| bodies.get(&request.request_id)) {
        content["size"] = json!(body.len());
        content["text"] = json!(body);
        if *base64 {
            content["encoding"] = json!("base64");
        }
    }

    json!({
        "startedDateTime": iso8601(request.wall_time.unwrap_or(0.0)),
        "time": time_ms,
        "request": {
            "method": request.method,
            "url": request.url,
            "httpVersion": "HTTP/1.1",
            "headers": headers_to_har(&request.request_headers),
            "queryString": [],
            "cookies": [],
            "headersSize": -1,
            "bodySize": -1,
        },
        "response": {
            "status": status,
            "statusText": request.status_text.clone().unwrap_or_default(),
            "httpVersion": "HTTP/1.1",
            "headers": headers_to_har(&request.response_headers),
            "content": content,
            "redirectURL": "",
            "headersSize": -1,
            "bodySize": -1,
        },
        "cache": {},
        "timings": { "send": 0.0, "wait": wait, "receive": receive },
        "_resourceType": request.resource_type,
        "_error": request.failure,
    })
}

fn headers_to_har(headers: &serde_json::Map<String, Value>) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| {
                let value = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                json!({ "name": name, "value": value })
            })
            .collect(),
    )
}

/// Format seconds since the epoch as an ISO 8601 UTC timestamp.
fn iso8601(epoch_seconds: f64) -> String {
    let seconds = epoch_seconds.floor() as i64;
    let millis = ((epoch_seconds - seconds as f64) * 1000.0).round() as i64;
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (remainder / 3600, (remainder % 3600) / 60, remainder % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since the Unix epoch to (year, month, day), after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::iso8601;

    #[test]
    fn formats_epoch_timestamps() {
        assert_eq!(iso8601(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(1_700_000_000.0), "2023-11-14T22:13:20.000Z");
    }
}
