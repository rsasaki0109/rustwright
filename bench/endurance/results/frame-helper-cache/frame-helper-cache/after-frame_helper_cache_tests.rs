//! Helpers belong to documents, rather than an ever-growing set of frame ids.
use super::BidiBrowser;
use crate::BidiError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Default)]
struct State {
    documents: HashMap<String, bool>,
    injections: usize,
    hold_injection: bool,
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

impl Remote {
    fn document(&self, context: &str) {
        self.state
            .lock()
            .unwrap()
            .documents
            .insert(context.into(), false);
    }

    fn destroy(&self, context: &str) {
        self.state.lock().unwrap().documents.remove(context);
    }

    fn has_helper(&self, context: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .documents
            .get(context)
            .copied()
            .unwrap_or(false)
    }

    fn injections(&self) -> usize {
        self.state.lock().unwrap().injections
    }
}

async fn remote() -> (Remote, BidiBrowser) {
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
        while let Some(Ok(message)) = ws.next().await {
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let params = &command["params"];
            let mut hold = false;
            let (result, error) = {
                let mut state = active.lock().unwrap();
                match command["method"].as_str().unwrap() {
                    "session.new" => (json!({"sessionId":"test","capabilities":{}}), None),
                    "browsingContext.create" => {
                        state.documents.insert("page".into(), false);
                        (json!({"context":"page"}), None)
                    }
                    "script.addPreloadScript" => (json!({"script":"helper"}), None),
                    "script.removePreloadScript" | "session.end" => (json!({}), None),
                    "browsingContext.close" => {
                        state.documents.remove(params["context"].as_str().unwrap());
                        (json!({}), None)
                    }
                    "script.evaluate" => {
                        let context = params["target"]["context"].as_str().unwrap();
                        let expression = params["expression"].as_str().unwrap();
                        if !state.documents.contains_key(context) {
                            (json!({}), Some("no such frame"))
                        } else if expression == "!!window.__rustwright" {
                            (
                                json!({"type":"success","result":{"type":"boolean","value":state.documents[context]}}),
                                None,
                            )
                        } else {
                            assert_eq!(expression, rustwright_common::INJECTED_SCRIPT);
                            state.documents.insert(context.into(), true);
                            state.injections += 1;
                            hold = std::mem::take(&mut state.hold_injection);
                            (
                                json!({"type":"success","result":{"type":"undefined"}}),
                                None,
                            )
                        }
                    }
                    method => panic!("unexpected command {method}"),
                }
            };
            if hold {
                arrived.notify_one();
                released.notified().await;
            }
            let reply = if let Some(error) = error {
                json!({"id":command["id"],"type":"error","error":error,"message":"document destroyed"})
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
            state,
            entered,
            release,
            task,
        },
        browser,
    )
}

#[tokio::test]
async fn live_frame_reuses_its_document_helper_without_reinjection() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    remote.document("frame");
    page.ensure_helper_in("frame").await.unwrap();
    let injected = remote.injections();
    for _ in 0..20 {
        page.ensure_helper_in("frame").await.unwrap();
    }
    assert!(remote.has_helper("frame"));
    assert_eq!(remote.injections(), injected);
    assert_eq!(browser.session().helper_preload_count(), 1);
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn same_frame_id_in_a_new_document_gets_its_helper_again() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    remote.document("frame");
    page.ensure_helper_in("frame").await.unwrap();
    let injected = remote.injections();
    // Navigation preserves the browsing-context id but replaces its document.
    remote.document("frame");
    page.ensure_helper_in("frame").await.unwrap();
    assert!(
        remote.has_helper("frame"),
        "a cached frame id cannot prove helper presence in a new document"
    );
    assert_eq!(remote.injections(), injected + 1);
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn destroyed_frame_is_not_treated_as_an_initialized_document() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    remote.document("frame");
    page.ensure_helper_in("frame").await.unwrap();
    remote.destroy("frame");
    assert!(
        matches!(page.ensure_helper_in("frame").await, Err(BidiError::Protocol { error, .. }) if error == "no such frame")
    );
    page.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn late_injection_reply_does_not_initialize_a_replacement_document() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    remote.document("frame");
    remote.state.lock().unwrap().hold_injection = true;
    let injecting = page.clone();
    let task = tokio::spawn(async move { injecting.ensure_helper_in("frame").await });
    tokio::time::timeout(Duration::from_secs(1), remote.entered.notified())
        .await
        .unwrap();
    assert!(remote.has_helper("frame"));
    remote.destroy("frame");
    remote.document("frame");
    remote.release.notify_one();
    task.await.unwrap().unwrap();
    assert!(!remote.has_helper("frame"));
    page.ensure_helper_in("frame").await.unwrap();
    assert!(
        remote.has_helper("frame"),
        "the old document's late success must not suppress new injection"
    );
    page.close().await.unwrap();
    browser.close().await.unwrap();
}
