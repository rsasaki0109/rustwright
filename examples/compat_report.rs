//! Observe real sites without logging in or interacting with their controls.
//!
//! Each requested operation has a finite deadline and an explicit JSON outcome.
//! HTTP denial/error responses are site observations, not proof of driver defects.
//! A failed operation produces exit1; invalid CLI arguments produce exit2.
//!
//! cargo run -p rustwright-examples --example compat_report -- \
//!   --browser firefox --headless --output target/reliability95/sites/firefox https://example.com/

use std::{
    collections::HashSet,
    fmt::Display,
    future::Future,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rustwright::{bidi::BidiBrowser, prelude::*, AnyPage};
use serde_json::{json, Value};
use tokio::{sync::oneshot, task::JoinHandle};

const HELP: &str = "Usage: compat_report [--browser chrome|firefox] [--headed|--headless]\n\
    [--output DIR] [--profile DIR] [--proxy http://HOST:PORT] [--viewport WIDTHxHEIGHT]\n\
    [--timeout-secs 1..300] [--idle-timeout-secs 1..300] [HTTP(S)_URL ...]\n\
    Defaults: headless Chrome, 1280x800, 30s per operation, 5s idle wait.\n\
    Browser shutdown allows at least 6s for its bounded graceful cleanup.\n\
    Without URLs observes Mercari, Rakuma, X, YouTube, TikTok and Instagram.\n\
    Exit0: requested operations completed (HTTP denial can still be observed).\n\
    Exit1: an operation/report failed; attribution to driver/site is undetermined.\n\
    Exit2: invalid arguments. Output directory must be new or empty.\n\
    --proxy accepts an explicit HTTP proxy without credentials; Firefox requires a fresh profile.";
const DEFAULT_SITES: &[&str] = &[
    "https://jp.mercari.com/",
    "https://fril.jp/",
    "https://x.com/",
    "https://www.youtube.com/",
    "https://www.tiktok.com/",
    "https://www.instagram.com/",
];
const CAPTURE_LIMIT: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Backend {
    Chrome,
    Firefox,
}
impl Backend {
    fn name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
        }
    }
}
struct Config {
    backend: Backend,
    headed: bool,
    profile: Option<PathBuf>,
    proxy: Option<Proxy>,
    output: PathBuf,
    width: i64,
    height: i64,
    timeout: Duration,
    idle_timeout: Duration,
    sites: Vec<String>,
}
impl Config {
    fn parse(args: impl IntoIterator<Item = String>) -> std::result::Result<Option<Self>, String> {
        let mut config = Self {
            backend: Backend::Chrome,
            headed: false,
            profile: None,
            proxy: None,
            output: PathBuf::new(),
            width: 1280,
            height: 800,
            timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(5),
            sites: Vec::new(),
        };
        let mut seen = HashSet::new();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Ok(None);
            }
            if arg.starts_with('-') {
                let key = if arg == "--headed" || arg == "--headless" {
                    "mode"
                } else {
                    &arg
                };
                if !seen.insert(key.to_owned()) {
                    return Err(format!("duplicate/conflicting option: {arg}"));
                }
            }
            match arg.as_str() {
                "--headed" => config.headed = true,
                "--headless" => config.headed = false,
                "--browser" => {
                    config.backend = match argument(&mut args, &arg)?.as_str() {
                        "chrome" => Backend::Chrome,
                        "firefox" => Backend::Firefox,
                        value => return Err(format!("unsupported browser: {value}")),
                    }
                }
                "--profile" => config.profile = Some(argument(&mut args, &arg)?.into()),
                "--proxy" => config.proxy = Some(Proxy::parse(&argument(&mut args, &arg)?)?),
                "--output" => config.output = argument(&mut args, &arg)?.into(),
                "--viewport" => {
                    let value = argument(&mut args, &arg)?;
                    let (width, height) = value
                        .split_once(['x', 'X'])
                        .ok_or("viewport must be WIDTHxHEIGHT")?;
                    config.width = width.parse().map_err(|_| "invalid viewport width")?;
                    config.height = height.parse().map_err(|_| "invalid viewport height")?;
                    if !(1..=8192).contains(&config.width) || !(1..=8192).contains(&config.height) {
                        return Err("viewport dimensions must be 1..8192".into());
                    }
                }
                "--timeout-secs" => config.timeout = seconds(argument(&mut args, &arg)?)?,
                "--idle-timeout-secs" => config.idle_timeout = seconds(argument(&mut args, &arg)?)?,
                value if value.starts_with('-') => return Err(format!("unknown option: {value}")),
                url => {
                    let authority = url
                        .strip_prefix("https://")
                        .or_else(|| url.strip_prefix("http://"))
                        .and_then(|rest| rest.split(['/', '?', '#']).next())
                        .unwrap_or("");
                    if authority.is_empty()
                        || authority.contains('@')
                        || url.chars().any(char::is_whitespace)
                    {
                        return Err(format!(
                            "expected an HTTP(S) URL without embedded credentials: {url}"
                        ));
                    }
                    config.sites.push(url.to_owned());
                }
            }
        }
        if config.backend == Backend::Firefox && config.profile.is_some() && config.proxy.is_some()
        {
            return Err("Firefox --proxy requires a fresh profile; omit --profile to preserve user preferences".into());
        }
        if config.output.as_os_str().is_empty() {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis();
            config.output = PathBuf::from(format!(
                "target/reliability95/sites/{}-{stamp}-{}",
                config.backend.name(),
                std::process::id()
            ));
        }
        if config.sites.is_empty() {
            config.sites = DEFAULT_SITES.iter().map(|s| (*s).into()).collect();
        }
        Ok(Some(config))
    }
}
#[derive(Debug)]
struct Proxy {
    url: String,
    host: String,
    port: u16,
}
impl Proxy {
    fn parse(value: &str) -> std::result::Result<Self, String> {
        let authority = value
            .strip_prefix("http://")
            .ok_or("proxy must be an http://HOST:PORT endpoint")?;
        let authority = authority.strip_suffix('/').unwrap_or(authority);
        if authority.is_empty()
            || authority.contains(['/', '?', '#', '@'])
            || authority.chars().any(char::is_whitespace)
        {
            return Err("proxy must be an HTTP endpoint without credentials, path or query".into());
        }
        let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
            let (host, suffix) = rest.split_once(']').ok_or("invalid IPv6 proxy")?;
            host.parse::<std::net::Ipv6Addr>()
                .map_err(|_| "invalid IPv6 proxy")?;
            let port = suffix.strip_prefix(':').ok_or("proxy port is required")?;
            (host, port)
        } else {
            let (host, port) = authority.rsplit_once(':').ok_or("proxy port is required")?;
            if host.is_empty()
                || !host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            {
                return Err("invalid proxy hostname".into());
            }
            (host, port)
        };
        let port = port
            .parse::<u16>()
            .map_err(|_| "proxy port must be 1..65535")?;
        if port == 0 {
            return Err("proxy port must be 1..65535".into());
        }
        Ok(Self {
            url: format!("http://{authority}"),
            host: host.into(),
            port,
        })
    }
    fn firefox_preferences(&self) -> String {
        format!("user_pref(\"network.proxy.type\", 1);\nuser_pref(\"network.proxy.http\", {});\nuser_pref(\"network.proxy.http_port\", {});\nuser_pref(\"network.proxy.ssl\", {});\nuser_pref(\"network.proxy.ssl_port\", {});\nuser_pref(\"network.proxy.no_proxies_on\", \"localhost, 127.0.0.1, ::1\");\nuser_pref(\"security.enterprise_roots.enabled\", true);\n", json!(self.host), self.port, json!(self.host), self.port)
    }
}

