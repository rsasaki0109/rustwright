//! Native BiDi interception ownership, cancellation and descendant routing.
use rustwright::{bidi::BidiPage, prelude::*};
use serde_json::{json, Value};
use std::time::Duration;
#[path = "support/intercept.rs"]
mod support;
use support::{Native, LIMIT};

async fn fetch(page: &BidiPage, url: &str) -> Value {
    tokio::time::timeout(
        LIMIT,
        page.evaluate(&format!("fetch({}).then(r=>r.text())", json!(url))),
    )
    .await
    .expect("fetch must not be stranded by an owned intercept")
    .unwrap()
}
async fn header(page: &BidiPage, url: &str) -> Value {
    tokio::time::timeout(
        LIMIT,
        page.evaluate(&format!(
            "fetch({}).then(r=>r.headers.get('x-fixture'))",
            json!(url)
        )),
    )
    .await
    .expect("response phase must continue")
    .unwrap()
}
async fn open(native: &Native) -> BidiPage {
    let page = native.browser.new_page().await.unwrap();
    page.goto(&format!("{}/page", native.fixture.base))
        .await
        .unwrap();
    page
}

#[tokio::test]
async fn firefox_phase_expansion_keeps_prior_rules_usable_before_acknowledgment() {
    let native = Native::start().await;
    let page = open(&native).await;
    let baseline = native.pages().await;
    for cycle in 1..=10 {
        let start = native.proxy.allocations().len();
        let old = format!("{}/api?active-upgrade={cycle}", native.fixture.base);
        let response = format!("{}/response?active-upgrade={cycle}", native.fixture.base);
        page.mock(&old, 200, "text/plain", "still-active")
            .await
            .unwrap();
        let (entered, release) = native.proxy.arm("network.addIntercept").await;
        let caller = page.clone();
        let pattern = response.clone();
        let task = tokio::spawn(async move {
            caller
                .route(
                    pattern,
                    RouteAction::SetResponseHeaders(vec![(
                        "X-Fixture".into(),
                        "uncommitted".into(),
                    )]),
                )
                .await
        });
        tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
        page.evaluate(&format!(
            "window.phaseFetch=fetch({}).then(r=>r.headers.get('x-fixture')); 'started'",
            json!(response)
        ))
        .await
        .unwrap();
        // The new-only response event must not prevent old-ID requests being served.
        native
            .proxy
            .blocked_event("network.responseStarted", &response)
            .await;
        assert_eq!(fetch(&page, &old).await, json!("still-active"));
        assert!(
            !task.is_finished(),
            "registration must still await its held reply"
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(LIMIT, page.evaluate("window.phaseFetch"))
                .await
                .expect("deferred response must continue after canceled expansion")
                .unwrap(),
            json!("original")
        );
        page.route(
            "*/never-active-upgrade*",
            RouteAction::SetRequestHeaders(vec![]),
        )
        .await
        .unwrap();
        assert_eq!(fetch(&page, &old).await, json!("still-active"));
        page.clear_routes().await.unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        native.known_absent(&ids).await;
        assert_eq!(native.pages().await, baseline);
        native.report("active-rules-during-phase-upgrade", cycle, &ids);
    }
    page.close().await.unwrap();
    drop(page);
    native.finish().await;
}

