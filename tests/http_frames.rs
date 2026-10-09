//! Frame and click regressions using shared HTTP reliability fixtures.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use rustwright::prelude::*;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

struct Fixture {
    base: String,
    task: JoinHandle<()>,
    network_requests: Arc<AtomicUsize>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let base = format!("http://{}", listener.local_addr().expect("fixture address"));
    let network_requests = Arc::new(AtomicUsize::new(0));
    let observed = network_requests.clone();
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.expect("accept fixture");
                    let observed = observed.clone();
                    requests.spawn(async move {
                        let mut request = Vec::new();
                        let read = tokio::time::timeout(Duration::from_secs(2), async {
                            loop {
                                let mut buffer = [0; 2048];
                                let count = socket.read(&mut buffer).await?;
                                request.extend_from_slice(&buffer[..count]);
                                if count == 0 || request.windows(4).any(|part| part == b"\r\n\r\n") || request.len() > 16384 {
                                    return std::io::Result::Ok(());
                                }
                            }
                        }).await;
                        if !matches!(read, Ok(Ok(()))) { return; }
                        let text = String::from_utf8_lossy(&request);
                        let path = text.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("").split('?').next().unwrap_or("");
                        if path == "/api/network" { observed.fetch_add(1, Ordering::SeqCst); }
                        let (status, body) = match path {
                            "/index.html" => ("200 OK", include_str!("../bench/reliability/site/index.html")),
                            "/frame.html" => ("200 OK", include_str!("../bench/reliability/site/frame.html")),
                            "/actionability.js" => ("200 OK", include_str!("../bench/reliability/site/actionability.js")),
                            "/network-frame.html" => ("200 OK", "<!DOCTYPE html><input id=value><script>window.initialResponse = fetch(new window.URL(location.href).searchParams.get('api') || '/api/network').then(r => { window.initialHeader = r.headers.get('x-fixture'); return r.text(); }).catch(() => 'blocked');</script>"),
                            "/xhr-frame.html" => ("200 OK", "<!DOCTYPE html><input id=value><script>window.initialResponse = new Promise(resolve => { const request = new XMLHttpRequest(); request.open('GET', '/api/network'); request.onload = () => resolve(request.responseText); request.onerror = request.onabort = () => resolve('blocked'); request.send(); });</script>"),
                            "/api/network" => ("200 OK", "network"),
                            "/api/cached" => ("200 OK", "network"),
                            "/api/request-headers" => ("200 OK", text.as_ref()),
                            "/slow-frame.html" => {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                                ("200 OK", include_str!("../bench/reliability/site/frame.html"))
                            }
                            "/delayed-child.html" => ("200 OK", "<!DOCTYPE html><script>setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<input id=late-child>'), 140)</script><body></body>"),
                            _ => ("404 Not Found", ""),
                        };
                        let mime = if path.ends_with(".js") { "text/javascript" } else { "text/html" };
                        let cache = if path == "/api/cached" { "Cache-Control: public, max-age=3600\r\n" } else { "" };
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}; charset=utf-8\r\nX-Fixture: original\r\n{cache}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                }
                _ = requests.join_next(), if !requests.is_empty() => {}
            }
        }
    });
    Fixture {
        base,
        task,
        network_requests,
    }
}

async fn open_network_frame(fixture: &Fixture, browser: &Browser, page: &Page) -> Frame {
    let url = format!(
        "{}/network-frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    page.evaluate(&format!("scheduleFrame(0, {url:?})"))
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        page.frame_locator("#late-frame")
            .locator("#value")
            .wait_for(WaitState::Visible),
    )
    .await
    .unwrap()
    .unwrap();
    let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
    assert_remote_frame(browser, &frame).await;
    frame
}

