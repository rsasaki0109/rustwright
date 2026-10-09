//! Shared live HTTP network-idle behavior, including response bodies and frames.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use rustwright::{bidi::BidiError, prelude::*};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{watch, Semaphore},
    task::{JoinHandle, JoinSet},
    time::Instant,
};

const LIMIT: Duration = Duration::from_secs(5);
static NEXT_PROFILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: String,
    profile: PathBuf,
    bodies: Arc<Semaphore>,
    headers: watch::Sender<Vec<String>>,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

impl Fixture {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let profile = std::env::temp_dir().join(format!(
            "rustwright-idle-parity-{}-{}",
            std::process::id(),
            NEXT_PROFILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&profile).unwrap();
        let bodies = Arc::new(Semaphore::new(0));
        let (headers, _) = watch::channel(Vec::<String>::new());
        let held = bodies.clone();
        let observed = headers.clone();
        let task = tokio::spawn(async move {
            let mut requests = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let bodies = held.clone();
                        let headers = observed.clone();
                        requests.spawn(async move {
                            let mut request = Vec::new();
                            loop {
                                let mut chunk = [0; 2048];
                                let Ok(count) = socket.read(&mut chunk).await else { return; };
                                if count == 0 { return; }
                                request.extend_from_slice(&chunk[..count]);
                                if request.len() > 16 * 1024 { return; }
                                if request.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                            }
                            let text = String::from_utf8_lossy(&request);
                            let Some(target) = text.lines().next().and_then(|line| line.split_whitespace().nth(1)) else { return; };
                            let target = target.to_string();
                            let path = target.split('?').next().unwrap();
                            let (status, extra, body) = match path {
                                "/page" | "/held-document" => ("200 OK", "", "<!DOCTYPE html><title>idle parity</title><body>ready</body>"),
                                "/frame-fetch" => ("200 OK", "", "<!DOCTYPE html><title>child</title><script>window.body = fetch('/stream?child').then(r => r.text()).catch(e => e.name);</script><body>child</body>"),
                                "/stream" | "/partial" => ("200 OK", "", "finished"),
                                "/redirect-a" => ("302 Found", "Location: /redirect-b?redirect\r\n", ""),
                                "/redirect-b" => ("307 Temporary Redirect", "Location: /stream?redirect\r\n", ""),
                                _ => ("404 Not Found", "", ""),
                            };
                            let length = body.len() + if path == "/partial" { 16 } else { 0 };
                            let response = format!("HTTP/1.1 {status}\r\n{extra}Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
                            if socket.write_all(response.as_bytes()).await.is_err() { return; }
                            headers.send_modify(|seen| seen.push(target.clone()));
                            if matches!(path, "/stream" | "/held-document") {
                                let Ok(permit) = bodies.acquire_owned().await else { return; };
                                permit.forget();
                            }
                            let _ = socket.write_all(body.as_bytes()).await;
                        });
                    },
                    _ = requests.join_next(), if !requests.is_empty() => {},
                }
            }
        });
        Self {
            base,
            profile,
            bodies,
            headers,
            task,
        }
    }

    async fn headers(&self, marker: &str) {
        let mut observed = self.headers.subscribe();
        tokio::time::timeout(LIMIT, async {
            loop {
                if observed.borrow().iter().any(|path| path.contains(marker)) {
                    return;
                }
                observed.changed().await.unwrap();
            }
        })
        .await
        .expect("fixture must observe actual response headers");
    }
}

enum BrowserHandle {
    Chrome(Browser),
    Firefox(BidiBrowser),
}

impl BrowserHandle {
    async fn new_page(&self) -> AnyPage {
        match self {
            Self::Chrome(browser) => browser.new_page().await.unwrap().into(),
            Self::Firefox(browser) => browser.new_page().await.unwrap().into(),
        }
    }

    async fn close(self) {
        match self {
            Self::Chrome(browser) => browser.close().await.unwrap(),
            Self::Firefox(browser) => browser.close().await.unwrap(),
        }
    }
}

