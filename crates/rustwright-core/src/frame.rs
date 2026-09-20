//! Frames and frame locators.
//!
//! Cross-origin iframes are handled through CDP execution contexts: every frame
//! gets its own context, and locators scoped to a frame evaluate inside it.

use serde_json::Value;

use crate::error::Result;
use crate::locator::Locator;
use crate::page::Page;
use rustwright_common::{Role, Selector};

/// A frame within a page.
#[derive(Clone)]
pub struct Frame {
    page: Page,
    frame_id: String,
    main: bool,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("frame_id", &self.frame_id)
            .field("main", &self.main)
            .field("url", &self.url())
            .finish()
    }
}

impl Frame {
    pub(crate) fn new(page: Page, frame_id: String, main: bool) -> Self {
        Self {
            page,
            frame_id,
            main,
        }
    }

    /// The CDP frame id.
    pub fn frame_id(&self) -> &str {
        &self.frame_id
    }

    /// Whether this is the page's main frame.
    pub fn is_main_frame(&self) -> bool {
        self.main
    }

    /// The page this frame belongs to.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The frame's current URL, if known.
    pub fn url(&self) -> String {
        self.page
            .frame_info(&self.frame_id)
            .map(|info| info.url)
            .unwrap_or_default()
    }

    /// The frame's name, if any.
    pub fn name(&self) -> String {
        self.page
            .frame_info(&self.frame_id)
            .map(|info| info.name)
            .unwrap_or_default()
    }

    /// Evaluate an expression in this frame.
    pub async fn evaluate(&self, expression: &str) -> Result<Value> {
        self.page
            .evaluate_in(expression, Some(&self.frame_id))
            .await
    }

    /// Create a locator scoped to this frame.
    pub fn locator(&self, selector: impl Into<Selector>) -> Locator {
        Locator::new_in_frame(self.page.clone(), selector.into(), self.frame_id.clone())
    }

    /// Create a text locator scoped to this frame.
    pub fn get_by_text(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::text(text, false))
    }

    /// Create a role locator scoped to this frame.
    pub fn get_by_role(&self, role: Role, name: Option<&str>) -> Locator {
        self.locator(Selector::role(role, name))
    }

    /// Create a placeholder locator scoped to this frame.
    pub fn get_by_placeholder(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::placeholder(text, false))
    }

    /// Create a label locator scoped to this frame.
    pub fn get_by_label(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::label(text, false))
    }

    /// Create a test-id locator scoped to this frame.
    pub fn get_by_test_id(&self, id: impl Into<String>) -> Locator {
        self.locator(Selector::test_id(id))
    }
}

/// A lazy reference to the frame inside an `<iframe>` element.
///
/// The frame is resolved on use, so a `FrameLocator` can be created before the
/// iframe has loaded and keeps working across navigations.
#[derive(Clone)]
pub struct FrameLocator {
    page: Page,
    selector: Selector,
}

impl std::fmt::Debug for FrameLocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameLocator")
            .field("selector", &self.selector.describe())
            .finish()
    }
}

impl FrameLocator {
    pub(crate) fn new(page: Page, selector: Selector) -> Self {
        Self { page, selector }
    }

    /// The selector used to find the iframe element.
    pub fn selector(&self) -> &Selector {
        &self.selector
    }

    /// Resolve the iframe element to its current [`Frame`].
    pub async fn resolve(&self) -> Result<Frame> {
        let frame_id = self.page.frame_id_for_element(&self.selector).await?;
        Ok(Frame::new(self.page.clone(), frame_id, false))
    }

    /// Create a locator scoped to the iframe's contents.
    pub fn locator(&self, selector: impl Into<Selector>) -> Locator {
        Locator::new_in_frame_element(self.page.clone(), selector.into(), self.selector.clone())
    }

    /// Create a text locator scoped to the iframe's contents.
    pub fn get_by_text(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::text(text, false))
    }

    /// Create a role locator scoped to the iframe's contents.
    pub fn get_by_role(&self, role: Role, name: Option<&str>) -> Locator {
        self.locator(Selector::role(role, name))
    }

    /// Create a placeholder locator scoped to the iframe's contents.
    pub fn get_by_placeholder(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::placeholder(text, false))
    }

    /// Create a label locator scoped to the iframe's contents.
    pub fn get_by_label(&self, text: impl Into<String>) -> Locator {
        self.locator(Selector::label(text, false))
    }

    /// Create a test-id locator scoped to the iframe's contents.
    pub fn get_by_test_id(&self, id: impl Into<String>) -> Locator {
        self.locator(Selector::test_id(id))
    }
}
