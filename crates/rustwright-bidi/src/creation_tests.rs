//! Cancellation after remote allocation and during page initialization.
use super::BidiBrowser;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Default)]
struct Resources {
    pages: HashSet<String>,
    owners: HashSet<String>,
    sessions: HashSet<String>,
    scripts: HashSet<String>,
}
struct Remote {
    state: Arc<Mutex<Resources>>,
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
async fn remote(hold: &'static str, fail: bool) -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(Resources {
        pages: HashSet::from(["existing".into()]),
        owners: HashSet::from(["existing-owner".into()]),
        ..Default::default()
    }));
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
            let result = {
                let mut state = active.lock().unwrap();
                match method {
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
                        state.pages.insert(id.clone());
                        json!({"context":id})
                    }
                    "browsingContext.close" => {
                        state.pages.remove(params["context"].as_str().unwrap());
                        json!({})
                    }
                    "browser.removeUserContext" => {
                        state.owners.remove(params["userContext"].as_str().unwrap());
                        json!({})
                    }
                    "script.addPreloadScript" => {
                        sequence += 1;
                        let id = format!("script-{sequence}");
                        if !fail {
                            state.scripts.insert(id.clone());
                        }
                        json!({"script":id})
                    }
                    "script.removePreloadScript" => {
                        state.scripts.remove(params["script"].as_str().unwrap());
                        json!({})
                    }
                    "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                    "session.end" => json!({}),
                    method => panic!("unexpected command {method}"),
                }
            };
            if method == hold && !held {
                held = true;
                arrived.notify_one();
                released.notified().await;
                if fail {
                    let response = json!({"id":command["id"],"type":"error","error":"unknown error","message":"initialization failed"});
                    ws.send(Message::text(response.to_string())).await.unwrap();
                    continue;
                }
            }
            if ws
                .send(Message::text(
                    json!({"id":command["id"],"type":"success","result":result}).to_string(),
                ))
                .await
                .is_err()
            {
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
        .expect("allocation/initialization must enter the server");
}
async fn restored(remote: &Remote, owners: usize, browser: &BidiBrowser) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let clean = {
                let state = remote.state.lock().unwrap();
                state.pages == HashSet::from(["existing".into()])
                    && state.owners.len() == owners
                    && state.owners.contains("existing-owner")
                    && state.sessions.is_empty()
                    && state.scripts.is_empty()
            };
            if clean && browser.session().connection().pending_command_count() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("cancelled or failed creation must release only its allocated resources");
}
#[tokio::test]
async fn cancelled_page_before_creation_reply_is_closed() {
    let (remote, browser) = remote("browsingContext.create", false).await;
    let context = browser.new_context().await.unwrap();
    let caller = context.clone();
    let task = tokio::spawn(async move { caller.new_page().await });
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    restored(&remote, 2, &browser).await;
    context.close().await.unwrap();
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_default_page_before_creation_reply_is_closed() {
    let (remote, browser) = remote("browsingContext.create", false).await;
    let task = {
        let session = browser.session().clone();
        tokio::spawn(async move {
            let browser = BidiBrowser {
                session,
                process: None,
            };
            browser.new_page().await
        })
    };
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_context_before_creation_reply_is_removed() {
    let (remote, browser) = remote("browser.createUserContext", false).await;
    let task = {
        let session = browser.session().clone();
        tokio::spawn(async move {
            let browser = BidiBrowser {
                session,
                process: None,
            };
            browser.new_context().await
        })
    };
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn cancelled_page_initialization_closes_created_page() {
    let (remote, browser) = remote("script.addPreloadScript", false).await;
    let task = {
        let session = browser.session().clone();
        tokio::spawn(async move {
            let browser = BidiBrowser {
                session,
                process: None,
            };
            browser.new_page().await
        })
    };
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    remote.release.notify_one();
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}
#[tokio::test]
async fn failed_page_initialization_closes_created_page() {
    let (remote, browser) = remote("script.addPreloadScript", true).await;
    let task = {
        let session = browser.session().clone();
        tokio::spawn(async move {
            let browser = BidiBrowser {
                session,
                process: None,
            };
            browser.new_page().await
        })
    };
    entered(&remote).await;
    remote.release.notify_one();
    assert!(task.await.unwrap().is_err());
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_after_reply_before_handoff_removes_context() {
    use std::{
        future::{poll_fn, Future},
        task::Poll,
    };
    let (remote, browser) = remote("browser.createUserContext", false).await;
    let mut future = Box::pin(browser.new_context());
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    entered(&remote).await;
    remote.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while browser.session().connection().pending_command_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    // The caller's future has not been polled since allocation began. A ready
    // identifier must remain guarded even while queued in the result channel.
    drop(future);
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_creation_with_silent_peer_releases_local_waiter_after_grace() {
    let (remote, browser) = remote("browsingContext.create", false).await;
    let task = {
        let session = browser.session().clone();
        tokio::spawn(async move {
            let browser = BidiBrowser {
                session,
                process: None,
            };
            browser.new_page().await
        })
    };
    entered(&remote).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::task::yield_now().await;
    assert_eq!(
        browser.session().connection().pending_command_count(),
        1,
        "keep the allocation waiter to recover a late identifier"
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        browser.session().connection().pending_command_count(),
        0,
        "a nonresponding peer must not retain the cancelled worker forever"
    );
    // Without an acknowledgment, the remote identifier is unknowable. This
    // guard bounds local waiting; it cannot prove remote cleanup after no reply.
    assert_eq!(remote.state.lock().unwrap().pages.len(), 2);
    drop(remote);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_initialization_outside_runtime_thread_cleans_up() {
    let (remote, browser) = remote("script.evaluate", false).await;
    let mut future = Box::pin(browser.new_page());
    tokio::select! {
        _ = &mut future => panic!("initialization must remain blocked"),
        _ = remote.entered.notified() => {},
    }
    // The origin runtime is still alive, but this thread has no runtime entry.
    std::thread::scope(|scope| scope.spawn(move || drop(future)).join().unwrap());
    remote.release.notify_one();
    restored(&remote, 1, &browser).await;
    browser.close().await.unwrap();
}
