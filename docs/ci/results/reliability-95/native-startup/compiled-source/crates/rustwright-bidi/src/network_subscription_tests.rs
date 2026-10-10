//! Network setup waits for acknowledged readiness despite caller cancellation.
use super::{BidiBrowser, BidiPage};
use crate::BidiError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Notify},
    task::JoinHandle,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    subscriptions: Arc<Mutex<usize>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    events: mpsc::UnboundedSender<Value>,
    task: JoinHandle<()>,
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.release.notify_one();
        self.task.abort();
    }
}

async fn remote(hold: bool, reject: bool) -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let subscriptions = Arc::new(Mutex::new(0));
    let subscribed = subscriptions.clone();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let arrived = entered.clone();
    let released = release.clone();
    let (events, mut event_rx) = mpsc::unbounded_channel::<Value>();
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
                message = ws.next() => match message {
                    Some(Ok(message)) => message,
                    _ => break,
                },
            };
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let mut failure = false;
            let result = match command["method"].as_str().unwrap() {
                "session.new" => json!({"sessionId":"test","capabilities":{}}),
                "browsingContext.create" => json!({"context":"page"}),
                "browsingContext.close" | "script.removePreloadScript" | "session.end" => json!({}),
                "script.addPreloadScript" => json!({"script":"helper"}),
                "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                "session.subscribe" => {
                    let attempt = {
                        let mut count = subscribed.lock().unwrap();
                        *count += 1;
                        *count
                    };
                    if attempt == 1 && hold {
                        arrived.notify_one();
                        loop {
                            tokio::select! {
                                () = released.notified() => break,
                                event = event_rx.recv() => {
                                    let Some(event) = event else { return; };
                                    if ws.send(Message::text(event.to_string())).await.is_err() {
                                        return;
                                    }
                                },
                            }
                        }
                    }
                    failure = attempt == 1 && reject;
                    json!({"subscription":format!("subscription-{attempt}")})
                }
                method => panic!("unexpected command {method}"),
            };
            let reply = if failure {
                json!({"id":command["id"],"type":"error","error":"invalid argument","message":"subscription rejected"})
            } else {
                json!({"id":command["id"],"type":"success","result":result})
            };
            if ws.send(Message::text(reply.to_string())).await.is_err() {
                break;
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    (
        Remote {
            subscriptions,
            entered,
            release,
            events,
            task,
        },
        browser,
    )
}

async fn entered(remote: &Remote) {
    tokio::time::timeout(Duration::from_secs(1), remote.entered.notified())
        .await
        .unwrap();
}

async fn still_waiting(task: &mut JoinHandle<crate::BidiResult<()>>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(30), task)
            .await
            .is_err(),
        "readiness must wait for subscription acknowledgment"
    );
}

fn pump_id(page: &BidiPage) -> tokio::task::Id {
    page.network_pump
        .lock()
        .unwrap()
        .as_ref()
        .expect("ready monitoring must have a pump")
        .task_id()
}

