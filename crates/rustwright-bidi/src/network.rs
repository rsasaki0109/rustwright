//! Opt-in network monitoring over WebDriver BiDi.
//!
//! This is the BiDi counterpart of the CDP backend's `network_requests`
//! diagnostics: it collects `network.beforeRequestSent`, `responseCompleted`
//! and `fetchError` events for a page.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::session::BidiSession;

/// A network request observed over BiDi.
#[derive(Debug, Clone)]
pub struct BidiNetworkRequest {
    /// The BiDi request id.
    pub request_id: String,
    /// Request URL.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Response status once known.
    pub status: Option<i64>,
    /// Response status text once known.
    pub status_text: Option<String>,
    /// Response MIME type once known.
    pub mime_type: Option<String>,
    /// Failure text if the request failed.
    pub failure: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BidiRequest {
    #[serde(default)]
    request: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    method: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BidiResponse {
    #[serde(default)]
    status: i64,
    #[serde(default)]
    status_text: String,
    #[serde(default)]
    mime_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeforeRequestSentParams {
    context: String,
    #[serde(default)]
    request: BidiRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseCompletedParams {
    context: String,
    #[serde(default)]
    request: BidiRequest,
    #[serde(default)]
    response: BidiResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchErrorParams {
    context: String,
    #[serde(default)]
    request: BidiRequest,
    #[serde(default)]
    error_text: String,
}

/// Stops the network pump when the last page clone is dropped.
pub(crate) struct NetworkPumpGuard {
    handle: JoinHandle<()>,
}

impl Drop for NetworkPumpGuard {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl BidiNetworkRequest {
    fn from_request(request: &BidiRequest) -> Self {
        Self {
            request_id: request.request.clone(),
            url: request.url.clone(),
            method: request.method.clone(),
            status: None,
            status_text: None,
            mime_type: None,
            failure: None,
        }
    }
}

pub(crate) fn spawn_network_pump(
    session: BidiSession,
    context: String,
    sink: Arc<Mutex<Vec<BidiNetworkRequest>>>,
) -> NetworkPumpGuard {
    let handle = tokio::spawn(async move {
        let mut events = session.events();
        loop {
            match events.recv().await {
                Ok(event) => dispatch(&context, &sink, &event.method, &event.params),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    NetworkPumpGuard { handle }
}

fn dispatch(
    context: &str,
    sink: &Arc<Mutex<Vec<BidiNetworkRequest>>>,
    method: &str,
    params: &Value,
) {
    match method {
        "network.beforeRequestSent" => {
            if let Ok(event) = serde_json::from_value::<BeforeRequestSentParams>(params.clone()) {
                if event.context != context || event.request.request.is_empty() {
                    return;
                }
                let mut requests = sink.lock().expect("bidi network mutex poisoned");
                if requests
                    .iter()
                    .any(|request| request.request_id == event.request.request)
                {
                    return;
                }
                requests.push(BidiNetworkRequest::from_request(&event.request));
            }
        }
        "network.responseCompleted" => {
            if let Ok(event) = serde_json::from_value::<ResponseCompletedParams>(params.clone()) {
                if event.context != context {
                    return;
                }
                let mut requests = sink.lock().expect("bidi network mutex poisoned");
                if let Some(request) = requests
                    .iter_mut()
                    .find(|request| request.request_id == event.request.request)
                {
                    request.status = Some(event.response.status);
                    request.status_text = Some(event.response.status_text.clone());
                    request.mime_type = Some(event.response.mime_type.clone());
                }
            }
        }
        "network.fetchError" => {
            if let Ok(event) = serde_json::from_value::<FetchErrorParams>(params.clone()) {
                if event.context != context {
                    return;
                }
                let mut requests = sink.lock().expect("bidi network mutex poisoned");
                if let Some(request) = requests
                    .iter_mut()
                    .find(|request| request.request_id == event.request.request)
                {
                    request.failure = Some(event.error_text.clone());
                }
            }
        }
        _ => {}
    }
}
