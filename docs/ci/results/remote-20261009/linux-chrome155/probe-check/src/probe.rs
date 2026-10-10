//! Failure-only CDP diagnostics; the original navigation error remains fatal.
use rustwright::{cdp::CdpConnection, Browser, Page};
use serde_json::json;
use std::time::Duration;

pub async fn diagnose(browser: &Browser, page: &Page, url: &str) {
    // Include HTTP endpoint discovery and WebSocket connection in the bound.
    eprintln!(
        "Chrome failure probe completion: {:?}",
        tokio::time::timeout(Duration::from_secs(5), diagnose_inner(browser, page, url)).await
    );
}

async fn diagnose_inner(browser: &Browser, page: &Page, url: &str) {
    eprintln!(
        "Chrome original navigation request snapshot: {:?}",
        page.network_requests()
    );
    let diagnostics = browser.diagnostics();
    eprintln!("Chrome launch diagnostics: {diagnostics:?}");
    print_stderr(browser);
    let Some(endpoint) = diagnostics.endpoint else {
        return;
    };
    eprintln!("Chrome failure probe: endpoint discovery start");
    let Ok(ws_url) = rustwright::cdp::discover_ws_url(&endpoint).await else {
        return;
    };
    eprintln!("Chrome failure probe: WebSocket connect start");
    let Ok(connection) = CdpConnection::connect(&ws_url).await else {
        return;
    };
    let mut events = connection.subscribe();
    let probe = async {
        eprintln!("Chrome failure probe sending: Target.attachToTarget");
        let attached = connection
            .send_raw(
                None,
                "Target.attachToTarget",
                json!({"targetId":page.target_id(),"flatten":true}),
            )
            .await?;
        let Some(session) = attached["sessionId"].as_str() else {
            return Ok::<(), rustwright::cdp::CdpError>(());
        };
        for method in ["Page.enable", "Network.enable", "Runtime.enable"] {
            eprintln!("Chrome failure probe sending: {method}");
            connection
                .send_raw(Some(session), method, json!({}))
                .await?;
        }
        for (method, params) in [
            ("Page.getFrameTree", json!({})),
            (
                "Runtime.evaluate",
                json!({"expression":"JSON.stringify({url:location.href,ready:document.readyState,title:document.title,content:document.documentElement?.outerHTML.slice(0,1000)})","returnByValue":true}),
            ),
            ("Page.navigate", json!({"url":url})),
        ] {
            eprintln!("Chrome failure probe sending: {method}");
            let result = connection.send_raw(Some(session), method, params).await;
            eprintln!("Chrome failure probe {method}: {result:?}");
        }
        let deadline = tokio::time::Instant::now() + Duration::from_millis(750);
        while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
            if event.session_id.as_deref() == Some(session)
                && (event.method.starts_with("Network.") || event.method.starts_with("Page."))
            {
                eprintln!(
                    "Chrome failure probe event: {} {}",
                    event.method, event.params
                );
            }
        }
        Ok(())
    };
    eprintln!("Chrome failure probe protocol: {:?}", probe.await);
    connection.close();
}

fn print_stderr(browser: &Browser) {
    if let Some(log) = browser.diagnostics().browser_log {
        // Bound log output as well as protocol waiting on the failure path.
        use std::io::Read;
        let stderr = std::fs::File::open(log).and_then(|file| {
            let mut bytes = Vec::new();
            file.take(16 * 1024).read_to_end(&mut bytes)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        });
        eprintln!("Chrome stderr (first 16 KiB): {stderr:?}");
    }
}

/// Bound all page-creation diagnostics, including connection establishment.
pub async fn diagnose_creation(browser: &Browser) {
    eprintln!(
        "Chrome page-creation probe completion: {:?}",
        tokio::time::timeout(Duration::from_secs(5), creation_inner(browser)).await
    );
}

async fn creation_inner(browser: &Browser) {
    let diagnostics = browser.diagnostics();
    eprintln!("Chrome page-creation launch diagnostics: {diagnostics:?}");
    eprintln!(
        "Chrome page-creation pending commands: {}",
        browser.pending_command_count()
    );
    print_stderr(browser);
    let Some(endpoint) = diagnostics.endpoint else {
        return;
    };
    eprintln!("Chrome page-creation probe: endpoint discovery start");
    let Ok(ws_url) = rustwright::cdp::discover_ws_url(&endpoint).await else {
        return;
    };
    eprintln!("Chrome page-creation probe: WebSocket connect start");
    let Ok(connection) = CdpConnection::connect(&ws_url).await else {
        return;
    };
    eprintln!("Chrome page-creation probe sending: Target.getTargets");
    let targets = connection
        .send_raw(None, "Target.getTargets", json!({}))
        .await;
    eprintln!("Chrome page-creation probe Target.getTargets: {targets:?}");
    eprintln!("Chrome page-creation probe sending: Target.getBrowserContexts");
    let contexts = connection
        .send_raw(None, "Target.getBrowserContexts", json!({}))
        .await;
    eprintln!("Chrome page-creation probe Target.getBrowserContexts: {contexts:?}");
    if let Ok(targets) = targets {
        if let Some(infos) = targets["targetInfos"].as_array() {
            for target in infos
                .iter()
                .filter(|target| target["type"] == "page")
                .take(2)
            {
                let Some(target_id) = target["targetId"].as_str() else {
                    continue;
                };
                eprintln!(
                    "Chrome page-creation probe sending: Target.attachToTarget target={target_id}"
                );
                let attached = connection
                    .send_raw(
                        None,
                        "Target.attachToTarget",
                        json!({"targetId":target_id,"flatten":true}),
                    )
                    .await;
                eprintln!("Chrome page-creation probe Target.attachToTarget: {attached:?}");
                let Ok(attached) = attached else {
                    continue;
                };
                let Some(session) = attached["sessionId"].as_str() else {
                    continue;
                };
                // Repeat only renderer setup observations; do not allocate pages
                // or replace the failed original Rustwright operation.
                for (method, params) in [
                    ("Page.enable", json!({})),
                    ("Page.getFrameTree", json!({})),
                    ("Runtime.enable", json!({})),
                    ("DOM.enable", json!({})),
                    ("Network.enable", json!({})),
                    (
                        "Runtime.evaluate",
                        json!({"expression":"document.readyState","returnByValue":true}),
                    ),
                ] {
                    eprintln!("Chrome page-creation probe sending: {method} target={target_id}");
                    let result = connection.send_raw(Some(session), method, params).await;
                    eprintln!("Chrome page-creation probe {method}: {result:?}");
                }
            }
        }
    }
    connection.close();
}
