//! The WebSocket transport and request/event router.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{CdpError, CdpResult};

/// The method name of the synthetic event emitted when the connection closes.
pub const DISCONNECTED_EVENT: &str = "__rustwright_disconnected__";

/// An event emitted by the browser for a session.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    /// The session the event belongs to, or `None` for browser-level events.
    pub session_id: Option<String>,
    /// The CDP method name, for example `Page.loadEventFired`.
    pub method: String,
    /// The event parameters.
    pub params: Value,
}

impl CdpEvent {
    /// Whether this is the synthetic disconnection marker.
    pub fn is_disconnected(&self) -> bool {
        self.method == DISCONNECTED_EVENT
    }
}

#[derive(Debug, Deserialize)]
struct IncomingMessage {
    id: Option<u64>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<IncomingError>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IncomingError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Serialize)]
struct OutgoingMessage<'a> {
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}

// Teardown must not wait indefinitely for a non-reading peer.
const CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);

struct PendingResponse {
    session_id: Option<String>,
    context_id: Option<i64>,
    method: String,
    sender: oneshot::Sender<CdpResult<Value>>,
}

type Pending = Mutex<HashMap<u64, PendingResponse>>;

struct Inner {
    next_id: AtomicU64,
    pending: Pending,
    outbound: mpsc::UnboundedSender<String>,
    shutdown: watch::Sender<bool>,
    events: broadcast::Sender<CdpEvent>,
    closed: AtomicBool,
}

impl Inner {
    fn fail_all(&self) {
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        for (_, response) in pending.drain() {
            let _ = response.sender.send(Err(CdpError::Closed));
        }
    }

    fn fail_session(&self, session_id: &str) {
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        let ids: Vec<_> = pending
            .iter()
            .filter(|(_, response)| response.session_id.as_deref() == Some(session_id))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(response) = pending.remove(&id) {
                let _ = response.sender.send(Err(CdpError::SessionDetached {
                    session_id: session_id.to_string(),
                    method: response.method,
                }));
            }
        }
    }

    fn fail_context(&self, session_id: &str, context_id: Option<i64>) {
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        let ids: Vec<_> = pending
            .iter()
            .filter(|(_, response)| {
                response.session_id.as_deref() == Some(session_id)
                    && response.context_id.is_some()
                    && context_id.is_none_or(|id| response.context_id == Some(id))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(response) = pending.remove(&id) {
                let _ = response.sender.send(Err(CdpError::ContextDestroyed {
                    session_id: session_id.to_string(),
                    context_id: response.context_id.expect("selected context"),
                    method: response.method,
                }));
            }
        }
    }

    fn mark_closed(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shutdown.send_replace(true);
        self.fail_all();
        let _ = self.events.send(CdpEvent {
            session_id: None,
            method: DISCONNECTED_EVENT.to_string(),
            params: Value::Null,
        });
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Background tasks hold only weak references. Dropping the last public
        // handle signals both socket halves, even when the peer stays idle.
        self.mark_closed();
    }
}

// Remove a request even when its response future times out or is dropped.
struct PendingRequest<'a> {
    inner: &'a Inner,
    id: u64,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        self.inner
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .remove(&self.id);
    }
}

/// A connection to a single browser-level CDP WebSocket.
///
/// Cloning is cheap: all clones share the same socket, request table and event
/// bus.
#[derive(Clone)]
pub struct CdpConnection {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for CdpConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpConnection")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl CdpConnection {
    /// Open a WebSocket connection to a browser-level debugger URL.
    pub async fn connect(ws_url: &str) -> CdpResult<Self> {
        let (socket, _response) = tokio_tungstenite::connect_async(ws_url).await?;
        Ok(Self::from_socket(socket))
    }

