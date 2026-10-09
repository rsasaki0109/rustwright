use super::*;
use serde_json::json;

fn ready() -> Activity {
    let mut state = Activity::new("root".into());
    state
        .bootstrap(
            &[BrowsingContextInfo {
                context: "root".into(),
                url: "about:blank".into(),
                parent: None,
                user_context: None,
                children: Some(vec![]),
            }],
            Instant::now(),
        )
        .unwrap();
    state
}
fn event(method: &str, params: Value) -> BidiEvent {
    BidiEvent {
        method: method.into(),
        params,
    }
}
fn start(context: &str, id: &str, hop: u64) -> BidiEvent {
    event(
        "network.beforeRequestSent",
        json!({"context":context,"redirectCount":hop,"request":{"request":id,"url":"http://localhost/slow"}}),
    )
}
fn finish(context: &str, id: &str, hop: u64) -> BidiEvent {
    event(
        "network.responseCompleted",
        json!({"context":context,"redirectCount":hop,"request":{"request":id}}),
    )
}
#[tokio::test(start_paused = true)]
async fn headers_leave_body_pending_and_errors_start_quiet_window() {
    let mut state = ready();
    let now = Instant::now();
    state.event(&start("root", "stream", 0), now);
    state.event(
        &event(
            "network.responseStarted",
            json!({"context":"root","request":{"request":"stream"}}),
        ),
        now,
    );
    assert!(state.deadline().unwrap().is_none());
    state.event(
        &event(
            "network.fetchError",
            json!({"context":"root","redirectCount":0,"request":{"request":"stream"}}),
        ),
        now,
    );
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
#[tokio::test(start_paused = true)]
async fn stale_redirect_terminal_and_duplicates_cannot_finish_new_hop() {
    let mut state = ready();
    let now = Instant::now();
    state.event(&start("root", "redirect", 0), now);
    state.event(&finish("root", "redirect", 0), now);
    state.event(&start("root", "redirect", 1), now);
    state.event(&finish("root", "redirect", 0), now);
    state.event(&start("root", "redirect", 0), now);
    assert!(state.deadline().unwrap().is_none());
    state.event(&finish("root", "redirect", 1), now);
    state.event(&finish("root", "redirect", 1), now + Duration::from_secs(2));
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
#[tokio::test(start_paused = true)]
async fn descendant_requests_count_and_destroying_subtree_preserves_other_work() {
    let mut state = ready();
    let now = Instant::now();
    for (id, parent) in [
        ("child", "root"),
        ("nested", "child"),
        ("sibling", "root"),
        ("foreign", "other-tab"),
    ] {
        state.event(
            &event(
                "browsingContext.contextCreated",
                json!({"context":id,"parent":parent}),
            ),
            now,
        );
    }
    state.event(&start("nested", "nested-fetch", 0), now);
    state.event(&start("sibling", "sibling-fetch", 0), now);
    state.event(&start("foreign", "foreign-fetch", 0), now);
    assert_eq!(state.requests.len(), 2);
    state.event(
        &event(
            "browsingContext.contextDestroyed",
            json!({"context":"child"}),
        ),
        now,
    );
    assert!(!state.owns("nested"));
    assert!(!state.owns("foreign"));
    assert!(state.deadline().unwrap().is_none());
    state.event(&finish("sibling", "sibling-fetch", 0), now);
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
#[tokio::test(start_paused = true)]
async fn completed_historical_start_and_non_http_activity_do_not_create_pending_work() {
    let mut state = ready();
    let now = Instant::now();
    state.event(&event("network.beforeRequestSent",json!({"context":"root","request":{"request":"late","url":"http://localhost/finished","timings":{"responseEnd":100.0}}})),now);
    for url in [
        "data:text/plain,hello",
        "about:blank",
        "ws://localhost/socket",
    ] {
        state.event(
            &event(
                "network.beforeRequestSent",
                json!({"context":"root","request":{"request":"non-http","url":url}}),
            ),
            now,
        );
    }
    assert!(state.requests.is_empty());
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
#[tokio::test(start_paused = true)]
async fn committed_new_document_preserves_incoming_request_and_new_subresources() {
    let mut state = ready();
    let now = Instant::now();
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"old"}),
        ),
        now,
    );
    state.event(&start("root", "old-fetch", 0), now);
    state.event(&event("network.beforeRequestSent",json!({"context":"root","navigation":"new","request":{"request":"new-document","url":"http://localhost/new"}})),now);
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    state.event(&start("root", "new-fetch", 0), now);
    state.event(
        &event(
            "browsingContext.domContentLoaded",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    assert_eq!(state.requests.len(), 2);
    assert!(state.requests.contains_key("new-document"));
    assert!(state.requests.contains_key("new-fetch"));
    state.event(&finish("root", "old-fetch", 0), now);
    assert!(state.deadline().unwrap().is_none());
    state.event(&finish("root", "new-document", 0), now);
    state.event(&finish("root", "new-fetch", 0), now);
    state.event(
        &event(
            "browsingContext.load",
            json!({"context":"root","navigation":"new"}),
        ),
        now + Duration::from_secs(1),
    );
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
#[tokio::test(start_paused = true)]
async fn repeated_document_and_request_churn_retains_only_live_ownership() {
    let mut state = ready();
    let now = Instant::now();
    for cycle in 0..1000 {
        let id = format!("child-{cycle}");
        state.event(
            &event(
                "browsingContext.contextCreated",
                json!({"context":id,"parent":"root"}),
            ),
            now,
        );
        state.event(&start(&id, &id, 0), now);
        state.event(&finish(&id, &id, 0), now);
        state.event(
            &event("browsingContext.contextDestroyed", json!({"context":id})),
            now,
        );
    }
    assert!(state.requests.is_empty());
    assert_eq!(state.parents.len(), 1);
    assert!(state.navigations.is_empty());
}
#[tokio::test(start_paused = true)]
async fn timeout_and_cancel_leave_shared_state_and_interrupted_quiet_window_intact() {
    let observer = Arc::new(IdleObserver::new("root".into()));
    *observer.state.lock().unwrap() = ready();
    observer
        .state
        .lock()
        .unwrap()
        .event(&start("root", "slow", 0), Instant::now());
    assert!(matches!(
        observer.wait(Duration::from_millis(10)).await,
        Err(BidiError::Timeout { .. })
    ));
    let wait = observer.clone();
    let cancelled = tokio::spawn(async move { wait.wait(Duration::from_secs(10)).await });
    tokio::task::yield_now().await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let wait = observer.clone();
    let active = tokio::spawn(async move { wait.wait(Duration::from_secs(10)).await });
    tokio::task::yield_now().await;
    observer
        .state
        .lock()
        .unwrap()
        .event(&finish("root", "slow", 0), Instant::now());
    observer.changes.send_replace(());
    tokio::time::advance(Duration::from_millis(400)).await;
    observer
        .state
        .lock()
        .unwrap()
        .event(&start("root", "later", 0), Instant::now());
    observer.changes.send_replace(());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(!active.is_finished());
    observer
        .state
        .lock()
        .unwrap()
        .event(&finish("root", "later", 0), Instant::now());
    observer.changes.send_replace(());
    tokio::task::yield_now().await;
    tokio::time::advance(QUIET_WINDOW).await;
    active.await.unwrap().unwrap();
}
#[tokio::test(start_paused = true)]
async fn discovered_incomplete_lost_and_closed_are_explicit_errors() {
    let observer = IdleObserver::new("root".into());
    assert!(matches!(
        observer.wait(Duration::from_secs(1)).await,
        Err(BidiError::NetworkObservationIncomplete)
    ));
    {
        let mut state = observer.state.lock().unwrap();
        *state = ready();
        state.fault = Some(Fault::Lost(100));
    }
    assert!(matches!(
        observer.wait(Duration::from_secs(1)).await,
        Err(BidiError::NetworkEventsLost { skipped: 100 })
    ));
    observer.mark_closed();
    assert!(matches!(
        observer.wait(Duration::from_secs(1)).await,
        Err(BidiError::Closed)
    ));
}

#[path = "protocol_tests.rs"]
mod protocol_tests;

#[tokio::test(start_paused = true)]
async fn already_quiet_page_still_observes_a_fresh_window_per_call() {
    let observer = Arc::new(IdleObserver::new("root".into()));
    *observer.state.lock().unwrap() = ready();
    tokio::time::advance(Duration::from_secs(2)).await;
    let waiting = observer.clone();
    let waiter = tokio::spawn(async move { waiting.wait(Duration::from_secs(2)).await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(499)).await;
    assert!(!waiter.is_finished());
    tokio::time::advance(Duration::from_millis(1)).await;
    waiter.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn request_delivered_after_triggering_command_ack_interrupts_fresh_window() {
    let observer = Arc::new(IdleObserver::new("root".into()));
    *observer.state.lock().unwrap() = ready();
    tokio::time::advance(Duration::from_secs(2)).await;
    let waiting = observer.clone();
    let waiter = tokio::spawn(async move { waiting.wait(Duration::from_secs(5)).await });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(40)).await;
    observer
        .state
        .lock()
        .unwrap()
        .event(&start("root", "delayed-event", 0), Instant::now());
    observer.changes.send_replace(());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(!waiter.is_finished());
    observer
        .state
        .lock()
        .unwrap()
        .event(&finish("root", "delayed-event", 0), Instant::now());
    observer.changes.send_replace(());
    tokio::task::yield_now().await;
    tokio::time::advance(QUIET_WINDOW).await;
    waiter.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn actual_broadcast_lag_invalidates_the_live_pump_and_releases_waiters() {
    let observer = IdleObserver::new("root".into());
    *observer.state.lock().unwrap() = ready();
    let (events, receiver) = broadcast::channel(2);
    let (_shutdown_sender, shutdown) = watch::channel(false);
    let pump = spawn_idle_pump(
        observer.state.clone(),
        observer.changes.clone(),
        receiver,
        shutdown,
    );
    *observer.pump.lock().unwrap() = Some(pump);
    // No await lets the real receiver overflow before its task first runs.
    for n in 0..10 {
        events.send(start("root", &format!("lag-{n}"), 0)).unwrap();
    }
    assert!(matches!(
        observer.wait(Duration::from_secs(1)).await,
        Err(BidiError::NetworkEventsLost { skipped: 8 })
    ));
    assert!(observer.state.lock().unwrap().requests.is_empty());
}

#[tokio::test(start_paused = true)]
async fn delayed_old_document_commit_cannot_retire_a_started_new_navigation() {
    let mut state = ready();
    let now = Instant::now();
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"old"}),
        ),
        now,
    );
    state.event(&start("root", "old-fetch", 0), now);
    state.event(&event("network.beforeRequestSent", json!({"context":"root","navigation":"new","request":{"request":"incoming","url":"http://localhost/new"}})), now);
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    state.event(
        &event(
            "browsingContext.domContentLoaded",
            json!({"context":"root","navigation":"old"}),
        ),
        now,
    );
    assert!(
        state.requests.contains_key("incoming"),
        "late old document commit must not remove the incoming navigation"
    );
    state.event(&finish("root", "old-fetch", 0), now);
    assert!(state.deadline().unwrap().is_none());
    state.event(
        &event(
            "browsingContext.domContentLoaded",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    state.event(&finish("root", "incoming", 0), now);
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}

#[tokio::test(start_paused = true)]
async fn incoming_document_before_navigation_started_survives_the_old_commit() {
    let mut state = ready();
    let now = Instant::now();
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"old"}),
        ),
        now,
    );
    state.event(&start("root", "old-fetch", 0), now);
    state.event(&event("network.beforeRequestSent", json!({"context":"root","navigation":"new","request":{"request":"incoming","url":"http://localhost/new"}})), now);
    // The new navigationStarted has not arrived; the old document now commits.
    state.event(
        &event(
            "browsingContext.domContentLoaded",
            json!({"context":"root","navigation":"old"}),
        ),
        now,
    );
    state.event(&finish("root", "old-fetch", 0), now);
    assert!(state.requests.contains_key("incoming"));
    assert!(state.deadline().unwrap().is_none());
    state.event(
        &event(
            "browsingContext.navigationStarted",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    state.event(
        &event(
            "browsingContext.domContentLoaded",
            json!({"context":"root","navigation":"new"}),
        ),
        now,
    );
    assert!(state.deadline().unwrap().is_none());
    state.event(&finish("root", "incoming", 0), now);
    assert_eq!(state.deadline().unwrap(), Some(now + QUIET_WINDOW));
}
