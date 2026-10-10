//! Backend-agnostic [`PageApi`] / [`LocatorApi`] implementations for the CDP
//! backend, so the same generic code can drive Chrome and Firefox.

use std::path::Path;
use std::time::Duration;

use rustwright_common::{LocatorApi, PageApi, Selector, WaitState};
use serde_json::Value;

use crate::error::Error;
use crate::locator::Locator;
use crate::page::{Page, Viewport};

impl PageApi for Page {
    type Error = Error;
    type Locator = Locator;

    async fn goto(&self, url: &str) -> Result<(), Self::Error> {
        Page::goto(self, url).await
    }

    async fn url(&self) -> Result<String, Self::Error> {
        Ok(Page::url(self))
    }

    async fn title(&self) -> Result<String, Self::Error> {
        Page::title(self).await
    }

    async fn content(&self) -> Result<String, Self::Error> {
        Page::content(self).await
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, Self::Error> {
        Page::evaluate(self, expression).await
    }

    async fn screenshot(&self, path: &Path) -> Result<(), Self::Error> {
        Page::screenshot(self, path).await
    }

    async fn set_viewport(
        &self,
        width: i64,
        height: i64,
        device_pixel_ratio: f64,
    ) -> Result<(), Self::Error> {
        Page::set_viewport(
            self,
            &Viewport {
                width,
                height,
                device_scale_factor: device_pixel_ratio,
                mobile: false,
            },
        )
        .await
    }

    async fn close(&self) -> Result<(), Self::Error> {
        Page::close(self).await
    }

    fn locator(&self, selector: Selector) -> Self::Locator {
        Page::locator(self, selector)
    }
}

impl LocatorApi for Locator {
    type Error = Error;

    async fn click(&self) -> Result<(), Self::Error> {
        Locator::click(self).await
    }

    async fn fill(&self, text: &str) -> Result<(), Self::Error> {
        Locator::fill(self, text).await
    }

    async fn text(&self) -> Result<String, Self::Error> {
        Locator::text(self).await
    }

    async fn text_content(&self) -> Result<String, Self::Error> {
        Locator::text_content(self).await
    }

    async fn is_visible(&self) -> Result<bool, Self::Error> {
        Locator::is_visible(self).await
    }

    async fn is_hidden(&self) -> Result<bool, Self::Error> {
        Locator::is_hidden(self).await
    }

    async fn is_enabled(&self) -> Result<bool, Self::Error> {
        Locator::is_enabled(self).await
    }

    async fn count(&self) -> Result<usize, Self::Error> {
        Locator::count(self).await
    }

    async fn get_attribute(&self, name: &str) -> Result<Option<String>, Self::Error> {
        Locator::get_attribute(self, name).await
    }

    async fn hover(&self) -> Result<(), Self::Error> {
        Locator::hover(self).await
    }

    async fn scroll_into_view_if_needed(&self) -> Result<(), Self::Error> {
        Locator::scroll_into_view_if_needed(self).await
    }

    async fn wait_for(&self, state: WaitState) -> Result<(), Self::Error> {
        Locator::wait_for(self, state).await
    }

    async fn wait_for_with_timeout(
        &self,
        state: WaitState,
        timeout: Duration,
    ) -> Result<(), Self::Error> {
        Locator::wait_for_with_timeout(self, state, timeout).await
    }

    fn first(&self) -> Self {
        Locator::first(self)
    }

    fn last(&self) -> Self {
        Locator::last(self)
    }

    fn nth(&self, index: i64) -> Self {
        Locator::nth(self, index)
    }
}
