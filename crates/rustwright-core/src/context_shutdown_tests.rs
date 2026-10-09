//! A remote reply barrier proves close remains active after a caller disappears.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    commands: mpsc::UnboundedReceiver<Value>,
    replies: mpsc::UnboundedSender<(Value, bool)>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn pair() -> (BrowserContext, Remote) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, commands) = mpsc::unbounded_channel();
    let (replies, mut rx) = mpsc::unbounded_channel::<(Value, bool)>();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        loop {
            tokio::select! {
                incoming = socket.next() => {
                    let Some(Ok(Message::Text(text))) = incoming else { break; };
                    tx.send(serde_json::from_str(&text).unwrap()).unwrap();
                }
                reply = rx.recv() => {
                    let Some((command, fail)) = reply else { break; };
                    let message = if fail {
                        json!({"id":command["id"],"error":{"code":-32000,"message":"disposal refused","data":"fixture"}})
                    } else { json!({"id":command["id"],"result":{}}) };
                    socket.send(Message::Text(message.to_string().into())).await.unwrap();
                }
            }
        }
    });
    let connection = CdpConnection::connect(&url).await.unwrap();
    (
        BrowserContext::new(connection, Some("owned".into())),
        Remote {
            commands,
            replies,
            task,
        },
    )
}
async fn command(remote: &mut Remote) -> Value {
    tokio::time::timeout(Duration::from_secs(1), remote.commands.recv())
        .await
        .unwrap()
        .unwrap()
}
fn closing(context: &BrowserContext) -> JoinHandle<Result<()>> {
    let context = context.clone();
    tokio::spawn(async move { context.close().await })
}
async fn assert_waiting(task: &mut JoinHandle<Result<()>>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(40), task)
            .await
            .is_err(),
        "close must await the original cleanup"
    );
}
async fn finished(task: JoinHandle<Result<()>>) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cancelled_attachment_wait_still_disposes() {
    let (context, mut remote) = pair().await;
    let lock = context.attachments.lock().await;
    let first = closing(&context);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !context.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    drop(lock);
    let dispose = command(&mut remote).await;
    assert_eq!(dispose["method"], "Target.disposeBrowserContext");
    remote.replies.send((dispose, false)).unwrap();
    context.close().await.unwrap();
    assert_eq!(context.connection.pending_command_count(), 0);
    context.connection.close();
}

#[tokio::test]
async fn concurrent_close_waits_for_disposal_acknowledgement() {
    let (context, mut remote) = pair().await;
    let first = closing(&context);
    let dispose = command(&mut remote).await;
    let mut second = closing(&context);
    assert_waiting(&mut second).await;
    remote.replies.send((dispose, false)).unwrap();
    finished(first).await.unwrap();
    finished(second).await.unwrap();
    assert!(remote.commands.try_recv().is_err(), "one disposal command");
    context.connection.close();
}

#[tokio::test]
async fn cancelled_disposal_wait_still_waits_for_acknowledgement() {
    let (context, mut remote) = pair().await;
    let first = closing(&context);
    let dispose = command(&mut remote).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let mut second = closing(&context);
    assert_waiting(&mut second).await;
    assert_eq!(context.connection.pending_command_count(), 1);
    remote.replies.send((dispose, false)).unwrap();
    finished(second).await.unwrap();
    assert_eq!(context.connection.pending_command_count(), 0);
    context.connection.close();
}

#[tokio::test]
async fn disposal_failure_is_visible_and_replayed_without_duplicate_command() {
    let (context, mut remote) = pair().await;
    let first = closing(&context);
    let dispose = command(&mut remote).await;
    remote.replies.send((dispose, true)).unwrap();
    for result in [finished(first).await, context.close().await] {
        assert!(
            matches!(result, Err(Error::Cdp(rustwright_cdp::CdpError::Protocol {code: -32000, ref method, ref data, ..})) if method == "Target.disposeBrowserContext" && data.as_deref() == Some("fixture")),
            "{result:?}"
        );
    }
    assert_eq!(context.connection.pending_command_count(), 0);
    assert!(remote.commands.try_recv().is_err());
    context.connection.close();
}

#[tokio::test]
async fn stalled_attachment_still_attempts_disposal_and_replays_timeout() {
    let (context, mut remote) = pair().await;
    let lock = context.attachments.lock().await;
    let first = closing(&context);
    let dispose = tokio::time::timeout(Duration::from_secs(7), remote.commands.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dispose["method"], "Target.disposeBrowserContext");
    remote.replies.send((dispose, false)).unwrap();
    for result in [finished(first).await, context.close().await] {
        assert!(
            matches!(result, Err(Error::Timeout {ref what, timeout}) if what == "context attachments during shutdown" && timeout == Duration::from_secs(5)),
            "{result:?}"
        );
    }
    assert_eq!(context.connection.pending_command_count(), 0);
    assert!(!context.connection.is_closed());
    drop(lock);
    context.connection.close();
}

#[tokio::test]
async fn stalled_disposal_has_bounded_replayed_error_without_waiter_leak() {
    let (context, mut remote) = pair().await;
    let first = closing(&context);
    let dispose = command(&mut remote).await;
    assert_eq!(dispose["method"], "Target.disposeBrowserContext");
    let result = tokio::time::timeout(Duration::from_secs(7), first)
        .await
        .unwrap()
        .unwrap();
    for result in [result, context.close().await] {
        assert!(
            matches!(result, Err(Error::Cdp(rustwright_cdp::CdpError::Timeout {ref method, timeout})) if method == "Target.disposeBrowserContext" && timeout == Duration::from_secs(5)),
            "{result:?}"
        );
    }
    assert_eq!(context.connection.pending_command_count(), 0);
    assert!(remote.commands.try_recv().is_err());
    assert!(!context.connection.is_closed());
    context.connection.close();
}

#[tokio::test]
async fn default_context_close_keeps_foreign_context_and_connection_open() {
    let (isolated, mut remote) = pair().await;
    let default = BrowserContext::new(isolated.connection.clone(), None);
    default.close().await.unwrap();
    default.close().await.unwrap();
    assert!(default.is_closed());
    assert!(!isolated.is_closed());
    isolated.ensure_open().unwrap();
    assert!(
        remote.commands.try_recv().is_err(),
        "no isolated context disposal or Browser.close"
    );
    let connection = isolated.connection.clone();
    let probe =
        tokio::spawn(async move { connection.send_raw(None, "Test.probe", json!({})).await });
    let request = command(&mut remote).await;
    assert_eq!(request["method"], "Test.probe");
    remote.replies.send((request, false)).unwrap();
    probe.await.unwrap().unwrap();
    assert_eq!(isolated.connection.pending_command_count(), 0);
    isolated.connection.close();
}
