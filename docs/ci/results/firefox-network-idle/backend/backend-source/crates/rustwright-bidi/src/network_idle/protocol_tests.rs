use super::*;
use crate::BidiBrowser;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::{
    net::TcpListener,
    sync::{mpsc, Notify},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    events: mpsc::UnboundedSender<Value>,
    commands: mpsc::UnboundedReceiver<Value>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.release.notify_one();
        self.task.abort();
    }
}
impl Remote {
    async fn command(&mut self, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let cmd = self.commands.recv().await.unwrap();
                if cmd["method"] == method {
                    return cmd;
                }
            }
        })
        .await
        .expect("peer must receive expected command")
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(1), self.entered.notified())
            .await
            .unwrap();
    }
    fn event(&self, event: BidiEvent) {
        self.events
            .send(json!({"type":"event","method":event.method,"params":event.params}))
            .unwrap();
    }
}
async fn remote(hold: Option<&'static str>, reject: bool, child: bool) -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let (events, mut event_rx) = mpsc::unbounded_channel::<Value>();
    let (commands, command_rx) = mpsc::unbounded_channel();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let arrived = entered.clone();
    let released = release.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut held = false;
        let mut pages = HashSet::new();
        let mut sequence = 0;
        loop {
            let msg = tokio::select! { event=event_rx.recv()=>{let Some(event)=event else{break;};if ws.send(Message::text(event.to_string())).await.is_err(){break;}continue;},msg=ws.next()=>match msg{Some(Ok(msg))=>msg,_=>break}};
            if !msg.is_text() {
                continue;
            }
            let cmd: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            commands.send(cmd.clone()).unwrap();
            let method = cmd["method"].as_str().unwrap();
            let p = &cmd["params"];
            let result = match method {
                "session.new" => json!({"sessionId":"idle","capabilities":{}}),
                "browsingContext.create" => {
                    sequence += 1;
                    let id = if sequence == 1 {
                        "root".to_owned()
                    } else {
                        format!("page{sequence}")
                    };
                    pages.insert(id.clone());
                    json!({"context":id})
                }
                "script.addPreloadScript" => json!({"script":"helper"}),
                "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                "browsingContext.getTree" => {
                    let selected = p["root"].as_str();
                    let contexts=pages.iter().filter(|id|selected.is_none_or(|root|id.as_str()==root)).map(|id|json!({"context":id,"url":"about:blank","children":if child{vec![json!({"context":"child","children":[{"context":"nested","children":[]}]})]}else{vec![]}})).collect::<Vec<_>>();
                    json!({"contexts":contexts})
                }
                "browsingContext.close" => {
                    pages.remove(p["context"].as_str().unwrap());
                    json!({})
                }
                "session.subscribe" | "script.removePreloadScript" | "session.end" => json!({}),
                m => panic!("unexpected command {m}"),
            };
            let should_hold = hold == Some(method) && !held;
            if should_hold {
                held = true;
                arrived.notify_one();
                loop {
                    tokio::select! {()=released.notified()=>break,event=event_rx.recv()=>{let Some(event)=event else{return;};if ws.send(Message::text(event.to_string())).await.is_err(){return;}}}
                }
            }
            let reply = if should_hold && reject {
                json!({"type":"error","id":cmd["id"],"error":"invalid argument","message":"rejected setup"})
            } else {
                json!({"type":"success","id":cmd["id"],"result":result})
            };
            if ws.send(Message::text(reply.to_string())).await.is_err() {
                break;
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    (
        Remote {
            events,
            commands: command_rx,
            entered,
            release,
            task,
        },
        browser,
    )
}
async fn delivered(rx: &mut broadcast::Receiver<BidiEvent>, method: &str, id: &str) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let e = rx.recv().await.unwrap();
            if e.method == method && e.params["request"]["request"] == id {
                break;
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn public_creation_waits_for_ack_and_retains_early_nested_requests() {
    let (remote, browser) = remote(Some("session.subscribe"), false, true).await;
    let browser = Arc::new(browser);
    let creating = browser.clone();
    let mut page = tokio::spawn(async move { creating.new_page().await });
    remote.entered().await;
    let mut wire = browser.session().events();
    remote.event(start("nested", "early", 0));
    delivered(&mut wire, "network.beforeRequestSent", "early").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut page)
            .await
            .is_err(),
        "new page must not be exposed before ACK"
    );
    remote.release.notify_one();
    let page = page.await.unwrap().unwrap();
    assert!(matches!(
        page.wait_for_network_idle_with_timeout(Duration::from_millis(750))
            .await,
        Err(BidiError::Timeout { .. })
    ));
    remote.event(finish("nested", "early", 0));
    page.wait_for_network_idle_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    page.close().await.unwrap();
    Arc::try_unwrap(browser).unwrap().close().await.unwrap();
}
#[tokio::test]
async fn discovered_handles_reuse_created_observer_and_unobserved_pages_are_rejected() {
    let (_remote, browser) = remote(None, false, false).await;
    let page = browser.new_page().await.unwrap();
    let found = browser.pages().await.unwrap().pop().unwrap();
    assert!(Arc::ptr_eq(&page.idle, &found.idle));
    found
        .wait_for_network_idle_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    let id = browser.session().create_context().await.unwrap();
    let cold = browser
        .pages()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.context_id() == id)
        .unwrap();
    assert!(matches!(
        cold.wait_for_network_idle_with_timeout(Duration::from_secs(1))
            .await,
        Err(BidiError::NetworkObservationIncomplete)
    ));
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelling_new_page_during_setup_closes_allocation_and_does_not_poison_retry() {
    let (mut remote, browser) = remote(Some("session.subscribe"), false, false).await;
    let browser = Arc::new(browser);
    let creating = browser.clone();
    let first = tokio::spawn(async move { creating.new_page().await });
    remote.entered().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    let closed = remote.command("browsingContext.close").await;
    assert_eq!(closed["params"]["context"], "root");
    let page = browser.new_page().await.unwrap();
    page.wait_for_network_idle_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    page.close().await.unwrap();
    Arc::try_unwrap(browser).unwrap().close().await.unwrap();
}
#[tokio::test]
async fn rejected_setup_closes_unexposed_page_and_next_creation_retries() {
    let (mut remote, browser) = remote(Some("session.subscribe"), true, false).await;
    let browser = Arc::new(browser);
    let creating = browser.clone();
    let first = tokio::spawn(async move { creating.new_page().await });
    remote.entered().await;
    remote.release.notify_one();
    assert!(
        matches!(first.await.unwrap(),Err(BidiError::Protocol{error,..}) if error=="invalid argument")
    );
    assert_eq!(
        remote.command("browsingContext.close").await["params"]["context"],
        "root"
    );
    let page = browser.new_page().await.unwrap();
    page.wait_for_network_idle_with_timeout(Duration::from_secs(2))
        .await
        .unwrap();
    page.close().await.unwrap();
    Arc::try_unwrap(browser).unwrap().close().await.unwrap();
}
#[tokio::test]
async fn held_snapshot_overflow_fails_setup_instead_of_claiming_idle() {
    let (mut remote, browser) = remote(Some("browsingContext.getTree"), false, false).await;
    let browser = Arc::new(browser);
    let creating = browser.clone();
    let first = tokio::spawn(async move { creating.new_page().await });
    remote.entered().await;
    let mut wire = browser.session().events();
    for n in 0..2200 {
        remote.event(start("root", &format!("overflow-{n}"), 0));
    }
    // A second receiver is also intentionally behind. Drain its lag indication
    // and wait for the last event to prove every event reached the transport.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match wire.recv().await {
                Ok(e) if e.params["request"]["request"] == "overflow-2199" => break,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .unwrap();
    remote.release.notify_one();
    assert!(matches!(first.await.unwrap(),Err(BidiError::NetworkEventsLost{skipped}) if skipped>0));
    assert_eq!(
        remote.command("browsingContext.close").await["params"]["context"],
        "root"
    );
    Arc::try_unwrap(browser).unwrap().close().await.unwrap();
}
#[tokio::test]
async fn page_close_and_disconnect_wake_pending_waiters() {
    for disconnect in [false, true] {
        let (remote, browser) = remote(None, false, false).await;
        let page = browser.new_page().await.unwrap();
        remote.event(start("root", "held", 0));
        tokio::time::timeout(Duration::from_secs(1), async {
            while page.idle.state.lock().unwrap().requests.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let waiting = page.clone();
        let waiter = tokio::spawn(async move { waiting.wait_for_network_idle().await });
        tokio::task::yield_now().await;
        if disconnect {
            remote.task.abort();
        } else {
            page.close().await.unwrap();
        }
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap(),
            Err(BidiError::Closed)
        ));
        browser.close().await.unwrap();
    }
}
#[tokio::test]
async fn dropped_last_observer_aborts_its_pump_without_a_registry_cycle() {
    let (_remote, browser) = remote(None, false, false).await;
    let page = browser.new_page().await.unwrap();
    let observer = Arc::downgrade(&page.idle);
    let task = page
        .idle
        .pump
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .abort_handle();
    drop(page);
    tokio::time::timeout(Duration::from_secs(1), async {
        while observer.strong_count() != 0 || !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_idle_setup_survives_cancel_and_reuses_one_acknowledged_pump() {
    let (mut remote, browser) = remote(Some("session.subscribe"), false, false).await;
    let root = browser.session().create_context().await.unwrap();
    let observer = browser.session().page_idle(&root, None);
    let starting = observer.clone();
    let session = browser.session().clone();
    let first = tokio::spawn(async move { starting.initialize(session).await });
    remote.entered().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let starting = observer.clone();
    let session = browser.session().clone();
    let mut second = tokio::spawn(async move { starting.initialize(session).await });
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut second)
        .await
        .is_err());
    remote.release.notify_one();
    second.await.unwrap().unwrap();
    let original = observer.pump.lock().unwrap().as_ref().unwrap().id();
    for _ in 0..20 {
        observer
            .initialize(browser.session().clone())
            .await
            .unwrap();
    }
    assert_eq!(
        observer.pump.lock().unwrap().as_ref().unwrap().id(),
        original
    );
    let mut subscriptions = 0;
    while let Ok(command) = remote.commands.try_recv() {
        subscriptions += usize::from(command["method"] == "session.subscribe");
    }
    assert_eq!(subscriptions, 1);
    observer.wait(Duration::from_secs(2)).await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn silent_idle_setup_has_a_finite_budget_and_a_late_ack_does_not_poison_retry() {
    let (remote, browser) = remote(Some("session.subscribe"), false, false).await;
    let root = browser.session().create_context().await.unwrap();
    let observer = browser.session().page_idle(&root, None);
    let starting = observer.clone();
    let session = browser.session().clone();
    let first = tokio::spawn(async move { starting.initialize(session).await });
    remote.entered().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    assert!(
        matches!(first.await.unwrap(), Err(BidiError::Timeout { timeout, .. }) if timeout == Duration::from_secs(5))
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while browser.session().connection().pending_command_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::resume();
    assert!(observer.pump.lock().unwrap().is_none());
    assert!(matches!(
        observer.wait(Duration::from_secs(1)).await,
        Err(BidiError::NetworkObservationIncomplete)
    ));
    remote.release.notify_one();
    observer
        .initialize(browser.session().clone())
        .await
        .unwrap();
    observer.wait(Duration::from_secs(2)).await.unwrap();
    browser.close().await.unwrap();
}
