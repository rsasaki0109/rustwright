//! The WebDriver BiDi WebSocket transport.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
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

enum Outbound {
    Text(String),
    Close,
}

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<BidiResult<Value>>>>,
    outbound: mpsc::UnboundedSender<Outbound>,
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
        self.closed.store(true, Ordering::SeqCst);
        self.fail_all();
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
                    tracing::debug!(%error, "bidi writer stopped");
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
            reader_inner.mark_closed();
        });

        Self { inner }
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
        self.inner
            .pending
            .lock()
            .expect("bidi pending mutex poisoned")
            .insert(id, tx);

        let message = OutgoingMessage { id, method, params };
        let text = serde_json::to_string(&message)?;
        if self.inner.outbound.send(Outbound::Text(text)).is_err() {
            self.inner
                .pending
                .lock()
                .expect("bidi pending mutex poisoned")
                .remove(&id);
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
