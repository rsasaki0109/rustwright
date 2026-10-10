//! Local reliability comparison; run through bench/reliability/run.py.

use std::time::{Duration, Instant};

use rustwright::prelude::*;
use serde_json::{json, Value};

const LIMIT: Duration = Duration::from_secs(2);
const SETUP_LIMIT: Duration = Duration::from_secs(10);
const CASES: &[&str] = &[
    "delayed_click",
    "delayed_frame",
    "cross_site_frame",
    "cross_site_navigation",
    "disabled_click",
    "covered_click",
    "moving_click",
    "clipped_click",
    "rotated_clipped_click",
    "http_disconnect_recovery",
    "browser_disconnect",
    "mock_fetch",
    "mock_navigation",
    "mock_frame",
    "mock_return",
    "mock_clear",
];

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let base = std::env::var("RUSTWRIGHT_RELIABILITY_URL")?;
    let samples: usize = std::env::var("RUSTWRIGHT_RELIABILITY_SAMPLES")?.parse()?;
    let selection =
        std::env::var("RUSTWRIGHT_RELIABILITY_CASES").unwrap_or_else(|_| CASES.join(","));
    let cases: Vec<_> = selection.split(',').collect();
    if cases.iter().any(|case| !CASES.contains(case)) {
        return Err("unknown reliability case".into());
    }
    let executable = std::env::var_os("RUSTWRIGHT_BENCH_CHROME").ok_or("browser path required")?;
    let chrome = Chrome::at(executable).headless(true);
    let browser = Browser::launch(chrome.clone()).await?;
    let version = browser.version().browser.clone();
    let launch_args = browser.diagnostics().launch_args;
    let mut viewport_checks = 0usize;
    let diagnostics =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url).await?;
    let mut records = Vec::new();
    for &case in &cases {
        for index in 0..=samples {
            let owned = if case == "browser_disconnect" {
                Some(Browser::launch(chrome.clone()).await?)
            } else {
                None
            };
            let active = owned.as_ref().unwrap_or(&browser);
            let page = active
                .new_page()
                .await
                .map_err(|error| format!("{case}[{index}] setup page: {error}"))?;
            page.set_viewport(&Viewport::hd()).await?;
            verify(
                page.evaluate(
                    "({width:innerWidth,height:innerHeight,device_scale_factor:devicePixelRatio})",
                )
                .await?,
                json!({"width": 1280, "height": 720, "device_scale_factor": 1}),
            )?;
            viewport_checks += 1;
            page.goto_with_timeout(&format!("{base}/index.html"), SETUP_LIMIT)
                .await
                .map_err(|error| format!("{case}[{index}] initial navigation: {error}"))?;
            match case {
                "delayed_click" => {
                    page.evaluate("scheduleButton(120)").await?;
                }
                "delayed_frame" => {
                    page.evaluate("scheduleFrame(120)").await?;
                }
                "cross_site_frame" => {
                    let remote = format!("{}/frame.html", base.replace("127.0.0.1", "localhost"));
                    page.evaluate(&format!("scheduleFrame(120, {remote:?})"))
                        .await?;
                }
                "cross_site_navigation" => {
                    let remote =
                        format!("{}/swap-frame.html", base.replace("127.0.0.1", "localhost"));
                    page.evaluate(&format!("scheduleFrameSwap(80, {remote:?})"))
                        .await?;
                }
                "disabled_click" | "covered_click" | "moving_click" => {
                    let kind = case.strip_suffix("_click").expect("click scenario");
                    page.evaluate(&format!("prepareActionability({kind:?}, 120)"))
                        .await?;
                }
                "clipped_click" | "rotated_clipped_click" => {
                    let kind = case.strip_suffix("_click").expect("geometry scenario");
                    page.evaluate(&format!("prepareGeometry({kind:?})")).await?;
                }
                "mock_navigation" | "mock_frame" | "mock_return" | "mock_clear" => {
                    page.mock("/api/mock", 201, "text/plain", "mocked").await?;
                    if case == "mock_return" {
                        let remote = format!(
                            "{}/network-frame.html",
                            base.replace("127.0.0.1", "localhost")
                        );
                        navigate_network_frame(&page, &remote).await?;
                        let frame = page.frame_locator("#late-frame").resolve().await?;
                        verify(
                            frame.evaluate("window.networkResult").await?,
                            network_expected(true),
                        )?;
                        verify_remote_frame(&frame, &diagnostics).await?;
                    } else if case == "mock_clear" {
                        page.goto_with_timeout(&format!("{base}/network-frame.html"), SETUP_LIMIT)
                            .await?;
                        verify(
                            page.evaluate("window.networkResult").await?,
                            network_expected(true),
                        )?;
                    }
                }
                _ => {}
            }
            let start = Instant::now();
            let outcome =
                tokio::time::timeout(LIMIT, run_case(case, &page, active, &base, &diagnostics))
                    .await;
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let error = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some("operation exceeded 2000ms watchdog".to_string()),
            };
            records.push(json!({
                "case": case, "index": index, "warmup": index == 0,
                "ok": error.is_none(), "error": error, "elapsed_ms": elapsed_ms,
            }));
            if let Some(owned) = owned {
                owned.close().await?;
            } else {
                page.close().await?;
            }
        }
    }
    browser.close().await?;
    diagnostics.close();
    println!(
        "{}",
        json!({
            "engine": "rustwright", "browser": version, "records": records,
            "launch_policy": {
                "headless": true,
                "sandbox": true,
                "viewport": {"width": 1280, "height": 720, "device_scale_factor": 1, "mobile": false},
                "context_policy": "reused main browser context; fresh browser/context for disconnect",
                "transport": "localhost CDP port",
                "driver_defaults_differ": true,
                "browser_launch_args": launch_args,
                "viewport_checks": viewport_checks,
            }
        })
    );
    Ok(())
}

