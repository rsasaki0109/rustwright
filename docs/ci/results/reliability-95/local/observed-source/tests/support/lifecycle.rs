//! Shared local HTTP fixture and response-delay proxy for lifecycle tests.
use futures_util::{SinkExt, StreamExt};
use rustwright::{
    bidi::{BidiBrowser, BidiContext},
    cdp::CdpConnection,
    prelude::*,
    AnyError,
};
use serde_json::{json, Value};
use std::{path::PathBuf, result::Result as TestResult, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};

pub(crate) const LIMIT: Duration = Duration::from_secs(5);
struct Gate {
    method: &'static str,
    armed: oneshot::Sender<()>,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}
pub(crate) struct Proxy {
    pub(crate) endpoint: String,
    control: mpsc::UnboundedSender<Gate>,
    task: JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Proxy {
    pub(crate) async fn arm(
        &self,
        method: &'static str,
    ) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (armed, ready) = oneshot::channel();
        let (entered, waiting) = oneshot::channel();
        let (release, released) = oneshot::channel();
        self.control
            .send(Gate {
                method,
                armed,
                entered,
                release: released,
            })
            .unwrap();
        tokio::time::timeout(LIMIT, ready).await.unwrap().unwrap();
        (waiting, release)
    }
}
pub(crate) async fn proxy(upstream: String) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let (control, mut commands) = mpsc::unbounded_channel::<Gate>();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut client = accept_async(socket).await.unwrap();
        let (mut browser, _) = connect_async(&upstream).await.unwrap();
        let mut active: Option<(String, oneshot::Sender<()>, oneshot::Receiver<()>)> = None;
        let mut held_id: Option<Value> = None;
        let mut held_reply: Option<Message> = None;
        loop {
            tokio::select! {
                gate = commands.recv() => {
                    let Some(gate) = gate else { break; };
                    assert!(active.is_none());
                    active = Some((gate.method.to_owned(), gate.entered, gate.release));
                    gate.armed.send(()).unwrap();
                },
                _ = async { (&mut active.as_mut().unwrap().2).await }, if held_reply.is_some() => {
                    client.send(held_reply.take().unwrap()).await.unwrap();
                    active = None;
                    held_id = None;
                },
                message = client.next() => {
                    let Some(Ok(message)) = message else { break; };
                    if message.is_close() { break; }
                    if message.is_text() {
                        let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                        if active.as_ref().is_some_and(|gate| command["method"] == gate.0) && held_id.is_none() {
                            held_id = Some(command["id"].clone());
                        }
                    }
                    if browser.send(message).await.is_err() { break; }
                },
                message = browser.next() => {
                    let Some(Ok(message)) = message else { break; };
                    if message.is_close() { break; }
                    if message.is_text() && held_id.is_some() {
                        let response: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                        if held_id.as_ref() == Some(&response["id"]) {
                            assert!(response.get("error").is_none(), "allocation must succeed remotely: {response}");
                            held_reply = Some(message);
                            let gate = active.as_mut().unwrap();
                            let (replacement, _) = oneshot::channel();
                            std::mem::replace(&mut gate.1, replacement).send(()).unwrap();
                            continue;
                        }
                    }
                    if client.send(message).await.is_err() { break; }
                },
            }
        }
    });
    Proxy {
        endpoint,
        control,
        task,
    }
}

pub(crate) struct Fixture {
    pub(crate) url: String,
    pub(crate) profile: PathBuf,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
pub(crate) async fn fixture(firefox: bool) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let profile = std::env::temp_dir().join(format!(
        "rustwright-create-{}-{}-{}",
        std::process::id(),
        firefox,
        address.port()
    ));
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=> {
                    let (mut socket,_)=accepted.unwrap();
                    requests.spawn(async move {
                        let mut request=Vec::new();
                        loop {
                            let mut bytes=[0;2048];
                            let Ok(n)=socket.read(&mut bytes).await else { return; };
                            if n==0 {return;}
                            request.extend_from_slice(&bytes[..n]);
                            if request.len()>16*1024 {return;}
                            if request.windows(4).any(|b|b==b"\r\n\r\n") {break;}
                        }
                        let body="<!DOCTYPE html><title>creation</title><input id=q>";
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=socket.write_all(response.as_bytes()).await;
                    });
                },
                _=requests.join_next(),if !requests.is_empty()=>{},
            }
        }
    });
    Fixture {
        url: format!("http://{address}/creation.html"),
        profile,
        task,
    }
}

