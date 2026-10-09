//! History lifecycle matching, early events, deadlines and cancellation.
use super::{BidiBrowser, BidiPage};
use crate::BidiError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    events: mpsc::UnboundedSender<Value>,
    commands: mpsc::UnboundedReceiver<Value>,
    subscriptions: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn remote(
    hold: Option<&'static str>,
    early: bool,
    reject: bool,
) -> (Remote, BidiBrowser, BidiPage) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let (events, mut event_rx) = mpsc::unbounded_channel::<Value>();
    let (commands, command_rx) = mpsc::unbounded_channel();
    let subscriptions = Arc::new(AtomicUsize::new(0));
    let subscribed = subscriptions.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        loop {
            let message = tokio::select! {
                event = event_rx.recv() => {
                    let Some(event) = event else { break; };
                    if ws.send(Message::text(event.to_string())).await.is_err() { break; }
                    continue;
                },
                message = ws.next() => match message { Some(Ok(message)) => message, _ => break },
            };
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let method = command["method"].as_str().unwrap();
            let result = match method {
                "session.new" => json!({"sessionId":"navigation","capabilities":{}}),
                "browsingContext.create" => json!({"context":"page"}),
                "script.addPreloadScript" => json!({"script":"helper"}),
                "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                "session.subscribe" => {
                    subscribed.fetch_add(1, Ordering::Relaxed);
                    json!({"subscription":"nav"})
                }
                "browsingContext.traverseHistory"
                | "browsingContext.reload"
                | "browsingContext.close"
                | "script.removePreloadScript"
                | "session.end" => json!({}),
                method => panic!("unexpected command {method}"),
            };
            commands.send(command.clone()).unwrap();
            if hold == Some(method) {
                continue;
            }
            if early && method == "browsingContext.traverseHistory" {
                for event in [
                    event("browsingContext.navigationStarted", "page", "current"),
                    event("browsingContext.load", "page", "current"),
                ] {
                    ws.send(Message::text(event.to_string())).await.unwrap();
                }
            }
            let reply = if reject && method == "browsingContext.traverseHistory" {
                json!({"type":"error","id":command["id"],"error":"no such history entry","message":"empty forward history"})
            } else {
                json!({"type":"success","id":command["id"],"result":result})
            };
            if ws.send(Message::text(reply.to_string())).await.is_err() {
                break;
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context, None);
    (
        Remote {
            events,
            commands: command_rx,
            subscriptions,
            task,
        },
        browser,
        page,
    )
}
fn event(method: &str, context: &str, navigation: &str) -> Value {
    json!({"type":"event","method":method,"params":{"context":context,"navigation":navigation,"url":"http://local/page"}})
}
async fn command(remote: &mut Remote, method: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let command = remote.commands.recv().await.unwrap();
            if command["method"] == method {
                return command;
            }
        }
    })
    .await
    .unwrap()
}
async fn pending(task: &mut JoinHandle<crate::BidiResult<()>>) {
    assert!(tokio::time::timeout(Duration::from_millis(30), task)
        .await
        .is_err());
}
async fn done(task: JoinHandle<crate::BidiResult<()>>) -> crate::BidiResult<()> {
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn lifecycle_before_command_acknowledgment_is_not_lost() {
    let (_remote, browser, page) = remote(None, true, false).await;
    page.go_back_with_timeout(Duration::from_secs(1))
        .await
        .unwrap();
    page.go_forward_with_timeout(Duration::from_secs(1))
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn stale_foreign_and_wrong_navigation_loads_do_not_finish_history() {
    let (mut remote, browser, page) = remote(None, false, false).await;
    let mut task = tokio::spawn(async move { page.go_back().await });
    let request = command(&mut remote, "browsingContext.traverseHistory").await;
    assert_eq!(request["params"], json!({"context":"page","delta":-1}));
    for event in [
        event("browsingContext.load", "page", "old"),
        event("browsingContext.navigationStarted", "other", "foreign"),
        event("browsingContext.load", "other", "foreign"),
    ] {
        remote.events.send(event).unwrap();
    }
    pending(&mut task).await;
    remote
        .events
        .send(event(
            "browsingContext.navigationStarted",
            "page",
            "current",
        ))
        .unwrap();
    remote
        .events
        .send(event("browsingContext.load", "page", "old"))
        .unwrap();
    remote
        .events
        .send(event("browsingContext.historyUpdated", "page", "current"))
        .unwrap();
    pending(&mut task).await;
    remote
        .events
        .send(event("browsingContext.load", "page", "current"))
        .unwrap();
    done(task).await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn same_document_history_and_fragment_commits_share_one_subscription() {
    let (mut remote, browser, page) = remote(None, false, false).await;
    for method in [
        "browsingContext.historyUpdated",
        "browsingContext.fragmentNavigated",
    ] {
        let task = tokio::spawn({
            let page = page.clone();
            async move { page.go_forward().await }
        });
        let request = command(&mut remote, "browsingContext.traverseHistory").await;
        assert_eq!(request["params"]["delta"], 1);
        remote
            .events
            .send(event(method, "page", "same-document"))
            .unwrap();
        done(task).await.unwrap();
    }
    assert_eq!(remote.subscriptions.load(Ordering::Relaxed), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn missing_history_entry_preserves_typed_protocol_error() {
    let (_remote, browser, page) = remote(None, false, true).await;
    assert!(
        matches!(page.go_forward().await, Err(BidiError::Protocol { method, error, .. }) if method == "browsingContext.traverseHistory" && error == "no such history entry")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn history_deadline_covers_lifecycle_and_releases_pending_command() {
    let (_remote, browser, page) =
        remote(Some("browsingContext.traverseHistory"), false, false).await;
    let timeout = Duration::from_millis(50);
    assert!(
        matches!(page.go_back_with_timeout(timeout).await, Err(BidiError::Timeout { method, timeout: actual }) if method == "browsingContext.traverseHistory" && actual == timeout)
    );
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_history_releases_command_and_next_traversal_recovers() {
    let (mut remote, browser, page) =
        remote(Some("browsingContext.traverseHistory"), false, false).await;
    let task = tokio::spawn({
        let page = page.clone();
        async move { page.go_back().await }
    });
    let first = command(&mut remote, "browsingContext.traverseHistory").await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    // A late acknowledgement of the cancelled operation cannot satisfy a new one.
    remote
        .events
        .send(json!({"type":"success","id":first["id"],"result":{}}))
        .unwrap();
    let task = tokio::spawn(async move { page.go_forward().await });
    let second = command(&mut remote, "browsingContext.traverseHistory").await;
    remote
        .events
        .send(event("browsingContext.historyUpdated", "page", "new"))
        .unwrap();
    remote
        .events
        .send(json!({"type":"success","id":second["id"],"result":{}}))
        .unwrap();
    done(task).await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn context_destruction_wakes_history_even_with_command_acknowledgment_held() {
    let (mut remote, browser, page) =
        remote(Some("browsingContext.traverseHistory"), false, false).await;
    let task = tokio::spawn({
        let page = page.clone();
        async move { page.go_back().await }
    });
    command(&mut remote, "browsingContext.traverseHistory").await;
    remote
        .events
        .send(event("browsingContext.contextDestroyed", "page", ""))
        .unwrap();
    assert!(matches!(done(task).await, Err(BidiError::Closed)));
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn connection_shutdown_wakes_history_waiter() {
    let (mut remote, browser, page) = remote(None, false, false).await;
    let task = tokio::spawn(async move { page.go_forward().await });
    command(&mut remote, "browsingContext.traverseHistory").await;
    browser.session().connection().close();
    assert!(matches!(done(task).await, Err(BidiError::Closed)));
}

#[tokio::test]
async fn subscription_setup_survives_cancellation_without_duplicate_registration() {
    let (mut remote, browser, page) = remote(Some("session.subscribe"), false, false).await;
    let task = tokio::spawn({
        let page = page.clone();
        async move { page.go_back().await }
    });
    let subscription = command(&mut remote, "session.subscribe").await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote
        .events
        .send(json!({"type":"success","id":subscription["id"],"result":{"subscription":"nav"}}))
        .unwrap();
    let task = tokio::spawn(async move { page.go_forward().await });
    command(&mut remote, "browsingContext.traverseHistory").await;
    remote
        .events
        .send(event("browsingContext.historyUpdated", "page", "new"))
        .unwrap();
    done(task).await.unwrap();
    assert_eq!(remote.subscriptions.load(Ordering::Relaxed), 1);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn history_timeout_includes_subscription_acknowledgment() {
    let (mut remote, browser, page) = remote(Some("session.subscribe"), false, false).await;
    let timeout = Duration::from_millis(50);
    assert!(
        matches!(page.go_back_with_timeout(timeout).await, Err(BidiError::Timeout { method, timeout: actual }) if method == "browsingContext.traverseHistory" && actual == timeout)
    );
    let subscription = command(&mut remote, "session.subscribe").await;
    remote
        .events
        .send(json!({"type":"success","id":subscription["id"],"result":{"subscription":"nav"}}))
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn reload_waits_for_complete_and_uses_one_explicit_deadline() {
    let (mut remote, browser, page) = remote(Some("browsingContext.reload"), false, false).await;
    let timeout = Duration::from_millis(50);
    let task = tokio::spawn({
        let page = page.clone();
        async move { page.reload_with_timeout(timeout).await }
    });
    let reload = command(&mut remote, "browsingContext.reload").await;
    assert_eq!(
        reload["params"],
        json!({"context":"page","wait":"complete"})
    );
    assert!(
        matches!(done(task).await, Err(BidiError::Timeout { method, timeout: actual }) if method == "browsingContext.reload" && actual == timeout)
    );
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    browser.close().await.unwrap();
}
