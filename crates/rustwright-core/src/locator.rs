//! Lazy, auto-waiting element locators.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::error::Result;
use crate::page::{Page, DEFAULT_TIMEOUT};
use rustwright_common::{Selector, WaitState};

/// How a locator is scoped to a frame, if at all.
#[derive(Clone)]
enum FrameScope {
    /// A known frame id.
    Id(String),
    /// An `<iframe>` element selector resolved on each use.
    Element(Selector),
}

/// A lazy reference to one or more elements on a [`Page`].
///
/// Locators do not resolve the element until an action is performed, so a
/// `Locator` can be created before the element exists and will still work once
/// it appears. Actions auto-wait for the element to be actionable.
#[derive(Clone)]
pub struct Locator {
    page: Page,
    selector: Selector,
    nth: Option<i64>,
    frame: Option<FrameScope>,
}

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Locator")
            .field("selector", &self.describe())
            .finish()
    }
}

impl Locator {
    pub(crate) fn new(page: Page, selector: Selector) -> Self {
        Self {
            page,
            selector,
            nth: None,
            frame: None,
        }
    }

    pub(crate) fn new_in_frame(page: Page, selector: Selector, frame_id: String) -> Self {
        Self {
            page,
            selector,
            nth: None,
            frame: Some(FrameScope::Id(frame_id)),
        }
    }

    pub(crate) fn new_in_frame_element(
        page: Page,
        selector: Selector,
        iframe_selector: Selector,
    ) -> Self {
        Self {
            page,
            selector,
            nth: None,
            frame: Some(FrameScope::Element(iframe_selector)),
        }
    }

    /// The selector strategy used by this locator.
    pub fn selector(&self) -> &Selector {
        &self.selector
    }

    /// The page this locator belongs to.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// A human-readable description, including any index.
    pub fn describe(&self) -> String {
        match self.nth {
            Some(index) => format!("{} nth={index}", self.selector.describe()),
            None => self.selector.describe(),
        }
    }

    async fn frame_id(&self) -> Result<Option<String>> {
        match &self.frame {
            None => Ok(None),
            Some(FrameScope::Id(id)) => Ok(Some(id.clone())),
            Some(FrameScope::Element(selector)) => {
                Ok(Some(self.page.frame_id_for_element(selector).await?))
            }
        }
    }

    /// The first matching element.
    pub fn first(&self) -> Locator {
        self.nth(0)
    }

    /// The last matching element.
    pub fn last(&self) -> Locator {
        self.nth(-1)
    }

    /// The `n`-th matching element. Negative indices count from the end.
    pub fn nth(&self, index: i64) -> Locator {
        let mut clone = self.clone();
        clone.nth = Some(index);
        clone
    }

    /// Every element matching the selector, as individual locators.
    ///
    /// The DOM is queried once; if it changes afterwards the returned locators
    /// re-resolve on use.
    pub async fn all(&self) -> Result<Vec<Locator>> {
        let count = self.count().await?;
        Ok((0..count as i64).map(|index| self.nth(index)).collect())
    }