pub(crate) enum Engine {
    Chrome(Arc<Browser>, CdpConnection),
    Firefox(Arc<BidiBrowser>),
}
pub(crate) enum Context {
    Chrome(BrowserContext),
    Firefox(BidiContext),
}
impl Context {
    pub(crate) async fn new_page(&self) -> TestResult<AnyPage, AnyError> {
        match self {
            Self::Chrome(c) => Ok(c.new_page().await?.into()),
            Self::Firefox(c) => Ok(c.new_page().await?.into()),
        }
    }
    pub(crate) async fn close(&self) {
        match self {
            Self::Chrome(c) => c.close().await.unwrap(),
            Self::Firefox(c) => c.close().await.unwrap(),
        }
    }
}
impl Engine {
    pub(crate) async fn new_context(&self) -> TestResult<Context, AnyError> {
        match self {
            Self::Chrome(b, _) => Ok(Context::Chrome(b.new_context().await?)),
            Self::Firefox(b) => Ok(Context::Firefox(b.new_context().await?)),
        }
    }
    pub(crate) async fn new_page(&self) -> TestResult<AnyPage, AnyError> {
        match self {
            Self::Chrome(b, _) => Ok(b.new_page().await?.into()),
            Self::Firefox(b) => Ok(b.new_page().await?.into()),
        }
    }
    pub(crate) fn pending(&self) -> usize {
        match self {
            Self::Chrome(b, _) => b.pending_command_count(),
            Self::Firefox(b) => b.session().connection().pending_command_count(),
        }
    }
    pub(crate) async fn counts(&self) -> Counts {
        let (mut pages, mut owners, helpers) = match self {
            Self::Chrome(_, observer) => {
                let pages = observer
                    .send_raw(None, "Target.getTargets", json!({}))
                    .await
                    .unwrap();
                let owners = observer
                    .send_raw(None, "Target.getBrowserContexts", json!({}))
                    .await
                    .unwrap();
                (
                    pages["targetInfos"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|p| p["type"] == "page")
                        .map(|p| p["targetId"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    owners["browserContextIds"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    None,
                )
            }
            Self::Firefox(b) => {
                let owners = b
                    .session()
                    .connection()
                    .send("browser.getUserContexts", json!({}))
                    .await
                    .unwrap();
                (
                    b.session()
                        .get_tree()
                        .await
                        .unwrap()
                        .into_iter()
                        .filter(|p| p.parent.is_none())
                        .map(|p| p.context)
                        .collect::<Vec<_>>(),
                    owners["userContexts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p["userContext"].as_str().unwrap().to_owned())
                        .collect::<Vec<_>>(),
                    Some(b.session().helper_preload_count()),
                )
            }
        };
        pages.sort();
        owners.sort();
        Counts {
            pages,
            owners,
            helpers,
        }
    }
    pub(crate) async fn close(self) {
        match self {
            Self::Chrome(b, observer) => {
                b.close().await.unwrap();
                observer.close();
            }
            Self::Firefox(b) => Arc::try_unwrap(b).unwrap().close().await.unwrap(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) pages: Vec<String>,
    pub(crate) owners: Vec<String>,
    pub(crate) helpers: Option<usize>,
}
pub(crate) async fn restored(engine: &Engine, baseline: &Counts) {
    tokio::time::timeout(LIMIT,async {
        loop {
            if engine.counts().await==*baseline && engine.pending()==0 {break;}
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("cancelled allocations must restore exact page/context IDs, helper ledger and pending count");
}
pub(crate) async fn healthy(page: &AnyPage, url: &str) {
    page.goto(url).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "creation");
    page.locator("#q")
        .fill("作成キャンセル後 Rust")
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("document.querySelector('#q').value")
            .await
            .unwrap(),
        json!("作成キャンセル後 Rust")
    );
}
