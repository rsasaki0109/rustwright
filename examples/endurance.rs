//! Linux HTTP endurance check; see bench/endurance/README.md.
use rustwright::{
    bidi::{BidiContext, BidiError},
    browser::LaunchedFirefox,
    prelude::*,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

type Check<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const LIMIT: Duration = Duration::from_secs(15);
const HTML: &str = "<!doctype html><title>endurance</title><input id=q><button id=go onclick='window.clicked=event.isTrusted'>Go</button>";
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
async fn fixture() -> Check<Fixture> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let profile = std::env::temp_dir().join(format!("rustwright-endurance-{}", std::process::id()));
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((mut socket, _)) = accepted else { break; };
                    requests.spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut bytes = [0; 2048];
                            let Ok(n) = socket.read(&mut bytes).await else { return; };
                            if n == 0 { return; }
                            request.extend_from_slice(&bytes[..n]);
                            if request.windows(4).any(|b| b == b"\r\n\r\n") { break; }
                            if request.len() > 16384 { return; }
                        }
                        let body = if String::from_utf8_lossy(&request).starts_with("GET /api ") { "live" } else { HTML };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    Ok(Fixture {
        base,
        profile,
        task,
    })
}
enum Engine {
    Chrome(Browser),
    Firefox(BidiBrowser, LaunchedFirefox),
}
enum Context {
    Chrome(BrowserContext),
    Firefox(BidiContext),
}
impl Context {
    async fn page(&self) -> Check<AnyPage> {
        Ok(match self {
            Self::Chrome(c) => c.new_page().await?.into(),
            Self::Firefox(c) => c.new_page().await?.into(),
        })
    }
    async fn pages(&self) -> Check<Vec<AnyPage>> {
        Ok(match self {
            Self::Chrome(c) => c
                .refresh_pages()
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
            Self::Firefox(c) => c.pages().await?.into_iter().map(Into::into).collect(),
        })
    }
    async fn close(&self) -> Check<()> {
        match self {
            Self::Chrome(c) => c.close().await?,
            Self::Firefox(c) => c.close().await?,
        };
        Ok(())
    }
}
impl Engine {
    async fn launch(name: &str, fixture: &Fixture) -> Check<Self> {
        // Heaptrack instrumentation belongs to the Rust driver for this opt-in
        // measurement. Clear it in Command's child environment, before even a
        // browser wrapper starts, rather than mixing child allocations into it.
        let driver_heap = std::env::var("RUSTWRIGHT_DRIVER_HEAP_PROFILE").as_deref() == Ok("1");
        match name {
            "chrome" => {
                let mut options = Chrome::installed().headless(true);
                if driver_heap {
                    options = options
                        .env("LD_PRELOAD", "")
                        .env("DUMP_HEAPTRACK_OUTPUT", "");
                }
                Ok(Self::Chrome(Browser::launch(options).await?))
            }
            "firefox" => {
                let mut options = Firefox::installed()
                    .headless(true)
                    .profile(&fixture.profile);
                if driver_heap {
                    options = options
                        .env("LD_PRELOAD", "")
                        .env("DUMP_HEAPTRACK_OUTPUT", "");
                }
                let process = LaunchedFirefox::launch(&options).await?;
                let browser = BidiBrowser::connect(process.ws_url()).await?;
                Ok(Self::Firefox(browser, process))
            }
            _ => Err("engine must be chrome or firefox".into()),
        }
    }
    fn pid(&self) -> u32 {
        match self {
            Self::Chrome(b) => b.diagnostics().pid.expect("launched browser"),
            Self::Firefox(_, p) => p.pid(),
        }
    }
    fn pending(&self) -> usize {
        match self {
            Self::Chrome(b) => b.pending_command_count(),
            Self::Firefox(b, _) => b.session().connection().pending_command_count(),
        }
    }
    fn version(&self) -> String {
        match self {
            Self::Chrome(b) => b.version().browser.clone(),
            Self::Firefox(b, _) => b.browser_version().unwrap_or("unknown").into(),
        }
    }
    async fn context(&self) -> Check<Context> {
        Ok(match self {
            Self::Chrome(b) => Context::Chrome(b.new_context().await?),
            Self::Firefox(b, _) => Context::Firefox(b.new_context().await?),
        })
    }
    async fn counts(&self, observer: Option<&rustwright::cdp::CdpConnection>) -> Check<Value> {
        match self {
            Self::Chrome(_) => {
                let c = observer.expect("Chrome observer");
                let targets = c.send_raw(None, "Target.getTargets", json!({})).await?;
                let contexts = c
                    .send_raw(None, "Target.getBrowserContexts", json!({}))
                    .await?;
                Ok(
                    json!({"page_targets": targets["targetInfos"].as_array().ok_or("target list missing")?.iter().filter(|t| t["type"] == "page").count(), "isolated_contexts": contexts["browserContextIds"].as_array().ok_or("context list missing")?.len()}),
                )
            }
            Self::Firefox(b, _) => {
                let tree = b.session().get_tree().await?;
                let contexts = b
                    .session()
                    .connection()
                    .send("browser.getUserContexts", json!({}))
                    .await?;
                Ok(
                    json!({"page_targets": tree.iter().filter(|t| t.parent.is_none()).count(), "isolated_contexts": contexts["userContexts"].as_array().ok_or("user context list missing")?.iter().filter(|c| c["userContext"] != "default").count(), "acknowledged_helper_preloads": b.session().helper_preload_count()}),
                )
            }
        }
    }
    async fn close(self) -> Check<()> {
        match self {
            Self::Chrome(b) => b.close().await?,
            Self::Firefox(b, mut p) => {
                b.close().await?;
                p.kill();
            }
        };
        Ok(())
    }
}
fn verify(value: Value, expected: Value) -> Check<()> {
    if value != expected {
        return Err(format!("expected {expected}, got {value}").into());
    }
    Ok(())
}