#[tokio::test]
async fn cross_site_network_mock_is_inherited_before_child_scripts_run() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 201, "text/plain", "mocked")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    let body = tokio::time::timeout(
        Duration::from_secs(2),
        frame.evaluate("window.initialResponse"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(body, json!("mocked"));
    browser.close().await.unwrap();
}

async fn open() -> (Fixture, Browser, Page) {
    let fixture = fixture().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch browser");
    let page = browser.new_page().await.expect("open page");
    page.goto(&format!("{}/index.html", fixture.base))
        .await
        .expect("load fixture");
    (fixture, browser, page)
}

#[tokio::test]
async fn fills_and_clicks_in_a_frame_added_after_the_action_starts() {
    let (_fixture, browser, page) = open().await;
    let frame = page.frame_locator("#late-frame");
    page.evaluate("scheduleFrame(150)")
        .await
        .expect("schedule frame");
    tokio::time::timeout(
        Duration::from_secs(2),
        frame.locator("#value").fill("payload"),
    )
    .await
    .expect("fill deadline")
    .expect("wait for frame and fill");
    frame
        .locator("#submit")
        .click()
        .await
        .expect("click frame button");
    tokio::time::timeout(Duration::from_secs(2), page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', resolve, {once: true}); })"))
        .await.expect("message deadline").expect("frame message");
    assert_eq!(
        page.evaluate("window.frameValue")
            .await
            .expect("submitted value"),
        json!("payload")
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn frame_wait_includes_iframe_resolution_in_its_deadline() {
    let (_fixture, browser, page) = open().await;
    let locator = page.frame_locator("#missing-frame").locator("#value");
    let duration = Duration::from_millis(120);
    let start = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        locator.wait_for_with_timeout(WaitState::Visible, duration),
    )
    .await
    .expect("watchdog")
    .expect_err("absent iframe must time out");
    assert!(
        matches!(error, Error::Timeout { timeout, .. } if timeout == duration),
        "{error}"
    );
    assert!(
        start.elapsed() >= duration,
        "must wait for the configured deadline"
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn waits_for_the_frame_document_to_load() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("const frame = document.createElement('iframe'); frame.id = 'late-frame'; frame.src = '/slow-frame.html'; document.querySelector('#host').append(frame)").await.expect("insert slow frame");
    let locator = page.frame_locator("#late-frame").locator("#value");
    tokio::time::timeout(Duration::from_secs(2), locator.fill("loaded"))
        .await
        .expect("fill deadline")
        .expect("wait for document and fill");
    let frame = page
        .frame_locator("#late-frame")
        .resolve()
        .await
        .expect("resolve loaded frame");
    assert_eq!(
        frame
            .evaluate("document.querySelector('#value').value")
            .await
            .expect("read value"),
        json!("loaded")
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn frame_wait_propagates_closed_page_errors_without_waiting_for_timeout() {
    let (_fixture, browser, page) = open().await;
    let locator = page.frame_locator("#missing-frame").locator("#value");
    page.close().await.expect("close page");
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        locator.wait_for_with_timeout(WaitState::Visible, Duration::from_secs(10)),
    )
    .await
    .expect("closed page must fail promptly")
    .expect_err("closed page error");
    assert!(error.is_closed(), "{error}");
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn waiting_locator_survives_frame_navigation() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("const frame = document.createElement('iframe'); frame.id = 'late-frame'; frame.src = '/index.html'; document.querySelector('#host').append(frame)")
        .await.expect("insert frame without input");
    let frame_locator = page.frame_locator("#late-frame");
    frame_locator
        .locator("#host")
        .wait_for_with_timeout(WaitState::Attached, Duration::from_secs(1))
        .await
        .expect("initial document ready");
    page.evaluate(
        "setTimeout(() => document.querySelector('#late-frame').src = '/frame.html', 120)",
    )
    .await
    .expect("schedule navigation");
    tokio::time::timeout(
        Duration::from_secs(2),
        frame_locator.locator("#value").fill("after-navigation"),
    )
    .await
    .expect("fill watchdog")
    .expect("retry after execution context replacement");
    let frame = frame_locator.resolve().await.expect("resolve frame");
    assert_eq!(
        frame
            .evaluate("document.querySelector('#value').value")
            .await
            .expect("read value"),
        json!("after-navigation")
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn frame_wait_preserves_javascript_errors() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("scheduleFrame(0)")
        .await
        .expect("schedule frame");
    let frame_locator = page.frame_locator("#late-frame");
    frame_locator
        .locator("#value")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(1))
        .await
        .expect("frame ready");
    let frame = frame_locator.resolve().await.expect("resolve frame");
    frame
        .evaluate("window.__rustwright.waitFor = () => { throw new Error('fixture failure'); }")
        .await
        .expect("inject error");
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        frame_locator
            .locator("#value")
            .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(10)),
    )
    .await
    .expect("error must propagate promptly")
    .expect_err("JavaScript error expected");
    assert!(
        matches!(error, Error::JavaScript(ref message) if message.contains("fixture failure")),
        "{error}"
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn frame_wait_deadline_also_bounds_the_child_element_wait() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("setTimeout(() => { const frame = document.createElement('iframe'); frame.id = 'late-frame'; frame.src = '/delayed-child.html'; document.querySelector('#host').append(frame); }, 70)")
        .await
        .expect("schedule frame");
    let duration = Duration::from_millis(180);
    let start = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_millis(500),
        page.frame_locator("#late-frame")
            .locator("#late-child")
            .wait_for_with_timeout(WaitState::Visible, duration),
    )
    .await
    .expect("frame resolution must not restart the deadline")
    .expect_err("missing child must time out");
    assert!(
        matches!(error, Error::Timeout { timeout, .. } if timeout == duration),
        "{error}"
    );
    assert!(start.elapsed() >= duration);
    // The child eventually exists, but must not satisfy the first wait by
    // granting a new full timeout after resolving the late iframe.
    page.frame_locator("#late-frame")
        .locator("#late-child")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(1))
        .await
        .expect("child eventually appears");
    browser.close().await.expect("close browser");
}

async fn click_case(kind: &str) {
    let (_fixture, browser, page) = open().await;
    page.evaluate(&format!("prepareActionability({kind:?}, 220)"))
        .await
        .expect("prepare click scenario");
    tokio::time::timeout(Duration::from_secs(2), page.locator("#target").click())
        .await
        .expect("click watchdog")
        .expect("click actionable element");
    let state = page.evaluate("({clicked: window.clicked, early: window.clickedBeforeReady, wrong: window.wrongClicks})")
        .await.expect("read actual click result");
    assert_eq!(
        state,
        json!({"clicked": true, "early": false, "wrong": 0}),
        "{kind}"
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_waits_until_button_is_enabled() {
    click_case("disabled").await;
}

#[tokio::test]
async fn click_waits_until_parent_fieldset_is_enabled() {
    click_case("fieldset").await;
}

#[tokio::test]
async fn click_waits_until_aria_disabled_ancestor_is_enabled() {
    click_case("aria").await;
}

#[tokio::test]
async fn click_waits_until_overlay_disappears() {
    click_case("covered").await;
}

#[tokio::test]
async fn click_waits_until_button_stops_moving() {
    click_case("moving").await;
}

#[tokio::test]
async fn click_rechecks_overlay_created_on_hover() {
    click_case("hover_cover").await;
}

#[tokio::test]
async fn click_reresolves_target_replaced_on_hover() {
    click_case("hover_replace").await;
}

#[tokio::test]
async fn click_timeout_reports_disabled_and_intercepted_targets_without_clicking() {
    let (_fixture, browser, page) = open().await;
    for (kind, reason) in [
        ("disabled", "disabled"),
        ("covered", "intercepts pointer events"),
    ] {
        page.evaluate(&format!("prepareActionability({kind:?}, 10000)"))
            .await
            .expect("prepare blocked button");
        let duration = Duration::from_millis(150);
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            page.locator("#target").click_with_timeout(duration),
        )
        .await
        .expect("short click deadline")
        .expect_err("blocked click must fail");
        assert!(
            matches!(error, Error::Timeout { timeout, .. } if timeout == duration),
            "{error}"
        );
        assert!(error.to_string().contains(reason), "{error}");
        assert_eq!(
            page.evaluate("window.clicked || window.wrongClicks > 0")
                .await
                .expect("read events"),
            json!(false)
        );
        // A timed-out remote preparation must not click after the caller returns.
        page.evaluate("document.querySelector('#target').disabled = false; document.querySelector('#cover')?.remove()")
            .await.expect("unblock after timeout");
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(
            page.evaluate("window.clicked || window.wrongClicks > 0")
                .await
                .expect("read late events"),
            json!(false)
        );
    }
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_deadline_includes_a_missing_frame() {
    let (_fixture, browser, page) = open().await;
    let duration = Duration::from_millis(120);
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        page.frame_locator("#missing")
            .locator("button")
            .click_with_timeout(duration),
    )
    .await
    .expect("click watchdog")
    .expect_err("missing frame deadline");
    assert!(
        matches!(error, Error::Timeout { timeout, .. } if timeout == duration),
        "{error}"
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_preserves_javascript_and_closed_page_errors() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("prepareActionability('ready', 0); window.__rustwright.prepareClick = () => { throw new Error('fixture click failure'); }")
        .await.expect("inject error");
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        page.locator("#target")
            .click_with_timeout(Duration::from_secs(10)),
    )
    .await
    .expect("JavaScript error should be prompt")
    .expect_err("JavaScript failure");
    assert!(
        matches!(error, Error::JavaScript(ref message) if message.contains("fixture click failure")),
        "{error}"
    );
    page.close().await.expect("close page");
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        page.locator("#target")
            .click_with_timeout(Duration::from_secs(10)),
    )
    .await
    .expect("closed page should be prompt")
    .expect_err("closed page");
    assert!(error.is_closed(), "{error}");
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_does_not_intercept_an_overlay_above_the_iframe() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("scheduleFrame(0)")
        .await
        .expect("insert frame");
    let frame = page.frame_locator("#late-frame");
    frame
        .locator("#value")
        .fill("safe")
        .await
        .expect("fill frame");
    page.evaluate("window.wrongClicks = 0; const cover = document.createElement('div'); cover.style.cssText = 'position:fixed;inset:0;z-index:100'; cover.onclick = () => window.wrongClicks++; document.body.append(cover); setTimeout(() => cover.remove(), 200)")
        .await.expect("cover iframe in parent");
    frame
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("wait for parent overlay");
    assert_eq!(
        page.evaluate("window.wrongClicks")
            .await
            .expect("wrong clicks"),
        json!(0)
    );
    tokio::time::timeout(Duration::from_secs(1), page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', resolve, {once: true}); })"))
        .await.expect("message watchdog").expect("submitted value");
    assert_eq!(
        page.evaluate("window.frameValue")
            .await
            .expect("submission"),
        json!("safe")
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_accepts_descendants_and_native_fieldset_legend_exceptions() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("document.querySelector('#host').innerHTML = '<fieldset disabled><legend><button id=target><span>Allowed</span></button></legend></fieldset>'; window.clicked = false; document.querySelector('#target').onclick = () => window.clicked = true")
        .await.expect("legend fixture");
    assert!(page
        .locator("#target")
        .is_enabled()
        .await
        .expect("legend enabled"));
    page.locator("#target")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("click first legend descendant");
    assert_eq!(
        page.evaluate("window.clicked").await.expect("clicked"),
        json!(true)
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_scrolls_an_offscreen_target_into_view() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("document.querySelector('#host').innerHTML = '<div style=height:3000px></div><button id=target>Offscreen</button>'; window.clicked = false; document.querySelector('#target').onclick = () => window.clicked = true")
        .await.expect("offscreen fixture");
    page.locator("#target")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("scroll and click");
    assert_eq!(
        page.evaluate("window.clicked").await.expect("clicked"),
        json!(true)
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_handles_a_one_pixel_target() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("document.querySelector('#host').innerHTML = '<div id=target style=\"position:fixed;left:80px;top:80px;width:1px;height:1px\"></div>'; window.clicked = false; document.querySelector('#target').onclick = () => window.clicked = true")
        .await.expect("small fixture");
    page.locator("#target")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("click thin target");
    assert_eq!(
        page.evaluate("window.clicked").await.expect("clicked"),
        json!(true)
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_deadline_bounds_a_stalled_preparation_promise() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("prepareActionability('ready', 0); window.__rustwright.prepareClick = () => new Promise(() => {})")
        .await.expect("stall preparation");
    let duration = Duration::from_millis(120);
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        page.locator("#target").click_with_timeout(duration),
    )
    .await
    .expect("stalled preparation deadline")
    .expect_err("must time out");
    assert!(
        matches!(error, Error::Timeout { timeout, .. } if timeout == duration),
        "{error}"
    );
    assert_eq!(
        page.evaluate("window.clicked")
            .await
            .expect("page remains usable"),
        json!(false)
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn checkbox_actions_wait_for_the_same_click_checks() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("document.querySelector('#host').innerHTML = '<fieldset disabled><input id=target type=checkbox></fieldset>'; setTimeout(() => document.querySelector('fieldset').disabled = false, 120)")
        .await.expect("disabled checkbox");
    let checkbox = page.locator("#target");
    assert!(!checkbox
        .is_enabled()
        .await
        .expect("fieldset disables control"));
    tokio::time::timeout(Duration::from_secs(1), checkbox.check())
        .await
        .expect("check watchdog")
        .expect("check");
    assert!(checkbox.is_checked().await.expect("checked"));
    checkbox.uncheck().await.expect("uncheck");
    assert!(!checkbox.is_checked().await.expect("unchecked"));
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn click_maps_an_offscreen_iframe_to_root_viewport_coordinates() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("const frame = document.createElement('iframe'); frame.id = 'late-frame'; frame.src = '/frame.html'; frame.style.marginTop = '3000px'; document.querySelector('#host').append(frame)")
        .await.expect("offscreen frame");
    let locator = page.frame_locator("#late-frame");
    locator
        .locator("#value")
        .wait_for_with_timeout(WaitState::Attached, Duration::from_secs(1))
        .await
        .expect("frame loaded");
    locator
        .resolve()
        .await
        .expect("resolve frame")
        .evaluate("document.querySelector('#value').value = 'offscreen-frame'")
        .await
        .expect("set frame input");
    locator
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("scroll frame and click");
    tokio::time::timeout(Duration::from_secs(1), page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', resolve, {once: true}); })"))
        .await.expect("message watchdog").expect("submission");
    assert_eq!(
        page.evaluate("window.frameValue").await.expect("submitted"),
        json!("offscreen-frame")
    );
    browser.close().await.expect("close browser");
}

async fn geometry_case(kind: &str) {
    let (_fixture, browser, page) = open().await;
    page.evaluate(&format!("prepareGeometry({kind:?})"))
        .await
        .expect("prepare geometry");
    page.locator("#target")
        .click_with_timeout(Duration::from_millis(600))
        .await
        .expect("choose a receiving point");
    assert_eq!(
        page.evaluate("window.clicked && window.wrongClicks === 0")
            .await
            .expect("click result"),
        json!(true),
        "{kind}"
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn geometry_click_finds_an_uncovered_point_away_from_the_center() {
    geometry_case("partial_cover").await;
}

#[tokio::test]
async fn geometry_click_handles_overflow_clipping() {
    geometry_case("clipped").await;
}

#[tokio::test]
async fn geometry_click_handles_a_rotated_element_clipped_by_the_viewport() {
    geometry_case("rotated_clipped").await;
}

#[tokio::test]
async fn geometry_click_handles_perspective_transforms() {
    geometry_case("perspective").await;
}

#[tokio::test]
async fn geometry_click_selects_an_uncovered_point_on_a_rotated_target() {
    geometry_case("rotated_partial_cover").await;
}

#[tokio::test]
async fn geometry_click_handles_a_clip_path_with_an_occluded_center() {
    geometry_case("clip_path").await;
}

#[tokio::test]
async fn geometry_click_uses_an_uncovered_inline_fragment() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("const host = document.querySelector('#host'); host.style.cssText = 'position:fixed;left:80px;top:100px;width:130px;font:16px/24px Arial'; host.innerHTML = '<a id=target href=\"#\">First line of a link that wraps onto several lines for geometry</a>'; window.clicked = false; window.wrongClicks = 0; document.querySelector('#target').onclick = e => { e.preventDefault(); window.clicked = true; window.clickY = e.clientY; }; const cover = document.createElement('div'); cover.style.cssText = 'position:fixed;left:80px;top:100px;width:130px;height:24px;z-index:100'; cover.onclick = () => window.wrongClicks++; host.append(cover)")
        .await.expect("multiline fixture");
    page.locator("#target")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("click another fragment");
    assert_eq!(
        page.evaluate("window.clicked && window.wrongClicks === 0 && window.clickY >= 124")
            .await
            .expect("fragment click"),
        json!(true)
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn geometry_click_handles_a_transformed_iframe() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("const frame = document.createElement('iframe'); frame.id = 'late-frame'; frame.src = '/frame.html'; frame.style.transform = 'rotate(20deg) skewX(10deg) scale(.8)'; document.querySelector('#host').append(frame)")
        .await.expect("transformed iframe");
    let frame = page.frame_locator("#late-frame");
    frame
        .locator("#value")
        .fill("transformed-frame")
        .await
        .expect("fill frame");
    frame
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("click transformed frame");
    tokio::time::timeout(Duration::from_secs(1), page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', resolve, {once: true}); })"))
        .await.expect("message watchdog").expect("submitted");
    assert_eq!(
        page.evaluate("window.frameValue").await.expect("value"),
        json!("transformed-frame")
    );
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn geometry_click_waits_for_transform_changes_with_identical_bounding_rects() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("prepareActionability('ready', 0); const button = document.querySelector('#target'); button.style.width = '80px'; button.style.height = '80px'; button.style.transform = 'rotate(45deg)'; window.actionReady = false; window.sameBounds = true; let previous = JSON.stringify(button.getBoundingClientRect().toJSON()); let n = 0; const stop = performance.now() + 150; function tick(now) { button.style.transform = n++ % 2 ? 'rotate(-45deg)' : 'rotate(45deg)'; const rect = JSON.stringify(button.getBoundingClientRect().toJSON()); window.sameBounds = window.sameBounds && previous === rect; previous = rect; if (now < stop) requestAnimationFrame(tick); else window.actionReady = true; } requestAnimationFrame(tick)")
        .await.expect("alternating transforms");
    page.locator("#target")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect("wait for stable transform");
    assert_eq!(
        page.evaluate("window.sameBounds && window.clicked && !window.clickedBeforeReady")
            .await
            .expect("stable transformed click"),
        json!(true)
    );
    browser.close().await.expect("close browser");
}

async fn submit_cross_frame(page: &Page, url: &str) {
    page.evaluate(&format!("scheduleFrame(120, {url:?})"))
        .await
        .expect("schedule cross-origin frame");
    let frame = page.frame_locator("#late-frame");
    tokio::time::timeout(Duration::from_secs(2), async {
        frame.locator("#value").wait_for_with_timeout(WaitState::Visible, Duration::from_millis(700)).await?;
        frame.locator("#value").fill("cross-origin").await?;
        frame.locator("#submit").click_with_timeout(Duration::from_millis(700)).await?;
        page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', resolve, {once: true}); })").await?;
        Result::Ok(())
    }).await.expect("cross-frame watchdog").expect("fill and submit cross-origin frame");
    assert_eq!(
        page.evaluate("window.frameValue")
            .await
            .expect("submitted value"),
        json!("cross-origin")
    );
    let resolved = frame.resolve().await.expect("resolve frame");
    assert_ne!(
        resolved
            .evaluate("location.origin")
            .await
            .expect("child origin"),
        page.evaluate("location.origin")
            .await
            .expect("parent origin")
    );
    assert_eq!(
        json!(resolved.url()),
        page.evaluate("document.querySelector('#late-frame').src")
            .await
            .expect("iframe URL")
    );
    assert_eq!(page.main_frame().url(), page.url());
}

#[tokio::test]
async fn cross_origin_frame_with_another_port_fills_and_submits() {
    let (_fixture, browser, page) = open().await;
    let child = fixture().await;
    submit_cross_frame(&page, &format!("{}/frame.html", child.base)).await;
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn cross_site_frame_fills_and_submits() {
    let (fixture, browser, page) = open().await;
    let child = fixture.base.replace("127.0.0.1", "localhost");
    submit_cross_frame(&page, &format!("{child}/frame.html")).await;
    browser.close().await.expect("close browser");
}

#[tokio::test]
async fn cross_site_frame_wait_survives_a_process_swap() {
    let (fixture, browser, page) = open().await;
    let url = format!(
        "{}/delayed-child.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    page.evaluate(&format!("scheduleFrame(0); setTimeout(() => document.querySelector('#late-frame').src = {url:?}, 80)"))
        .await.expect("schedule cross-site navigation");
    page.frame_locator("#late-frame")
        .locator("#late-child")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(1))
        .await
        .expect("wait through process swap");
    browser.close().await.expect("close browser");
}

async fn assert_remote_frame(browser: &Browser, frame: &Frame) {
    let connection =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
            .await
            .expect("connect diagnostic session");
    let targets = connection
        .send_raw(None, "Target.getTargets", json!({}))
        .await
        .expect("list targets");
    assert!(
        targets["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["type"] == "iframe" && target["targetId"] == frame.frame_id()),
        "must exercise an actual OOPIF: {targets}"
    );
    connection.close();
}

#[tokio::test]
async fn cross_site_click_checks_parent_overlays_and_transformed_scrolled_coordinates() {
    let (fixture, browser, page) = open().await;
    let url = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    page.evaluate(&format!("scheduleFrame(0, {url:?}); setTimeout(() => {{ const f = document.querySelector('#late-frame'); f.style.marginTop = '2000px'; f.style.transform = 'perspective(700px) rotateY(15deg) rotate(12deg) scale(.8)'; }}, 0)"))
        .await.expect("prepare remote frame");
    let frame = page.frame_locator("#late-frame");
    frame
        .locator("#value")
        .fill("transformed-remote")
        .await
        .expect("fill remote input");
    assert_remote_frame(&browser, &frame.resolve().await.unwrap()).await;
    page.evaluate("window.wrongClicks = 0; const cover = document.createElement('div'); cover.id = 'remote-cover'; cover.style.cssText = 'position:fixed;inset:0;z-index:999'; cover.onclick = () => window.wrongClicks++; document.body.append(cover)")
        .await.expect("parent overlay");
    let error = frame
        .locator("#submit")
        .click_with_timeout(Duration::from_millis(180))
        .await
        .expect_err("parent cover blocks input");
    assert!(error.is_timeout(), "{error}");
    assert_eq!(
        page.evaluate("[window.frameValue, window.wrongClicks]")
            .await
            .unwrap(),
        json!([null, 0])
    );
    page.evaluate("document.querySelector('#remote-cover').remove()")
        .await
        .unwrap();
    frame
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(2))
        .await
        .expect("mapped click");
    page.evaluate("new Promise(resolve => { if (frameValue !== null) resolve(); else addEventListener('message', resolve, {once:true}); })").await.unwrap();
    assert_eq!(
        page.evaluate("window.frameValue").await.unwrap(),
        json!("transformed-remote")
    );
    assert_eq!(page.evaluate("scrollY > 0").await.unwrap(), json!(true));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_frame_keeps_working_after_reload_and_swaps_back_to_the_parent_site() {
    let (fixture, browser, page) = open().await;
    let remote = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &remote).await;
    let locator = page.frame_locator("#late-frame");
    let frame = locator.resolve().await.unwrap();
    assert_remote_frame(&browser, &frame).await;
    frame
        .evaluate("location.reload()")
        .await
        .expect("reload remote frame");
    locator
        .locator("#value")
        .fill("reloaded")
        .await
        .expect("fill after reload");
    assert_eq!(
        locator
            .resolve()
            .await
            .unwrap()
            .evaluate("document.querySelector('#value').value")
            .await
            .unwrap(),
        json!("reloaded")
    );
    let local = format!("{}/delayed-child.html", fixture.base);
    page.evaluate(&format!(
        "document.querySelector('#late-frame').src = {local:?}"
    ))
    .await
    .unwrap();
    locator
        .locator("#late-child")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(2))
        .await
        .expect("swap back to parent renderer");
    locator
        .locator("#late-child")
        .fill("local-again")
        .await
        .unwrap();
    assert_eq!(
        locator
            .resolve()
            .await
            .unwrap()
            .evaluate("document.querySelector('#late-child').value")
            .await
            .unwrap(),
        json!("local-again")
    );
    assert_eq!(
        page.main_frame().url(),
        format!("{}/index.html", fixture.base)
    );
    assert_eq!(page.url(), page.main_frame().url());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_sibling_sessions_do_not_mix_contexts() {
    let (fixture, browser, page) = open().await;
    let first = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    let second = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "second.localhost")
    );
    page.evaluate(&format!("scheduleFrame(0, {first:?}); const second = document.createElement('iframe'); second.id = 'second-frame'; second.src = {second:?}; document.querySelector('#host').append(second)"))
        .await.unwrap();
    let left = page.frame_locator("#late-frame");
    let right = page.frame_locator("#second-frame");
    tokio::time::timeout(Duration::from_secs(3), async {
        let first_input = left.locator("#value");
        let second_input = right.locator("#value");
        tokio::try_join!(first_input.fill("first"), second_input.fill("second"))
    })
    .await
    .expect("siblings watchdog")
    .expect("fill sibling contexts");
    for (locator, value) in [(&left, "first"), (&right, "second")] {
        let frame = locator.resolve().await.unwrap();
        assert_remote_frame(&browser, &frame).await;
        assert_eq!(
            frame
                .evaluate("document.querySelector('#value').value")
                .await
                .unwrap(),
            json!(value)
        );
    }
    left.resolve()
        .await
        .unwrap()
        .evaluate("location.reload()")
        .await
        .unwrap();
    right
        .locator("#value")
        .fill("unaffected")
        .await
        .expect("other session survives context destruction");
    assert_eq!(
        right
            .resolve()
            .await
            .unwrap()
            .evaluate("document.querySelector('#value').value")
            .await
            .unwrap(),
        json!("unaffected")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_nested_renderer_click_maps_both_frame_boundaries() {
    let (fixture, browser, page) = open().await;
    let outer_url = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &outer_url).await;
    let outer = page.frame_locator("#late-frame").resolve().await.unwrap();
    assert_remote_frame(&browser, &outer).await;
    let inner_url = format!("{}/frame.html", fixture.base);
    outer.evaluate(&format!("const inner = document.createElement('iframe'); inner.name = 'nested-remote'; const u = new URL({inner_url:?}); u.searchParams.set('parentOrigin', location.origin); inner.src = u.href; inner.style.cssText = 'width:350px;height:120px;transform:rotate(-8deg);margin:25px'; document.body.append(inner); window.nestedValue = null; addEventListener('message', e => {{ if (e.source === inner.contentWindow && e.origin === u.origin && e.data?.kind === 'submitted') window.nestedValue = e.data.value; }})"))
        .await.unwrap();
    page.evaluate("document.querySelector('#late-frame').style.cssText = 'width:500px;height:350px;margin:80px;transform:rotate(12deg)'").await.unwrap();
    let inner = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(frame) = page
                .frames()
                .into_iter()
                .find(|frame| frame.name() == "nested-remote")
            {
                frame
                    .locator("#value")
                    .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(1))
                    .await
                    .unwrap();
                break frame;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("discover nested frame");
    assert_remote_frame(&browser, &inner).await;
    inner
        .locator("#value")
        .fill("nested-payload")
        .await
        .unwrap();
    inner
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(2))
        .await
        .expect("nested mapped click");
    tokio::time::timeout(Duration::from_secs(1), outer.evaluate("new Promise(resolve => { if (nestedValue !== null) resolve(); else addEventListener('message',resolve,{once:true}); })")).await.unwrap().unwrap();
    assert_eq!(
        outer.evaluate("window.nestedValue").await.unwrap(),
        json!("nested-payload")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_frame_removal_cleans_up_and_a_new_frame_can_be_resolved() {
    let (fixture, browser, page) = open().await;
    let remote = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &remote).await;
    let old = page.frame_locator("#late-frame").resolve().await.unwrap();
    assert_remote_frame(&browser, &old).await;
    page.evaluate("document.querySelector('#late-frame').remove()")
        .await
        .unwrap();
    page.frame_locator("#late-frame")
        .locator("#value")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(120))
        .await
        .expect_err("removed frame times out");
    assert!(page
        .frames()
        .iter()
        .all(|frame| frame.frame_id() != old.frame_id()));
    page.evaluate("window.frameValue = null").await.unwrap();
    submit_cross_frame(&page, &remote).await;
    assert_ne!(
        page.frame_locator("#late-frame")
            .resolve()
            .await
            .unwrap()
            .frame_id(),
        old.frame_id()
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_already_loaded_frame_is_initialized_when_connecting() {
    let (fixture, browser, page) = open().await;
    let remote = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &remote).await;
    let connected = Browser::connect(&browser.version().web_socket_debugger_url)
        .await
        .unwrap();
    let adopted = connected
        .pages()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.target_id() == page.target_id())
        .expect("adopt original page");
    let frame = adopted.frame_locator("#late-frame");
    frame
        .locator("#value")
        .fill("adopted")
        .await
        .expect("helper injected into existing child");
    assert_eq!(
        frame
            .resolve()
            .await
            .unwrap()
            .evaluate("document.querySelector('#value').value")
            .await
            .unwrap(),
        json!("adopted")
    );
    assert!(frame.resolve().await.unwrap().url().starts_with(&remote));
    assert_eq!(adopted.url(), page.url());
    connected.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_isolated_worlds_do_not_replace_default_context_and_errors_propagate() {
    let (fixture, browser, page) = open().await;
    let remote = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &remote).await;
    let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
    let connection =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
            .await
            .unwrap();
    let attached = connection
        .send_raw(
            None,
            "Target.attachToTarget",
            json!({"targetId":frame.frame_id(),"flatten":true}),
        )
        .await
        .unwrap();
    let session = attached["sessionId"].as_str().unwrap();
    let isolated = connection
        .send_raw(
            Some(session),
            "Page.createIsolatedWorld",
            json!({"frameId":frame.frame_id(),"worldName":"regression-isolated"}),
        )
        .await
        .unwrap();
    let context = isolated["executionContextId"].as_i64().unwrap();
    let helper = connection.send_raw(Some(session),"Runtime.evaluate",json!({"contextId":context,"expression":"typeof window.__rustwright","returnByValue":true})).await.unwrap();
    assert_eq!(helper["result"]["value"], json!("undefined"));
    frame
        .locator("#value")
        .fill("default-world")
        .await
        .expect("keep the default context");
    assert_eq!(
        frame
            .evaluate("document.querySelector('#value').value")
            .await
            .unwrap(),
        json!("default-world")
    );
    frame
        .evaluate(
            "window.__rustwright.prepareClick = () => { throw new Error('remote-click-error'); }",
        )
        .await
        .unwrap();
    let error = frame
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(1))
        .await
        .expect_err("real child JS error");
    assert!(
        matches!(error,Error::JavaScript(ref message) if message.contains("remote-click-error")),
        "{error}"
    );
    frame
        .evaluate("window.__rustwright.prepareClick = () => new Promise(() => {})")
        .await
        .unwrap();
    let duration = Duration::from_millis(120);
    let start = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        frame.locator("#submit").click_with_timeout(duration),
    )
    .await
    .unwrap()
    .expect_err("child preparation deadline");
    assert!(
        matches!(error,Error::Timeout{timeout,..} if timeout==duration),
        "{error}"
    );
    assert!(start.elapsed() < Duration::from_millis(600));
    connection.close();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_click_retries_when_its_renderer_detaches_during_preparation() {
    let (fixture, browser, page) = open().await;
    let remote = format!(
        "{}/frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    submit_cross_frame(&page, &remote).await;
    page.evaluate("window.frameValue = null; addEventListener('message', e => { if (e.data?.kind === 'preparing' && e.source === document.querySelector('#late-frame').contentWindow) { window.swappedDuringClick = true; window.expectedFrameOrigin = location.origin; document.querySelector('#late-frame').src = '/frame.html'; } });").await.unwrap();
    let locator = page.frame_locator("#late-frame");
    locator.resolve().await.unwrap().evaluate("window.__rustwright.prepareClick = function () { parent.postMessage({kind:'preparing'},new URL(location.href).searchParams.get('parentOrigin')); return new Promise(() => {}); }").await.unwrap();
    locator
        .locator("#submit")
        .click_with_timeout(Duration::from_secs(2))
        .await
        .expect("retry a detached pre-input session");
    tokio::time::timeout(Duration::from_secs(1),page.evaluate("new Promise(resolve => { if (frameValue !== null) resolve(); else addEventListener('message',resolve,{once:true}); })")).await.unwrap().unwrap();
    assert_eq!(
        page.evaluate("[window.swappedDuringClick, window.frameValue]")
            .await
            .unwrap(),
        json!([true, ""])
    );
    browser.close().await.unwrap();
}

async fn network_evaluate(frame: &Frame, expression: &str) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(3), frame.evaluate(expression))
        .await
        .expect("network watchdog")
        .expect("frame network evaluation")
}

async fn recorded_requests(
    page: &Page,
    url: &str,
    count: usize,
) -> Vec<rustwright::NetworkRequest> {
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let requests: Vec<_> = page
                .network_requests()
                .into_iter()
                .filter(|request| request.url == url && request.finished.is_some())
                .collect();
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| {
        panic!(
            "missing completed requests for {url}: {:?}",
            page.network_requests()
        )
    })
}

#[tokio::test]
async fn cross_site_network_diagnostics_capture_initial_request_and_response_body() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("network")
    );
    let url = format!(
        "{}/api/network",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    let requests = recorded_requests(&page, &url, 1).await;
    assert_eq!(
        requests.len(),
        1,
        "do not duplicate requests across sessions"
    );
    assert_eq!(requests[0].status, Some(200));
    assert!(requests[0].failure.is_none());
    let har = page.har_with_bodies().await.unwrap();
    let entries: Vec<_> = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["request"]["url"] == url)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["response"]["content"]["text"], "network");
    let document = page
        .network_requests()
        .into_iter()
        .find(|request| {
            request.url.contains("localhost") && request.url.contains("/network-frame.html")
        })
        .unwrap();
    assert!(
        document.finished.is_some(),
        "document completion must transfer to the child session: {document:?}"
    );
    assert!(document.failure.is_none());
    let documents: Vec<_> = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["request"]["url"] == document.url)
        .collect();
    assert_eq!(documents.len(), 1);
    assert!(documents[0]["response"]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("window.initialResponse"));
    browser.close().await.unwrap();
}

async fn assert_har_body(page: &Page, url: &str, body: &str) {
    recorded_requests(page, url, 1).await;
    let har = tokio::time::timeout(Duration::from_secs(3), page.har_with_bodies())
        .await
        .unwrap()
        .unwrap();
    let entries: Vec<_> = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["request"]["url"] == url)
        .collect();
    assert_eq!(entries.len(), 1, "exactly one HAR entry for {url}");
    assert_eq!(
        entries[0]["response"]["content"]["text"], body,
        "body for {url}"
    );
}

#[tokio::test]
async fn cross_site_network_diagnostics_keep_concurrent_parent_and_child_bodies_separate() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    network_evaluate(&frame, "window.initialResponse").await;
    let expression = "fetch('/api/request-headers').then(r => r.text())";
    let (parent, child) = tokio::join!(
        page.evaluate(expression),
        network_evaluate(&frame, expression)
    );
    let parent = parent.unwrap();
    assert_ne!(parent, child, "the Host headers differ between origins");
    assert_har_body(
        &page,
        &format!("{}/api/request-headers", fixture.base),
        parent.as_str().unwrap(),
    )
    .await;
    assert_har_body(
        &page,
        &format!(
            "{}/api/request-headers",
            fixture.base.replace("127.0.0.1", "localhost")
        ),
        child.as_str().unwrap(),
    )
    .await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_diagnostics_follow_reloads_and_repeated_process_swaps() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    network_evaluate(&frame, "window.initialResponse").await;
    let remote = fixture.base.replace("127.0.0.1", "localhost");
    for (stage, base) in [&fixture.base, &remote, &fixture.base, &remote]
        .into_iter()
        .enumerate()
    {
        let destination =
            format!("{base}/network-frame.html?api=%2Fapi%2Fnetwork%3Fstage%3D{stage}");
        let frame = navigate_network_child(&page, &destination).await;
        if base == &remote {
            assert_remote_frame(&browser, &frame).await;
        }
        assert_eq!(
            network_evaluate(&frame, "window.initialResponse").await,
            "network"
        );
        assert_har_body(
            &page,
            &format!("{base}/api/network?stage={stage}"),
            "network",
        )
        .await;
    }
    let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
    network_evaluate(&frame, "location.reload(); true").await;
    page.frame_locator("#late-frame")
        .locator("#value")
        .wait_for(WaitState::Visible)
        .await
        .unwrap();
    let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        "network"
    );
    let requests = recorded_requests(&page, &format!("{remote}/api/network?stage=3"), 2).await;
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.status == Some(200) && request.failure.is_none()));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_diagnostics_capture_nested_renderer_bodies() {
    let (fixture, browser, page) = open().await;
    let outer = open_network_frame(&fixture, &browser, &page).await;
    network_evaluate(&outer, "window.initialResponse").await;
    let url = format!(
        "{}/network-frame.html?api=%2Fapi%2Fnetwork%3Fnested%3D1",
        fixture.base
    );
    network_evaluate(&outer, &format!("new Promise(resolve => {{ const f = document.createElement('iframe'); f.id = 'nested-network'; f.onload = () => resolve(true); f.src = {url:?}; document.body.append(f); }})")).await;
    let inner = page
        .frames()
        .into_iter()
        .find(|frame| frame.url() == url)
        .expect("nested frame");
    assert_remote_frame(&browser, &inner).await;
    assert_eq!(
        network_evaluate(&inner, "window.initialResponse").await,
        "network"
    );
    assert_har_body(
        &page,
        &format!("{}/api/network?nested=1", fixture.base),
        "network",
    )
    .await;
    assert_har_body(
        &page,
        &format!(
            "{}/api/network",
            fixture.base.replace("127.0.0.1", "localhost")
        ),
        "network",
    )
    .await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_diagnostics_capture_mocked_responses_and_blocked_failures() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 201, "text/plain", "mock-body")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        "mock-body"
    );
    let remote = fixture.base.replace("127.0.0.1", "localhost");
    let url = format!("{remote}/api/network");
    let requests = recorded_requests(&page, &url, 1).await;
    assert_eq!(requests[0].status, Some(201));
    assert_har_body(&page, &url, "mock-body").await;
    page.route(
        "/api/binary",
        RouteAction::Fulfill {
            status: 200,
            content_type: "application/octet-stream".into(),
            body: vec![0, 1, 2],
        },
    )
    .await
    .unwrap();
    assert_eq!(network_evaluate(&frame, "fetch('/api/binary').then(r => r.arrayBuffer()).then(b => Array.from(new Uint8Array(b)))").await, json!([0, 1, 2]));
    let binary_url = format!("{remote}/api/binary");
    assert_har_body(&page, &binary_url, "AAEC").await;
    let har = page.har_with_bodies().await.unwrap();
    let binary = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["request"]["url"] == binary_url)
        .unwrap();
    assert_eq!(binary["response"]["content"]["encoding"], "base64");
    page.clear_routes().await.unwrap();
    page.block("**/api/network?blocked").await.unwrap();
    assert_eq!(
        network_evaluate(
            &frame,
            "fetch('/api/network?blocked').then(() => false, () => true)"
        )
        .await,
        true
    );
    let blocked_url = format!("{url}?blocked");
    let requests = recorded_requests(&page, &blocked_url, 1).await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].failure.is_some());
    let har = page.har();
    let blocked = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["request"]["url"] == blocked_url)
        .unwrap();
    assert_eq!(blocked["response"]["status"], 0);
    assert!(blocked["_error"].is_string());
    page.clear_routes().await.unwrap();
    assert_eq!(
        network_evaluate(
            &frame,
            "fetch('/api/network?recovered').then(r => r.text())"
        )
        .await,
        "network"
    );
    assert_har_body(&page, &format!("{url}?recovered"), "network").await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_diagnostics_retain_detached_requests_without_reusing_the_parent_session(
) {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    network_evaluate(&frame, "window.initialResponse").await;
    let remote_url = format!(
        "{}/api/network",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    recorded_requests(&page, &remote_url, 1).await;
    page.evaluate("document.querySelector('#late-frame').remove()")
        .await
        .unwrap();
    page.evaluate("fetch('/api/network?parent').then(r => r.text())")
        .await
        .unwrap();
    assert_har_body(
        &page,
        &format!("{}/api/network?parent", fixture.base),
        "network",
    )
    .await;
    let har = page.har_with_bodies().await.unwrap();
    let entry = har["log"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["request"]["url"] == remote_url)
        .unwrap();
    assert_eq!(entry["response"]["status"], 200);
    assert!(
        entry["response"]["content"].get("text").is_none(),
        "a detached session's body is unavailable"
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_existing_child_adds_clears_and_readds_mocks() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("network")
    );
    page.mock("/api/network", 202, "text/plain", "first")
        .await
        .unwrap();
    page.mock("/api/network", 203, "text/plain", "second")
        .await
        .unwrap();
    let expression = "fetch('/api/network').then(async r => [r.status, await r.text()])";
    assert_eq!(
        network_evaluate(&frame, expression).await,
        json!([202, "first"])
    );
    assert_eq!(
        page.evaluate(expression).await.unwrap(),
        json!([202, "first"])
    );
    // Unmatched requests continue while interception remains enabled.
    assert_eq!(
        network_evaluate(&frame, "fetch('/missing').then(r => r.status)").await,
        json!(404)
    );
    page.clear_routes().await.unwrap();
    assert_eq!(
        network_evaluate(&frame, expression).await,
        json!([200, "network"])
    );
    assert_eq!(
        page.evaluate(expression).await.unwrap(),
        json!([200, "network"])
    );
    page.clear_routes().await.unwrap();
    page.mock("/api/network", 201, "text/plain", "readded")
        .await
        .unwrap();
    assert_eq!(
        network_evaluate(&frame, expression).await,
        json!([201, "readded"])
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_block_is_inherited_and_clear_restores_requests() {
    let (fixture, browser, page) = open().await;
    page.block("/api/network").await.unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("blocked")
    );
    let expression = "fetch('/api/network').then(() => false, () => true)";
    assert_eq!(network_evaluate(&frame, expression).await, json!(true));
    assert_eq!(page.evaluate(expression).await.unwrap(), json!(true));
    page.clear_routes().await.unwrap();
    assert_eq!(
        network_evaluate(&frame, "fetch('/api/network').then(r => r.text())").await,
        json!("network")
    );
    // Add blocking after the child already exists as well.
    page.block("/api/network").await.unwrap();
    assert_eq!(network_evaluate(&frame, expression).await, json!(true));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_response_stage_upgrades_existing_and_new_children() {
    let (fixture, browser, page) = open().await;
    page.route("/passthrough", RouteAction::Continue)
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("network")
    );
    page.route(
        "/api/network",
        RouteAction::SetResponseHeaders(vec![
            ("X-Fixture".into(), "changed".into()),
            ("X-Added".into(), "new".into()),
        ]),
    )
    .await
    .unwrap();
    let expression = "fetch('/api/network').then(async r => [r.status, r.headers.get('x-fixture'), r.headers.get('x-added'), r.headers.get('content-type'), await r.text()])";
    let expected = json!([200, "changed", "new", "text/html; charset=utf-8", "network"]);
    assert_eq!(network_evaluate(&frame, expression).await, expected);
    assert_eq!(page.evaluate(expression).await.unwrap(), expected);
    // A second renderer must inherit response-stage interception before scripts.
    page.evaluate("document.querySelector('#late-frame').remove()")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(
            &frame,
            "window.initialResponse.then(() => window.initialHeader)"
        )
        .await,
        json!("changed")
    );
    page.clear_routes().await.unwrap();
    assert_eq!(network_evaluate(&frame, "fetch('/api/network').then(r => [r.headers.get('x-fixture'), r.headers.get('x-added')])").await, json!(["original", null]));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_request_headers_apply_to_child_without_losing_original_headers() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    page.route(
        "/api/request-headers",
        RouteAction::SetRequestHeaders(vec![("X-Test".into(), "override".into())]),
    )
    .await
    .unwrap();
    assert_eq!(network_evaluate(&frame, "fetch('/api/request-headers', {headers: {'X-Keep': 'kept'}}).then(r => r.text()).then(t => [t.toLowerCase().includes('x-test: override'), t.toLowerCase().includes('x-keep: kept')])").await, json!([true, true]));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_rules_cover_first_script_requests_after_process_swaps() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "persistent")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("persistent")
    );
    assert_eq!(fixture.network_requests.load(Ordering::SeqCst), 0);
    // Await load from the parent so reads cannot accidentally use the old context.
    for destination in [
        "/network-frame.html".to_string(),
        format!(
            "{}/network-frame.html",
            fixture.base.replace("127.0.0.1", "localhost")
        ),
    ] {
        page.evaluate(&format!("new Promise(resolve => {{ const f = document.querySelector('#late-frame'); f.onload = () => resolve(true); f.src = {destination:?}; }})")).await.unwrap();
        let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
        assert_eq!(
            network_evaluate(&frame, "window.initialResponse").await,
            json!("persistent"),
            "first inline-script request at {destination}"
        );
        assert_eq!(
            network_evaluate(&frame, "fetch('/api/network').then(r => r.text())").await,
            json!("persistent"),
            "destination: {destination}"
        );
    }
    page.evaluate("new Promise(resolve => { const f = document.querySelector('#late-frame'); f.onload = () => resolve(true); f.src = f.src; })").await.unwrap();
    let frame = page.frame_locator("#late-frame").resolve().await.unwrap();
    assert_remote_frame(&browser, &frame).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("persistent")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_nested_child_inherits_routes_and_clear_reaches_every_renderer() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "nested")
        .await
        .unwrap();
    let outer = open_network_frame(&fixture, &browser, &page).await;
    let url = format!("{}/network-frame.html", fixture.base);
    network_evaluate(&outer, &format!("new Promise(resolve => {{ const f = document.createElement('iframe'); f.id = 'nested-network'; f.onload = () => resolve(true); f.src = {url:?}; document.body.append(f); }})")).await;
    let inner = page
        .frames()
        .into_iter()
        .find(|f| f.url() == url)
        .expect("nested frame");
    assert_remote_frame(&browser, &inner).await;
    assert_eq!(
        network_evaluate(&inner, "window.initialResponse").await,
        json!("nested")
    );
    page.clear_routes().await.unwrap();
    for frame in [&outer, &inner] {
        assert_eq!(
            network_evaluate(frame, "fetch('/api/network').then(r => r.text())").await,
            json!("network")
        );
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_route_changes_and_child_attachment_are_serialized() {
    let (fixture, browser, page) = open().await;
    let (result, frame) = tokio::join!(
        page.mock("/api/network", 200, "text/plain", "concurrent"),
        open_network_frame(&fixture, &browser, &page)
    );
    result.unwrap();
    assert_eq!(
        network_evaluate(&frame, "fetch('/api/network').then(r => r.text())").await,
        json!("concurrent")
    );
    page.evaluate("document.querySelector('#late-frame').remove()")
        .await
        .unwrap();
    let (result, frame) = tokio::join!(
        page.clear_routes(),
        open_network_frame(&fixture, &browser, &page)
    );
    result.unwrap();
    assert_eq!(
        network_evaluate(&frame, "fetch('/api/network').then(r => r.text())").await,
        json!("network")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_mock_overrides_previously_cached_responses() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    let expression = "fetch('/api/cached').then(r => r.text())";
    for _ in 0..2 {
        assert_eq!(network_evaluate(&frame, expression).await, json!("network"));
        assert_eq!(page.evaluate(expression).await.unwrap(), json!("network"));
    }
    page.mock("/api/cached", 200, "text/plain", "uncached-mock")
        .await
        .unwrap();
    assert_eq!(
        network_evaluate(&frame, expression).await,
        json!("uncached-mock")
    );
    assert_eq!(
        page.evaluate(expression).await.unwrap(),
        json!("uncached-mock")
    );
    page.clear_routes().await.unwrap();
    assert_eq!(network_evaluate(&frame, expression).await, json!("network"));
    browser.close().await.unwrap();
}

async fn navigate_network_child(page: &Page, destination: &str) -> Frame {
    tokio::time::timeout(Duration::from_secs(3), page.evaluate(&format!("new Promise(resolve => {{ const frame = document.querySelector('#late-frame'); frame.onload = () => resolve(true); frame.src = {destination:?}; }})")))
        .await.expect("navigation watchdog").expect("navigate child");
    page.frame_locator("#late-frame").resolve().await.unwrap()
}

#[tokio::test]
async fn cross_site_network_gate_covers_fetch_and_xhr_on_repeated_returns_without_server_access() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "first-request")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("first-request")
    );
    for iteration in 0..10 {
        for path in ["/network-frame.html", "/xhr-frame.html"] {
            let frame = navigate_network_child(&page, path).await;
            assert_eq!(
                network_evaluate(&frame, "window.initialResponse").await,
                json!("first-request"),
                "iteration {iteration}, path {path}"
            );
            assert_eq!(network_evaluate(&frame, "[window.fetch.toString().includes('[native code]'), XMLHttpRequest.prototype.open.toString().includes('[native code]')]").await, json!([true, true]));
            let url = format!("{}{path}", fixture.base.replace("127.0.0.1", "localhost"));
            let frame = navigate_network_child(&page, &url).await;
            assert_remote_frame(&browser, &frame).await;
            assert_eq!(
                network_evaluate(&frame, "window.initialResponse").await,
                json!("first-request"),
                "iteration {iteration}, path {path}"
            );
        }
    }
    assert_eq!(
        fixture.network_requests.load(Ordering::SeqCst),
        0,
        "mocked requests must never reach the server"
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_gate_blocks_the_first_request_on_return_to_parent() {
    let (fixture, browser, page) = open().await;
    page.block("/api/network").await.unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("blocked")
    );
    for path in ["/network-frame.html", "/xhr-frame.html"] {
        let frame = navigate_network_child(&page, path).await;
        assert_eq!(
            network_evaluate(&frame, "window.initialResponse").await,
            json!("blocked")
        );
        let url = format!("{}{path}", fixture.base.replace("127.0.0.1", "localhost"));
        let frame = navigate_network_child(&page, &url).await;
        assert_eq!(
            network_evaluate(&frame, "window.initialResponse").await,
            json!("blocked")
        );
    }
    assert_eq!(
        fixture.network_requests.load(Ordering::SeqCst),
        0,
        "blocked requests must never reach the server"
    );
    page.clear_routes().await.unwrap();
    let frame = navigate_network_child(&page, "/network-frame.html").await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("network")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_gate_changes_the_first_response_header_on_return_to_parent() {
    let (fixture, browser, page) = open().await;
    page.route(
        "/api/network",
        RouteAction::SetResponseHeaders(vec![("X-Fixture".into(), "first-header".into())]),
    )
    .await
    .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(
            &frame,
            "window.initialResponse.then(() => window.initialHeader)"
        )
        .await,
        json!("first-header")
    );
    let frame = navigate_network_child(&page, "/network-frame.html").await;
    assert_eq!(
        network_evaluate(
            &frame,
            "window.initialResponse.then(() => window.initialHeader)"
        )
        .await,
        json!("first-header")
    );
    // A main-frame navigation must also resume and receive the same rule.
    page.goto(&format!("{}/network-frame.html", fixture.base))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.initialResponse.then(() => window.initialHeader)")
            .await
            .unwrap(),
        json!("first-header")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_gate_resumes_application_debugger_statements_and_clears_cleanly() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "active")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(
            &frame,
            "debugger; fetch('/api/network').then(r => r.text())"
        )
        .await,
        json!("active")
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), page.evaluate("debugger; 42"))
            .await
            .unwrap()
            .unwrap(),
        json!(42)
    );
    page.clear_routes().await.unwrap();
    let frame = navigate_network_child(&page, "/network-frame.html").await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("network")
    );
    page.mock("/api/network", 200, "text/plain", "restarted")
        .await
        .unwrap();
    let remote = format!(
        "{}/network-frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    let frame = navigate_network_child(&page, &remote).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("restarted")
    );
    let frame = navigate_network_child(&page, "/network-frame.html").await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("restarted")
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn network_gate_is_inert_for_other_debuggers_after_routes_are_cleared() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "active")
        .await
        .unwrap();
    page.clear_routes().await.unwrap();
    assert_gate_removed(&fixture, &browser, &page).await;
    browser.close().await.unwrap();
}

