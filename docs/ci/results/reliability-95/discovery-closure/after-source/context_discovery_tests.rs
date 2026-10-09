//! Discovery must distinguish a vanished target from a detached session on a live target.

use super::*;
use futures_util::{SinkExt, StreamExt};
use rustwright_cdp::CdpError;
use serde_json::Value;
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy)]
enum Proof {
    Missing,
    Present,
    ProtocolError,
    NearMatch,
    InitializationError,
    Stalled,
    Disconnect,
}

struct Peer {
    context: BrowserContext,
    commands: mpsc::UnboundedReceiver<Value>,
    server: JoinHandle<()>,
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.context.connection.close();
        self.server.abort();
    }
}

impl Peer {
    async fn new(proof: Proof) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, commands) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let mut stalled = None;
            while let Some(Ok(message)) = socket.next().await {
                if !message.is_text() {
                    continue;
                }
                let command: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if tx.send(command.clone()).is_err() {
                    break;
                }
                let id = command["id"].clone();
                let response = match command["method"].as_str().unwrap() {
                    "Target.getTargets" => json!({"id":id,"result":{"targetInfos":[{
                        "targetId":"vanishing-tab","type":"page","browserContextId":"isolated"
                    }]}}),
                    "Target.attachToTarget" => {
                        json!({"id":id,"result":{"sessionId":"partial-session"}})
                    }
                    "Page.enable" if matches!(proof, Proof::InitializationError) => {
                        json!({"id":id,"error":{"code":-32000,"message":"initialization failed"}})
                    }
                    "Page.enable" => {
                        socket.send(Message::Text(json!({"method":"Target.detachedFromTarget",
                            "params":{"sessionId":"partial-session","targetId":"vanishing-tab"}
                        }).to_string().into())).await.unwrap();
                        continue;
                    }
                    "Target.getTargetInfo" => {
                        assert_eq!(command["params"]["targetId"], "vanishing-tab");
                        assert!(
                            command.get("sessionId").is_none(),
                            "proof uses the browser session"
                        );
                        match proof {
                            Proof::Present => json!({"id":id,"result":{"targetInfo":{
                                "targetId":"vanishing-tab","type":"page","browserContextId":"isolated"
                            }}}),
                            Proof::ProtocolError => {
                                json!({"id":id,"error":{"code":-32000,"message":"proof unavailable"}})
                            }
                            Proof::NearMatch => {
                                json!({"id":id,"error":{"code":-32602,"message":"No target with given id found elsewhere"}})
                            }
                            Proof::Stalled => {
                                stalled = Some(id);
                                continue;
                            }
                            Proof::Disconnect => {
                                socket.close(None).await.unwrap();
                                break;
                            }
                            Proof::Missing | Proof::InitializationError => missing(id),
                        }
                    }
                    "Test.releaseProof" => {
                        socket
                            .send(Message::Text(
                                missing(stalled.take().expect("pending proof"))
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap();
                        json!({"id":id,"result":{}})
                    }
                    "Target.detachFromTarget" | "Target.disposeBrowserContext" => {
                        json!({"id":id,"result":{}})
                    }
                    method => panic!("unexpected discovery command: {method}"),
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let connection = CdpConnection::connect(&url).await.unwrap();
        Self {
            context: BrowserContext::new(connection, Some("isolated".into())),
            commands,
            server,
        }
    }

    async fn observed(&mut self, method: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let command = self.commands.recv().await.expect("peer remains connected");
                if command["method"] == method {
                    return command;
                }
            }
        })
        .await
        .expect("requested command must arrive")
    }
}

fn missing(id: Value) -> Value {
    json!({"id":id,"error":{"code":-32602,"message":"No target with given id found"}})
}

#[tokio::test]
async fn vanished_target_during_page_enable_is_skipped_after_browser_proof() {
    let mut peer = Peer::new(Proof::Missing).await;
    let result = peer.context.refresh_pages().await;
    println!("stale snapshot refresh: {result:?}");
    assert!(
        matches!(result, Ok(ref pages) if pages.is_empty()),
        "{result:?}"
    );
    peer.observed("Target.getTargetInfo").await;
    assert!(peer.context.pages().is_empty());
}

#[tokio::test]
async fn vanished_popup_during_initialization_keeps_waiting_until_deadline() {
    let peer = Peer::new(Proof::Missing).await;
    let result = peer.context.wait_for_page(Duration::from_millis(100)).await;
    println!("vanished popup wait: {result:?}");
    assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
    assert!(peer.context.pages().is_empty());
}

#[tokio::test]
async fn detached_session_on_existing_target_remains_an_initialization_error() {
    let mut peer = Peer::new(Proof::Present).await;
    let result = peer.context.refresh_pages().await;
    assert!(
        matches!(result, Err(Error::Cdp(CdpError::SessionDetached { ref method, .. })) if method == "Page.enable"),
        "{result:?}"
    );
    peer.observed("Target.detachFromTarget").await;
    assert!(peer.context.pages().is_empty());
}

#[tokio::test]
async fn target_proof_protocol_error_is_not_hidden() {
    let peer = Peer::new(Proof::ProtocolError).await;
    let result = peer.context.refresh_pages().await;
    assert!(
        matches!(result, Err(Error::Cdp(CdpError::Protocol { ref method, code:-32000, .. })) if method == "Target.getTargetInfo"),
        "{result:?}"
    );
}

#[tokio::test]
async fn near_match_missing_target_message_is_not_accepted_as_proof() {
    let peer = Peer::new(Proof::NearMatch).await;
    let result = peer.context.refresh_pages().await;
    assert!(
        matches!(result, Err(Error::Cdp(CdpError::Protocol { ref method, code:-32602, .. })) if method == "Target.getTargetInfo"),
        "{result:?}"
    );
}

#[tokio::test]
async fn ordinary_initialization_protocol_error_is_not_reinterpreted_as_closure() {
    let mut peer = Peer::new(Proof::InitializationError).await;
    let result = peer.context.refresh_pages().await;
    assert!(
        matches!(result, Err(Error::Cdp(CdpError::Protocol { ref method, code:-32000, .. })) if method == "Page.enable"),
        "{result:?}"
    );
    peer.observed("Target.detachFromTarget").await;
    while let Ok(command) = peer.commands.try_recv() {
        assert_ne!(command["method"], "Target.getTargetInfo");
    }
}

#[tokio::test]
async fn socket_loss_during_target_proof_remains_browser_closed() {
    let mut peer = Peer::new(Proof::Disconnect).await;
    let context = peer.context.clone();
    let refresh = tokio::spawn(async move { context.refresh_pages().await });
    peer.observed("Target.getTargetInfo").await;
    let result = refresh.await.unwrap();
    assert!(matches!(result, Err(Error::BrowserClosed)), "{result:?}");
}

#[tokio::test]
async fn context_closure_during_target_proof_remains_context_closed() {
    let mut peer = Peer::new(Proof::Stalled).await;
    let context = peer.context.clone();
    let refresh = tokio::spawn(async move { context.refresh_pages().await });
    peer.observed("Target.getTargetInfo").await;
    let context = peer.context.clone();
    let close = tokio::spawn(async move { context.close().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !peer.context.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    peer.context
        .connection
        .send_raw(None, "Test.releaseProof", json!({}))
        .await
        .unwrap();
    let result = refresh.await.unwrap();
    assert!(matches!(result, Err(Error::ContextClosed)), "{result:?}");
    close.await.unwrap().unwrap();
}