async fn open(firefox: bool) -> (Fixture, BrowserHandle, AnyPage) {
    let headless = match std::env::var("RUSTWRIGHT_HEADLESS").as_deref() {
        Ok("0" | "false") => false,
        Ok("1" | "true") | Err(std::env::VarError::NotPresent) => true,
        value => panic!("RUSTWRIGHT_HEADLESS must be 0, false, 1 or true: {value:?}"),
    };
    #[cfg(target_os = "linux")]
    if !headless {
        assert!(
            ["DISPLAY", "WAYLAND_DISPLAY"]
                .iter()
                .any(|name| { std::env::var(name).is_ok_and(|display| !display.is_empty()) }),
            "Headed Linux browsers require DISPLAY or WAYLAND_DISPLAY"
        );
    }
    if firefox && !headless {
        assert!(
            std::env::var_os("MOZ_HEADLESS").is_none_or(|value| value.is_empty()),
            "Unset MOZ_HEADLESS to run Firefox with visible windows"
        );
    }
    eprintln!("Requested browser mode: headless={headless}");
    let fixture = Fixture::new().await;
    let browser = if firefox {
        let browser = BidiBrowser::launch(
            Firefox::installed()
                .headless(headless)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must launch; no skip");
        println!("backend=firefox version={:?}", browser.browser_version());
        BrowserHandle::Firefox(browser)
    } else {
        let browser = Browser::launch(Chrome::installed().headless(headless))
            .await
            .expect("Chrome must launch; no skip");
        println!("backend=chrome version={}", browser.version().browser);
        BrowserHandle::Chrome(browser)
    };
    let page = browser.new_page().await;
    page.goto(&format!("{}/page", fixture.base)).await.unwrap();
    idle(&page).await;
    (fixture, browser, page)
}

async fn idle(page: &AnyPage) {
    page.wait_for_network_idle_with_timeout(LIMIT)
        .await
        .expect("network-idle watchdog");
}

async fn busy(page: &AnyPage) {
    let result = page
        // Longer than Firefox's conservative 500 ms observation floor, so a
        // broken pending tracker cannot satisfy this by timing out every call.
        .wait_for_network_idle_with_timeout(Duration::from_millis(750))
        .await;
    assert!(
        matches!(
            result,
            Err(AnyError::Chrome(Error::Timeout { .. }))
                | Err(AnyError::Firefox(
                    BidiError::Timeout { .. } | BidiError::WaitTimeout(_)
                ))
        ),
        "unfinished response must prevent idle: {result:?}"
    );
}

async fn start_fetch(page: &AnyPage, url: &str) {
    page.evaluate(&format!(
        "window.body = fetch({url:?}).then(r => r.text()).catch(e => e.name); true"
    ))
    .await
    .unwrap();
}

async fn release_body(fixture: &Fixture, page: &AnyPage) {
    let released = Instant::now();
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("window.body").await.unwrap(), "finished");
    idle(page).await;
    assert!(
        released.elapsed() >= Duration::from_millis(450),
        "500 ms quiet must follow body completion, not headers"
    );
}

async fn body_timeout_reuse(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/stream?body").await;
    fixture.headers("stream?body").await;
    busy(&page).await;
    release_body(&fixture, &page).await;
    idle(&page).await;
    browser.close().await;
}

async fn redirects(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/redirect-a").await;
    fixture.headers("stream?redirect").await;
    busy(&page).await;
    release_body(&fixture, &page).await;
    browser.close().await;
}