fn argument(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> std::result::Result<String, String> {
    args.next()
        .filter(|v| !v.is_empty() && !v.starts_with('-'))
        .ok_or_else(|| format!("{flag} requires a value"))
}
fn seconds(value: String) -> std::result::Result<Duration, String> {
    let count = value
        .parse::<u64>()
        .map_err(|_| "timeout must be seconds in 1..300")?;
    if !(1..=300).contains(&count) {
        return Err("timeout must be seconds in 1..300".into());
    }
    Ok(Duration::from_secs(count))
}

async fn operation<T, E: Display>(
    name: &str,
    deadline: Duration,
    future: impl Future<Output = std::result::Result<T, E>>,
    records: &mut Vec<Value>,
) -> Option<T> {
    let started = Instant::now();
    let (value, status, error) = match tokio::time::timeout(deadline, future).await {
        Ok(Ok(value)) => (Some(value), "passed", None),
        Ok(Err(error)) => (None, "failed", Some(error.to_string())),
        Err(_) => (
            None,
            "timed_out",
            Some(format!("deadline exceeded after {deadline:?}")),
        ),
    };
    records.push(json!({"operation":name,"status":status,"elapsed_ms":started.elapsed().as_millis(),
        "deadline_ms":deadline.as_millis(),"error":error,"failure_attribution":if status == "passed" {Value::Null} else {json!("undetermined; an operation error is not proof of a driver defect")}}));
    value
}
fn all_passed(records: &[Value]) -> bool {
    records.iter().all(|r| r["status"] == "passed")
}
fn observation(url: Option<&str>, statuses: &[i64]) -> &'static str {
    if statuses.iter().any(|s| matches!(s, 401 | 403 | 429)) {
        "http_access_denied"
    } else if statuses.iter().any(|s| (400..600).contains(s)) {
        "http_error_response"
    } else if url.is_some_and(|u| {
        u.starts_with("chrome-error://")
            || u.starts_with("about:neterror")
            || u.starts_with("about:certerror")
    }) {
        "browser_error_page"
    } else if statuses.iter().any(|s| (200..400).contains(s)) {
        "http_document_observed"
    } else {
        "document_status_unknown"
    }
}

