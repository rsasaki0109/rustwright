//! Foreground create captures a selected context replaced during addTab.
//!
//! Firefox 157 starts the old tab's visibility waiter before allocation. This
//! remote models replacement at that boundary, not transient command rejection.
use super::BidiBrowser;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct State {
    pages: HashMap<String, String>,
    allocated: Vec<String>,
    retired: Vec<String>,
    activated: Vec<String>,
    selected_generation: usize,
}
struct Remote {
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn remote() -> (Remote, BidiBrowser) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(State {
        pages: HashMap::from([("startup".to_owned(), "default".to_owned())]),
        allocated: Vec::new(),
        retired: Vec::new(),
        activated: Vec::new(),
        selected_generation: 0,
    }));
    let current = state.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        while let Some(Ok(message)) = ws.next().await {
            if !message.is_text() {
                continue;
            }
            let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            let params = &command["params"];
            let method = command["method"].as_str().unwrap();
            let mut visibility_failure = false;
            let result = {
                let mut state = current.lock().unwrap();
                match method {
                    "session.new" => json!({"sessionId":"startup","capabilities":{}}),
                    "browser.createUserContext" => json!({"userContext":"owned"}),
                    "browsingContext.create" => {
                        let captured = state.selected_generation;
                        let context = format!("page-{}", state.allocated.len() + 1);
                        let owner = params["userContext"]
                            .as_str()
                            .unwrap_or("default")
                            .to_owned();
                        state.pages.insert(context.clone(), owner);
                        state.allocated.push(context.clone());
                        // addTab replaces the previously selected startup
                        // browsing context while its old visibility wait runs.
                        state.selected_generation += 1;
                        visibility_failure =
                            params["background"] != true && captured != state.selected_generation;
                        json!({"context":context})
                    }
                    "browsingContext.activate" => {
                        let context = params["context"].as_str().unwrap();
                        assert!(
                            state.pages.contains_key(context),
                            "activation must target the acknowledged allocation"
                        );
                        // This separate command observes the current selected
                        // context, rather than create's pre-addTab capture.
                        state.activated.push(context.to_owned());
                        json!({})
                    }
                    "browsingContext.getTree" => {
                        let root = params["root"].as_str();
                        let contexts = state.pages.iter().filter(|(id, _)| root.is_none_or(|root| id.as_str() == root))
                            .map(|(id, owner)| json!({"context":id,"userContext":owner,"url":"about:blank","children":[]})).collect::<Vec<_>>();
                        json!({"contexts":contexts})
                    }
                    "browsingContext.close" => {
                        let context = params["context"].as_str().unwrap();
                        assert!(state.pages.remove(context).is_some());
                        state.retired.push(context.to_owned());
                        json!({})
                    }
                    "browser.removeUserContext" => {
                        state.pages.retain(|_, owner| {
                            Some(owner.as_str()) != params["userContext"].as_str()
                        });
                        json!({})
                    }
                    "script.addPreloadScript" => json!({"script":"helper"}),
                    "script.evaluate" => json!({"type":"success","result":{"type":"undefined"}}),
                    "session.subscribe" | "script.removePreloadScript" | "session.end" => json!({}),
                    method => panic!("unexpected command {method}"),
                }
            };
            let reply = if visibility_failure {
                json!({"id":command["id"],"type":"error","error":"no such frame","message":"DiscardedBrowsingContextError: BrowsingContext does no longer exist"})
            } else {
                json!({"id":command["id"],"type":"success","result":result})
            };
            if ws.send(Message::text(reply.to_string())).await.is_err() {
                break;
            }
        }
    });
    let browser = BidiBrowser::connect(&endpoint).await.unwrap();
    (Remote { state, task }, browser)
}

#[tokio::test]
async fn startup_replacement_does_not_fail_or_duplicate_default_page_allocation() {
    let (remote, browser) = remote().await;
    let page = browser
        .new_page()
        .await
        .expect("old selected context replacement must not invalidate our newly allocated page");
    {
        let state = remote.state.lock().unwrap();
        assert_eq!(state.allocated, vec![page.context_id().to_owned()]);
        assert_eq!(
            state.activated,
            vec![page.context_id().to_owned()],
            "returned page retains foreground behavior"
        );
        assert_eq!(state.pages[page.context_id()], "default");
    }
    page.close().await.unwrap();
    {
        let state = remote.state.lock().unwrap();
        assert_eq!(state.retired, vec![page.context_id().to_owned()]);
        assert_eq!(
            state.pages,
            HashMap::from([("startup".to_owned(), "default".to_owned())])
        );
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn startup_replacement_preserves_isolated_page_owner_and_activation() {
    let (remote, browser) = remote().await;
    let context = browser.new_context().await.unwrap();
    let page = context
        .new_page()
        .await
        .expect("allocation and activation must preserve the explicit user context");
    {
        let state = remote.state.lock().unwrap();
        assert_eq!(state.allocated, vec![page.context_id().to_owned()]);
        assert_eq!(state.activated, vec![page.context_id().to_owned()]);
        assert_eq!(state.pages[page.context_id()], context.user_context_id());
    }
    page.close().await.unwrap();
    context.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn foreground_protocol_control_fails_after_allocating_on_replacement() {
    let (remote, browser) = remote().await;
    let result = browser
        .session()
        .connection()
        .send(
            "browsingContext.create",
            json!({"type":"tab","background":false}),
        )
        .await;
    assert!(
        matches!(result, Err(crate::BidiError::Protocol { method, error, .. }) if method == "browsingContext.create" && error == "no such frame")
    );
    let context = {
        let state = remote.state.lock().unwrap();
        assert_eq!(state.allocated.len(), 1);
        assert!(state.activated.is_empty());
        state.allocated[0].clone()
    };
    // The raw control knows the fixture's hidden allocation only to retire it.
    browser.session().close_context(&context).await.unwrap();
    browser.close().await.unwrap();
}
