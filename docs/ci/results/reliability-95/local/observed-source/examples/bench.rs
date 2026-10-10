//! Micro-benchmark used for the README performance comparison.
//!
//! It measures driver overhead, not browser speed: launch-to-first-page,
//! per-navigation latency, per-`evaluate` round-trip, and the driver process's
//! peak RSS (`VmHWM`). The spawned browser's memory is deliberately excluded.
//!
//! ```sh
//! cargo run --release -p rustwright-examples --example bench
//! RUSTWRIGHT_BENCH_GOTOS=50 RUSTWRIGHT_BENCH_EVALS=5000 \
//!   cargo run --release -p rustwright-examples --example bench
//! ```
//!
//! Environment variables:
//! - `RUSTWRIGHT_BENCH_URL` (default: a small `data:` page)
//! - `RUSTWRIGHT_BENCH_GOTOS` (default 30)
//! - `RUSTWRIGHT_BENCH_EVALS` (default 2000)

use std::time::Instant;

use rustwright::prelude::*;

const DEFAULT_URL: &str =
    "data:text/html,<title>bench</title><h1 id=title>bench</h1><input id=q><button id=go>Go</button><div id=out>ok</div>";

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("RUSTWRIGHT_BENCH_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let gotos: u32 = env_number("RUSTWRIGHT_BENCH_GOTOS", 30);
    let evals: u32 = env_number("RUSTWRIGHT_BENCH_EVALS", 2000);

    let t0 = Instant::now();
    let browser = Browser::launch(Chrome::installed().headless(true)).await?;
    let page = browser.new_page().await?;
    page.goto(&url).await?;
    let launch_ms = t0.elapsed().as_secs_f64() * 1000.0;

    for _ in 0..5 {
        page.goto(&url).await?;
    }

    let g0 = Instant::now();
    for _ in 0..gotos {
        page.goto(&url).await?;
    }
    let goto_avg = g0.elapsed().as_secs_f64() * 1000.0 / f64::from(gotos);

    for _ in 0..100 {
        page.evaluate("1+1").await?;
    }
    let e0 = Instant::now();
    for _ in 0..evals {
        page.evaluate("1+1").await?;
    }
    let eval_avg = e0.elapsed().as_secs_f64() * 1000.0 / f64::from(evals);

    let version = browser.version().browser.clone();
    println!(
        "{{\"engine\": \"rustwright\", \"browser\": {version:?}, \"launch_ms\": {launch_ms:.1}, \"goto_avg_ms\": {goto_avg:.3}, \"eval_avg_ms\": {eval_avg:.4}, \"vm_hwm_kb\": {}, \"gotos\": {gotos}, \"evals\": {evals}}}",
        read_vm_hwm_kb()
    );

    browser.close().await?;
    Ok(())
}

fn env_number(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn read_vm_hwm_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmHWM:").map(str::to_string))
        })
        .and_then(|value| {
            value
                .trim()
                .trim_end_matches(" kB")
                .trim()
                .parse::<u64>()
                .ok()
        })
        .unwrap_or(0)
}