    fn from_socket<S>(socket: S) -> Self
    where
        S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
            + Send
            + 'static,
    {
        let (mut sink, mut source) = socket.split();
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();
        let (events_tx, _) = broadcast::channel(2048);
        let (shutdown, mut reader_shutdown) = watch::channel(false);
        let mut writer_shutdown = reader_shutdown.clone();

        let inner = Arc::new(Inner {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            outbound: outbound_tx,
            shutdown,
            events: events_tx,
            closed: AtomicBool::new(false),
        });

        let writer_inner = Arc::downgrade(&inner);
        tokio::spawn(async move {
            loop {
                let text = tokio::select! {
                    biased;
                    _ = writer_shutdown.changed() => break,
                    text = outbound_rx.recv() => match text {
                        Some(text) => text,
                        None => break,
                    },
                };
                let result = tokio::select! {
                    biased;
                    _ = writer_shutdown.changed() => break,
                    result = sink.send(Message::text(text)) => result,
                };
                if let Err(error) = result {
                    tracing::debug!(%error, "CDP writer stopped");
                    break;
                }
            }
            if let Some(inner) = writer_inner.upgrade() {
                inner.mark_closed();
            }
            // Give a close frame a bounded chance to flush, then drop the socket.
            let _ = tokio::time::timeout(CLOSE_FLUSH_TIMEOUT, sink.close()).await;
        });

        let reader_inner = Arc::downgrade(&inner);
        tokio::spawn(async move {
            loop {
                let item = tokio::select! {
                    biased;
                    _ = reader_shutdown.changed() => break,
                    item = source.next() => match item {
                        Some(item) => item,
                        None => break,
                    },
                };
                match item {
                    Ok(message) => {
                        if message.is_close() {
                            break;
                        }
                        if message.is_text() || message.is_binary() {
                            match message.into_text() {
                                Ok(text) => {
                                    let Some(inner) = reader_inner.upgrade() else {
                                        break;
                                    };
                                    handle_message(text.as_str(), &inner);
                                }
                                Err(error) => {
                                    tracing::debug!(%error, "dropping non-utf8 CDP frame")
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "CDP reader stopped");
                        break;
                    }
                }
            }
            if let Some(inner) = reader_inner.upgrade() {
                inner.mark_closed();
            }
        });

        Self { inner }
    }

    /// Number of commands currently awaiting a response on this connection.
    ///
    /// This is a point-in-time diagnostic; concurrent commands can change it.
    /// Timed-out and cancelled commands are removed even if the browser is
    /// still executing them.
    pub fn pending_command_count(&self) -> usize {
        self.inner
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .len()
    }

    /// Whether the underlying socket has been closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Subscribe to the connection's event bus.
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.inner.events.subscribe()
    }

    /// Send a raw CDP command. `session_id` selects the target session, or
    /// `None` for browser-level commands.
    pub async fn send_raw(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> CdpResult<Value> {
        self.send_with_timeout(session_id, method, params, None)
            .await
    }

    /// Send a raw CDP command with an explicit timeout.
    ///
    /// Timing out or dropping this future removes its local response waiter.
    /// It does not cancel a command already sent to the browser.
    pub async fn send_with_timeout(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> CdpResult<Value> {
        if self.is_closed() {
            return Err(CdpError::Closed);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        let context_id = if method == "Runtime.evaluate" {
            params["contextId"].as_i64()
        } else {
            None
        };

        let message = OutgoingMessage {
            id,
            method,
            params: Some(params),
            session_id,
        };
        let text = serde_json::to_string(&message)?;
        {
            let mut pending = self.inner.pending.lock().expect("pending mutex poisoned");
            // Checking while holding the lock prevents registration after close
            // has drained the pending requests.
            if self.is_closed() {
                return Err(CdpError::Closed);
            }
            pending.insert(
                id,
                PendingResponse {
                    session_id: session_id.map(str::to_string),
                    context_id,
                    method: method.to_string(),
                    sender: tx,
                },
            );
        }
        let _pending_request = PendingRequest {
            inner: &self.inner,
            id,
        };
        if self.inner.outbound.send(text).is_err() {
            return Err(CdpError::Closed);
        }

        let response = match timeout {
            Some(duration) => tokio::time::timeout(duration, rx)
                .await
                .map_err(|_| CdpError::Timeout {
                    method: method.to_string(),
                    timeout: duration,
                })?
                .map_err(|_| CdpError::Closed)?,
            None => rx.await.map_err(|_| CdpError::Closed)?,
        };

        match response {
            Ok(value) => Ok(value),
            Err(CdpError::Protocol {
                code,
                message,
                data,
                ..
            }) => Err(CdpError::Protocol {
                method: method.to_string(),
                code,
                message,
                data,
            }),
            Err(other) => Err(other),
        }
    }

    /// Close all clones, fail pending commands and stop both socket tasks.
    ///
    /// The writer attempts to flush a close frame for at most 250ms. Repeated
    /// calls are harmless; this synchronous method signals asynchronous cleanup.
    pub fn close(&self) {
        self.inner.mark_closed();
    }
}

fn handle_message(text: &str, inner: &Inner) {
    let message: IncomingMessage = match serde_json::from_str(text) {
        Ok(message) => message,
        Err(error) => {
            tracing::debug!(%error, "failed to parse CDP message");
            return;
        }
    };

    if let Some(id) = message.id {
        if let Some(response) = inner
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .remove(&id)
        {
            let result = match message.error {
                Some(error) => Err(CdpError::Protocol {
                    method: String::new(),
                    code: error.code,
                    message: error.message,
                    data: error.data,
                }),
                None => Ok(message.result.unwrap_or(Value::Null)),
            };
            let _ = response.sender.send(result);
        } else {
            tracing::debug!(id, "received response for unknown request id");
        }
        return;
    }

    if let Some(method) = message.method {
        let params = message.params.unwrap_or(Value::Null);
        if method == "Target.detachedFromTarget" {
            if let Some(id) = params["sessionId"].as_str() {
                inner.fail_session(id);
            }
        }
        if let Some(session_id) = message.session_id.as_deref() {
            if method == "Runtime.executionContextDestroyed" {
                if let Some(id) = params["executionContextId"].as_i64() {
                    inner.fail_context(session_id, Some(id));
                }
            } else if method == "Runtime.executionContextsCleared" {
                inner.fail_context(session_id, None);
            }
        }
        let event = CdpEvent {
            session_id: message.session_id,
            method,
            params,
        };
        let _ = inner.events.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, DuplexStream};
    use tokio_tungstenite::{tungstenite::protocol::Role, WebSocketStream};

    async fn connection() -> (CdpConnection, WebSocketStream<DuplexStream>) {
        let (client, server) = duplex(4096);
        let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        (CdpConnection::from_socket(client), server)
    }

    async fn next_command(server: &mut WebSocketStream<DuplexStream>) -> Value {
        let message = server.next().await.expect("request").expect("valid frame");
        serde_json::from_str(message.to_text().expect("text frame")).expect("JSON command")
    }

    async fn assert_usable_after_late_response(
        connection: &CdpConnection,
        server: &mut WebSocketStream<DuplexStream>,
        old_id: u64,
    ) {
        let request = connection.send_raw(None, "Test.next", Value::Null);
        let respond = async {
            let command = next_command(server).await;
            let id = command["id"].as_u64().expect("request id");
            assert_ne!(id, old_id);
            for (id, result) in [(old_id, "stale"), (id, "current")] {
                let response = serde_json::json!({"id": id, "result": result});
                server
                    .send(Message::text(response.to_string()))
                    .await
                    .expect("send response");
            }
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(request, respond)
        })
        .await
        .expect("next request should complete");
        assert_eq!(
            result.expect("next request succeeds"),
            serde_json::json!("current")
        );
        assert_eq!(connection.pending_command_count(), 0);
    }

    #[tokio::test]
    async fn protocol_error_preserves_command_details() {
        let (connection, mut server) = connection().await;
        let request = connection.send_raw(None, "Test.error", Value::Null);
        let respond = async {
            let command = next_command(&mut server).await;
            let id = command["id"].as_u64().expect("request id");
            let response = serde_json::json!({"id": id, "error": {"code": -32602, "message": "bad params", "data": "details"}});
            server
                .send(Message::text(response.to_string()))
                .await
                .expect("send error");
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(request, respond)
        })
        .await
        .expect("protocol error should complete");
        assert!(
            matches!(result, Err(CdpError::Protocol { method, code, message, data }) if method == "Test.error" && code == -32602 && message == "bad params" && data.as_deref() == Some("details"))
        );
        assert_eq!(connection.pending_command_count(), 0);
        connection.close();
    }

    #[tokio::test]
    async fn timed_out_request_is_removed() {
        let (connection, mut server) = connection().await;
        let timeout = Duration::from_millis(10);
        let (result, command) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                connection.send_with_timeout(None, "Test.command", Value::Null, Some(timeout)),
                next_command(&mut server),
            )
        })
        .await
        .expect("command must be sent and time out");
        let old_id = command["id"].as_u64().expect("request id");
        assert!(
            matches!(result, Err(CdpError::Timeout { method, timeout: actual })
            if method == "Test.command" && actual == timeout)
        );
        assert_eq!(connection.pending_command_count(), 0);
        assert_usable_after_late_response(&connection, &mut server, old_id).await;
        connection.close();
    }

    #[tokio::test]
    async fn cancelled_request_is_removed() {
        let (connection, mut server) = connection().await;
        let old_id = {
            let request = connection.send_raw(None, "Test.command", Value::Null);
            tokio::pin!(request);
            let command = tokio::select! {
                command = next_command(&mut server) => command,
                result = &mut request => panic!("request completed without a response: {result:?}"),
                _ = tokio::time::sleep(Duration::from_secs(1)) => panic!("command was not sent"),
            };
            assert_eq!(connection.pending_command_count(), 1);
            // Leaving this scope drops the awaiting future, cancelling the command.
            command["id"].as_u64().expect("request id")
        };
        assert_eq!(connection.pending_command_count(), 0);
        assert_usable_after_late_response(&connection, &mut server, old_id).await;
        connection.close();
    }
    #[tokio::test]
    async fn detachment_fails_only_its_sessions_pending_requests() {
        let (connection, mut server) = connection().await;
        let first_connection = connection.clone();
        let first = tokio::spawn(async move {
            first_connection
                .send_raw(Some("child-one"), "Runtime.evaluate", Value::Null)
                .await
        });
        let old = tokio::time::timeout(Duration::from_secs(1), next_command(&mut server))
            .await
            .unwrap();
        let second_connection = connection.clone();
        let second = tokio::spawn(async move {
            second_connection
                .send_raw(Some("child-two"), "Runtime.evaluate", Value::Null)
                .await
        });
        let current = tokio::time::timeout(Duration::from_secs(1), next_command(&mut server))
            .await
            .unwrap();
        server.send(Message::text(serde_json::json!({"sessionId":"parent", "method":"Target.detachedFromTarget", "params":{"sessionId":"child-one"}}).to_string())).await.unwrap();
        let detached = tokio::time::timeout(Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(detached,Err(CdpError::SessionDetached {session_id,method}) if session_id=="child-one" && method=="Runtime.evaluate")
        );
        assert_eq!(connection.pending_command_count(), 1);
        assert!(!connection.is_closed());
        for (id, result) in [(&old["id"], "late"), (&current["id"], "unaffected")] {
            server
                .send(Message::text(
                    serde_json::json!({"id":id,"result":result}).to_string(),
                ))
                .await
                .unwrap();
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), second)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            serde_json::json!("unaffected")
        );
        assert_usable_after_late_response(&connection, &mut server, old["id"].as_u64().unwrap())
            .await;
        connection.close();
    }
    #[tokio::test]
    async fn context_destruction_is_scoped_to_both_session_and_context() {
        let (connection, mut server) = connection().await;
        let mut tasks = Vec::new();
        let mut commands = Vec::new();
        for (session, context) in [("first", 7), ("first", 8), ("second", 7)] {
            let connection = connection.clone();
            tasks.push(tokio::spawn(async move {
                connection
                    .send_raw(
                        Some(session),
                        "Runtime.evaluate",
                        serde_json::json!({"contextId":context,"awaitPromise":true}),
                    )
                    .await
            }));
            commands.push(
                tokio::time::timeout(Duration::from_secs(1), next_command(&mut server))
                    .await
                    .unwrap(),
            );
        }
        server.send(Message::text(serde_json::json!({"sessionId":"first","method":"Runtime.executionContextDestroyed","params":{"executionContextId":7}}).to_string())).await.unwrap();
        let first = tasks.remove(0);
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(1),first).await.unwrap().unwrap(),Err(CdpError::ContextDestroyed{session_id,context_id:7,method}) if session_id=="first" && method=="Runtime.evaluate")
        );
        assert_eq!(connection.pending_command_count(), 2);
        server.send(Message::text(serde_json::json!({"sessionId":"first","method":"Runtime.executionContextsCleared","params":{}}).to_string())).await.unwrap();
        let next = tasks.remove(0);
        assert!(
            matches!(tokio::time::timeout(Duration::from_secs(1),next).await.unwrap().unwrap(),Err(CdpError::ContextDestroyed{session_id,context_id:8,..}) if session_id=="first")
        );
        assert_eq!(connection.pending_command_count(), 1);
        for command in commands {
            server
                .send(Message::text(
                    serde_json::json!({"id":command["id"],"result":"current"}).to_string(),
                ))
                .await
                .unwrap();
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), tasks.remove(0))
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            serde_json::json!("current")
        );
        assert!(connection.inner.pending.lock().unwrap().is_empty());
        connection.close();
    }
}

#[cfg(test)]
#[path = "transport_lifetime_tests.rs"]
mod transport_lifetime_tests;
