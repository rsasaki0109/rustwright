//! The WebSocket transport and request/event router.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
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

enum Outbound {
    Text(String),
    Close,
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<CdpResult<Value>>>>;

struct Inner {
    next_id: AtomicU64,
    pending: Pending,
    outbound: mpsc::UnboundedSender<Outbound>,
    events: broadcast::Sender<CdpEvent>,
    closed: AtomicBool,
}

impl Inner {
    fn fail_all(&self) {
        let mut pending = self.pending.lock().expect("pending mutex poisoned");
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(CdpError::Closed));
        }
    }

    fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.fail_all();
        let _ = self.events.send(CdpEvent {
            session_id: None,
            method: DISCONNECTED_EVENT.to_string(),
            params: Value::Null,
        });
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

        let inner = Arc::new(Inner {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            outbound: outbound_tx,
            events: events_tx,
            closed: AtomicBool::new(false),
        });

        let writer_inner = inner.clone();
        tokio::spawn(async move {
            while let Some(outbound) = outbound_rx.recv().await {
                let result = match outbound {
                    Outbound::Text(text) => sink.send(Message::text(text)).await,
                    Outbound::Close => {
                        let _ = sink.close().await;
                        break;
                    }
                };
                if let Err(error) = result {
                    tracing::debug!(%error, "CDP writer stopped");
                    break;
                }
            }
            writer_inner.mark_closed();
        });

        let reader_inner = inner.clone();
        tokio::spawn(async move {
            while let Some(item) = source.next().await {
                match item {
                    Ok(message) => {
                        if message.is_close() {
                            break;
                        }
                        if message.is_text() || message.is_binary() {
                            match message.into_text() {
                                Ok(text) => handle_message(text.as_str(), &reader_inner),
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
            reader_inner.mark_closed();
        });

        Self { inner }
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
        self.inner
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .insert(id, tx);

        let message = OutgoingMessage {
            id,
            method,
            params: Some(params),
            session_id,
        };
        let text = serde_json::to_string(&message)?;
        if self.inner.outbound.send(Outbound::Text(text)).is_err() {
            self.inner
                .pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&id);
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

    /// Gracefully close the connection.
    pub fn close(&self) {
        let _ = self.inner.outbound.send(Outbound::Close);
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
        if let Some(sender) = inner
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
            let _ = sender.send(result);
        } else {
            tracing::debug!(id, "received response for unknown request id");
        }
        return;
    }

    if let Some(method) = message.method {
        let event = CdpEvent {
            session_id: message.session_id,
            method,
            params: message.params.unwrap_or(Value::Null),
        };
        let _ = inner.events.send(event);
    }
}
