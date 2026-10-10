//! Protocol acknowledgments outlive cancelled close callers.
use super::{BidiBrowser, BidiContext, BidiPage};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Default)]
struct State {
    owners: HashSet<String>,
    pages: HashMap<String, String>,
    scripts: HashSet<String>,
    commands: Vec<String>,
}
struct Remote {
    state: Arc<Mutex<State>>,
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
async fn remote(hold: &'static str, fail_first: bool) -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(State::default()));
    let active = state.clone();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let arrived = entered.clone();
    let released = release.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut held = false;
        let mut sequence = 0;
        while let Some(Ok(message)) = ws.next().await {
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let method = command["method"].as_str().unwrap();
            let params = &command["params"];
            let fail = fail_first && method == hold && !held;
            let (result, error) = {
                let mut state = active.lock().unwrap();
                state.commands.push(method.to_owned());
                let mut error = None;
                let result = match method {
                    "session.new" => json!({"sessionId":"test","capabilities":{}}),
                    "browser.createUserContext" => {
                        sequence += 1;
                        let id = format!("owner-{sequence}");
                        state.owners.insert(id.clone());
                        json!({"userContext":id})
                    }
                    "browsingContext.create" => {
                        sequence += 1;
                        let id = format!("page-{sequence}");
                        state.pages.insert(
                            id.clone(),
                            params["userContext"]
                                .as_str()
                                .unwrap_or("default")
                                .to_owned(),
                        );
                        json!({"context":id})
                    }
                    "browser.removeUserContext" => {
                        let id = params["userContext"].as_str().unwrap();
                        if fail {
                            error = Some("unknown error");
                        } else if !state.owners.remove(id) {
                            error = Some("no such user context");
                        } else {
                            state.pages.retain(|_, owner| owner != id);
                        }
                        json!({})
                    }
                    "browsingContext.close" => {
                        if state
                            .pages
                            .remove(params["context"].as_str().unwrap())
                            .is_none()
                        {
                            error = Some("no such frame");
                        }
                        json!({})
                    }
                    "script.addPreloadScript" => {
                        sequence += 1;
                        let id = format!("script-{sequence}");
                        state.scripts.insert(id.clone());
                        json!({"script":id})
                    }
                    "script.removePreloadScript" => {
                        if fail {
                            error = Some("unknown error");
                        } else if !state.scripts.remove(params["script"].as_str().unwrap()) {
                            error = Some("no such script");
                        }
                        json!({})
                    }
                    "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                    "session.end" => json!({}),
                    method => panic!("unexpected command {method}"),
                };
                (result, error)
            };
            if method == hold && !held {
                held = true;
                arrived.notify_one();
                released.notified().await;
            }
            let response = match error {
                Some(error) => {
                    json!({"id":command["id"],"type":"error","error":error,"message":"test refusal"})
                }
                None => json!({"id":command["id"],"type":"success","result":result}),
            };
            if ws.send(Message::text(response.to_string())).await.is_err() {
                break;
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    (
        Remote {
            state,
            entered,
            release,
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
async fn clean(remote: &Remote, browser: &BidiBrowser) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let clean = {
                let state = remote.state.lock().unwrap();
                state.owners.is_empty() && state.pages.is_empty() && state.scripts.is_empty()
            };
            if clean
                && browser.session().helper_preload_count() == 0
                && browser.session().connection().pending_command_count() == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("close worker must finish with retained page handles");
}
async fn populated(browser: &BidiBrowser) -> (BidiContext, BidiPage) {
    let context = browser.new_context().await.unwrap();
    let page = context.new_page().await.unwrap();
    (context, page)
}
#[tokio::test]
async fn cancelled_context_close_before_reply_removes_retained_helper() {
    let (remote, browser) = remote("browser.removeUserContext", false).await;
    let (context, _page) = populated(&browser).await;
    let task = tokio::spawn(async move { context.close().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    clean(&remote, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_context_close_during_helper_removal_finishes() {
    let (remote, browser) = remote("script.removePreloadScript", false).await;
    let (context, _page) = populated(&browser).await;
    let task = tokio::spawn(async move { context.close().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    clean(&remote, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_page_close_during_helper_removal_closes_page() {
    let (remote, browser) = remote("script.removePreloadScript", false).await;
    let page = browser.new_page().await.unwrap();
    let retained = page.clone();
    let task = tokio::spawn(async move { page.close().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    clean(&remote, &browser).await;
    drop(retained);
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_browser_close_closes_retained_connection() {
    let (remote, browser) = remote("session.end", false).await;
    let connection = browser.session().connection().clone();
    let task = tokio::spawn(async move { browser.close().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !connection.is_closed() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("cancelled browser close must signal retained connection");
    assert_eq!(connection.pending_command_count(), 0);
}
#[tokio::test]
async fn browser_close_bounds_silent_end_and_closes_retained_connection() {
    let (remote, browser) = remote("session.end", false).await;
    let connection = browser.session().connection().clone();
    let task = tokio::spawn(async move { browser.close().await });
    entered(&remote).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    assert!(
        connection.is_closed(),
        "silent graceful end needs bounded local shutdown"
    );
    assert_eq!(connection.pending_command_count(), 0);
    task.await.unwrap().unwrap();
}
#[tokio::test]
async fn cancelled_context_close_bounds_silent_remote_waiter() {
    let (remote, browser) = remote("browser.removeUserContext", false).await;
    let (context, _page) = populated(&browser).await;
    let task = tokio::spawn(async move { context.close().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::task::yield_now().await;
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    // The remote context was removed before its silent acknowledgment, but the
    // worker cannot infer success. Preserve its helper for a later explicit retry.
    assert!(!browser.session().connection().is_closed());
    remote.release.notify_one();
}
#[tokio::test]
async fn remote_context_error_is_returned_and_retry_cleans_helper() {
    let (remote, browser) = remote("browser.removeUserContext", true).await;
    let (context, _page) = populated(&browser).await;
    let caller = context.clone();
    let task = tokio::spawn(async move { caller.close().await });
    entered(&remote).await;
    remote.release.notify_one();
    assert!(
        matches!(task.await.unwrap(), Err(crate::BidiError::Protocol { error, .. }) if error == "unknown error")
    );
    assert_eq!(browser.session().helper_preload_count(), 1);
    context.close().await.unwrap();
    clean(&remote, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn concurrent_and_repeated_context_close_remove_helper_once() {
    let (remote, browser) = remote("browser.removeUserContext", false).await;
    let (context, _page) = populated(&browser).await;
    let caller = context.clone();
    let first = tokio::spawn(async move { caller.close().await });
    entered(&remote).await;
    let caller = context.clone();
    let second = tokio::spawn(async move { caller.close().await });
    remote.release.notify_one();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    context.close().await.unwrap();
    clean(&remote, &browser).await;
    assert_eq!(
        remote
            .state
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|method| method.as_str() == "script.removePreloadScript")
            .count(),
        1
    );
    browser.close().await.unwrap();
}
#[tokio::test]
async fn failed_helper_removal_is_retried_after_context_is_gone() {
    let (remote, browser) = remote("script.removePreloadScript", true).await;
    let (context, _page) = populated(&browser).await;
    let caller = context.clone();
    let first = tokio::spawn(async move { caller.close().await });
    entered(&remote).await;
    remote.release.notify_one();
    first.await.unwrap().unwrap();
    assert_eq!(browser.session().helper_preload_count(), 1);
    context.close().await.unwrap();
    clean(&remote, &browser).await;
    browser.close().await.unwrap();
}
