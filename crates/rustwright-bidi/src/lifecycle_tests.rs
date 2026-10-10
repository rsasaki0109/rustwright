//! Protocol regressions for helper registrations owned by page handles.
use crate::BidiBrowser;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    scripts: Arc<Mutex<HashSet<String>>>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Remote {
    fn script_count(&self) -> usize {
        self.scripts.lock().unwrap().len()
    }
}
async fn remote() -> (Remote, BidiBrowser) {
    remote_with_pause(None).await
}
async fn remote_with_pause(pause: Option<(Arc<Notify>, Arc<Notify>)>) -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let scripts = Arc::new(Mutex::new(HashSet::new()));
    let active = scripts.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        let mut sequence = 0;
        let mut pause = pause;
        let mut pages = HashMap::<String, String>::new();
        let mut page_sequence = 0;
        let mut owner_sequence = 0;
        while let Some(Ok(message)) = ws.next().await {
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let params = &command["params"];
            let result = match command["method"].as_str().unwrap() {
                "session.new" => {
                    json!({"sessionId":"test","capabilities":{"browserVersion":"mock"}})
                }
                "browser.createUserContext" => {
                    owner_sequence += 1;
                    json!({"userContext":format!("owned-{owner_sequence}")})
                }
                "browsingContext.create" => {
                    page_sequence += 1;
                    let context = format!("tab-{page_sequence}");
                    pages.insert(
                        context.clone(),
                        params["userContext"]
                            .as_str()
                            .unwrap_or("default")
                            .to_string(),
                    );
                    json!({"context":context})
                }
                "browsingContext.getTree" => {
                    json!({"contexts":pages.iter().map(|(context,owner)|json!({"context":context,"url":"about:blank","userContext":owner,"children":[]})).collect::<Vec<_>>()})
                }
                "script.addPreloadScript" => {
                    sequence += 1;
                    let id = format!("script-{sequence}");
                    active.lock().unwrap().insert(id.clone());
                    if let Some((created, release)) = pause.take() {
                        created.notify_one();
                        release.notified().await;
                    }
                    json!({"script":id})
                }
                "script.removePreloadScript" => {
                    assert!(
                        active
                            .lock()
                            .unwrap()
                            .remove(params["script"].as_str().unwrap()),
                        "removing an unregistered script"
                    );
                    json!({})
                }
                "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                "browser.removeUserContext" => {
                    pages.retain(|_, owner| Some(owner.as_str()) != params["userContext"].as_str());
                    json!({})
                }
                "browsingContext.close" => {
                    pages.remove(params["context"].as_str().unwrap());
                    json!({})
                }
                "browsingContext.activate" | "session.subscribe" | "session.end" => json!({}),
                method => panic!("unexpected command {method}"),
            };
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
    (Remote { scripts, task }, browser)
}

#[tokio::test]
async fn repeated_discovery_reuses_the_page_helper_registration() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    let (a, b) = tokio::join!(browser.pages(), browser.pages());
    let mut discovered = a.unwrap();
    discovered.extend(b.unwrap());
    for _ in 0..20 {
        discovered.extend(browser.pages().await.unwrap());
    }
    assert_eq!(
        remote.script_count(),
        1,
        "discovery must not add persistent helpers per handle"
    );
    assert_eq!(browser.session().helper_preload_count(), 1);
    page.close().await.unwrap();
    assert_eq!(
        remote.script_count(),
        0,
        "closing any handle releases the shared helper"
    );
    assert_eq!(browser.session().helper_preload_count(), 0);
    drop(discovered);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn context_close_releases_helpers_even_while_page_handles_are_retained() {
    let (remote, browser) = remote().await;
    let context = browser.new_context().await.unwrap();
    let page = context.new_page().await.unwrap();
    assert_eq!(remote.script_count(), 1);
    context.close().await.unwrap();
    assert_eq!(
        remote.script_count(),
        0,
        "context closure must release its page's preload"
    );
    assert_eq!(browser.session().helper_preload_count(), 0);
    // Keep the page alive until after the check; Drop alone cannot satisfy it.
    drop(page);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn dropping_the_last_page_handle_releases_its_helper_registration() {
    let (remote, browser) = remote().await;
    let page = browser.new_page().await.unwrap();
    let clone = page.clone();
    drop(page);
    assert_eq!(
        remote.script_count(),
        1,
        "a live clone still owns the helper"
    );
    drop(clone);
    tokio::time::timeout(Duration::from_secs(1), async {
        while remote.script_count() != 0 || browser.session().helper_preload_count() != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("last handle's cleanup must reach the browser");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn closing_one_context_preserves_other_context_helpers() {
    let (remote, browser) = remote().await;
    let first = browser.new_context().await.unwrap();
    let second = browser.new_context().await.unwrap();
    let first_page = first.new_page().await.unwrap();
    let second_page = second.new_page().await.unwrap();
    assert_eq!(remote.script_count(), 2);
    first.close().await.unwrap();
    assert_eq!(
        remote.script_count(),
        1,
        "other context keeps its registration"
    );
    assert_eq!(second.pages().await.unwrap().len(), 1);
    assert_eq!(remote.script_count(), 1, "discovery does not duplicate it");
    second.close().await.unwrap();
    assert_eq!(remote.script_count(), 0);
    drop((first_page, second_page));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelling_helper_registration_still_releases_the_remote_script() {
    let created = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (remote, browser) = remote_with_pause(Some((created.clone(), release.clone()))).await;
    let context = browser.new_context().await.unwrap();
    let creating = context.clone();
    let task = tokio::spawn(async move { creating.new_page().await });
    tokio::time::timeout(Duration::from_secs(1), created.notified())
        .await
        .unwrap();
    assert_eq!(
        remote.script_count(),
        1,
        "browser registered before replying"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while remote.script_count() != 0 || browser.session().helper_preload_count() != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("cancelled registration must preserve the identifier for cleanup");
    context.close().await.unwrap();
    assert_eq!(browser.session().helper_preload_count(), 0);
    browser.close().await.unwrap();
}