async fn assert_gate_removed(fixture: &Fixture, browser: &Browser, page: &Page) {
    let connection =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
            .await
            .unwrap();
    let mut events = connection.subscribe();
    let attached = connection
        .send_raw(
            None,
            "Target.attachToTarget",
            json!({"targetId":page.target_id(), "flatten":true}),
        )
        .await
        .unwrap();
    let session = attached["sessionId"].as_str().unwrap().to_string();
    connection
        .send_raw(Some(&session), "Debugger.enable", json!({}))
        .await
        .unwrap();
    let observed = Arc::new(AtomicUsize::new(0));
    let paused = observed.clone();
    let commands = connection.clone();
    let observer = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            if event.session_id.as_deref() == Some(&session) && event.method == "Debugger.paused" {
                paused.fetch_add(1, Ordering::SeqCst);
                let _ = commands
                    .send_raw(Some(&session), "Debugger.resume", json!({}))
                    .await;
            }
        }
    });
    tokio::time::timeout(
        Duration::from_secs(3),
        page.goto(&format!("{}/network-frame.html", fixture.base)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        page.evaluate("window.initialResponse").await.unwrap(),
        json!("network")
    );
    assert_eq!(
        observed.load(Ordering::SeqCst),
        0,
        "inactive gate must not pause another debugger"
    );
    observer.abort();
    connection.close();
}