enum Engine {
    Chrome(Browser),
    Firefox(BidiBrowser),
}
impl Engine {
    async fn launch(config: &Config, profile: &Path) -> std::result::Result<Self, String> {
        if config.headed {
            if cfg!(target_os = "linux")
                && !["DISPLAY", "WAYLAND_DISPLAY"]
                    .iter()
                    .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
            {
                return Err("headed Linux browsers require DISPLAY or WAYLAND_DISPLAY".into());
            }
            if config.backend == Backend::Firefox
                && std::env::var_os("MOZ_HEADLESS").is_some_and(|v| !v.is_empty())
            {
                return Err("unset MOZ_HEADLESS to request headed Firefox".into());
            }
        }
        let key = format!("RUSTWRIGHT_{}", config.backend.name().to_uppercase());
        if let Some(path) = std::env::var_os(&key) {
            if !Path::new(&path).is_file() {
                return Err(format!("{key} is not an installed executable file"));
            }
        }
        match config.backend {
            Backend::Chrome => {
                let mut options = Chrome::installed().headless(!config.headed);
                if let Some(profile) = &config.profile {
                    options = options.profile(profile);
                }
                if let Some(proxy) = &config.proxy {
                    options = options
                        .arg(format!("--proxy-server={}", proxy.url))
                        .arg("--proxy-bypass-list=localhost;127.0.0.1;[::1]");
                }
                Browser::launch(options)
                    .await
                    .map(Self::Chrome)
                    .map_err(|e| e.to_string())
            }
            Backend::Firefox => {
                let options = Firefox::installed()
                    .headless(!config.headed)
                    .profile(profile);
                if options.is_sandboxed() {
                    return Err("sandboxed Firefox ignores isolated profiles; select a regular Firefox executable".into());
                }
                BidiBrowser::launch(options)
                    .await
                    .map(Self::Firefox)
                    .map_err(|e| e.to_string())
            }
        }
    }
    async fn page(&self) -> std::result::Result<AnyPage, String> {
        match self {
            Self::Chrome(b) => b
                .new_page()
                .await
                .map(Into::into)
                .map_err(|e| e.to_string()),
            Self::Firefox(b) => b
                .new_page()
                .await
                .map(Into::into)
                .map_err(|e| e.to_string()),
        }
    }
    fn diagnostics(&self) -> Value {
        match self {
            Self::Chrome(b) => {
                let d = b.diagnostics();
                json!({"backend":"chrome","version":d.product,"protocol_version":d.protocol_version,
                    "user_agent":d.user_agent,"executable":d.executable,"launch_args":d.launch_args,
                    "profile":d.user_data_dir,"ephemeral_profile":d.ephemeral_profile,"pid":d.pid,"browser_log":d.browser_log})
            }
            Self::Firefox(b) => json!({"backend":"firefox","version":b.browser_version(),
                "capabilities":b.session().capabilities(),"session_id":b.session().session_id()}),
        }
    }
    async fn close(self) -> std::result::Result<(), String> {
        match self {
            Self::Chrome(b) => b.close().await.map_err(|e| e.to_string()),
            Self::Firefox(b) => b.close().await.map_err(|e| e.to_string()),
        }
    }
}

