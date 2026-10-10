//! The WebDriver BiDi WebSocket transport.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{BidiError, BidiResult};

/// An event emitted by the remote end.
#[derive(Debug, Clone)]
pub struct BidiEvent {
    /// The BiDi event method, for example `browsingContext.load`.
    pub method: String,
    /// The event parameters.
    pub params: Value,
}

#[derive(Debug, Deserialize)]
struct IncomingMessage {
    id: Option<u64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    result: Option<Value>,
    error: Option<String>,
    message: Option<String>,
    method: Option<String>,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct OutgoingMessage<'a> {
    id: u64,
    method: &'a str,
    params: Value,
}

// Teardown must not wait indefinitely for a non-reading peer.
const CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<BidiResult<Value>>>>,
    outbound: mpsc::UnboundedSender<String>,
    shutdown: watch::Sender<bool>,
    events: broadcast::Sender<BidiEvent>,
    closed: AtomicBool,
}

impl Inner {
    fn fail_all(&self) {
        let mut pending = self.pending.lock().expect("bidi pending mutex poisoned");
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(BidiError::Closed));
        }
    }

    fn mark_closed(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shutdown.send_replace(true);
        self.fail_all();
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
            .expect("bidi pending mutex poisoned")
            .remove(&self.id);
    }
}

/// A connection to a WebDriver BiDi endpoint.
#[derive(Clone)]
pub struct BidiConnection {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for BidiConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiConnection")
            .field("closed", &self.is_closed())
            .finish()
    }
}

impl BidiConnection {
    /// Open a WebSocket connection to a BiDi endpoint.
    pub async fn connect(ws_url: &str) -> BidiResult<Self> {
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
                    tracing::debug!(%error, "bidi writer stopped");
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
                                    tracing::debug!(%error, "dropping non-utf8 bidi frame")
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%error, "bidi reader stopped");
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

    /// Whether the socket has been closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Subscribe to BiDi events.
    pub fn subscribe(&self) -> broadcast::Receiver<BidiEvent> {
        self.inner.events.subscribe()
    }

    /// Send a BiDi command and return its result.
    pub async fn send(&self, method: &str, params: Value) -> BidiResult<Value> {
        self.send_with_timeout(method, params, None).await
    }

    /// Send a BiDi command with an explicit timeout.
    ///
    /// Timing out or dropping this future removes its local response waiter.
    /// It does not cancel a command already sent to the remote end.
    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> BidiResult<Value> {
        if self.is_closed() {
            return Err(BidiError::Closed);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();

        let message = OutgoingMessage { id, method, params };
        let text = serde_json::to_string(&message)?;
        {
            let mut pending = self
                .inner
                .pending
                .lock()
                .expect("bidi pending mutex poisoned");
            // Checking while holding the lock prevents registration after close
            // has drained the pending requests.
            if self.is_closed() {
                return Err(BidiError::Closed);
            }
            pending.insert(id, tx);
        }
        let _pending_request = PendingRequest {
            inner: &self.inner,
            id,
        };
        if self.inner.outbound.send(text).is_err() {
            return Err(BidiError::Closed);
        }

        let response = match timeout {
            Some(duration) => tokio::time::timeout(duration, rx)
                .await
                .map_err(|_| BidiError::Timeout {
                    method: method.to_string(),
                    timeout: duration,
                })?
                .map_err(|_| BidiError::Closed)?,
            None => rx.await.map_err(|_| BidiError::Closed)?,
        };

        match response {
            Ok(value) => Ok(value),
            Err(BidiError::Protocol { error, message, .. }) => Err(BidiError::Protocol {
                method: method.to_string(),
                error,
                message,
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
            tracing::debug!(%error, "failed to parse bidi message");
            return;
        }
    };

    if let Some(id) = message.id {
        if let Some(sender) = inner
            .pending
            .lock()
            .expect("bidi pending mutex poisoned")
            .remove(&id)
        {
            let result = if message.kind.as_deref() == Some("error") {
                Err(BidiError::Protocol {
                    method: String::new(),
                    error: message.error.unwrap_or_else(|| "unknown".to_string()),
                    message: message.message.unwrap_or_default(),
                })
            } else {
                Ok(message.result.unwrap_or(Value::Null))
            };
            let _ = sender.send(result);
        }
        return;
    }

    if let Some(method) = message.method {
        let _ = inner.events.send(BidiEvent {
            method,
            params: message.params.unwrap_or(Value::Null),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, DuplexStream};
    use tokio_tungstenite::{tungstenite::protocol::Role, WebSocketStream};

    async fn connection() -> (BidiConnection, WebSocketStream<DuplexStream>) {
        let (client, server) = duplex(4096);
        let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        (BidiConnection::from_socket(client), server)
    }

    async fn next_command(server: &mut WebSocketStream<DuplexStream>) -> Value {
        let message = server.next().await.expect("request").expect("valid frame");
        serde_json::from_str(message.to_text().expect("text frame")).expect("JSON command")
    }

    async fn assert_usable_after_late_response(
        connection: &BidiConnection,
        server: &mut WebSocketStream<DuplexStream>,
        old_id: u64,
    ) {
        let request = connection.send("Test.next", Value::Null);
        let respond = async {
            let command = next_command(server).await;
            let id = command["id"].as_u64().expect("request id");
            assert_ne!(id, old_id);
            for (id, result) in [(old_id, "stale"), (id, "current")] {
                let response = serde_json::json!({"type": "success", "id": id, "result": result});
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
        let request = connection.send("Test.error", Value::Null);
        let respond = async {
            let command = next_command(&mut server).await;
            let id = command["id"].as_u64().expect("request id");
            let response = serde_json::json!({"type": "error", "id": id, "error": "invalid argument", "message": "bad params"});
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
            matches!(result, Err(BidiError::Protocol { method, error, message }) if method == "Test.error" && error == "invalid argument" && message == "bad params")
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
                connection.send_with_timeout("Test.command", Value::Null, Some(timeout)),
                next_command(&mut server),
            )
        })
        .await
        .expect("command must be sent and time out");
        let old_id = command["id"].as_u64().expect("request id");
        assert!(
            matches!(result, Err(BidiError::Timeout { method, timeout: actual })
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
            let request = connection.send("Test.command", Value::Null);
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
}

#[cfg(test)]
#[path = "transport_lifetime_tests.rs"]
mod transport_lifetime_tests;