#[tokio::test]
async fn firefox_intercept_cancellation_preserves_live_requests_and_prior_rules() {
    let native = Native::start().await;
    let page = open(&native).await;
    let baseline = native.pages().await;
    for cycle in 1..=10 {
        let start = native.proxy.allocations().len();
        let url = format!("{}/api?cancel-add={cycle}", native.fixture.base);
        let (entered, release) = native.proxy.arm("network.addIntercept").await;
        let caller = page.clone();
        let pattern = url.clone();
        let task =
            tokio::spawn(async move { caller.mock(pattern, 200, "text/plain", "canceled").await });
        tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
        page.evaluate(&format!(
            "window.pendingFetch=fetch({}).then(r=>r.text()); 'started'",
            json!(url)
        ))
        .await
        .unwrap();
        native.proxy.blocked(&url).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(LIMIT, page.evaluate("window.pendingFetch"))
                .await
                .expect("pre-ack blocked request must be released during cancellation cleanup")
                .unwrap(),
            json!("live")
        );
        page.clear_routes().await.unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        assert_eq!(ids.len(), 1);
        native.known_absent(&ids).await;
        assert_eq!(native.pages().await, baseline);
        native.report("cancel-add-with-blocked-request", cycle, &ids);

        let start = native.proxy.allocations().len();
        let url = format!("{}/api?cancel-clear={cycle}", native.fixture.base);
        page.mock(&url, 200, "text/plain", "old").await.unwrap();
        assert_eq!(fetch(&page, &url).await, json!("old"));
        let (entered, release) = native.proxy.arm("network.removeIntercept").await;
        let caller = page.clone();
        let task = tokio::spawn(async move { caller.clear_routes().await });
        tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let caller = page.clone();
        let repeated = tokio::spawn(async move { caller.clear_routes().await });
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !repeated.is_finished(),
            "second clear must await acknowledged retirement"
        );
        assert_eq!(fetch(&page, &url).await, json!("live"));
        release.send(()).unwrap();
        tokio::time::timeout(LIMIT, repeated)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        native.known_absent(&ids).await;
        assert_eq!(native.pages().await, baseline);
        native.report("cancel-clear", cycle, &ids);

        let start = native.proxy.allocations().len();
        let old = format!("{}/api?old={cycle}", native.fixture.base);
        let response = format!("{}/api?response={cycle}", native.fixture.base);
        page.mock(&old, 200, "text/plain", "preserved")
            .await
            .unwrap();
        let (entered, release) = native.proxy.arm("network.addIntercept").await;
        let caller = page.clone();
        let pattern = response.clone();
        let task = tokio::spawn(async move {
            caller
                .route(
                    pattern,
                    RouteAction::SetResponseHeaders(vec![("X-Fixture".into(), "canceled".into())]),
                )
                .await
        });
        tokio::time::timeout(LIMIT, entered).await.unwrap().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        // A subsequent mutation waits behind the protected rollback without clearing prior rules.
        page.route("*/never-match*", RouteAction::SetRequestHeaders(vec![]))
            .await
            .unwrap();
        assert_eq!(fetch(&page, &old).await, json!("preserved"));
        assert_eq!(header(&page, &response).await, json!("original"));
        page.route(
            &response,
            RouteAction::SetResponseHeaders(vec![("X-Fixture".into(), "replaced".into())]),
        )
        .await
        .unwrap();
        assert_eq!(header(&page, &response).await, json!("replaced"));
        page.clear_routes().await.unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        native.known_absent(&ids).await;
        assert_eq!(native.pages().await, baseline);
        native.report("cancel-phase-upgrade", cycle, &ids);
    }
    page.close().await.unwrap();
    drop(page);
    native.finish().await;
}

#[tokio::test]
async fn firefox_shared_handles_drop_and_close_retire_only_owned_intercepts() {
    let native = Native::start().await;
    let keeper = open(&native).await;
    let keep_url = format!("{}/api?keeper", native.fixture.base);
    keeper
        .mock(&keep_url, 200, "text/plain", "keeper")
        .await
        .unwrap();
    let baseline = native.pages().await;
    for cycle in 1..=10 {
        let start = native.proxy.allocations().len();
        let context = native.browser.new_context().await.unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&format!("{}/page", native.fixture.base))
            .await
            .unwrap();
        let id = page.context_id().to_owned();
        let url = format!("{}/api?drop={cycle}", native.fixture.base);
        page.mock(&url, 200, "text/plain", "shared").await.unwrap();
        let discovered = context
            .pages()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.context_id() == id)
            .unwrap();
        drop(page);
        assert_eq!(fetch(&discovered, &url).await, json!("shared"));
        assert_eq!(
            native.proxy.allocations().len(),
            start + 1,
            "rediscovery must share one owned registration"
        );
        drop(discovered);
        let ids = native.proxy.allocations()[start..].to_vec();
        native.proxy.retired(&ids).await;
        native.known_absent(&ids).await;
        let recovered = context
            .pages()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.context_id() == id)
            .unwrap();
        assert_eq!(fetch(&recovered, &url).await, json!("live"));
        assert_eq!(fetch(&keeper, &keep_url).await, json!("keeper"));
        context.close().await.unwrap();
        assert_eq!(native.pages().await, baseline);
        native.report("shared-handles-final-drop", cycle, &ids);

        let start = native.proxy.allocations().len();
        let context = native.browser.new_context().await.unwrap();
        let page = context.new_page().await.unwrap();
        page.goto(&format!("{}/page", native.fixture.base))
            .await
            .unwrap();
        page.mock(&url, 200, "text/plain", "closed").await.unwrap();
        let retained = page.clone();
        page.close().await.unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        native.known_absent(&ids).await;
        assert!(retained.title().await.is_err());
        assert!(retained
            .mock(&url, 200, "text/plain", "invalid")
            .await
            .is_err());
        context.close().await.unwrap();
        assert_eq!(fetch(&keeper, &keep_url).await, json!("keeper"));
        assert_eq!(native.pages().await, baseline);
        native.report("page-close-retained-handles", cycle, &ids);

        let start = native.proxy.allocations().len();
        let context = native.browser.new_context().await.unwrap();
        let first = context.new_page().await.unwrap();
        let second = context.new_page().await.unwrap();
        for p in [&first, &second] {
            p.goto(&format!("{}/page", native.fixture.base))
                .await
                .unwrap();
            p.mock(&url, 200, "text/plain", "owned").await.unwrap();
        }
        context.close().await.unwrap();
        let ids = native.proxy.allocations()[start..].to_vec();
        assert_eq!(ids.len(), 2);
        native.known_absent(&ids).await;
        for p in [&first, &second] {
            assert!(p.title().await.is_err());
            assert!(p.block("*").await.is_err());
        }
        assert_eq!(fetch(&keeper, &keep_url).await, json!("keeper"));
        assert_eq!(native.pages().await, baseline);
        native.report("user-context-close-retained-pages", cycle, &ids);
    }
    keeper.clear_routes().await.unwrap();
    native.known_absent(&native.proxy.allocations()).await;
    keeper.close().await.unwrap();
    drop(keeper);
    native.finish().await;
}