#[derive(Default)]
struct CaptureData {
    events: Vec<Value>,
    dropped: u64,
    truncated: bool,
}
struct Capture {
    data: Arc<Mutex<CaptureData>>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
    subscription: String,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Capture {
    async fn start(browser: &BidiBrowser, context: &str) -> std::result::Result<Self, String> {
        let mut receiver = browser.session().events();
        let data = Arc::new(Mutex::new(CaptureData::default()));
        let sink = data.clone();
        let (stop, mut stopped) = oneshot::channel();
        let root = context.to_owned();
        let task = tokio::spawn(async move {
            let mut owned = HashSet::from([root]);
            loop {
                let event = tokio::select! { value=receiver.recv()=>value, _=&mut stopped=> {
                    let mut drained = 0;
                    for _ in 0..4096 {
                        match receiver.try_recv() {
                            Ok(event)=> { record_event(&event.method,&event.params,&mut owned,&sink); drained += 1; },
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n))=>sink.lock().unwrap().dropped+=n,
                            Err(_)=>break,
                        }
                    }
                    if drained == 4096 && !receiver.is_empty() { sink.lock().unwrap().truncated = true; }
                    break;
                }};
                match event {
                    Ok(event) => record_event(&event.method, &event.params, &mut owned, &sink),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        sink.lock().unwrap().dropped += n
                    }
                    Err(_) => break,
                }
            }
        });
        let mut capture = Self {
            data,
            stop: Some(stop),
            task,
            subscription: String::new(),
        };
        let response = browser.session().connection().send("session.subscribe", json!({"contexts":[context],"events":[
            "log.entryAdded","browsingContext.contextCreated","browsingContext.contextDestroyed",
            "browsingContext.navigationStarted","browsingContext.fragmentNavigated","browsingContext.domContentLoaded","browsingContext.load",
            "network.beforeRequestSent","network.responseStarted","network.responseCompleted","network.fetchError"]})).await;
        let subscription = match response {
            Ok(value) => value["subscription"]
                .as_str()
                .map(str::to_owned)
                .ok_or("BiDi did not return a scoped subscription ID".to_owned()),
            Err(error) => Err(error.to_string()),
        };
        match subscription {
            Ok(subscription) => {
                capture.subscription = subscription;
                Ok(capture)
            }
            Err(error) => Err(error),
        }
    }
    async fn finish(&mut self) -> std::result::Result<Value, String> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        (&mut self.task).await.map_err(|e| e.to_string())?;
        let data = self.data.lock().map_err(|e| e.to_string())?;
        Ok(
            json!({"events":data.events,"dropped_events":data.dropped,"complete":data.dropped==0 && !data.truncated,"drain_truncated":data.truncated,
            "scope":"page and observed descendant contexts; diagnostic fields exclude HTTP headers"}),
        )
    }
}
fn record_event(
    method: &str,
    params: &Value,
    owned: &mut HashSet<String>,
    sink: &Mutex<CaptureData>,
) {
    if method == "browsingContext.contextCreated"
        && params["parent"].as_str().is_some_and(|p| owned.contains(p))
    {
        if let Some(context) = params["context"].as_str() {
            owned.insert(context.to_owned());
        }
    }
    let context = params["context"]
        .as_str()
        .or_else(|| params["source"]["context"].as_str());
    if !context.is_some_and(|c| owned.contains(c)) {
        return;
    }
    let selected = if method.starts_with("network.") {
        json!({"context":context,"navigation":params["navigation"],"request":{
            "request":params["request"]["request"],"url":params["request"]["url"],
            "method":params["request"]["method"],"destination":params["request"]["destination"]},
            "response":{"status":params["response"]["status"],"statusText":params["response"]["statusText"],"mimeType":params["response"]["mimeType"]},
            "errorText":params["errorText"],"timestamp":params["timestamp"]})
    } else {
        params.clone()
    };
    let mut data = sink.lock().unwrap();
    if data.events.len() == CAPTURE_LIMIT {
        data.dropped += 1;
    } else {
        data.events.push(json!({"method":method,"params":selected}));
    }
}

