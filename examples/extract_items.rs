//! Extract listing items (title, price, link) from a search/listing page.
//!
//! This is *user code*, not core: it takes a URL and a CSS selector, then reads
//! the text and links. No site-specific workarounds are involved.
//!
//! ```sh
//! cargo run -p rustwright-examples --example extract_items -- \
//!   "https://jp.mercari.com/search?keyword=iphone" "a[href*='/item/']"
//! ```

use std::time::Duration;

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "https://jp.mercari.com/search?keyword=iphone".to_string());
    let selector = args
        .next()
        .unwrap_or_else(|| "a[href*='/item/']".to_string());

    let browser = Browser::launch(Chrome::installed().headless(true)).await?;
    let page = browser.new_page().await?;
    page.set_viewport(&Viewport::new(1280, 900)).await?;
    page.goto_with_timeout(&url, Duration::from_secs(60))
        .await?;
    let _ = page
        .wait_for_load_state_with_timeout(LoadState::NetworkIdle, Duration::from_secs(15))
        .await;

    // Nudge lazy-loaded results into the DOM.
    page.scroll_by(0.0, 1500.0).await?;
    page.wait_for_timeout(Duration::from_secs(2)).await;

    let script = format!(
        r#"(() => {{
            const nodes = [...document.querySelectorAll({selector:?})];
            const items = nodes.slice(0, 12).map((node) => {{
                const link = node.href || (node.querySelector('a') || {{}}).href || '';
                const text = (node.innerText || node.textContent || '').replace(/\s+/g, ' ').trim();
                const price = (text.match(/[¥￥]?\s?[\d,]+(?=\s*円?)/) || [''])[0].trim();
                return {{ link, text: text.slice(0, 90), price }};
            }});
            return JSON.stringify({{ count: nodes.length, items }});
        }})()"#
    );
    let data = page.evaluate(&script).await?;
    println!("{}", data.as_str().unwrap_or_default());

    browser.close().await?;
    Ok(())
}
