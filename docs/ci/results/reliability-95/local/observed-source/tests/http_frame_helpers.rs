//! Long-lived Firefox page helper use across destroyed and replaced frame realms.
use rustwright::{bidi::BidiBrowser, prelude::*};
use serde_json::{json, Value};
use std::{collections::HashSet, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const LIMIT: Duration = Duration::from_secs(5);

struct Fixture {
    base: String,
    profile: PathBuf,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}
async fn open() -> (Fixture, BidiBrowser, rustwright::bidi::BidiPage) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
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
                            let Ok(n)=socket.read(&mut bytes).await else {return};
                            if n==0 || request.len()+n>16*1024 {return}
                            request.extend_from_slice(&bytes[..n]);
                            if request.windows(4).any(|b|b==b"\r\n\r\n") {break}
                        }
                        let body="<!doctype html><title>frame helper</title><input id=q><button id=go onclick='window.clicked=event.isTrusted'>Go</button>";
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=socket.write_all(response.as_bytes()).await;
                    });
                },
                _=requests.join_next(), if !requests.is_empty()=>{},
            }
        }
    });
    let fixture = Fixture {
        base: format!("http://{address}"),
        profile: std::env::temp_dir().join(format!(
            "rustwright-frame-helper-{}-{}",
            std::process::id(),
            address.port()
        )),
        task,
    };
    let browser = BidiBrowser::launch(
        Firefox::installed()
            .headless(true)
            .profile(&fixture.profile),
    )
    .await
    .expect("Firefox must start; no skip");
    eprintln!("Firefox version: {:?}", browser.browser_version());
    let page = browser.new_page().await.unwrap();
    page.goto(&format!("{}/page", fixture.base)).await.unwrap();
    (fixture, browser, page)
}

async fn counts(browser: &BidiBrowser) -> Value {
    let mut pages: Vec<_> = browser
        .session()
        .get_tree()
        .await
        .unwrap()
        .into_iter()
        .map(|info| (info.context, info.children.unwrap_or_default().len()))
        .collect();
    pages.sort();
    let users = browser
        .session()
        .connection()
        .send("browser.getUserContexts", json!({}))
        .await
        .unwrap();
    let mut users: Vec<_> = users["userContexts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["userContext"].as_str().unwrap().to_owned())
        .collect();
    users.sort();
    json!({"pages":pages,"users":users,"helpers":browser.session().helper_preload_count()})
}

async fn append_frame(page: &rustwright::bidi::BidiPage, url: &str) {
    page.evaluate(&format!("new Promise(resolve=>{{const f=document.createElement('iframe');f.id='child';f.onload=()=>resolve(true);f.src={};document.body.append(f)}})",json!(url))).await.unwrap();
}

#[tokio::test]
async fn firefox_same_page_reuses_helpers_across_1000_destroyed_frames() {
    let (fixture, browser, page) = open().await;
    let baseline = counts(&browser).await;
    let mut observed_ids = HashSet::new();
    for cycle in 1..=1000 {
        tokio::time::timeout(LIMIT, async {
            let base = if cycle % 2 == 0 {
                fixture.base.replace("127.0.0.1", "localhost")
            } else {
                fixture.base.clone()
            };
            append_frame(&page, &format!("{base}/frame?cycle={cycle}")).await;
            let frame = page.frame_locator("#child").await.unwrap();
            assert!(
                observed_ids.insert(frame.context_id().to_owned()),
                "every appended frame must have a fresh context"
            );
            assert_eq!(frame.locator("#q").count().await.unwrap(), 1);
            if cycle % 100 == 0 {
                frame.locator("#q").fill("継続フレーム Rust").await.unwrap();
                assert_eq!(
                    frame
                        .evaluate("document.querySelector('#q').value")
                        .await
                        .unwrap(),
                    json!("継続フレーム Rust")
                );
                frame.locator("#go").click().await.unwrap();
                assert_eq!(frame.evaluate("window.clicked").await.unwrap(), json!(true));
            }
            page.evaluate("document.querySelector('#child').remove();true")
                .await
                .unwrap();
            assert!(
                frame.locator("#q").count().await.is_err(),
                "a retained destroyed frame must report a protocol failure"
            );
        })
        .await
        .expect("frame churn watchdog");
        if cycle % 100 == 0 {
            assert_eq!(counts(&browser).await, baseline);
            assert_eq!(browser.session().connection().pending_command_count(), 0);
            assert_eq!(page.title().await.unwrap(), "frame helper");
            eprintln!(
                "{}",
                json!({"kind":"frame-churn","cycle":cycle,"frame_ids_seen":observed_ids.len(),"counts":baseline,"pending":0})
            );
        }
    }
    page.close().await.unwrap();
    assert_eq!(browser.session().helper_preload_count(), 0);
    drop(page);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn firefox_helper_presence_tracks_current_frame_document() {
    let (fixture, browser, page) = open().await;
    append_frame(&page, &format!("{}/frame", fixture.base)).await;
    let frame = page.frame_locator("#child").await.unwrap();
    assert_eq!(frame.locator("#q").count().await.unwrap(), 1);
    // A preload may also run after navigation, so remove the current realm's
    // helper explicitly to ensure the locator detects absence rather than an ID.
    frame
        .evaluate("delete window.__rustwright;true")
        .await
        .unwrap();
    assert_eq!(frame.locator("#q").count().await.unwrap(), 1);
    for cycle in 1..=10 {
        let base = if cycle % 2 == 0 {
            fixture.base.replace("127.0.0.1", "localhost")
        } else {
            fixture.base.clone()
        };
        page.evaluate(&format!("new Promise(resolve=>{{const f=document.querySelector('#child');f.onload=()=>resolve(true);f.src={}}})",json!(format!("{base}/reload?cycle={cycle}")))).await.unwrap();
        let reloaded = page.frame_locator("#child").await.unwrap();
        // Firefox may retain a browsing-context ID across a document replacement.
        reloaded
            .evaluate("delete window.__rustwright;true")
            .await
            .unwrap();
        assert_eq!(reloaded.locator("#q").count().await.unwrap(), 1);
        reloaded.locator("#q").fill("再読込 Rust").await.unwrap();
        assert_eq!(
            reloaded
                .evaluate("document.querySelector('#q').value")
                .await
                .unwrap(),
            json!("再読込 Rust")
        );
    }
    assert_eq!(browser.session().helper_preload_count(), 1);
    page.close().await.unwrap();
    assert_eq!(browser.session().helper_preload_count(), 0);
    drop(frame);
    drop(page);
    browser.close().await.unwrap();
}
