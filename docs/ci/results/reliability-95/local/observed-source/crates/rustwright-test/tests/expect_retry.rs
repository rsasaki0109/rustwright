//! Assertion retry tests against a local HTTP page.

use std::future::Future;
use std::time::Duration;

use rustwright_test::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::Instant;

struct HttpFixture {
    url: String,
    task: JoinHandle<()>,
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture() -> HttpFixture {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let url = format!(
        "http://{}/",
        listener.local_addr().expect("fixture address")
    );
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.expect("accept fixture request");
                    requests.spawn(async move {
                        let mut buffer = [0; 2048];
                        if !matches!(tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buffer)).await, Ok(Ok(n)) if n > 0) {
                            return;
                        }
                        if buffer.starts_with(b"GET /slow ") {
                            tokio::time::sleep(Duration::from_millis(150)).await;
                        }
                        let body = "<!DOCTYPE html><title>Assertions</title><h1 id=status>Loading</h1><ul><li>one</li></ul><button id=button disabled>Go</button>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        socket.write_all(response.as_bytes()).await.expect("write fixture");
                    });
                }
                _ = requests.join_next(), if !requests.is_empty() => {}
            }
        }
    });
    HttpFixture { url, task }
}

#[rustwright_test]
async fn waits_for_delayed_count(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    assert_eq!(context.page.locator("li").count().await?, 1);
    context.page.evaluate("setTimeout(() => document.querySelector('ul').insertAdjacentHTML('beforeend', '<li>two</li><li>three</li>'), 200)").await?;
    expect(context.page.locator("li")).to_have_count(3).await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_delayed_text(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context
        .page
        .evaluate("setTimeout(() => document.querySelector('h1').textContent = 'Ready', 200)")
        .await?;
    expect(context.page.locator("h1"))
        .to_have_text("Ready")
        .await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_delayed_contained_text(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context
        .page
        .evaluate("setTimeout(() => document.querySelector('h1').textContent = 'Ready to go', 200)")
        .await?;
    expect(context.page.locator("h1"))
        .with_timeout(Duration::from_secs(1))
        .to_contain_text("Ready")
        .await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_delayed_attribute(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context.page.evaluate("setTimeout(() => document.querySelector('h1').setAttribute('data-state', 'ready'), 200)").await?;
    expect(context.page.locator("h1"))
        .with_timeout(Duration::from_secs(1))
        .to_have_attribute("data-state", "ready")
        .await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_visibility_changes(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context.page.evaluate("document.querySelector('h1').style.display = 'none'; setTimeout(() => document.querySelector('h1').style.display = '', 200)").await?;
    expect(context.page.locator("h1"))
        .with_timeout(Duration::from_secs(1))
        .to_be_visible()
        .await?;
    context
        .page
        .evaluate("setTimeout(() => document.querySelector('h1').remove(), 200)")
        .await?;
    expect(context.page.locator("h1"))
        .with_timeout(Duration::from_secs(1))
        .to_be_hidden()
        .await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_enabled_button(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context
        .page
        .evaluate("setTimeout(() => document.querySelector('button').disabled = false, 200)")
        .await?;
    expect(context.page.locator("button"))
        .with_timeout(Duration::from_secs(1))
        .to_be_enabled()
        .await?;
    Ok(())
}

#[rustwright_test]
async fn waits_for_element_to_appear(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context.page.evaluate("setTimeout(() => document.body.insertAdjacentHTML('beforeend', '<div id=late>Late</div>'), 200)").await?;
    expect(context.page.locator("#late"))
        .with_timeout(Duration::from_secs(1))
        .to_have_text("Late")
        .await?;
    Ok(())
}

#[tokio::test]
async fn waits_for_a_frame_execution_context_before_counting() {
    let fixture = fixture().await;
    let browser = rustwright::Browser::launch(rustwright::Chrome::installed().headless(true))
        .await
        .expect("launch browser");
    let page = browser.new_page().await.expect("open page");
    page.goto(&fixture.url).await.expect("load fixture");
    page.evaluate("const frame = document.createElement('iframe'); frame.id = 'slow'; frame.src = '/slow'; document.body.append(frame)")
        .await.expect("insert slow frame");
    let locator = rustwright::AnyLocator::Chrome(page.frame_locator("#slow").locator("#button"));
    expect(locator)
        .with_timeout(Duration::from_secs(1))
        .to_have_count(1)
        .await
        .expect("retry missing frame context");
    browser.close().await.expect("close browser");
}

async fn assertion_failure<F>(assertion: F) -> String
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    let mut task = tokio::spawn(assertion);
    let outcome = match tokio::time::timeout(Duration::from_secs(2), &mut task).await {
        Ok(outcome) => outcome,
        Err(_) => {
            task.abort();
            panic!("assertion exceeded its short timeout");
        }
    };
    let error = outcome.expect_err("assertion should panic");
    assert!(error.is_panic(), "assertion task should not be cancelled");
    *error
        .into_panic()
        .downcast::<String>()
        .expect("panic message")
}

#[rustwright_test]
async fn timeout_reports_last_observed_count(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    context.page.evaluate("setTimeout(() => document.querySelector('ul').insertAdjacentHTML('beforeend', '<li>two</li>'), 100)").await?;
    let expectation = expect(context.page.locator("li")).with_timeout(Duration::from_millis(300));
    let start = Instant::now();
    let message = assertion_failure(async move { expectation.to_have_count(3).await }).await;
    assert!(start.elapsed() >= Duration::from_millis(300));
    assert!(message.contains("css=li to have count 3"), "{message}");
    assert!(message.contains("timed out after 300ms"), "{message}");
    assert!(message.contains("got 2"), "{message}");
    Ok(())
}

#[rustwright_test]
async fn assertion_deadline_bounds_missing_element_reads(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    let expectation =
        expect(context.page.locator("#missing")).with_timeout(Duration::from_millis(100));
    let message = assertion_failure(async move { expectation.to_have_text("Ready").await }).await;
    assert!(message.contains("css=#missing to have text"), "{message}");
    assert!(message.contains("timed out after 100ms"), "{message}");
    let expectation =
        expect(context.page.locator("#missing")).with_timeout(Duration::from_millis(100));
    let message =
        assertion_failure(async move { expectation.to_have_attribute("state", "ready").await })
            .await;
    assert!(
        message.contains("css=#missing to have attribute"),
        "{message}"
    );
    assert!(message.contains("timed out after 100ms"), "{message}");
    // Absent elements must not satisfy even empty-text assertions.
    let expectation =
        expect(context.page.locator("#missing")).with_timeout(Duration::from_millis(100));
    let message = assertion_failure(async move { expectation.to_contain_text("").await }).await;
    assert!(message.contains("timed out after 100ms"), "{message}");
    Ok(())
}

#[rustwright_test]
async fn backend_errors_are_returned_without_retry(context: TestContext) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    let expectation = expect(context.page.locator("li")).with_timeout(Duration::from_secs(5));
    context.page.close().await?;
    let result = tokio::time::timeout(Duration::from_secs(1), expectation.to_have_count(3))
        .await
        .expect("closed page error should be immediate");
    assert!(result
        .expect_err("closed page should fail")
        .to_string()
        .contains("closed"));
    Ok(())
}

#[rustwright_test]
async fn matching_conditions_return_without_waiting_for_timeout(
    context: TestContext,
) -> Result<()> {
    let fixture = fixture().await;
    context.page.goto(&fixture.url).await?;
    tokio::time::timeout(Duration::from_secs(1), async {
        expect(context.page.locator("li")).to_have_count(1).await?;
        expect(context.page.locator("#missing"))
            .to_have_count(0)
            .await?;
        expect(context.page.locator("#missing"))
            .to_be_hidden()
            .await?;
        expect(context.page.locator("h1"))
            .to_have_text("Loading")
            .await?;
        Ok::<(), rustwright_test::AnyError>(())
    })
    .await
    .expect("matching assertions should not wait five seconds")?;
    Ok(())
}
