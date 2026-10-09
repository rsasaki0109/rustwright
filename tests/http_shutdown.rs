//! Native cancellation and concurrent completion of resource shutdown.
use rustwright::{
    bidi::BidiBrowser,
    browser::{LaunchedBrowser, LaunchedFirefox},
    cdp::CdpConnection,
    prelude::*,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/lifecycle.rs"]
mod lifecycle;
use lifecycle::{fixture, healthy, proxy, restored, Engine, LIMIT};

async fn scenario(firefox: bool) {
    let fixture = fixture(firefox).await;
    let chrome;
    let firefox_process;
    let upstream = if firefox {
        firefox_process = LaunchedFirefox::launch(
            &Firefox::installed()
                .headless(true)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must start; no skip");
        firefox_process.ws_url().to_owned()
    } else {
        chrome = LaunchedBrowser::launch(&Chrome::installed().headless(true))
            .await
            .expect("Chrome must start; no skip");
        chrome.ws_url().to_owned()
    };
    let proxy = proxy(upstream.clone()).await;
    let engine = if firefox {
        Engine::Firefox(Arc::new(
            BidiBrowser::connect(&proxy.endpoint).await.unwrap(),
        ))
    } else {
        Engine::Chrome(
            Arc::new(Browser::connect(&proxy.endpoint).await.unwrap()),
            CdpConnection::connect(&upstream).await.unwrap(),
        )
    };
    let version = match &engine {
        Engine::Chrome(browser, _) => browser.version().browser.clone(),
        Engine::Firefox(browser) => browser.browser_version().unwrap().to_owned(),
    };
    let keeper = engine.new_page().await.unwrap();
    healthy(&keeper, &fixture.url).await;
    let baseline = engine.counts().await;
    let modes: &[&str] = if firefox {
        &["context-response", "context-helper", "page-helper"]
    } else {
        &["context-page-response", "context-response"]
    };
    for &mode in modes {
        for cycle in 1..=10 {
            let context = Arc::new(engine.new_context().await.unwrap());
            let first = context.new_page().await.unwrap();
            let second = context.new_page().await.unwrap();
            healthy(&first, &fixture.url).await;
            healthy(&second, &fixture.url).await;
            let mut page_baseline = engine.counts().await;
            let method = match (firefox, mode) {
                (false, "context-page-response") => "Target.closeTarget",
                (false, _) => "Target.disposeBrowserContext",
                (true, "context-response") => "browser.removeUserContext",
                (true, _) => "script.removePreloadScript",
            };
            let (entered, release) = proxy.arm(method).await;
            let owner = context.clone();
            let page = first.clone();
            let cancelled = tokio::spawn(async move {
                if mode == "page-helper" {
                    page.close().await.unwrap();
                } else {
                    owner.close().await;
                }
            });
            tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
            assert!(
                engine.pending() > 0,
                "shutdown must await the held response"
            );
            cancelled.abort();
            assert!(cancelled.await.unwrap_err().is_cancelled());
            let owner = context.clone();
            let page = first.clone();
            let repeated = tokio::spawn(async move {
                if mode == "page-helper" {
                    page.close().await.unwrap();
                } else {
                    owner.close().await;
                }
            });
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert!(
                !repeated.is_finished(),
                "concurrent close must await cleanup"
            );
            release.send(()).unwrap();
            tokio::time::timeout(LIMIT, repeated)
                .await
                .unwrap()
                .unwrap();
            if mode == "page-helper" {
                let AnyPage::Firefox(page) = &first else {
                    unreachable!()
                };
                page_baseline.pages.retain(|id| id != page.context_id());
                page_baseline.helpers = page_baseline.helpers.map(|n| n - 1);
                restored(&engine, &page_baseline).await;
                assert_eq!(second.title().await.unwrap(), "creation");
                context.close().await;
            }
            restored(&engine, &baseline).await;
            assert!(first.title().await.is_err());
            assert!(second.title().await.is_err());
            assert_eq!(keeper.title().await.unwrap(), "creation");
            let next = engine.new_context().await.unwrap();
            let next_page = next.new_page().await.unwrap();
            healthy(&next_page, &fixture.url).await;
            next.close().await;
            restored(&engine, &baseline).await;
            eprintln!(
                "{}",
                json!({
                    "engine": if firefox {"firefox"} else {"chrome"},
                    "mode": mode, "cycle": cycle, "version": version,
                    "pending": engine.pending(), "page_ids": baseline.pages,
                    "context_ids": baseline.owners, "helper_preloads": baseline.helpers,
                    "retained_closed_handles": 2, "keeper_preserved": true,
                    "concurrent_close_waited": true, "subsequent_context_usable": true
                })
            );
        }
    }
    keeper.close().await.unwrap();
    drop(keeper);
    engine.close().await;
}

#[tokio::test]
async fn chrome_cancelled_shutdown_completes_native_cleanup() {
    scenario(false).await;
}

#[tokio::test]
async fn firefox_cancelled_shutdown_completes_native_cleanup() {
    scenario(true).await;
}

#[tokio::test]
async fn firefox_cancelled_monitor_setup_still_waits_for_subscription() {
    let fixture = fixture(true).await;
    let process = LaunchedFirefox::launch(
        &Firefox::installed()
            .headless(true)
            .profile(&fixture.profile),
    )
    .await
    .expect("Firefox must start; no skip");
    let proxy = proxy(process.ws_url().to_owned()).await;
    let browser = BidiBrowser::connect(&proxy.endpoint).await.unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&fixture.url).await.unwrap();
    let (entered, release) = proxy.arm("session.subscribe").await;
    let caller = page.clone();
    let cancelled = tokio::spawn(async move { caller.start_network_monitoring().await });
    tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
    assert!(browser.session().connection().pending_command_count() > 0);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let caller = page.clone();
    let repeated = tokio::spawn(async move { caller.start_network_monitoring().await });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !repeated.is_finished(),
        "monitor setup must await actual subscription readiness"
    );
    release.send(()).unwrap();
    tokio::time::timeout(LIMIT, repeated)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for cycle in 1..=10 {
        page.start_network_monitoring().await.unwrap();
        let url = format!("{}?monitor-cycle={cycle}", fixture.url);
        page.evaluate(&format!("fetch({}).then(r=>r.text())", json!(url)))
            .await
            .unwrap();
        tokio::time::timeout(LIMIT, async {
            loop {
                if page
                    .network_requests()
                    .iter()
                    .any(|r| r.url == url && r.status == Some(200))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("monitor must observe response completion after cancelled setup");
    }
    assert_eq!(browser.session().connection().pending_command_count(), 0);
    assert_eq!(page.title().await.unwrap(), "creation");
    page.close().await.unwrap();
    drop(page);
    browser.close().await.unwrap();
}
