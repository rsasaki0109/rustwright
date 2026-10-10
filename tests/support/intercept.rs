//! Local HTTP fixture and transparent acknowledgment gate for interception.
use futures_util::{SinkExt, StreamExt};
use rustwright::{bidi::BidiBrowser, browser::LaunchedFirefox, prelude::*};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::{accept_async, connect_async};

pub const LIMIT: Duration = Duration::from_secs(5);
struct Gate {
    method: &'static str,
    armed: oneshot::Sender<()>,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}
pub struct Proxy {
    pub endpoint: String,
    control: mpsc::UnboundedSender<Gate>,
    pub trace: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Proxy {
    pub async fn arm(&self, method: &'static str) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
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
    pub fn allocations(&self) -> Vec<String> {
        let trace = self.trace.lock().unwrap();
        let methods = trace
            .iter()
            .filter_map(|v| Some((v.get("id")?.as_u64()?, v.get("method")?.as_str()?)))
            .collect::<HashMap<_, _>>();
        trace
            .iter()
            .filter(|v| {
                v.get("id")
                    .and_then(Value::as_u64)
                    .and_then(|id| methods.get(&id).copied())
                    == Some("network.addIntercept")
            })
            .filter_map(|v| v["result"]["intercept"].as_str().map(str::to_owned))
            .collect()
    }
    pub async fn blocked(&self, url: &str) {
        self.blocked_event("network.beforeRequestSent", url).await;
    }
    pub async fn blocked_event(&self, method: &str, url: &str) {
        tokio::time::timeout(LIMIT, async { loop {
            if self.trace.lock().unwrap().iter().any(|v| v["method"] == method && v["params"]["isBlocked"] == true && v["params"]["request"]["url"] == url) {break;}
            tokio::time::sleep(Duration::from_millis(5)).await;
        }}).await.expect("native request must reach the remotely installed intercept before its acknowledgment is delivered");
    }
    pub async fn retired(&self, ids: &[String]) {
        tokio::time::timeout(LIMIT, async {
            loop {
                let removed = {
                    let trace = self.trace.lock().unwrap();
                    ids.iter().all(|id| {
                        trace
                            .iter()
                            .filter(|v| {
                                v["method"] == "network.removeIntercept"
                                    && v["params"]["intercept"] == *id
                            })
                            .any(|command| {
                                trace.iter().any(|reply| {
                                    reply["id"] == command["id"] && reply["type"] == "success"
                                })
                            })
                    })
                };
                if removed {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect(
            "last page owner must retire every known intercept while the remote page remains live",
        );
    }
}
async fn proxy(upstream: &str) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}/session", listener.local_addr().unwrap());
    let (control, mut commands) = mpsc::unbounded_channel::<Gate>();
    let trace = Arc::new(Mutex::new(Vec::<Value>::new()));
    let observed = trace.clone();
    let upstream = upstream.to_owned();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut client = accept_async(socket).await.unwrap();
        let (mut browser, _) = connect_async(upstream).await.unwrap();
        let mut active: Option<Gate> = None;
        let mut held_id = None;
        let mut held_reply = None;
        loop {
            tokio::select! {
                gate = commands.recv() => {let Some(gate) = gate else {break}; assert!(active.is_none()); let (replacement, _) = oneshot::channel(); let mut gate = gate; std::mem::replace(&mut gate.armed, replacement).send(()).unwrap(); active = Some(gate);},
                _ = async { (&mut active.as_mut().unwrap().release).await }, if held_reply.is_some() => { if client.send(held_reply.take().unwrap()).await.is_err() {break;} active = None; held_id = None; },
                message = client.next() => {let Some(Ok(message)) = message else {break}; if message.is_close() {break;}
                    if message.is_text() { let v:Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                        if active.as_ref().is_some_and(|g| v["method"] == g.method) && held_id.is_none() { held_id = Some(v["id"].clone()); }
                        observed.lock().unwrap().push(v);
                    }
                    if browser.send(message).await.is_err() {break;}
                },
                message = browser.next() => {let Some(Ok(message)) = message else {break}; if message.is_close() {break;}
                    if message.is_text() {let v:Value = serde_json::from_str(message.to_text().unwrap()).unwrap(); observed.lock().unwrap().push(v.clone());
                        if held_id.as_ref() == Some(&v["id"]) {assert!(v.get("error").is_none(), "native command must succeed before gate: {v}"); held_reply = Some(message); let (replacement, _) = oneshot::channel(); std::mem::replace(&mut active.as_mut().unwrap().entered, replacement).send(()).unwrap(); continue;}
                    }
                    if client.send(message).await.is_err() {break;}
                }
            }
        }
    });
    Proxy {
        endpoint,
        control,
        trace,
        task,
    }
}
pub struct Fixture {
    pub base: String,
    pub profile: PathBuf,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let profile = std::env::temp_dir().join(format!(
        "rustwright-intercept-{}-{}",
        std::process::id(),
        address.port()
    ));
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=> {let (mut socket,_)=accepted.unwrap(); requests.spawn(async move {
                    let mut request=Vec::new();
                    loop {
                        let mut buffer=[0;2048];
                        let Ok(n)=socket.read(&mut buffer).await else {return};
                        if n==0{return;}
                        request.extend_from_slice(&buffer[..n]);
                        if request.len()>16384{return;}
                        if request.windows(4).any(|v|v==b"\r\n\r\n"){break;}
                    }
                    let text=String::from_utf8_lossy(&request); let path=text.lines().next().and_then(|line|line.split_whitespace().nth(1)).unwrap_or("").split('?').next().unwrap();
                    let body=match path {"/page"|"/frame"=>"<!DOCTYPE html><title>intercepts</title><input id=q>".to_owned(),"/headers"=>text.to_string(),_=>"live".to_owned()};
                    let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nX-Fixture: original\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=socket.write_all(response.as_bytes()).await;
                });}, _=requests.join_next(),if !requests.is_empty()=>{}
            }
        }
    });
    Fixture {
        base: format!("http://{address}"),
        profile,
        task,
    }
}
pub struct Native {
    pub fixture: Fixture,
    pub proxy: Proxy,
    pub browser: BidiBrowser,
    _process: LaunchedFirefox,
}
impl Native {
    pub async fn start() -> Self {
        let fixture = fixture().await;
        let process = LaunchedFirefox::launch(
            &Firefox::installed()
                .headless(true)
                .profile(&fixture.profile),
        )
        .await
        .expect("installed Firefox must start; no skip");
        let proxy = proxy(process.ws_url()).await;
        let browser = BidiBrowser::connect(&proxy.endpoint).await.unwrap();
        Self {
            fixture,
            proxy,
            browser,
            _process: process,
        }
    }
    pub async fn known_absent(&self, ids: &[String]) {
        for id in ids {
            let error = self
                .browser
                .session()
                .remove_intercept(id)
                .await
                .expect_err("owned native intercept must already be removed");
            assert!(
                matches!(error,rustwright::bidi::BidiError::Protocol {ref error,..} if error=="no such intercept"),
                "unexpected removal result: {error:?}"
            );
        }
        assert_eq!(
            self.browser.session().connection().pending_command_count(),
            0
        );
    }
    pub async fn pages(&self) -> Vec<String> {
        let mut ids = self
            .browser
            .session()
            .get_tree()
            .await
            .unwrap()
            .into_iter()
            .filter(|v| v.parent.is_none())
            .map(|v| v.context)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
    pub fn report(&self, mode: &str, cycle: usize, ids: &[String]) {
        eprintln!(
            "{}",
            json!({"mode":mode,"cycle":cycle,"version":self.browser.browser_version(),"known_intercept_ids":ids,"known_intercepts_absent":true,"pending":self.browser.session().connection().pending_command_count()})
        );
    }
    pub async fn finish(self) {
        self.browser.close().await.unwrap();
    }
}