fn chrome_diagnostics(page: &rustwright::Page, urls: &[String]) -> (Value, Vec<i64>) {
    let requests = page.network_requests();
    let status = requests
        .iter()
        .filter(|r| {
            r.resource_type == "Document"
                && urls
                    .iter()
                    .any(|u| without_fragment(u) == without_fragment(&r.url))
        })
        .filter_map(|r| r.status)
        .collect();
    (
        json!({"scope":"Chrome page diagnostics","console":page.console_messages().iter().map(|r|json!({"level":r.level,"text":r.text,"timestamp":r.timestamp})).collect::<Vec<_>>(),
        "javascript_errors":page.errors().iter().map(|r|json!({"message":r.message,"timestamp":r.timestamp})).collect::<Vec<_>>(),
        "navigations":page.navigations().iter().map(|r|json!({"url":r.url,"frame_id":r.frame_id,"loader_id":r.loader_id,"mime_type":r.mime_type})).collect::<Vec<_>>(),
        "network":requests.iter().map(|r|json!({"request_id":r.request_id,"url":r.url,"method":r.method,"resource_type":r.resource_type,"status":r.status,"status_text":r.status_text,"mime_type":r.mime_type,"failure":r.failure,"started":r.started,"responded":r.responded,"finished":r.finished})).collect::<Vec<_>>(),
        "dialogs":page.dialogs().iter().map(|r|json!({"type":r.dialog_type,"message":r.message})).collect::<Vec<_>>()}),
        status,
    )
}
fn without_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

