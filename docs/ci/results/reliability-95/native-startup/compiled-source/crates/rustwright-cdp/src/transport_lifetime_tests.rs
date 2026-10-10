//! Transport lifetime regressions; sockets stay alive until both halves drop.
use super::*;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{duplex, DuplexStream};
use tokio_tungstenite::{
    tungstenite::{protocol::Role, Error as WsError},
    WebSocketStream,
};

type Connection = CdpConnection;
type Error = CdpError;

#[tokio::test]
async fn repeated_close_emits_one_disconnection_marker() {
    let (connection, server) = pair().await;
    let weak = Arc::downgrade(&connection.inner);
    let mut events = connection.subscribe();
    connection.close();
    connection.close();
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(event.is_disconnected());
    drop(server);
    drop(connection);
    until(|| weak.upgrade().is_none()).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .unwrap(),
        Err(broadcast::error::RecvError::Closed)
    ));
}

async fn send(connection: &Connection) -> Result<Value, Error> {
    connection.send_raw(None, "Test.pending", Value::Null).await
}
async fn pair() -> (Connection, WebSocketStream<DuplexStream>) {
    let (client, server) = duplex(4096);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    (Connection::from_socket(client), server)
}
async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("transport resources must be released within one second");
}

#[tokio::test]
async fn peer_loss_releases_waiters_and_inner() {
    let (connection, mut server) = pair().await;
    let weak = Arc::downgrade(&connection.inner);
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let connection = connection.clone();
        tasks.push(tokio::spawn(async move { send(&connection).await }));
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        for _ in 0..32 {
            assert!(server.next().await.unwrap().unwrap().is_text());
        }
    })
    .await
    .unwrap();
    assert_eq!(connection.pending_command_count(), 32);
    // No WebSocket closing handshake: simulate a crashed remote process.
    drop(server);
    for task in tasks {
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(Error::Closed)), "{result:?}");
    }
    assert!(connection.is_closed());
    assert_eq!(connection.pending_command_count(), 0);
    drop(connection);
    until(|| weak.upgrade().is_none()).await;
}

#[tokio::test]
async fn last_clone_drop_releases_inner_and_socket() {
    let (connection, mut server) = pair().await;
    let weak = Arc::downgrade(&connection.inner);
    let retained = connection.clone();
    drop(connection);
    let (result, ()) = tokio::join!(send(&retained), async {
        let message = server.next().await.unwrap().unwrap();
        let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        server
            .send(Message::text(
                serde_json::json!({"id":command["id"],"result":"alive"}).to_string(),
            ))
            .await
            .unwrap();
    });
    assert_eq!(result.unwrap(), serde_json::json!("alive"));
    drop(retained);
    until(|| weak.upgrade().is_none()).await;
    let received = tokio::time::timeout(Duration::from_secs(1), server.next())
        .await
        .expect("last handle drop must disconnect the peer");
    match received {
        None | Some(Err(_)) => {}
        Some(Ok(message)) if message.is_close() => {}
        other => panic!("unexpected message after last handle dropped: {other:?}"),
    }
}

#[derive(Default)]
struct Probe {
    writing: AtomicBool,
    dropped: AtomicBool,
}
struct FaultSocket {
    incoming: mpsc::UnboundedReceiver<Result<Message, WsError>>,
    probe: Arc<Probe>,
    fail_write: bool,
}
impl Drop for FaultSocket {
    fn drop(&mut self) {
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}
impl futures_util::Stream for FaultSocket {
    type Item = Result<Message, WsError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().incoming).poll_recv(cx)
    }
}
impl futures_util::Sink<Message> for FaultSocket {
    type Error = WsError;
    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.fail_write {
            Poll::Ready(Err(WsError::ConnectionClosed))
        } else {
            Poll::Ready(Ok(()))
        }
    }
    fn start_send(self: Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
        self.probe.writing.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Pending
    }
    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Pending
    }
}
fn faulty(
    fail_write: bool,
) -> (
    Connection,
    mpsc::UnboundedSender<Result<Message, WsError>>,
    Arc<Probe>,
) {
    let (tx, incoming) = mpsc::unbounded_channel();
    let probe = Arc::new(Probe::default());
    let socket = FaultSocket {
        incoming,
        probe: probe.clone(),
        fail_write,
    };
    (Connection::from_socket(socket), tx, probe)
}
#[tokio::test]
async fn writer_failure_releases_idle_reader_and_socket() {
    let (connection, _keep_reader_open, probe) = faulty(true);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), send(&connection))
            .await
            .unwrap(),
        Err(Error::Closed)
    ));
    assert_eq!(connection.pending_command_count(), 0);
    until(|| probe.dropped.load(Ordering::SeqCst)).await;
    assert!(matches!(send(&connection).await, Err(Error::Closed)));
}
#[tokio::test]
async fn explicit_close_interrupts_blocked_write_and_close() {
    let (connection, _keep_reader_open, probe) = faulty(false);
    let waiting = connection.clone();
    let task = tokio::spawn(async move { send(&waiting).await });
    until(|| probe.writing.load(Ordering::SeqCst)).await;
    connection.close();
    connection.close();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Closed)
    ));
    assert_eq!(connection.pending_command_count(), 0);
    until(|| probe.dropped.load(Ordering::SeqCst)).await;
}
#[tokio::test]
async fn reader_failure_interrupts_blocked_write_and_close() {
    let (connection, incoming, probe) = faulty(false);
    let waiting = connection.clone();
    let task = tokio::spawn(async move { send(&waiting).await });
    until(|| probe.writing.load(Ordering::SeqCst)).await;
    incoming
        .send(Err(WsError::Io(std::io::Error::from(
            std::io::ErrorKind::ConnectionReset,
        ))))
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::Closed)
    ));
    assert!(connection.is_closed());
    assert_eq!(connection.pending_command_count(), 0);
    until(|| probe.dropped.load(Ordering::SeqCst)).await;
}
