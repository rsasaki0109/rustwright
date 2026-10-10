//! Deterministically cancel real browser allocations after remote execution.
use rustwright::{
    bidi::BidiBrowser,
    browser::{LaunchedBrowser, LaunchedFirefox},
    cdp::CdpConnection,
    prelude::*,
};
use serde_json::json;
use std::sync::Arc;
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
    let engine = Arc::new(if firefox {
        Engine::Firefox(Arc::new(
            BidiBrowser::connect(&proxy.endpoint).await.unwrap(),
        ))
    } else {
        Engine::Chrome(
            Arc::new(Browser::connect(&proxy.endpoint).await.unwrap()),
            CdpConnection::connect(&upstream).await.unwrap(),
        )
    });
    let version = match &*engine {
        Engine::Chrome(browser, _) => browser.version().browser.clone(),
        Engine::Firefox(browser) => browser.browser_version().unwrap().to_owned(),
    };
    let keeper = engine.new_page().await.unwrap();
    healthy(&keeper, &fixture.url).await;
    let initial = engine.counts().await;
    for mode in [
        "context",
        "default-create",
        "default-init",
        "isolated-create",
        "isolated-init",
    ] {
        let context = if mode.starts_with("isolated") {
            Some(Arc::new(engine.new_context().await.unwrap()))
        } else {
            None
        };
        let baseline = engine.counts().await;
        let method = match (firefox, mode) {
            (false, "context") => "Target.createBrowserContext",
            (true, "context") => "browser.createUserContext",
            (false, "default-init" | "isolated-init") => "Page.enable",
            (true, "default-init" | "isolated-init") => "script.addPreloadScript",
            (false, _) => "Target.createTarget",
            (true, _) => "browsingContext.create",
        };
        for cycle in 1..=10 {
            let (entered, release) = proxy.arm(method).await;
            let caller = engine.clone();
            let owner = context.clone();
            let task = tokio::spawn(async move {
                if mode == "context" {
                    caller.new_context().await.map(|_| ())
                } else if let Some(owner) = owner {
                    owner.new_page().await.map(|_| ())
                } else {
                    caller.new_page().await.map(|_| ())
                }
            });
            tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
            assert!(
                engine.pending() > 0,
                "the caller must be awaiting the held response"
            );
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            release.send(()).unwrap();
            restored(&engine, &baseline).await;
            assert_eq!(keeper.title().await.unwrap(), "creation");
            let page = if let Some(context) = &context {
                context.new_page().await.unwrap()
            } else {
                engine.new_page().await.unwrap()
            };
            healthy(&page, &fixture.url).await;
            page.close().await.unwrap();
            restored(&engine, &baseline).await;
            eprintln!(
                "{}",
                json!({"engine":if firefox {"firefox"} else {"chrome"},"mode":mode,"cycle":cycle,"pending":engine.pending(),"pages":baseline.pages.len(),"contexts":baseline.owners.len(),"helper_preloads":baseline.helpers,"version":version,"page_ids":baseline.pages,"context_ids":baseline.owners,"existing_page_preserved":true,"subsequent_page_usable":true})
            );
        }
        if let Some(context) = context {
            context.close().await;
        }
        restored(&engine, &initial).await;
    }
    keeper.close().await.unwrap();
    drop(keeper);
    Arc::try_unwrap(engine).ok().unwrap().close().await;
}
#[tokio::test]
async fn chrome_cancelled_creation_releases_native_resources() {
    scenario(false).await;
}
#[tokio::test]
async fn firefox_cancelled_creation_releases_native_resources() {
    scenario(true).await;
}
