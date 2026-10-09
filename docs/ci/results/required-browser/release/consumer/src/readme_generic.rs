use rustwright::prelude::*;

pub async fn fill_and_read<P>(page: &P, url: &str, text: &str) -> std::result::Result<String, P::Error>
where
    P: PageApi,
    P::Locator: LocatorApi<Error = P::Error>,
{
    page.goto(url).await?;
    page.locator(Selector::css("input[name=q]")).fill(text).await?;
    let value = page
        .evaluate("document.querySelector('input[name=q]').value")
        .await?;
    Ok(value.as_str().unwrap_or_default().to_string())
}
