//! Opt-in network monitoring over WebDriver BiDi.
//!
//! This is the BiDi counterpart of the CDP backend's `network_requests`
//! diagnostics: it collects `network.beforeRequestSent`, `responseCompleted`
//! and `fetchError` events for a page.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use rustwright_common::{Route, RouteAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::{connection::BidiEvent, session::BidiSession};

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
    #[serde(default)]
    headers: Vec<HeaderEntry>,
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
    #[serde(default)]
    headers: Vec<HeaderEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BeforeRequestSentParams {
    context: String,
    #[serde(default)]
    is_blocked: bool,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseStartedParams {
    #[serde(default)]
    is_blocked: bool,
    #[serde(default)]
    request: BidiRequest,
    #[serde(default)]
    response: BidiResponse,
}

/// Stops the network pump when the last page clone is dropped.
pub(crate) struct NetworkPumpGuard {
    handle: JoinHandle<()>,
}

#[cfg(test)]
impl NetworkPumpGuard {
    pub(crate) fn task_id(&self) -> tokio::task::Id {
        self.handle.id()
    }

    pub(crate) fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.handle.abort_handle()
    }
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
    mut events: broadcast::Receiver<BidiEvent>,
    context: String,
    sink: Arc<Mutex<Vec<BidiNetworkRequest>>>,
    mut shutdown: watch::Receiver<bool>,
    mut closed: watch::Receiver<bool>,
) -> NetworkPumpGuard {
    // The receiver was registered before the remote subscription handshake.
    // Retain those queued events even before this task is first scheduled.
    let handle = tokio::spawn(async move {
        loop {
            if *shutdown.borrow() || *closed.borrow() {
                break;
            }
            tokio::select! {
                event = events.recv() => match event {
                    Ok(event) if event.method == "browsingContext.contextDestroyed" && event.params["context"] == context => break,
                    Ok(event) => dispatch(&context, &sink, &event.method, &event.params),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = shutdown.changed() => break,
                _ = closed.changed() => break,
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

// -- Request interception ---------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UrlPattern {
    #[serde(rename = "type")]
    pub kind: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddInterceptParams {
    pub phases: Vec<String>,
    pub url_patterns: Vec<UrlPattern>,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RemoveInterceptParams {
    pub intercept: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContinueRequestParams {
    pub request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<HeaderEntry>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContinueResponseParams {
    pub request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_phrase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<HeaderEntry>>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FailRequestParams {
    pub request: String,
}

/// A `network.BytesValue`: either UTF-8 text or base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BytesValue {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: String,
}

/// A `network.Header`: the value is itself a [`BytesValue`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HeaderEntry {
    pub name: String,
    pub value: BytesValue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvideResponseParams {
    pub request: String,
    pub status_code: i64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<BytesValue>,
}

/// Drives interception: answers every blocked `beforeRequestSent` event using
/// the registered routes, continuing anything that does not match.
// Separate from the transport's broadcast capacity: awaiting an allocation
// must not drain unlimited unknown events into a second unbounded buffer.
const MAX_DEFERRED_INTERCEPT_EVENTS: usize = 2048;
fn defer_intercept(
    events: &mut Vec<crate::BidiEvent>,
    event: crate::BidiEvent,
    overflowed: &mut bool,
) {
    if events.len() < MAX_DEFERRED_INTERCEPT_EVENTS {
        events.push(event)
    } else if !*overflowed {
        *overflowed = true;
        tracing::warn!("BiDi pending interception event queue overflowed; a blocked request may require caller recovery");
    }
}
pub(crate) struct InterceptRegistry {
    pub(crate) ids: Mutex<HashSet<String>>,
    allocating: watch::Sender<bool>,
}
impl Default for InterceptRegistry {
    fn default() -> Self {
        let (allocating, _) = watch::channel(false);
        Self {
            ids: Mutex::new(HashSet::new()),
            allocating,
        }
    }
}
impl InterceptRegistry {
    pub(crate) fn begin_allocation(&self) {
        self.allocating.send_replace(true);
    }
    pub(crate) fn finish_allocation(&self) {
        self.allocating.send_replace(false);
    }
}
enum PumpControl {
    Retire(String, oneshot::Sender<()>),
    Stop,
}
pub(crate) struct InterceptPump {
    handle: JoinHandle<()>,
    control: mpsc::UnboundedSender<PumpControl>,
}
impl InterceptPump {
    pub(crate) async fn retire(&self, id: &str) {
        let (tx, rx) = oneshot::channel();
        if self
            .control
            .send(PumpControl::Retire(id.to_owned(), tx))
            .is_ok()
        {
            let _ = tokio::time::timeout(crate::session::SHUTDOWN_TIMEOUT, rx).await;
        }
    }
    pub(crate) async fn stop(mut self) {
        let _ = self.control.send(PumpControl::Stop);
        let _ = tokio::time::timeout(crate::session::SHUTDOWN_TIMEOUT, &mut self.handle).await;
    }
}
impl Drop for InterceptPump {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(crate) fn spawn_intercept_pump(
    session: BidiSession,
    routes: Arc<Mutex<Vec<Route>>>,
    owned: Arc<InterceptRegistry>,
) -> InterceptPump {
    let mut events = session.events();
    let mut shutdown = session.connection().shutdown_receiver();
    let mut allocating = owned.allocating.subscribe();
    let (control, mut commands) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        let mut deferred = Vec::new();
        let mut overflowed = false;
        loop {
            if *shutdown.borrow() {
                break;
            }
            tokio::select! {
                biased;
                _=shutdown.changed()=>break,
                command=commands.recv()=>{
                    // Drain the finite prefix received before removal's reply.
                    // Unknown IDs during allocation stay deferred; known old
                    // routes continue without waiting for the new ID's reply.
                    let queued=events.len();
                    for _ in 0..queued {match events.try_recv(){
                        Ok(event)=>{if let Some(event)=dispatch_intercept(&session,&routes,&owned,event).await{defer_intercept(&mut deferred,event,&mut overflowed)}},
                        Err(broadcast::error::TryRecvError::Lagged(_))=>continue,
                        Err(_)=>break,
                    }}
                    deferred=replay_intercepts(&session,&routes,&owned,deferred).await;
                    match command {
                        Some(PumpControl::Retire(id,done))=>{owned.ids.lock().expect("bidi intercept IDs mutex poisoned").remove(&id);let _=done.send(());},
                        Some(PumpControl::Stop)|None=>break,
                    }
                }
                _=allocating.changed()=>{
                    deferred=replay_intercepts(&session,&routes,&owned,deferred).await;
                }
                event=events.recv()=>match event {
                    Ok(event)=>{if let Some(event)=dispatch_intercept(&session,&routes,&owned,event).await{defer_intercept(&mut deferred,event,&mut overflowed)}},
                    Err(broadcast::error::RecvError::Lagged(_))=>continue,
                    Err(broadcast::error::RecvError::Closed)=>break,
                },
            }
        }
    });
    InterceptPump { handle, control }
}
async fn replay_intercepts(
    session: &BidiSession,
    routes: &Arc<Mutex<Vec<Route>>>,
    owned: &InterceptRegistry,
    events: Vec<crate::BidiEvent>,
) -> Vec<crate::BidiEvent> {
    let mut deferred = Vec::new();
    let mut overflowed = false;
    for event in events {
        if let Some(event) = dispatch_intercept(session, routes, owned, event).await {
            defer_intercept(&mut deferred, event, &mut overflowed)
        }
    }
    deferred
}
async fn dispatch_intercept(
    session: &BidiSession,
    routes: &Arc<Mutex<Vec<Route>>>,
    owned: &InterceptRegistry,
    event: crate::BidiEvent,
) -> Option<crate::BidiEvent> {
    if !matches!(
        event.method.as_str(),
        "network.beforeRequestSent" | "network.responseStarted"
    ) {
        return None;
    }
    if event.params.get("isBlocked").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let ids = event.params.get("intercepts").and_then(Value::as_array)?;
    let (ours, allocating) = {
        let known = owned.ids.lock().expect("bidi intercept IDs mutex poisoned");
        let ours = ids
            .iter()
            .any(|id| id.as_str().is_some_and(|id| known.contains(id)));
        // Snapshot the pending flag before unlocking. Otherwise publication of
        // the new ID followed by pending=false could make an early owned event
        // appear foreign between these two observations.
        (ours, *owned.allocating.borrow())
    };
    if !ours {
        return if allocating { Some(event) } else { None };
    }
    let operation = async {
        match event.method.as_str() {
            "network.beforeRequestSent" => {
                handle_request_stage(session, routes, &event.params).await
            }
            "network.responseStarted" => {
                handle_response_stage(session, routes, &event.params).await
            }
            _ => {}
        }
    };
    let _ = tokio::time::timeout(crate::session::SHUTDOWN_TIMEOUT, operation).await;
    None
}

fn route_action(routes: &Arc<Mutex<Vec<Route>>>, url: &str) -> Option<RouteAction> {
    let routes = routes.lock().expect("bidi routes mutex poisoned");
    routes
        .iter()
        .find(|route| route.matches(url))
        .map(|route| route.action.clone())
}

async fn handle_request_stage(
    session: &BidiSession,
    routes: &Arc<Mutex<Vec<Route>>>,
    params: &Value,
) {
    let Ok(params) = serde_json::from_value::<BeforeRequestSentParams>(params.clone()) else {
        return;
    };
    if !params.is_blocked {
        return;
    }
    let request_id = params.request.request.clone();
    if request_id.is_empty() {
        return;
    }
    match route_action(routes, &params.request.url) {
        Some(RouteAction::Abort) => {
            let _ = session.fail_request(&request_id).await;
        }
        Some(RouteAction::Fulfill {
            status,
            content_type,
            body,
        }) => {
            let _ = session
                .provide_response(&request_id, status, &content_type, &body)
                .await;
        }
        Some(RouteAction::SetRequestHeaders(overrides)) => {
            let headers = merge_headers(&params.request.headers, &overrides);
            let _ = session
                .continue_request_with_headers(&request_id, Some(headers))
                .await;
        }
        _ => {
            let _ = session.continue_request(&request_id).await;
        }
    }
}

async fn handle_response_stage(
    session: &BidiSession,
    routes: &Arc<Mutex<Vec<Route>>>,
    params: &Value,
) {
    let Ok(params) = serde_json::from_value::<ResponseStartedParams>(params.clone()) else {
        return;
    };
    if !params.is_blocked {
        return;
    }
    let request_id = params.request.request.clone();
    if request_id.is_empty() {
        return;
    }
    match route_action(routes, &params.request.url) {
        Some(RouteAction::Abort) => {
            let _ = session.fail_request(&request_id).await;
        }
        Some(RouteAction::Fulfill {
            status,
            content_type,
            body,
        }) => {
            let _ = session
                .provide_response(&request_id, status, &content_type, &body)
                .await;
        }
        Some(RouteAction::SetResponseHeaders(overrides)) => {
            let headers = merge_headers(&params.response.headers, &overrides);
            let _ = session
                .continue_response(&request_id, None, Some(headers))
                .await;
        }
        _ => {
            let _ = session.continue_response(&request_id, None, None).await;
        }
    }
}

/// Merge overrides onto the original headers (case-insensitive by name).
fn merge_headers(original: &[HeaderEntry], overrides: &[(String, String)]) -> Vec<HeaderEntry> {
    let mut merged: BTreeMap<String, HeaderEntry> = original
        .iter()
        .map(|header| (header.name.to_ascii_lowercase(), header.clone()))
        .collect();
    for (name, value) in overrides {
        merged.insert(
            name.to_ascii_lowercase(),
            HeaderEntry {
                name: name.clone(),
                value: BytesValue {
                    kind: "string".to_string(),
                    value: value.clone(),
                },
            },
        );
    }
    merged.into_values().collect()
}

#[cfg(test)]
mod interception_queue_tests {
    use super::*;
    #[test]
    fn deferred_allocation_events_have_a_separate_bounded_queue() {
        let mut events = Vec::new();
        let mut overflowed = false;
        for request in 0..MAX_DEFERRED_INTERCEPT_EVENTS + 100 {
            defer_intercept(
                &mut events,
                crate::BidiEvent {
                    method: "network.beforeRequestSent".to_owned(),
                    params: serde_json::json!({"isBlocked":true,"request":{"request":request}}),
                },
                &mut overflowed,
            );
        }
        assert_eq!(events.len(), MAX_DEFERRED_INTERCEPT_EVENTS);
        assert!(overflowed);
        assert_eq!(events.first().unwrap().params["request"]["request"], 0);
    }
}