#[tokio::test]
async fn firefox_routing_phases_and_child_frame_requests_remain_functional() {
    let native = Native::start().await;
    let page = open(&native).await;
    let api = format!("{}/api", native.fixture.base);
    page.mock("*/api*", 201, "text/plain", "first")
        .await
        .unwrap();
    page.mock("*/api*", 202, "text/plain", "second")
        .await
        .unwrap();
    assert_eq!(fetch(&page, &api).await, json!("first"));
    let headers = format!("{}/headers", native.fixture.base);
    page.route(
        format!("{headers}*"),
        RouteAction::SetRequestHeaders(vec![("X-Request".into(), "replacement".into())]),
    )
    .await
    .unwrap();
    assert!(fetch(&page, &headers)
        .await
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("x-request: replacement"));
    let response = format!("{}/response", native.fixture.base);
    page.route(
        "*/response*",
        RouteAction::SetResponseHeaders(vec![("X-Fixture".into(), "modified".into())]),
    )
    .await
    .unwrap();
    assert_eq!(header(&page, &response).await, json!("modified"));
    page.block("*/abort*").await.unwrap();
    assert_eq!(
        page.evaluate("fetch('/abort').then(()=>false,()=>true)")
            .await
            .unwrap(),
        json!(true)
    );
    for host in ["127.0.0.1", "localhost"] {
        let frame_url = format!("{}/frame", native.fixture.base.replace("127.0.0.1", host));
        tokio::time::timeout(LIMIT, page.evaluate(&format!("new Promise(resolve=>{{document.querySelector('iframe')?.remove();const f=document.createElement('iframe'); f.src={};f.onload=()=>resolve(true);document.body.append(f)}})",json!(frame_url)))).await.expect("child document must load without being stranded").unwrap();
        let frame = page
            .frames()
            .await
            .unwrap()
            .into_iter()
            .find(|f| f.context_id() != page.context_id())
            .unwrap();
        let body = tokio::time::timeout(LIMIT, frame.evaluate("fetch('/api').then(r=>r.text())"))
            .await
            .expect("descendant request must use parent-owned intercept")
            .unwrap();
        assert_eq!(body, json!("first"));
        let h = tokio::time::timeout(
            LIMIT,
            frame.evaluate("fetch('/response').then(r=>r.headers.get('x-fixture'))"),
        )
        .await
        .expect("descendant response must continue")
        .unwrap();
        assert_eq!(h, json!("modified"));
    }
    page.clear_routes().await.unwrap();
    native.known_absent(&native.proxy.allocations()).await;
    assert_eq!(fetch(&page, &api).await, json!("live"));
    native.report(
        "request-response-child-frame",
        1,
        &native.proxy.allocations(),
    );
    page.close().await.unwrap();
    drop(page);
    native.finish().await;
}