async fn site(browser: &Engine, config: &Config, index: usize, url: &str) -> Value {
    let mut ops = Vec::new();
    let Some(page) = operation("new_page", config.timeout, browser.page(), &mut ops).await else {
        return json!({"url":url,"operations":ops,"operations_succeeded":false,"observation":"no_page"});
    };
    operation(
        "viewport",
        config.timeout,
        page.set_viewport(config.width, config.height, 1.0),
        &mut ops,
    )
    .await;
    let mut capture = if let (Engine::Firefox(browser), AnyPage::Firefox(page)) = (browser, &page) {
        operation(
            "diagnostic_subscription",
            config.timeout,
            Capture::start(browser, page.context_id()),
            &mut ops,
        )
        .await
    } else {
        None
    };
    if let AnyPage::Firefox(page) = &page {
        operation(
            "network_monitoring",
            config.timeout,
            page.start_network_monitoring(),
            &mut ops,
        )
        .await;
    }
    operation("navigation", config.timeout, page.goto(url), &mut ops).await;
    operation(
        "network_idle",
        config.idle_timeout,
        page.wait_for_network_idle_with_timeout(config.idle_timeout),
        &mut ops,
    )
    .await;
    let final_url = operation("final_url", config.timeout, page.url(), &mut ops).await;
    let title = operation("title", config.timeout, page.title(), &mut ops).await;
    let screenshot = config.output.join(format!("{index:03}.png"));
    let screenshot_written = operation(
        "screenshot",
        config.timeout,
        page.screenshot(&screenshot),
        &mut ops,
    )
    .await
    .is_some();
    let mut statuses = Vec::new();
    let diagnostics = match &page {
        AnyPage::Chrome(page) => {
            let mut urls = vec![url.to_owned()];
            urls.extend(final_url.iter().cloned());
            urls.extend(page.navigations().into_iter().map(|n| n.url));
            let (data, observed) = chrome_diagnostics(page, &urls);
            statuses = observed;
            data
        }
        AnyPage::Firefox(page) => {
            let captured = if let Some(capture) = &mut capture {
                operation(
                    "diagnostic_capture",
                    config.timeout,
                    capture.finish(),
                    &mut ops,
                )
                .await
            } else {
                None
            };
            if let Some(data) = &captured {
                if data["complete"] != true {
                    ops.push(json!({"operation":"diagnostic_completeness","status":"failed","error":"events lost or diagnostic event cap exceeded"}));
                }
                for event in data["events"].as_array().into_iter().flatten() {
                    let p = &event["params"];
                    if p["context"] == page.context_id() && !p["navigation"].is_null() {
                        if let Some(status) = p["response"]["status"].as_i64() {
                            statuses.push(status);
                        }
                    }
                }
            }
            let events = captured
                .as_ref()
                .and_then(|data| data["events"].as_array())
                .cloned()
                .unwrap_or_default();
            let logs = events.iter().filter(|e| e["method"] == "log.entryAdded");
            let console = logs
                .clone()
                .filter(|e| e["params"]["type"] == "console")
                .map(|e| e["params"].clone())
                .collect::<Vec<_>>();
            let javascript_errors = logs
                .filter(|e| e["params"]["type"] == "javascript")
                .map(|e| e["params"].clone())
                .collect::<Vec<_>>();
            let navigations = events
                .iter()
                .filter(|e| {
                    e["method"]
                        .as_str()
                        .is_some_and(|m| m.starts_with("browsingContext."))
                })
                .cloned()
                .collect::<Vec<_>>();
            json!({"capture":captured,"console":console,"javascript_errors":javascript_errors,"navigations":navigations,
                "network_snapshot_scope":"root context only; scoped raw events also include descendants",
                "network":page.network_requests().iter().map(|r|json!({"request_id":r.request_id,"url":r.url,"method":r.method,"status":r.status,"status_text":r.status_text,"mime_type":r.mime_type,"failure":r.failure})).collect::<Vec<_>>()})
        }
    };
    if let (Engine::Firefox(browser), Some(capture)) = (browser, capture.as_ref()) {
        operation(
            "diagnostic_unsubscribe",
            config.timeout,
            browser.session().connection().send(
                "session.unsubscribe",
                json!({"subscriptions":[capture.subscription]}),
            ),
            &mut ops,
        )
        .await;
    }
    operation("page_close", config.timeout, page.close(), &mut ops).await;
    json!({"requested_url":url,"final_url":final_url,"title":title,"screenshot_requested_path":screenshot,"screenshot":if screenshot_written {json!(screenshot)} else {Value::Null},
        "observed_document_statuses":statuses,"document_status_attribution":match config.backend {
            Backend::Chrome=>"URL-matched Document candidates; same-URL iframe attribution is ambiguous",
            Backend::Firefox=>"root browsing context with navigation ID"
        },"document_status_selection":match config.backend {
            Backend::Chrome=>"Document requests matching requested/final/main-frame navigation URLs; public network snapshots lack frame IDs and redirect history",
            Backend::Firefox=>"scoped network response events with root context and a navigation ID; repeated lifecycle events can repeat a status"
        },"observation":observation(final_url.as_deref(),&statuses),
        "operations_succeeded":all_passed(&ops),"operations":ops,"diagnostics":diagnostics})
}