async fn abort(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    page.evaluate("window.controller = new AbortController(); window.body = fetch('/stream?abort', {signal: controller.signal}).then(r => r.text()).catch(e => e.name); true").await.unwrap();
    fixture.headers("stream?abort").await;
    busy(&page).await;
    let aborted = Instant::now();
    assert_eq!(
        page.evaluate("controller.abort(); window.body")
            .await
            .unwrap(),
        "AbortError"
    );
    idle(&page).await;
    assert!(aborted.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn partial_body(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/partial?truncated").await;
    fixture.headers("partial?truncated").await;
    assert_ne!(page.evaluate("window.body").await.unwrap(), "finished");
    idle(&page).await;
    browser.close().await;
}

async fn add_frame(page: &AnyPage, url: &str) {
    page.evaluate(&format!("new Promise(resolve => {{ const f = document.createElement('iframe'); f.id = 'child'; f.onload = () => resolve(true); f.src = {url:?}; document.body.append(f); }})")).await.unwrap();
}

async fn nested_children(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    add_frame(&page, "/page").await;
    page.evaluate("new Promise(resolve => { const d = document.getElementById('child').contentDocument; const f = d.createElement('iframe'); f.id = 'nested'; f.onload = () => resolve(true); f.src = '/frame-fetch'; d.body.append(f); })").await.unwrap();
    fixture.headers("stream?child").await;
    busy(&page).await;
    let released = Instant::now();
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("document.getElementById('child').contentWindow.document.getElementById('nested').contentWindow.body").await.unwrap(), "finished");
    idle(&page).await;
    assert!(released.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn cross_origin_child(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let child = Fixture::new().await;
    add_frame(&page, &format!("{}/frame-fetch", child.base)).await;
    child.headers("stream?child").await;
    busy(&page).await;
    let released = Instant::now();
    child.bodies.add_permits(1);
    idle(&page).await;
    assert!(released.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn child_removal(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    add_frame(&page, "/frame-fetch").await;
    fixture.headers("stream?child").await;
    busy(&page).await;
    let removed = Instant::now();
    page.evaluate("document.getElementById('child').remove(); true")
        .await
        .unwrap();
    idle(&page).await;
    assert!(removed.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn document_replacement(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/stream?old-document").await;
    fixture.headers("stream?old-document").await;
    busy(&page).await;
    let navigating = {
        let page = page.clone();
        let url = format!("{}/held-document?new-document", fixture.base);
        tokio::spawn(async move { page.goto(&url).await })
    };
    fixture.headers("held-document?new-document").await;
    busy(&page).await;
    // Both old and new sockets can still be waiting in the fixture; release
    // both, independently of whether the browser already aborted the old one.
    let released = Instant::now();
    fixture.bodies.add_permits(2);
    tokio::time::timeout(LIMIT, navigating)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    idle(&page).await;
    assert!(released.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn foreign_tab(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    let other = browser.new_page().await;
    other.goto(&format!("{}/page", fixture.base)).await.unwrap();
    idle(&other).await;
    start_fetch(&other, "/stream?foreign").await;
    fixture.headers("stream?foreign").await;
    busy(&other).await;
    idle(&page).await;
    release_body(&fixture, &other).await;
    if let BrowserHandle::Firefox(browser) = &browser {
        let AnyPage::Firefox(original) = &page else {
            unreachable!()
        };
        let alias = browser
            .pages()
            .await
            .unwrap()
            .into_iter()
            .find(|page| page.context_id() == original.context_id())
            .unwrap();
        alias
            .wait_for_network_idle_with_timeout(LIMIT)
            .await
            .expect("discovery must share the existing complete observer");

        let context = browser.session().create_context().await.unwrap();
        let cold = browser
            .pages()
            .await
            .unwrap()
            .into_iter()
            .find(|page| page.context_id() == context)
            .unwrap();
        cold.goto(&format!("{}/page", fixture.base)).await.unwrap();
        cold.evaluate("window.body = fetch('/stream?cold-discovered').then(r => r.text()).catch(e => e.name); true").await.unwrap();
        fixture.headers("stream?cold-discovered").await;
        assert!(matches!(
            cold.wait_for_network_idle_with_timeout(LIMIT).await,
            Err(BidiError::NetworkObservationIncomplete)
        ));
        cold.close().await.unwrap();
    }
    browser.close().await;
}

async fn interrupted_quiet(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/stream?first").await;
    fixture.headers("stream?first").await;
    busy(&page).await;
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("window.body").await.unwrap(), "finished");
    let waiting = {
        let page = page.clone();
        tokio::spawn(async move { page.wait_for_network_idle_with_timeout(LIMIT).await })
    };
    start_fetch(&page, "/stream?interrupted").await;
    fixture.headers("stream?interrupted").await;
    busy(&page).await;
    // More than the old quiet window elapses while the second body is held.
    tokio::time::sleep(Duration::from_millis(550)).await;
    assert!(!waiting.is_finished(), "new traffic must interrupt quiet");
    let released = Instant::now();
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("window.body").await.unwrap(), "finished");
    tokio::time::timeout(LIMIT, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(released.elapsed() >= Duration::from_millis(450));
    browser.close().await;
}

async fn cancelled_waiter(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/stream?cancel").await;
    fixture.headers("stream?cancel").await;
    busy(&page).await;
    let waiting = {
        let page = page.clone();
        tokio::spawn(async move { page.wait_for_network_idle_with_timeout(LIMIT).await })
    };
    tokio::task::yield_now().await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    busy(&page).await;
    release_body(&fixture, &page).await;
    browser.close().await;
}

fn assert_closed(result: std::result::Result<(), AnyError>) {
    assert!(
        matches!(result, Err(AnyError::Chrome(ref error)) if error.is_closed())
            || matches!(result, Err(AnyError::Firefox(BidiError::Closed))),
        "closed page/browser must wake idle waiter: {result:?}"
    );
}

async fn page_closure(firefox: bool) {
    // Each Firefox cycle starts a fresh process/profile and immediately creates
    // its first page. This exercises startup context replacement, not a retry:
    // the first creation/closure failure still fails the whole scenario.
    let cycles = if firefox { 5 } else { 1 };
    for cycle in 1..=cycles {
        let (fixture, browser, page) = open(firefox).await;
        assert_eq!(
            page.evaluate("document.visibilityState").await.unwrap(),
            "visible",
            "newly created pages retain foreground visibility"
        );
        start_fetch(&page, "/stream?close-page").await;
        fixture.headers("stream?close-page").await;
        let waiting = {
            let page = page.clone();
            tokio::spawn(async move { page.wait_for_network_idle_with_timeout(LIMIT).await })
        };
        page.close().await.unwrap();
        assert_closed(tokio::time::timeout(LIMIT, waiting).await.unwrap().unwrap());
        assert_closed(page.wait_for_network_idle().await);
        browser.close().await;
        println!(
            "page_closure startup cycle={cycle}/{cycles} backend={}",
            if firefox { "firefox" } else { "chrome" }
        );
    }
}

async fn browser_closure(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    start_fetch(&page, "/stream?close-browser").await;
    fixture.headers("stream?close-browser").await;
    let waiting = {
        let page = page.clone();
        tokio::spawn(async move { page.wait_for_network_idle_with_timeout(LIMIT).await })
    };
    browser.close().await;
    assert_closed(tokio::time::timeout(LIMIT, waiting).await.unwrap().unwrap());
    assert_closed(page.wait_for_network_idle().await);
}

macro_rules! both {
    ($chrome:ident, $firefox:ident, $case:ident) => {
        #[tokio::test]
        async fn $chrome() {
            $case(false).await;
        }
        #[tokio::test]
        async fn $firefox() {
            $case(true).await;
        }
    };
}

both!(
    chrome_body_timeout_reuse,
    firefox_body_timeout_reuse,
    body_timeout_reuse
);
both!(chrome_redirects, firefox_redirects, redirects);
both!(chrome_abort, firefox_abort, abort);
both!(chrome_partial_body, firefox_partial_body, partial_body);
both!(
    chrome_nested_children,
    firefox_nested_children,
    nested_children
);
both!(
    chrome_cross_origin_child,
    firefox_cross_origin_child,
    cross_origin_child
);
both!(chrome_child_removal, firefox_child_removal, child_removal);
both!(
    chrome_document_replacement,
    firefox_document_replacement,
    document_replacement
);
both!(chrome_foreign_tab, firefox_foreign_tab, foreign_tab);
both!(
    chrome_interrupted_quiet,
    firefox_interrupted_quiet,
    interrupted_quiet
);
both!(
    chrome_cancelled_waiter,
    firefox_cancelled_waiter,
    cancelled_waiter
);
both!(chrome_page_closure, firefox_page_closure, page_closure);
both!(
    chrome_browser_closure,
    firefox_browser_closure,
    browser_closure
);
