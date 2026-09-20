//! The same backend-agnostic code drives Chrome (CDP) and Firefox (BiDi)
//! through [`PageApi`] / [`LocatorApi`].

use rustwright::bidi::BidiBrowser;
use rustwright::prelude::*;

const PAGE: &str = "data:text/html,<title>unified</title>\
    <input name='q'>\
    <button id='go'>Go</button>\
    <div id='out'>waiting</div>";

fn chrome_available() -> bool {
    Chrome::installed().executable_path().is_ok()
}

fn firefox_available() -> bool {
    Firefox::installed().executable_path().is_ok()
}

/// Backend-agnostic: fill an input and read the value back by property.
async fn fill_and_read<P>(page: &P, text: &str) -> std::result::Result<String, P::Error>
where
    P: PageApi,
    P::Locator: LocatorApi<Error = P::Error>,
{
    page.goto(PAGE).await?;
    page.locator(Selector::css("input[name=q]"))
        .fill(text)
        .await?;
    let value = page
        .evaluate("document.querySelector('input[name=q]').value")
        .await?;
    Ok(value.as_str().unwrap_or_default().to_string())
}

/// Backend-agnostic: wire a handler, click by role, and wait for text.
async fn click_and_wait<P>(page: &P) -> std::result::Result<String, P::Error>
where
    P: PageApi,
    P::Locator: LocatorApi<Error = P::Error>,
{
    page.goto(PAGE).await?;
    page.evaluate(
        "document.getElementById('go').addEventListener('click', () => { \
           document.getElementById('out').textContent = 'clicked'; })",
    )
    .await?;
    page.get_by_role(Role::Button, Some("Go")).click().await?;
    page.get_by_text("clicked")
        .wait_for(WaitState::Visible)
        .await?;
    page.locator(Selector::css("#out")).text().await
}

#[tokio::test]
async fn same_code_on_chrome() {
    if !chrome_available() {
        return;
    }
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch chrome");
    let page = browser.new_page().await.expect("page");

    assert_eq!(
        fill_and_read(&page, "chrome").await.expect("fill"),
        "chrome"
    );
    assert_eq!(click_and_wait(&page).await.expect("click"), "clicked");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn same_code_on_firefox() {
    if !firefox_available() {
        return;
    }
    let browser = BidiBrowser::launch(Firefox::installed().headless(true))
        .await
        .expect("launch firefox");
    let page = browser.new_page().await.expect("page");

    assert_eq!(
        fill_and_read(&page, "firefox").await.expect("fill"),
        "firefox"
    );
    assert_eq!(click_and_wait(&page).await.expect("click"), "clicked");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn dynamic_backends_in_one_vec() {
    // `AnyPage` erases the backend, so mixed pages live in a single collection.
    let mut pages: Vec<AnyPage> = Vec::new();
    let mut chrome_browser: Option<Browser> = None;
    let mut firefox_browser: Option<BidiBrowser> = None;

    if chrome_available() {
        let browser = Browser::launch(Chrome::installed().headless(true))
            .await
            .expect("launch chrome");
        pages.push(browser.new_page().await.expect("chrome page").into());
        chrome_browser = Some(browser);
    }
    if firefox_available() {
        let browser = BidiBrowser::launch(Firefox::installed().headless(true))
            .await
            .expect("launch firefox");
        pages.push(browser.new_page().await.expect("firefox page").into());
        firefox_browser = Some(browser);
    }

    assert!(!pages.is_empty(), "at least one browser is available");

    for page in &pages {
        let backend = page.backend_name();
        assert_eq!(fill_and_read(page, backend).await.expect("fill"), backend);
    }

    if let Some(browser) = chrome_browser {
        let _ = browser.close().await;
    }
    if let Some(browser) = firefox_browser {
        let _ = browser.close().await;
    }
}