async fn cycle(engine: &Engine, fixture: &Fixture, index: usize) -> Check<()> {
    let context = engine.context().await?;
    let mut retained_page = None;
    let result: Check<()> = async {
        let page = context.page().await?;
        retained_page = Some(page.clone());
        page.goto(&format!("{}/page", fixture.base)).await?;
        page.locator("#q").fill("endurance 日本語").await?;
        verify(
            page.evaluate("document.querySelector('#q').value").await?,
            json!("endurance 日本語"),
        )?;
        let error = page
            .locator("#absent")
            .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(10))
            .await;
        let error = error.err().ok_or("absent locator unexpectedly succeeded")?;
        if !matches!(
            error,
            AnyError::Chrome(rustwright::Error::Timeout { .. })
                | AnyError::Firefox(BidiError::WaitTimeout(_))
        ) {
            return Err(format!("unexpected wait error: {error}").into());
        }
        let discovered = context.pages().await?;
        if discovered.len() != 1 {
            return Err(format!("expected one discovered page, got {}", discovered.len()).into());
        }
        verify(discovered[0].title().await?.into(), json!("endurance"))?;
        drop(discovered);
        if index % 10 == 0 {
            // Ensure the command was actually sent before cancelling its future.
            let waiting = page.clone();
            let mut task = tokio::spawn(async move {
                waiting
                    .evaluate("window.cancelStarted=true;new Promise(()=>{})")
                    .await
            });
            let started = async {
                loop {
                    if task.is_finished() {
                        return Err("never-resolving evaluation unexpectedly finished".into());
                    }
                    if page.evaluate("window.cancelStarted===true").await? == json!(true) {
                        return Ok::<_, Box<dyn Error + Send + Sync>>(());
                    }
                    tokio::task::yield_now().await;
                }
            };
            let started = tokio::time::timeout(Duration::from_secs(2), started).await;
            task.abort();
            let joined = (&mut task).await;
            started??;
            if !joined.is_err_and(|error| error.is_cancelled()) {
                return Err("evaluation did not cancel".into());
            }
            verify(page.title().await?.into(), json!("endurance"))?;
            route_and_frames(&page, fixture).await?;
        }
        if index % 2 == 0 {
            page.close().await?;
        }
        Ok(())
    }
    .await;
    // Exercise both explicit page close and closing a context with an open page.
    let closed = context.close().await;
    drop(retained_page);
    result?;
    closed
}
// A small workload isolates page wrappers and helper injection from routing,
// input, failed waits and cancelled evaluations in the full endurance workload.
async fn basic_cycle(engine: &Engine, fixture: &Fixture, index: usize) -> Check<()> {
    let context = engine.context().await?;
    let mut retained = None;
    let result: Check<()> = async {
        let page = context.page().await?;
        retained = Some(page.clone());
        page.goto(&format!("{}/page", fixture.base)).await?;
        verify(page.title().await?.into(), json!("endurance"))?;
        let pages = context.pages().await?;
        if pages.len() != 1 {
            return Err("expected one discovered page".into());
        }
        verify(pages[0].title().await?.into(), json!("endurance"))?;
        if index % 2 == 0 {
            page.close().await?;
        }
        Ok(())
    }
    .await;
    let closed = context.close().await;
    drop(retained);
    result?;
    closed
}
async fn default_cycle(engine: &Engine, fixture: &Fixture, index: usize) -> Check<()> {
    let Engine::Chrome(browser) = engine else {
        return Err("default-context workload requires Chrome".into());
    };
    let page = browser.new_page().await?;
    let retained = page.clone();
    page.goto(&format!("{}/page", fixture.base)).await?;
    page.locator("#q").fill("default 日本語").await?;
    verify(
        page.evaluate("document.querySelector('#q').value").await?,
        json!("default 日本語"),
    )?;
    if index % 2 == 0 {
        page.close().await?;
    } else {
        // External closure must also release tracking. Deliberately never call
        // pages()/refresh_pages(): their lazy pruning hid the original defect.
        let observer =
            rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
                .await?;
        let closed = observer
            .send_raw(
                None,
                "Target.closeTarget",
                json!({"targetId":page.target_id()}),
            )
            .await;
        observer.close();
        closed?;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !page.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await?;
    }
    if !retained.is_closed() {
        return Err("retained page handle did not close".into());
    }
    Ok(())
}
async fn raw_cycle(engine: &Engine, fixture: &Fixture, index: usize, helper: bool) -> Check<()> {
    let Engine::Firefox(browser, _) = engine else {
        return Err("direct BiDi workloads require Firefox".into());
    };
    // Keep the same transport, session, launcher and fixture. Bypass BidiPage,
    // BidiContext and their ownership bookkeeping, rather than comparing a
    // Python process with a Rust process.
    let connection = browser.session().connection();
    let user = connection
        .send("browser.createUserContext", json!({}))
        .await?;
    let user = user["userContext"].as_str().ok_or("missing user context")?;
    let mut preload = None;
    let result: Check<()> = async {
        let tab = connection.send("browsingContext.create", json!({"type":"tab", "userContext":user})).await?;
        let tab = tab["context"].as_str().ok_or("missing tab")?;
        if helper {
            let script = include_str!("../crates/rustwright-common/src/injected.js");
            let added = connection.send("script.addPreloadScript", json!({"functionDeclaration":format!("function () {{ {script} }}"), "contexts":[tab]})).await?;
            preload = Some(added["script"].as_str().ok_or("missing script id")?.to_owned());
            browser.session().evaluate(tab, script).await?;
        }
        connection.send("browsingContext.navigate", json!({"context":tab, "url":format!("{}/page", fixture.base), "wait":"complete"})).await?;
        verify(browser.session().evaluate(tab, "document.title").await?, json!("endurance"))?;
        let tree = connection.send("browsingContext.getTree", json!({})).await?;
        let pages: Vec<_> = tree["contexts"].as_array().ok_or("missing tree")?.iter().filter(|p| p["userContext"] == user).collect();
        if pages.len() != 1 || pages[0]["context"] != tab {
            return Err("direct discovery did not find exactly the created tab".into());
        }
        verify(browser.session().evaluate(tab, "document.title").await?, json!("endurance"))?;
        if index % 2 == 0 {
            if let Some(id) = preload.as_ref() {
                connection.send("script.removePreloadScript", json!({"script":id})).await?;
                preload = None;
            }
            connection.send("browsingContext.close", json!({"context":tab})).await?;
        }
        Ok(())
    }.await;
    let closed = connection
        .send("browser.removeUserContext", json!({"userContext":user}))
        .await;
    let removed = if let Some(id) = preload {
        connection
            .send("script.removePreloadScript", json!({"script":id}))
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    result?;
    closed?;
    removed?;
    Ok(())
}
async fn route_and_frames(page: &AnyPage, fixture: &Fixture) -> Check<()> {
    match page {
        AnyPage::Chrome(p) => p.mock("*/api", 200, "text/plain", "mocked").await?,
        AnyPage::Firefox(p) => p.mock("*/api", 200, "text/plain", "mocked").await?,
    }
    verify(
        page.evaluate("fetch('/api').then(r=>r.text())").await?,
        json!("mocked"),
    )?;
    match page {
        AnyPage::Chrome(p) => p.clear_routes().await?,
        AnyPage::Firefox(p) => p.clear_routes().await?,
    }
    verify(
        page.evaluate("fetch('/api').then(r=>r.text())").await?,
        json!("live"),
    )?;
    for cross_origin in [false, true, false] {
        let base = if cross_origin {
            fixture.base.replace("127.0.0.1", "localhost")
        } else {
            fixture.base.clone()
        };
        page.evaluate(&format!("new Promise(resolve=>{{let f=document.querySelector('iframe');if(!f){{f=document.createElement('iframe');document.body.append(f)}}f.onload=()=>resolve(true);f.src={};}})", json!(format!("{base}/frame")))).await?;
        let clicked = match page {
            AnyPage::Chrome(p) => {
                let f = p.frame_locator("iframe").resolve().await?;
                f.locator("#go").click().await?;
                f.evaluate("window.clicked").await?
            }
            AnyPage::Firefox(p) => {
                let f = p.frame_locator("iframe").await?;
                f.locator("#go").click().await?;
                f.evaluate("window.clicked").await?
            }
        };
        verify(clicked, json!(true))?;
    }
    page.evaluate("document.querySelector('iframe').remove()")
        .await?;
    Ok(())
}
fn process_memory(browser_pid: u32) -> Check<Value> {
    let mut processes = HashMap::new();
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let field = |name: &str| {
            status.lines().find_map(|line| {
                line.strip_prefix(name)?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        };
        if let (Some(parent), Some(rss)) = (field("PPid:"), field("VmRSS:")) {
            processes.insert(pid, (parent as u32, rss));
        }
    }
    let mut descendants = vec![browser_pid];
    let mut i = 0;
    while i < descendants.len() {
        let parent = descendants[i];
        for (&pid, &(ppid, _)) in &processes {
            if ppid == parent {
                descendants.push(pid);
            }
        }
        i += 1;
    }
    let driver = processes
        .get(&std::process::id())
        .ok_or("driver RSS missing")?
        .1;
    if !processes.contains_key(&browser_pid) {
        return Err("browser RSS missing".into());
    }
    let rss: u64 = descendants
        .iter()
        .filter_map(|pid| processes.get(pid).map(|p| p.1))
        .sum();
    let pss = |pid: u32| -> Option<u64> {
        let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
        rollup.lines().find_map(|line| {
            line.strip_prefix("Pss:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
    };
    // Only publish a tree total if every sampled descendant has a readable PSS.
    // Processes may exit during sampling; never treat unavailable data as zero.
    let tree_pss = descendants
        .iter()
        .map(|&pid| pss(pid))
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum::<u64>());
    let by_process: Vec<_> = descendants.iter().map(|&pid| json!({"pid":pid,"rss_kib":processes.get(&pid).map(|p| p.1),"pss_kib":pss(pid),"root":pid == browser_pid})).collect();
    Ok(
        json!({"driver_rss_kib": driver, "browser_tree_rss_kib": rss, "browser_processes": descendants.len(), "driver_pss_kib":pss(std::process::id()), "browser_tree_pss_kib":tree_pss, "browser_by_process":by_process}),
    )
}
fn emit(value: Value) {
    println!("{value}");
}
#[tokio::main]
async fn main() -> Check<()> {
    if !cfg!(target_os = "linux") {
        return Err("RSS measurement requires Linux /proc".into());
    }
    let mut args = std::env::args().skip(1);
    let name = args.next().ok_or(
        "usage: endurance chrome|firefox [cycles=1000] [warmup=100] [full|basic|raw|raw-helper|default]",
    )?;
    let cycles: usize = args.next().unwrap_or("1000".into()).parse()?;
    let warmup: usize = args.next().unwrap_or("100".into()).parse()?;
    let workload = args.next().unwrap_or("full".into());
    if cycles == 0
        || args.next().is_some()
        || !matches!(
            workload.as_str(),
            "full" | "basic" | "raw" | "raw-helper" | "default"
        )
        || (name != "firefox" && workload.starts_with("raw"))
        || (name != "chrome" && workload == "default")
    {
        return Err("invalid cycles, arguments or workload/engine combination".into());
    }
    let fixture = fixture().await?;
    let engine = Engine::launch(&name, &fixture).await?;
    let observer = match &engine {
        Engine::Chrome(b) => Some(
            rustwright::cdp::CdpConnection::connect(&b.version().web_socket_debugger_url).await?,
        ),
        _ => None,
    };
    let started = Instant::now();
    emit(
        json!({"kind":"metadata", "engine":name,"workload":workload,"version":engine.version(),"cycles":cycles,"warmup":warmup,"browser_pid":engine.pid(),"driver_pid":std::process::id(),"driver_heap_profile":std::env::var("RUSTWRIGHT_DRIVER_HEAP_PROFILE").as_deref() == Ok("1"),"memory":"/proc VmRSS in KiB; browser tree sum double-counts shared pages", "sessions":"not measured; page targets and isolated contexts are measured"}),
    );
    let result = async {
        let baseline = tokio::time::timeout(LIMIT, engine.counts(observer.as_ref())).await??;
        for i in 0..=warmup+cycles {
            if i > 0 {
                let step = async {
                    match workload.as_str() {
                        "basic" => basic_cycle(&engine, &fixture, i).await,
                        "default" => default_cycle(&engine, &fixture, i).await,
                        "raw" | "raw-helper" => raw_cycle(&engine, &fixture, i, workload == "raw-helper").await,
                        _ => cycle(&engine, &fixture, i).await,
                    }
                };
                tokio::time::timeout(LIMIT, step).await.map_err(|_| format!("cycle {i} exceeded 15s"))?.map_err(|error| format!("cycle {i}: {error}"))?;
            }
            if i == 0 || i == warmup || i % 100 == 0 || i == warmup+cycles {
                // Let asynchronous detach/renderer shutdown complete before sampling.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let counts = tokio::time::timeout(LIMIT, engine.counts(observer.as_ref())).await??;
                let pending = engine.pending();
                emit(json!({"kind":"sample", "completed":i,"measured_completed":i.saturating_sub(warmup),"elapsed_ms":started.elapsed().as_millis(),"counts":counts,"pending_commands":pending,"memory":process_memory(engine.pid())?}));
                if counts != baseline { return Err(format!("residual targets/contexts: {counts}, baseline {baseline}").into()); }
                if pending != 0 { return Err(format!("residual pending commands: {pending}").into()); }
            }
        }
        Ok::<_,Box<dyn Error + Send + Sync>>(())
    }.await;
    let close = tokio::time::timeout(LIMIT, engine.close()).await;
    if let Some(observer) = observer {
        observer.close();
    }
    emit(
        json!({"kind":"result","success":result.is_ok() && matches!(&close,Ok(Ok(()))),"error":result.as_ref().err().map(ToString::to_string),"shutdown_error":match &close { Ok(Ok(()))=>None,Ok(Err(e))=>Some(e.to_string()),Err(e)=>Some(e.to_string()) },"elapsed_ms":started.elapsed().as_millis()}),
    );
    result?;
    close??;
    Ok(())
}
