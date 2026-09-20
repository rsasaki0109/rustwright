//! Compatibility report for real-world SPA/EC/SNS sites.
//!
//! Loads each URL in an installed Chrome and prints what a normal browser would
//! experience: final URL, navigation chain, console errors, page errors, failed
//! requests and non-2xx document responses, plus a screenshot. It performs no
//! site-specific workarounds; it exists to make environment differences
//! observable.
//!
//! ```sh
//! cargo run -p rustwright-examples --example compat_report
//! cargo run -p rustwright-examples --example compat_report -- --headed --profile ./target/profile
//! cargo run -p rustwright-examples --example compat_report -- https://example.com
//! ```

use std::path::PathBuf;
use std::time::Duration;

use rustwright::prelude::*;

const DEFAULT_SITES: &[(&str, &str)] = &[
    ("mercari", "https://jp.mercari.com/"),
    ("rakuma", "https://fril.jp/"),
    ("x", "https://x.com/"),
    ("youtube", "https://www.youtube.com/"),
    ("tiktok", "https://www.tiktok.com/"),
    ("instagram", "https://www.instagram.com/"),
];

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_args();
    let mut chrome = Chrome::installed().headless(!config.headed);
    if let Some(profile) = &config.profile {
        chrome = chrome.profile(profile);
    }

    println!(
        "launching {}...",
        if config.headed {
            "headed Chrome"
        } else {
            "headless Chrome"
        }
    );
    let browser = Browser::launch(chrome).await?;
    println!("browser: {}", browser.version().browser);
    browser
        .default_context()
        .set_viewport(config.viewport)
        .await?;

    let mut results = Vec::new();
    for (name, url) in &config.sites {
        println!("\n=== {name} <{url}> ===");
        results.push(report_site(&browser, name, url).await);
    }

    println!("\n=== summary ===");
    for result in &results {
        println!(
            "{:<10} {:<8} {} | {}",
            result.name,
            if result.ok { "OK" } else { "ERROR" },
            result.title,
            result.note
        );
    }

    browser.close().await?;
    Ok(())
}

struct Config {
    headed: bool,
    profile: Option<PathBuf>,
    viewport: Viewport,
    sites: Vec<(String, String)>,
}

impl Config {
    fn from_args() -> Self {
        let mut config = Config {
            headed: false,
            profile: None,
            viewport: Viewport::new(1280, 800),
            sites: Vec::new(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headed" => config.headed = true,
                "--profile" => config.profile = args.next().map(PathBuf::from),
                "--viewport" => {
                    if let Some(value) = args.next() {
                        if let Some((width, height)) = parse_viewport(&value) {
                            config.viewport = Viewport::new(width, height);
                        }
                    }
                }
                other if other.starts_with("http") => {
                    config.sites.push((host_of(other), other.to_string()));
                }
                other => eprintln!("ignoring unknown argument: {other}"),
            }
        }
        if config.sites.is_empty() {
            config.sites = DEFAULT_SITES
                .iter()
                .map(|(name, url)| (name.to_string(), url.to_string()))
                .collect();
        }
        config
    }
}

struct SiteResult {
    name: String,
    ok: bool,
    title: String,
    note: String,
}

async fn report_site(browser: &Browser, name: &str, url: &str) -> SiteResult {
    let page = match browser.new_page().await {
        Ok(page) => page,
        Err(error) => {
            return SiteResult {
                name: name.to_string(),
                ok: false,
                title: String::new(),
                note: format!("new_page failed: {error}"),
            }
        }
    };

    let navigation = page.goto_with_timeout(url, Duration::from_secs(45)).await;
    if let Err(error) = &navigation {
        println!("  navigation: ERROR {error}");
    } else {
        println!("  navigation: ok");
    }
    // Best-effort: give SPA frameworks a chance to settle without hanging.
    let _ = page
        .wait_for_load_state_with_timeout(LoadState::NetworkIdle, Duration::from_secs(10))
        .await;

    let final_url = page.url();
    let title = page.title().await.unwrap_or_default();
    let error_page = final_url.starts_with("chrome-error://");
    println!("  final url:  {final_url}");
    println!("  title:      {title}");

    let navigations = page.navigations();
    if navigations.len() > 1 {
        println!("  navigations ({}):", navigations.len());
        for navigation in navigations.iter().take(10) {
            println!("    - {}", navigation.url);
        }
    }

    let console = page.console_messages();
    let console_errors: Vec<_> = console
        .iter()
        .filter(|message| message.level == "error")
        .collect();
    println!(
        "  console:    {} messages, {} errors",
        console.len(),
        console_errors.len()
    );
    for message in console_errors.iter().take(5) {
        println!("    ! {}", truncate(&message.text, 160));
    }

    let errors = page.errors();
    println!("  js errors:  {}", errors.len());
    for error in errors.iter().take(5) {
        println!("    ! {}", truncate(&error.message, 160));
    }

    let requests = page.network_requests();
    let failed: Vec<_> = requests
        .iter()
        .filter(|request| request.failure.is_some())
        .collect();
    let bad_status: Vec<_> = requests
        .iter()
        .filter(|request| {
            request.resource_type == "Document"
                && request
                    .status
                    .is_some_and(|status| !(200..400).contains(&status))
        })
        .collect();
    println!(
        "  network:    {} requests, {} failed, {} bad document status",
        requests.len(),
        failed.len(),
        bad_status.len()
    );
    for request in failed.iter().take(5) {
        println!(
            "    ! {} -> {}",
            truncate(&request.url, 100),
            request.failure.clone().unwrap_or_default()
        );
    }
    for request in bad_status.iter().take(5) {
        println!(
            "    ! {} -> {}",
            truncate(&request.url, 100),
            request.status.unwrap_or_default()
        );
    }

    let dialogs = page.dialogs();
    if !dialogs.is_empty() {
        println!("  dialogs:    {} (auto-dismissed)", dialogs.len());
    }

    let screenshot = PathBuf::from("target/compat").join(format!("{name}.png"));
    if page.screenshot(&screenshot).await.is_ok() {
        println!("  screenshot: {}", screenshot.display());
    }

    let _ = page.close().await;

    let ok = navigation.is_ok() && !error_page;
    let note = if ok {
        format!("{} requests", requests.len())
    } else if error_page {
        format!("chrome error page at {final_url}")
    } else {
        format!(
            "{}; {} console errors; {} js errors",
            truncate(&navigation.unwrap_err().to_string(), 100),
            console_errors.len(),
            errors.len()
        )
    };
    SiteResult {
        name: name.to_string(),
        ok,
        title: truncate(&title, 40),
        note,
    }
}

fn parse_viewport(value: &str) -> Option<(i64, i64)> {
    let (width, height) = value.split_once(['x', 'X'])?;
    Some((width.trim().parse().ok()?, height.trim().parse().ok()?))
}

fn host_of(url: &str) -> String {
    let without_scheme = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    without_scheme
        .split('/')
        .next()
        .unwrap_or("site")
        .replace('.', "_")
        .to_string()
}

fn truncate(value: &str, max: usize) -> String {
    let mut result: String = value.chars().take(max).collect();
    if value.chars().count() > max {
        result.push('…');
    }
    result.replace(['\n', '\r'], " ")
}