struct ProfileGuard(Option<PathBuf>);
impl ProfileGuard {
    fn cleanup(&mut self) -> std::result::Result<(), std::io::Error> {
        if let Some(path) = &self.0 {
            if path.exists() {
                std::fs::remove_dir_all(path)?;
            }
        }
        self.0 = None;
        Ok(())
    }
}
impl Drop for ProfileGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
fn save(path: &Path, value: &Value) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}
async fn run(config: Config) -> std::result::Result<bool, String> {
    if config.output.exists()
        && std::fs::read_dir(&config.output)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err("output directory is not empty; choose a new directory".into());
    }
    std::fs::create_dir_all(&config.output).map_err(|e| e.to_string())?;
    let report_path = config.output.join("report.json");
    let profile = config
        .profile
        .clone()
        .unwrap_or_else(|| config.output.join("firefox-profile"));
    let mut profile_guard = ProfileGuard(
        (config.backend == Backend::Firefox && config.profile.is_none()).then(|| profile.clone()),
    );
    let mut report = json!({"schema_version":1,"status":"running","backend":config.backend.name(),
        "started_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_millis(),
        "headed_requested":config.headed,"mode_verification":"requested launch flags and environment preflight; native window visibility is not probed","requested_profile":config.profile,"firefox_profile":if config.backend==Backend::Firefox {json!(profile)} else {Value::Null},"persistent_profile_requested":config.profile.is_some(),
        "proxy":config.proxy.as_ref().map(|p|p.url.as_str()),"proxy_policy":"explicit proxy only; localhost/loopback bypass; no credentials stored",
        "certificate_policy":if config.proxy.is_some() && config.backend==Backend::Firefox {"Firefox enterprise system CA roots; certificate verification enabled"} else {"browser default certificate verification enabled"},
        "viewport":{"width":config.width,"height":config.height},"timeout_seconds":config.timeout.as_secs(),
        "idle_timeout_seconds":config.idle_timeout.as_secs(),"platform":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH},
        "sites":[],"operations":[],"purpose":"read-only site observations; HTTP denial is not a driver-defect attribution",
        "diagnostic_limits":"event snapshots end before page close; Chrome backend has no public lost-event counter; Firefox cap/lag is explicit; a timed-out subscription can remain active until page/browser close"});
    save(&report_path, &report)?;
    let mut operations = Vec::new();
    if config.backend == Backend::Firefox {
        let prepared = operation(
            "profile_prepare",
            config.timeout,
            async {
                if let Some(proxy) = &config.proxy {
                    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
                    std::fs::write(profile.join("user.js"), proxy.firefox_preferences())
                        .map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(())
            },
            &mut operations,
        )
        .await;
        if prepared.is_none() {
            operation(
                "profile_cleanup",
                config.timeout,
                async { profile_guard.cleanup() },
                &mut operations,
            )
            .await;
            report["operations"] = json!(operations);
            report["status"] = json!("operation_failed");
            save(&report_path, &report)?;
            return Ok(false);
        }
    }
    let Some(browser) = operation(
        "browser_launch",
        config.timeout,
        Engine::launch(&config, &profile),
        &mut operations,
    )
    .await
    else {
        operation(
            "profile_cleanup",
            config.timeout,
            async { profile_guard.cleanup() },
            &mut operations,
        )
        .await;
        report["operations"] = json!(operations);
        report["status"] = json!("operation_failed");
        save(&report_path, &report)?;
        return Ok(false);
    };
    report["browser"] = browser.diagnostics();
    let mut sites = Vec::new();
    for (index, url) in config.sites.iter().enumerate() {
        let result = site(&browser, &config, index, url).await;
        println!(
            "{}: {} (operations {})",
            url,
            result["observation"],
            if result["operations_succeeded"] == true {
                "passed"
            } else {
                "failed"
            }
        );
        sites.push(result);
        report["sites"] = json!(sites);
        save(&report_path, &report)?;
    }
    let closed = operation(
        "browser_close",
        config.timeout.max(Duration::from_secs(6)),
        browser.close(),
        &mut operations,
    )
    .await;
    if closed.is_some() {
        operation(
            "profile_cleanup",
            config.timeout,
            async { profile_guard.cleanup() },
            &mut operations,
        )
        .await;
    } else {
        report["retained_profile_after_failed_shutdown"] = json!(profile_guard.0.take());
    }
    let success =
        all_passed(&operations) && sites.iter().all(|s| s["operations_succeeded"] == true);
    report["operations"] = json!(operations);
    report["status"] = json!(if success {
        "completed"
    } else {
        "operation_failed"
    });
    save(&report_path, &report)?;
    println!("Report: {}", report_path.display());
    Ok(success)
}
#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::parse(std::env::args().skip(1)) {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("{error}\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match run(config).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(args: &[&str]) -> std::result::Result<Option<Config>, String> {
        Config::parse(args.iter().map(|s| (*s).to_owned()))
    }
    #[test]
    fn cli_rejects_missing_values_unknown_flags_and_invalid_modes() {
        for args in [
            vec!["--browser"],
            vec!["--output"],
            vec!["--profile", "--headed"],
            vec!["--browser", "webkit"],
            vec!["--headed", "--headless"],
            vec!["--unknown"],
            vec!["--viewport", "0x800"],
            vec!["--viewport", "1280"],
            vec!["--timeout-secs", "0"],
            vec!["--idle-timeout-secs", "301"],
            vec!["httpbad"],
            vec!["http:///missing"],
            vec!["https://user:pass@example.com"],
        ] {
            assert!(config(&args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn cli_selects_firefox_and_preserves_all_requested_sites() {
        let cfg = config(&[
            "--browser",
            "firefox",
            "--headed",
            "--output",
            "target/report",
            "--viewport",
            "900x700",
            "https://example.com/",
            "http://127.0.0.1:8080/",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(cfg.backend, Backend::Firefox);
        assert!(cfg.headed);
        assert_eq!((cfg.width, cfg.height), (900, 700));
        assert_eq!(cfg.sites.len(), 2);
        assert!(config(&["--help"]).unwrap().is_none());
    }
    #[test]
    fn proxy_validation_preserves_cert_checks_and_user_profiles() {
        for value in [
            "https://proxy:8080",
            "http://user:pass@proxy:8080",
            "http://proxy:0",
            "http://proxy:70000",
            "http://proxy:8080/path",
            "http://proxy",
        ] {
            assert!(Proxy::parse(value).is_err(), "{value}");
        }
        let proxy = Proxy::parse("http://proxy:8080/").unwrap();
        assert_eq!(proxy.url, "http://proxy:8080");
        assert_eq!(Proxy::parse("http://[::1]:8080").unwrap().host, "::1");
        assert!(proxy
            .firefox_preferences()
            .contains("security.enterprise_roots.enabled"));
        assert!(config(&[
            "--browser",
            "firefox",
            "--profile",
            "my-profile",
            "--proxy",
            "http://proxy:8080"
        ])
        .is_err());
    }
    #[test]
    fn http_denial_and_error_pages_are_not_successful_document_observations() {
        assert_eq!(
            observation(Some("https://example.com"), &[403]),
            "http_access_denied"
        );
        assert_eq!(
            observation(Some("https://example.com"), &[429]),
            "http_access_denied"
        );
        assert_eq!(
            observation(Some("https://example.com"), &[503]),
            "http_error_response"
        );
        assert_eq!(
            observation(Some("chrome-error://chromewebdata/"), &[]),
            "browser_error_page"
        );
        assert_eq!(
            observation(Some("https://example.com"), &[]),
            "document_status_unknown"
        );
        assert!(!all_passed(&[
            json!({"status":"passed"}),
            json!({"status":"failed"})
        ]));
    }
}