async fn run_case(
    case: &str,
    page: &Page,
    browser: &Browser,
    base: &str,
    diagnostics: &rustwright::cdp::CdpConnection,
) -> Result<()> {
    match case {
        "mock_fetch" => {
            page.mock("/api/mock", 201, "text/plain", "mocked").await?;
            let script = "Promise.all([fetch('/api/mock').then(async r => [r.status, await r.text()]), new Promise(resolve => {const r = new XMLHttpRequest(); r.open('GET','/api/mock'); r.onload = () => resolve([r.status,r.responseText]); r.onerror = r.onabort = () => resolve(['error','']); r.send();})]).then(responses => ({responses,nativeFetch:fetch.toString().includes('[native code]'),nativeXHR:XMLHttpRequest.prototype.open.toString().includes('[native code]')}))";
            verify(page.evaluate(script).await?, network_expected(true))
        }
        "mock_navigation" | "mock_clear" => {
            if case == "mock_clear" {
                page.clear_routes().await?;
            }
            page.goto_with_timeout(&format!("{base}/network-frame.html"), LIMIT)
                .await?;
            verify(
                page.evaluate("window.networkResult").await?,
                network_expected(case != "mock_clear"),
            )
        }
        "mock_frame" | "mock_return" => {
            let destination = if case == "mock_frame" {
                format!(
                    "{}/network-frame.html",
                    base.replace("127.0.0.1", "localhost")
                )
            } else {
                "/network-frame.html".to_string()
            };
            navigate_network_frame(page, &destination).await?;
            let frame = page.frame_locator("#late-frame").resolve().await?;
            verify(
                frame.evaluate("window.networkResult").await?,
                network_expected(true),
            )?;
            if case == "mock_frame" {
                verify_remote_frame(&frame, diagnostics).await?;
            }
            Ok(())
        }
        "disabled_click"
        | "covered_click"
        | "moving_click"
        | "clipped_click"
        | "rotated_clipped_click" => {
            page.locator("#target").click().await?;
            verify(
                page.evaluate(
                    "window.clicked && !window.clickedBeforeReady && window.wrongClicks === 0",
                )
                .await?,
                json!(true),
            )
        }
        "delayed_click" => {
            page.locator("#late").click().await?;
            verify(page.evaluate("window.clicked").await?, json!(true))
        }
        "delayed_frame" | "cross_site_frame" | "cross_site_navigation" => {
            let frame = page.frame_locator("#late-frame");
            let input = if case == "cross_site_navigation" {
                "#swap-value"
            } else {
                "#value"
            };
            frame.locator(input).fill("payload").await?;
            frame.locator("#submit").click().await?;
            // Read in the child to check the value, and wait for postMessage
            // delivery in the parent; an API success alone is not a pass.
            page.evaluate("new Promise(resolve => { if (window.frameValue !== null) resolve(); else window.addEventListener('message', () => resolve(), {once: true}); })").await?;
            verify(page.evaluate("window.frameValue").await?, json!("payload"))?;
            if case != "delayed_frame" {
                let resolved = frame.resolve().await?;
                let targets = diagnostics
                    .send_raw(None, "Target.getTargets", json!({}))
                    .await?;
                let remote = targets["targetInfos"].as_array().is_some_and(|targets| {
                    targets.iter().any(|target| {
                        target["type"] == "iframe" && target["targetId"] == resolved.frame_id()
                    })
                });
                verify(json!(remote), json!(true))?;
                verify(
                    resolved.evaluate("location.hostname").await?,
                    json!("localhost"),
                )?;
            }
            Ok(())
        }
        "http_disconnect_recovery" => {
            let result = page.evaluate("fetch('/disconnect').then(() => 'unexpected response').catch(error => error instanceof TypeError ? 'network-error' : String(error))").await?;
            verify(result, json!("network-error"))?;
            page.goto_with_timeout(&format!("{base}/index.html"), LIMIT)
                .await?;
            verify(page.evaluate("window.fixtureReady").await?, json!(true))
        }
        "browser_disconnect" => {
            let waiting = page.evaluate("window.pending = true; new Promise(() => {})");
            let disconnect = async {
                loop {
                    if page.evaluate("window.pending === true").await? == json!(true) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                browser.close().await
            };
            let (result, closed) = tokio::join!(waiting, disconnect);
            closed?;
            match result {
                Err(error) if error.is_closed() => Ok(()),
                result => Err(Error::JavaScript(format!(
                    "expected closed connection error, got {result:?}"
                ))),
            }
        }
        _ => unreachable!(),
    }
}

async fn navigate_network_frame(page: &Page, url: &str) -> Result<()> {
    page.evaluate(&format!("navigateNetworkFrame({url:?})"))
        .await?;
    Ok(())
}

async fn verify_remote_frame(
    frame: &Frame,
    diagnostics: &rustwright::cdp::CdpConnection,
) -> Result<()> {
    let targets = diagnostics
        .send_raw(None, "Target.getTargets", json!({}))
        .await?;
    let remote = targets["targetInfos"].as_array().is_some_and(|targets| {
        targets
            .iter()
            .any(|target| target["type"] == "iframe" && target["targetId"] == frame.frame_id())
    });
    verify(json!(remote), json!(true))?;
    verify(
        frame.evaluate("location.hostname").await?,
        json!("localhost"),
    )
}

fn network_expected(mocked: bool) -> Value {
    let (status, body) = if mocked {
        (201, "mocked")
    } else {
        (200, "network")
    };
    json!({"responses":[[status,body],[status,body]], "nativeFetch":true,"nativeXHR":true})
}

fn verify(actual: Value, expected: Value) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::JavaScript(format!(
            "postcondition: expected {expected}, got {actual}"
        )))
    }
}