async fn observe_requests(remote: &Remote, page: &BidiPage) {
    for id in ["one", "two"] {
        remote.events.send(json!({"type":"event","method":"network.beforeRequestSent","params":{"context":"page","request":{"request":id,"url":format!("http://localhost/{id}"),"method":"GET"}}})).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while page.network_requests().len() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("monitoring must observe both requests after setup");
}

#[tokio::test]
async fn concurrent_session_subscription_waits_for_the_same_successful_handshake() {
    let (remote, browser) = remote(true, false).await;
    let session = browser.session().clone();
    let first = tokio::spawn(async move { session.ensure_network_subscription().await });
    entered(&remote).await;
    let session = browser.session().clone();
    let mut second = tokio::spawn(async move { session.ensure_network_subscription().await });
    still_waiting(&mut second).await;
    remote.release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(*remote.subscriptions.lock().unwrap(), 1);
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelling_subscription_waiter_keeps_acknowledgment_owned() {
    let (remote, browser) = remote(true, false).await;
    let session = browser.session().clone();
    let first = tokio::spawn(async move { session.ensure_network_subscription().await });
    entered(&remote).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let session = browser.session().clone();
    let mut next = tokio::spawn(async move { session.ensure_network_subscription().await });
    still_waiting(&mut next).await;
    remote.release.notify_one();
    next.await.unwrap().unwrap();
    assert_eq!(*remote.subscriptions.lock().unwrap(), 1);
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn rejected_subscription_preserves_error_and_queued_caller_retries() {
    let (remote, browser) = remote(true, true).await;
    let session = browser.session().clone();
    let first = tokio::spawn(async move { session.ensure_network_subscription().await });
    entered(&remote).await;
    let session = browser.session().clone();
    let mut next = tokio::spawn(async move { session.ensure_network_subscription().await });
    still_waiting(&mut next).await;
    remote.release.notify_one();
    assert!(
        matches!(first.await.unwrap(), Err(BidiError::Protocol { error, .. }) if error == "invalid argument")
    );
    next.await.unwrap().unwrap();
    browser
        .session()
        .ensure_network_subscription()
        .await
        .unwrap();
    assert_eq!(*remote.subscriptions.lock().unwrap(), 2);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_monitoring_setup_finishes_with_one_reused_pump() {
    let (remote, browser) = remote(true, false).await;
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context, None);
    let starting = page.clone();
    let first = tokio::spawn(async move { starting.start_network_monitoring().await });
    entered(&remote).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let starting = page.clone();
    let mut next = tokio::spawn(async move { starting.start_network_monitoring().await });
    still_waiting(&mut next).await;
    remote.release.notify_one();
    next.await.unwrap().unwrap();
    let original = pump_id(&page);
    let mut starts = Vec::new();
    for _ in 0..20 {
        let starting = page.clone();
        starts.push(tokio::spawn(async move {
            starting.start_network_monitoring().await
        }));
    }
    for start in starts {
        start.await.unwrap().unwrap();
    }
    assert_eq!(pump_id(&page), original);
    assert_eq!(*remote.subscriptions.lock().unwrap(), 1);
    observe_requests(&remote, &page).await;
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn rejected_monitoring_setup_can_retry_without_installing_a_failed_pump() {
    let (remote, browser) = remote(false, true).await;
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context, None);
    assert!(
        matches!(page.start_network_monitoring().await, Err(BidiError::Protocol { error, .. }) if error == "invalid argument")
    );
    assert!(page.network_pump.lock().unwrap().is_none());
    page.start_network_monitoring().await.unwrap();
    let original = pump_id(&page);
    page.start_network_monitoring().await.unwrap();
    assert_eq!(pump_id(&page), original);
    assert_eq!(*remote.subscriptions.lock().unwrap(), 2);
    observe_requests(&remote, &page).await;
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn silent_subscription_has_a_bounded_local_waiter() {
    let (remote, browser) = remote(true, false).await;
    let session = browser.session().clone();
    let waiting = tokio::spawn(async move { session.ensure_network_subscription().await });
    entered(&remote).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    let result = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("subscription setup must finish within its five-second local bound")
        .unwrap();
    tokio::time::resume();
    assert!(
        matches!(result, Err(BidiError::Timeout { method, timeout }) if method == "session.subscribe" && timeout == Duration::from_secs(5))
    );
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    remote.release.notify_one();
    // Acknowledgment loss leaves remote state uncertain; a local retry is allowed.
    browser
        .session()
        .ensure_network_subscription()
        .await
        .unwrap();
    assert_eq!(*remote.subscriptions.lock().unwrap(), 2);
    browser.close().await.unwrap();
}

async fn pre_ack_events_remain_observable(cancel_first_waiter: bool) {
    let (remote, browser) = remote(true, false).await;
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context, None);
    let mut transport_events = browser.session().events();
    let starting = page.clone();
    let mut setup = tokio::spawn(async move { starting.start_network_monitoring().await });
    entered(&remote).await;
    if cancel_first_waiter {
        setup.abort();
        assert!(setup.await.unwrap_err().is_cancelled());
        let starting = page.clone();
        setup = tokio::spawn(async move { starting.start_network_monitoring().await });
    }
    for event in [
        json!({"type":"event","method":"network.beforeRequestSent","params":{"context":"page","request":{"request":"pre-ack","url":"http://localhost/pre-ack","method":"GET"}}}),
        json!({"type":"event","method":"network.responseCompleted","params":{"context":"page","request":{"request":"pre-ack"},"response":{"status":201,"statusText":"Created","mimeType":"text/plain"}}}),
    ] {
        remote.events.send(event).unwrap();
    }
    // The peer withholds ACK until the connection has dispatched completion.
    // This proves both events predate local setup readiness, without a sleep.
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let event = transport_events.recv().await.unwrap();
            if event.method == "network.responseCompleted"
                && event.params["request"]["request"] == "pre-ack"
            {
                break;
            }
        }
    })
    .await
    .expect("the transport must receive pre-ack completion before ACK is released");
    still_waiting(&mut setup).await;
    assert!(page.network_pump.lock().unwrap().is_none());
    remote.release.notify_one();
    setup.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let requests = page.network_requests();
            if requests.len() == 1 && requests[0].status == Some(201) {
                assert_eq!(requests[0].request_id, "pre-ack");
                assert_eq!(requests[0].url, "http://localhost/pre-ack");
                assert_eq!(requests[0].method, "GET");
                assert_eq!(requests[0].status_text.as_deref(), Some("Created"));
                assert_eq!(requests[0].mime_type.as_deref(), Some("text/plain"));
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("monitoring must retain request and completion received before subscribe ACK");
    let original = pump_id(&page);
    page.start_network_monitoring().await.unwrap();
    assert_eq!(pump_id(&page), original);
    assert_eq!(*remote.subscriptions.lock().unwrap(), 1);
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn monitoring_retains_events_received_before_subscription_acknowledgment() {
    pre_ack_events_remain_observable(false).await;
}

#[tokio::test]
async fn cancelled_monitoring_retains_events_received_before_subscription_acknowledgment() {
    pre_ack_events_remain_observable(true).await;
}

fn diagnostic_pump(page: &BidiPage) -> tokio::task::AbortHandle {
    page.network_pump
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .abort_handle()
}

async fn finished(pump: &tokio::task::AbortHandle) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !pump.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closed resource must release its diagnostic pump without dropping retained handles");
}

#[tokio::test]
async fn retained_page_diagnostics_stop_on_browser_close_without_remote_events() {
    let (_remote, browser) = remote(false, false).await;
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context, None);
    page.start_network_monitoring().await.unwrap();
    let pump = diagnostic_pump(&page);
    browser.close().await.unwrap();
    assert!(page.session.connection().is_closed());
    finished(&pump).await;
    assert!(matches!(
        page.start_network_monitoring().await,
        Err(BidiError::Closed)
    ));
}

#[tokio::test]
async fn closing_one_page_alias_stops_all_diagnostics_without_a_destroy_event() {
    let (_remote, browser) = remote(false, false).await;
    let context = browser.session().create_context().await.unwrap();
    let page = BidiPage::from_context(browser.session().clone(), context.clone(), None);
    let alias = BidiPage::from_context(browser.session().clone(), context, None);
    page.start_network_monitoring().await.unwrap();
    alias.start_network_monitoring().await.unwrap();
    let first = diagnostic_pump(&page);
    let second = diagnostic_pump(&alias);
    page.close().await.unwrap();
    assert!(!browser.session().connection().is_closed());
    finished(&first).await;
    finished(&second).await;
    assert!(matches!(
        alias.start_network_monitoring().await,
        Err(BidiError::Closed)
    ));
    browser.close().await.unwrap();
}