async fn cancel_configuration_after_start(future: impl std::future::Future<Output = Result<()>>) {
    let mut future = Box::pin(future);
    std::future::poll_fn(|context| {
        assert!(
            future.as_mut().poll(context).is_pending(),
            "configuration starts asynchronously"
        );
        std::task::Poll::Ready(())
    })
    .await;
    // The owned routing lock stays with the configuration task until it finishes.
    drop(future);
}

#[tokio::test]
async fn network_gate_cancelled_updates_finish_before_later_cleanup_or_reconfiguration() {
    let (fixture, browser, page) = open().await;
    cancel_configuration_after_start(page.mock(
        "/api/network",
        200,
        "text/plain",
        "cancelled-caller",
    ))
    .await;
    page.clear_routes().await.unwrap();
    page.mock("/api/network", 200, "text/plain", "before-clear")
        .await
        .unwrap();
    cancel_configuration_after_start(page.clear_routes()).await;
    page.mock("/api/network", 200, "text/plain", "after-clear")
        .await
        .unwrap();
    let frame = open_network_frame(&fixture, &browser, &page).await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("after-clear")
    );
    let frame = navigate_network_child(&page, "/network-frame.html").await;
    assert_eq!(
        network_evaluate(&frame, "window.initialResponse").await,
        json!("after-clear")
    );
    page.clear_routes().await.unwrap();
    assert_gate_removed(&fixture, &browser, &page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cross_site_network_gate_restores_the_first_request_when_nested_child_returns_to_outer() {
    let (fixture, browser, page) = open().await;
    page.mock("/api/network", 200, "text/plain", "nested-return")
        .await
        .unwrap();
    let outer = open_network_frame(&fixture, &browser, &page).await;
    let inner_url = format!("{}/network-frame.html", fixture.base);
    network_evaluate(&outer, &format!("new Promise(resolve => {{ const frame = document.createElement('iframe'); frame.id = 'nested-network'; frame.onload = () => resolve(true); frame.src = {inner_url:?}; document.body.append(frame); }})")).await;
    let inner = page
        .frames()
        .into_iter()
        .find(|frame| frame.url() == inner_url)
        .unwrap();
    assert_remote_frame(&browser, &inner).await;
    assert_eq!(
        network_evaluate(&inner, "window.initialResponse").await,
        json!("nested-return")
    );
    network_evaluate(&outer, "new Promise(resolve => { const frame = document.querySelector('#nested-network'); frame.onload = () => resolve(true); frame.src = '/network-frame.html'; })").await;
    assert_eq!(
        network_evaluate(&inner, "window.initialResponse").await,
        json!("nested-return")
    );
    assert_eq!(fixture.network_requests.load(Ordering::SeqCst), 0);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn network_gate_keeps_cached_resources_mocked_across_navigation_and_frame_returns() {
    let (fixture, browser, page) = open().await;
    let frame = open_network_frame(&fixture, &browser, &page).await;
    let expression = "fetch('/api/cached').then(r => r.text())";
    // Prime both origins' caches before interception is enabled.
    assert_eq!(network_evaluate(&frame, expression).await, json!("network"));
    assert_eq!(page.evaluate(expression).await.unwrap(), json!("network"));
    page.mock("/api/cached", 200, "text/plain", "cached-resource-mock")
        .await
        .unwrap();
    let local = "/network-frame.html?api=%2Fapi%2Fcached";
    let remote = format!("{}{local}", fixture.base.replace("127.0.0.1", "localhost"));
    for destination in [local, &remote, local, &remote] {
        let frame = navigate_network_child(&page, destination).await;
        assert_eq!(
            network_evaluate(&frame, "window.initialResponse").await,
            json!("cached-resource-mock"),
            "destination {destination}"
        );
    }
    page.goto(&format!("{}{local}", fixture.base))
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("window.initialResponse").await.unwrap(),
        json!("cached-resource-mock")
    );
    page.clear_routes().await.unwrap();
    page.reload().await.unwrap();
    assert_eq!(
        page.evaluate("window.initialResponse").await.unwrap(),
        json!("network")
    );
    browser.close().await.unwrap();
}