    /// Click the element, waiting for it to become visible first.
    pub async fn click(&self) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .click_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await
    }

    /// Replace the element's value, waiting for it to become visible first.
    pub async fn fill(&self, text: impl AsRef<str>) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .fill_spec(
                &self.spec(),
                &self.describe(),
                text.as_ref(),
                frame_id.as_deref(),
            )
            .await
    }

    /// Focus the element and insert text without replacing its value.
    pub async fn type_text(&self, text: impl AsRef<str>) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .focus_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await?;
        self.page.keyboard_insert_text(text.as_ref()).await
    }

    /// Focus the element and press a key.
    pub async fn press(&self, key: &str) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .focus_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await?;
        self.page.press_key(key).await
    }

    /// Move the mouse over the element (reveals hover menus and tooltips).
    pub async fn hover(&self) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .hover_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await
    }

    /// Scroll the element into the viewport.
    pub async fn scroll_into_view_if_needed(&self) -> Result<()> {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .scroll_into_view_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await
    }

    /// Assign files to a file input.
    pub async fn set_input_files<I, P>(&self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<std::path::Path>,
    {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        let files: Vec<PathBuf> = files
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect();
        self.page
            .set_input_files_spec(&self.spec(), &self.describe(), frame_id.as_deref(), &files)
            .await
    }

    /// Ensure a checkbox/radio is checked.
    pub async fn check(&self) -> Result<()> {
        self.set_checked(true).await
    }

    /// Ensure a checkbox/radio is unchecked.
    pub async fn uncheck(&self) -> Result<()> {
        self.set_checked(false).await
    }

    async fn set_checked(&self, checked: bool) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        if self.is_checked().await? == checked {
            return Ok(());
        }
        let frame_id = self.frame_id().await?;
        self.page
            .click_spec(&self.spec(), &self.describe(), frame_id.as_deref())
            .await
    }

    /// Select an option in a `<select>` by value.
    pub async fn select_option(&self, value: impl AsRef<str>) -> Result<()> {
        self.wait_for(WaitState::Visible).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .select_option_spec(
                &self.spec(),
                &self.describe(),
                value.as_ref(),
                frame_id.as_deref(),
            )
            .await
    }

    /// The element's rendered (inner) text, trimmed.
    pub async fn text(&self) -> Result<String> {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .text_spec(&self.spec(), &self.describe(), true, frame_id.as_deref())
            .await
    }

    /// The element's `textContent`, trimmed.
    pub async fn text_content(&self) -> Result<String> {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .text_spec(&self.spec(), &self.describe(), false, frame_id.as_deref())
            .await
    }

    /// Whether the element exists and is visible.
    pub async fn is_visible(&self) -> Result<bool> {
        let frame_id = self.frame_id().await?;
        self.page
            .spec_is_visible(&self.spec(), frame_id.as_deref())
            .await
    }

    /// Whether the element is absent or not visible.
    pub async fn is_hidden(&self) -> Result<bool> {
        Ok(!self.is_visible().await?)
    }

    /// Whether the element is enabled.
    pub async fn is_enabled(&self) -> Result<bool> {
        let frame_id = self.frame_id().await?;
        self.page
            .spec_is_enabled(&self.spec(), frame_id.as_deref())
            .await
    }

    /// Whether a checkbox/radio is checked.
    pub async fn is_checked(&self) -> Result<bool> {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .spec_is_checked(&self.spec(), &self.describe(), frame_id.as_deref())
            .await
    }

    /// The number of elements matching the selector.
    pub async fn count(&self) -> Result<usize> {
        let frame_id = self.frame_id().await?;
        self.page
            .spec_count(&self.spec(), frame_id.as_deref())
            .await
    }

    /// The value of an attribute, or `None` if absent.
    pub async fn get_attribute(&self, name: &str) -> Result<Option<String>> {
        self.wait_for(WaitState::Attached).await?;
        let frame_id = self.frame_id().await?;
        self.page
            .get_attribute_spec(&self.spec(), &self.describe(), name, frame_id.as_deref())
            .await
    }

    /// Wait for the element to reach `state`, using the default timeout.
    pub async fn wait_for(&self, state: WaitState) -> Result<()> {
        self.wait_for_with_timeout(state, DEFAULT_TIMEOUT).await
    }

    /// Wait for the element to reach `state` with an explicit timeout.
    pub async fn wait_for_with_timeout(&self, state: WaitState, timeout: Duration) -> Result<()> {
        let frame_id = self.frame_id().await?;
        self.page
            .wait_for_spec(
                &self.spec(),
                &self.describe(),
                state,
                timeout,
                frame_id.as_deref(),
            )
            .await
    }

    /// The JSON spec sent to the injected page script, including the index.
    fn spec(&self) -> Value {
        let mut spec = self.selector.to_spec();
        if let Some(index) = self.nth {
            if let Value::Object(ref mut map) = spec {
                map.insert("nth".to_string(), Value::from(index));
            }
        }
        spec
    }
}
