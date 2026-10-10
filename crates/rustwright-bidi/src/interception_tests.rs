//! Protocol regressions for page-owned interception registrations.
use crate::{BidiBrowser, BidiError};
use futures_util::{SinkExt, StreamExt};
use rustwright_common::RouteAction;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Notify},
    task::JoinHandle,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Default)]
struct State {
    ids: HashSet<String>,
    methods: HashMap<String, usize>,
    fail: Option<&'static str>,
    pause: Option<&'static str>,
    reject_removal: bool,
}
struct Remote {
    state: Arc<Mutex<State>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    events: mpsc::UnboundedSender<Value>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Remote {
    fn count(&self) -> usize {
        self.state.lock().unwrap().ids.len()
    }
    fn calls(&self, m: &str) -> usize {
        *self.state.lock().unwrap().methods.get(m).unwrap_or(&0)
    }
    fn fail(&self, m: &'static str) {
        self.state.lock().unwrap().fail = Some(m)
    }
    fn pause(&self, m: &'static str) {
        self.state.lock().unwrap().pause = Some(m)
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(2), self.entered.notified())
            .await
            .unwrap();
    }
    fn id(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .ids
            .iter()
            .next()
            .unwrap()
            .clone()
    }
    fn event(&self, method: &str, ids: Vec<String>, context: &str) {
        self.events.send(json!({"type":"event","method":method,"params":{"context":context,"isBlocked":true,"intercepts":ids,"request":{"request":"request-1","url":if method=="network.responseStarted"{"http://fixture/response"}else{"http://fixture/match"},"headers":[]},"response":{"headers":[]}}})).unwrap();
    }
    async fn calls_is(&self, method: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.calls(method) < count {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn count_is(&self, n: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.count() != n {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}
async fn remote() -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(State::default()));
    let remote = state.clone();
    let entered = Arc::new(Notify::new());
    let signal = entered.clone();
    let release = Arc::new(Notify::new());
    let resume = release.clone();
    let (events, mut incoming) = mpsc::unbounded_channel::<Value>();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut next = 0;
        let mut held = None;
        let mut pages = HashMap::<String, String>::new();
        loop {
            tokio::select! {
             Some(event)=incoming.recv()=>{ws.send(Message::text(event.to_string())).await.unwrap();}
             _=resume.notified(),if held.is_some()=>{ws.send(Message::text(held.take().unwrap())).await.unwrap();}
             message=ws.next()=>{let Some(Ok(message))=message else {break};if !message.is_text(){continue}
              let command:Value=serde_json::from_str(message.to_text().unwrap()).unwrap();let method=command["method"].as_str().unwrap();let p=&command["params"];
              let (fail,pause)={let mut s=remote.lock().unwrap();*s.methods.entry(method.to_owned()).or_default()+=1;let fail=s.fail==Some(method)||(s.reject_removal&&method=="network.removeIntercept");let pause=s.pause==Some(method);if fail{s.fail=None}
              if pause{s.pause=None}(fail,pause)};
              let reply=if fail {json!({"id":command["id"],"type":"error","error":"unknown error","message":"fixture rejection"})}else{
               let result=match method{
                "session.new"=>json!({"sessionId":"mock","capabilities":{}}),
                "browser.createUserContext"=>json!({"userContext":"owned"}),
                "browsingContext.create"=>{next+=1;let tab=format!("tab-{next}");pages.insert(tab.clone(),p["userContext"].as_str().unwrap_or("default").to_owned());json!({"context":tab})},
                "browsingContext.getTree"=>json!({"contexts":pages.iter().map(|(id,owner)|json!({"context":id,"userContext":owner,"children":[]})).collect::<Vec<_>>() }),
                "script.addPreloadScript"=>{next+=1;json!({"script":format!("helper-{next}")})},
                "script.evaluate"=>json!({"type":"success","result":{"type":"undefined"}}),
                "network.addIntercept"=>{next+=1;let id=format!("intercept-{next}");remote.lock().unwrap().ids.insert(id.clone());json!({"intercept":id})},
                "network.removeIntercept"=>{if remote.lock().unwrap().ids.remove(p["intercept"].as_str().unwrap()){json!({})}else{json!({"fixtureMissing":true})}},
                "browsingContext.close"=>{pages.remove(p["context"].as_str().unwrap());json!({})},
                "browser.removeUserContext"=>{pages.retain(|_,owner|Some(owner.as_str())!=p["userContext"].as_str());json!({})},
                "browsingContext.activate" | "session.subscribe"|"script.removePreloadScript"|"session.end"|"network.continueRequest"|"network.continueResponse"|"network.failRequest"=>json!({}),
                other=>panic!("unexpected method {other}")
               };if result["fixtureMissing"]==true{json!({"id":command["id"],"type":"error","error":"no such intercept","message":"already removed"})}else{json!({"id":command["id"],"type":"success","result":result})}
              }.to_string();
              if pause {held=Some(reply);signal.notify_one();}else if ws.send(Message::text(reply)).await.is_err(){break}
             }
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    (
        Remote {
            state,
            entered,
            release,
            events,
            task,
        },
        browser,
    )
}
#[tokio::test]
async fn rejected_registration_is_retryable() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.fail("network.addIntercept");
    assert!(p.block("*one*").await.is_err());
    p.block("*two*").await.unwrap();
    assert_eq!(r.count(), 1);
    assert_eq!(r.calls("network.addIntercept"), 2);
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn removal_error_is_typed_and_retains_the_id() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    r.fail("network.removeIntercept");
    assert!(matches!(
        p.clear_routes().await,
        Err(BidiError::Protocol { .. })
    ));
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn canceled_registration_reclaims_the_late_id() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.pause("network.addIntercept");
    let c = p.clone();
    let task = tokio::spawn(async move { c.block("*").await });
    r.entered().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    r.count_is(0).await;
    p.block("*").await.unwrap();
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn clear_continues_after_waiter_cancellation() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    r.pause("network.removeIntercept");
    let c = p.clone();
    let task = tokio::spawn(async move { c.clear_routes().await });
    r.entered().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    tokio::time::sleep(Duration::from_millis(30)).await;
    p.block("*").await.unwrap();
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn concurrent_registration_does_not_return_before_ack() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.pause("network.addIntercept");
    let c = p.clone();
    let first = tokio::spawn(async move { c.block("*one*").await });
    r.entered().await;
    let c = p.clone();
    let second = tokio::spawn(async move { c.block("*two*").await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!second.is_finished());
    r.release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn final_routed_page_handle_drop_removes_registration() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    drop(p);
    r.count_is(0).await;
    assert_eq!(b.pages().await.unwrap().len(), 1);
}
#[tokio::test]
async fn discovered_handles_share_routes_and_keep_registration_alive() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*one*").await.unwrap();
    let discovered = b.pages().await.unwrap().pop().unwrap();
    discovered.block("*two*").await.unwrap();
    assert_eq!(r.count(), 1);
    drop(p);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(r.count(), 1);
    discovered.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn page_close_retires_routes_while_handle_is_retained() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    p.close().await.unwrap();
    assert_eq!(r.count(), 0);
    assert!(p.block("*").await.is_err());
}
#[tokio::test]
async fn user_context_close_retires_only_owned_routes() {
    let (r, b) = remote().await;
    let keeper = b.new_page().await.unwrap();
    keeper.block("*").await.unwrap();
    let c = b.new_context().await.unwrap();
    let p = c.new_page().await.unwrap();
    p.block("*").await.unwrap();
    c.close().await.unwrap();
    assert_eq!(r.count(), 1);
    assert!(p.block("*").await.is_err());
    keeper.clear_routes().await.unwrap();
}
#[tokio::test]
async fn failed_phase_expansion_preserves_existing_registration() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*one*").await.unwrap();
    r.fail("network.removeIntercept");
    assert!(matches!(
        p.route("*two*", RouteAction::SetResponseHeaders(vec![]))
            .await,
        Err(BidiError::Protocol { .. })
    ));
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn failed_broader_registration_keeps_previous_phases_usable() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let previous = r.id();
    r.fail("network.addIntercept");
    assert!(p
        .route("*response*", RouteAction::SetResponseHeaders(vec![]))
        .await
        .is_err());
    assert_eq!(r.count(), 1);
    assert_eq!(r.id(), previous);
    r.event("network.beforeRequestSent", vec![previous], "child-frame");
    r.calls_is("network.failRequest", 1).await;
    p.block("*later*").await.unwrap();
    assert_eq!(r.calls("network.addIntercept"), 2);
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn canceled_pre_ack_event_continues_instead_of_being_lost() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.pause("network.addIntercept");
    let c = p.clone();
    let task = tokio::spawn(async move { c.block("*match*").await });
    r.entered().await;
    r.event("network.beforeRequestSent", vec![r.id()], "child-frame");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    r.calls_is("network.continueRequest", 1).await;
    r.count_is(0).await;
    assert_eq!(r.calls("network.failRequest"), 0);
}
#[tokio::test]
async fn descendant_events_use_owned_ids_and_foreign_intercepts_are_untouched() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let owned = r.id();
    r.event(
        "network.beforeRequestSent",
        vec!["foreign".into()],
        p.context_id(),
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(r.calls("network.failRequest"), 0);
    assert_eq!(r.calls("network.continueRequest"), 0);
    r.event(
        "network.beforeRequestSent",
        vec![owned, "foreign".into()],
        "child-frame",
    );
    r.calls_is("network.failRequest", 1).await;
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn retiring_intercept_continues_blocked_requests_until_ack() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let owned = r.id();
    r.pause("network.removeIntercept");
    let c = p.clone();
    let clear = tokio::spawn(async move { c.clear_routes().await });
    r.entered().await;
    r.event("network.beforeRequestSent", vec![owned], "child-frame");
    r.calls_is("network.continueRequest", 1).await;
    assert!(!clear.is_finished());
    r.release.notify_one();
    clear.await.unwrap().unwrap();
    assert_eq!(p.routing.event_id_count().await, 0);
}
#[tokio::test]
async fn failed_drop_retirement_is_reused_by_discovered_handle() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let previous = r.id();
    r.fail("network.removeIntercept");
    drop(p);
    r.calls_is("network.removeIntercept", 1).await;
    let p = b.pages().await.unwrap().pop().unwrap();
    p.block("*another*").await.unwrap();
    assert_ne!(r.id(), previous);
    assert_eq!(r.calls("network.removeIntercept"), 2);
    assert_eq!(r.calls("network.addIntercept"), 2);
    p.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn repeated_canceled_upgrades_do_not_retain_historical_intercept_ids() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    for _ in 0..40 {
        r.pause("network.addIntercept");
        let c = p.clone();
        let upgrade = tokio::spawn(async move {
            c.route("*response*", RouteAction::SetResponseHeaders(vec![]))
                .await
        });
        r.entered().await;
        upgrade.abort();
        assert!(upgrade.await.unwrap_err().is_cancelled());
        r.release.notify_one();
        r.count_is(1).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while p.routing.event_id_count().await != 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    p.clear_routes().await.unwrap();
    assert_eq!(p.routing.event_id_count().await, 0);
}
#[tokio::test]
async fn cancellation_after_old_phase_removal_preserves_previous_rules() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    r.pause("network.removeIntercept");
    let c = p.clone();
    let upgrade = tokio::spawn(async move {
        c.route("*response*", RouteAction::SetResponseHeaders(vec![]))
            .await
    });
    r.entered().await;
    upgrade.abort();
    assert!(upgrade.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    p.block("*later*").await.unwrap();
    assert_eq!(r.count(), 1);
    r.event("network.beforeRequestSent", vec![r.id()], "child-frame");
    r.calls_is("network.failRequest", 1).await;
    r.event("network.responseStarted", vec![r.id()], "child-frame");
    r.calls_is("network.continueResponse", 1).await;
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn closing_during_unacknowledged_add_closes_remote_and_removes_helper() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.pause("network.addIntercept");
    let c = p.clone();
    let registration = tokio::spawn(async move { c.block("*").await });
    r.entered().await;
    let c = p.clone();
    let closing = tokio::spawn(async move { c.close().await });
    r.calls_is("browsingContext.close", 1).await;
    r.calls_is("script.removePreloadScript", 1).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(100), p.block("*"))
            .await
            .unwrap(),
        Err(BidiError::Closed)
    ));
    r.release.notify_one();
    assert!(registration.await.unwrap().is_err());
    closing.await.unwrap().unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn context_close_marks_every_page_closed_before_waiting_for_late_add() {
    let (r, b) = remote().await;
    let context = b.new_context().await.unwrap();
    let first = context.new_page().await.unwrap();
    let second = context.new_page().await.unwrap();
    second.block("*").await.unwrap();
    r.pause("network.addIntercept");
    let page = first.clone();
    let registration = tokio::spawn(async move { page.block("*").await });
    r.entered().await;
    let c = context.clone();
    let closing = tokio::spawn(async move { c.close().await });
    r.calls_is("browser.removeUserContext", 1).await;
    r.calls_is("script.removePreloadScript", 2).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(100), second.block("*"))
            .await
            .unwrap(),
        Err(BidiError::Closed)
    ));
    r.release.notify_one();
    assert!(registration.await.unwrap().is_err());
    closing.await.unwrap().unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn route_removal_failure_does_not_skip_native_close_or_helper_cleanup() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    r.fail("network.removeIntercept");
    assert!(
        matches!(p.close().await,Err(BidiError::Protocol{method,..})if method=="network.removeIntercept")
    );
    assert_eq!(r.calls("browsingContext.close"), 1);
    assert_eq!(r.calls("script.removePreloadScript"), 1);
    assert_eq!(r.count(), 1);
    p.clear_routes().await.unwrap();
    assert_eq!(r.count(), 0);
}
#[tokio::test]
async fn final_handle_drop_outside_runtime_uses_its_origin_runtime() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    std::thread::spawn(move || drop(p)).join().unwrap();
    r.count_is(0).await;
    assert_eq!(b.pages().await.unwrap().len(), 1);
}
#[tokio::test]
async fn persistent_drop_rejection_exits_after_explicit_browser_close() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*").await.unwrap();
    let core = p.routing.weak_core();
    let connection = b.session().connection().clone();
    r.state.lock().unwrap().reject_removal = true;
    drop(p);
    r.calls_is("network.removeIntercept", 1).await;
    assert!(core.upgrade().is_some());
    b.close().await.unwrap();
    assert!(connection.is_closed());
    tokio::time::timeout(Duration::from_secs(2), async {
        while core.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(connection.pending_command_count(), 0);
}
#[tokio::test]
async fn acknowledged_late_removal_is_reconciled_before_a_new_route() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let previous = r.id();
    r.pause("network.removeIntercept");
    assert!(
        matches!(p.clear_routes().await,Err(BidiError::Timeout{method,..})if method=="network.removeIntercept")
    );
    assert_eq!(r.count(), 0);
    p.block("*match*").await.unwrap();
    assert_eq!(r.count(), 1);
    assert_ne!(r.id(), previous);
    assert_eq!(r.calls("network.removeIntercept"), 2);
    assert_eq!(r.calls("network.addIntercept"), 2);
    r.release.notify_one();
    r.event("network.beforeRequestSent", vec![r.id()], "child-frame");
    r.calls_is("network.failRequest", 1).await;
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn queued_unobserved_registration_handoff_is_rolled_back() {
    use std::{
        future::Future,
        task::{Context, Poll},
    };
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    r.pause("network.addIntercept");
    let mut registration = Box::pin(p.block("*match*"));
    let mut context = Context::from_waker(futures_util::task::noop_waker_ref());
    assert!(matches!(
        registration.as_mut().poll(&mut context),
        Poll::Pending
    ));
    r.entered().await;
    r.release.notify_one();
    // The operation published its ID, but the public future has never been
    // polled again to observe and accept the queued handoff.
    tokio::time::timeout(Duration::from_secs(2), async {
        while p.routing.event_id_count().await != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(registration);
    r.count_is(0).await;
    p.block("*later*").await.unwrap();
    assert_eq!(r.calls("network.addIntercept"), 2);
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn previous_routes_keep_dispatching_while_broader_allocation_is_unacknowledged() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    let previous = r.id();
    r.pause("network.addIntercept");
    let c = p.clone();
    let upgrade = tokio::spawn(async move {
        c.route("*response*", RouteAction::SetResponseHeaders(vec![]))
            .await
    });
    r.entered().await;
    r.event("network.beforeRequestSent", vec![previous], "child-frame");
    r.calls_is("network.failRequest", 1).await;
    assert!(!upgrade.is_finished());
    upgrade.abort();
    assert!(upgrade.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    r.count_is(1).await;
    p.clear_routes().await.unwrap();
}
#[tokio::test]
async fn canceled_reconciliation_restores_previous_committed_rules() {
    let (r, b) = remote().await;
    let p = b.new_page().await.unwrap();
    p.block("*match*").await.unwrap();
    r.fail("network.removeIntercept");
    assert!(p
        .route("*response*", RouteAction::SetResponseHeaders(vec![]))
        .await
        .is_err());
    r.pause("network.removeIntercept");
    let c = p.clone();
    let retry = tokio::spawn(async move { c.block("*candidate*").await });
    r.entered().await;
    retry.abort();
    assert!(retry.await.unwrap_err().is_cancelled());
    r.release.notify_one();
    r.count_is(1).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while p.routing.event_id_count().await != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    r.event("network.beforeRequestSent", vec![r.id()], "child-frame");
    r.calls_is("network.failRequest", 1).await;
    p.clear_routes().await.unwrap();
}
