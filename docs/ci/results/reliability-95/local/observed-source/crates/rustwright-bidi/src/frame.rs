//! Frames over WebDriver BiDi.
//!
//! BiDi models frames as child browsing contexts, so a frame is just another
//! context with its own realm; locators scoped to it evaluate there.

use rustwright_common::{Role, Selector};
use serde_json::Value;

use crate::browser::BidiPage;
use crate::error::BidiResult;
use crate::locator::BidiLocator;
use crate::session::BrowsingContextInfo;

/// A frame within a [`BidiPage`].
#[derive(Clone)]
pub struct BidiFrame {
    page: BidiPage,
    context: String,
    url: String,
}

impl std::fmt::Debug for BidiFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiFrame")
            .field("context", &self.context)
            .field("url", &self.url)
            .finish()
    }
}

impl BidiFrame {
    pub(crate) fn new(page: BidiPage, context: String, url: String) -> Self {
        Self { page, context, url }
    }

    /// The BiDi browsing context id of this frame.
    pub fn context_id(&self) -> &str {
        &self.context
    }

    /// Whether this is the page's main frame.
    pub fn is_main_frame(&self) -> bool {
        self.context == self.page.context_id()
    }

    /// The page this frame belongs to.
    pub fn page(&self) -> &BidiPage {
        &self.page
    }

    /// The frame's URL as last reported by the frame tree.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Evaluate an expression in this frame.
    pub async fn evaluate(&self, expression: &str) -> BidiResult<Value> {
        self.page.evaluate_in(&self.context, expression).await
    }

    /// Create a locator scoped to this frame.
    pub fn locator(&self, selector: impl Into<Selector>) -> BidiLocator {
        BidiLocator::new_in_context(self.page.clone(), self.context.clone(), selector.into())
    }

    /// Create a text locator scoped to this frame.
    pub fn get_by_text(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::text(text, false))
    }

    /// Create a role locator scoped to this frame.
    pub fn get_by_role(&self, role: Role, name: Option<&str>) -> BidiLocator {
        self.locator(Selector::role(role, name))
    }

    /// Create a placeholder locator scoped to this frame.
    pub fn get_by_placeholder(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::placeholder(text, false))
    }

    /// Create a label locator scoped to this frame.
    pub fn get_by_label(&self, text: impl Into<String>) -> BidiLocator {
        self.locator(Selector::label(text, false))
    }

    /// Create a test-id locator scoped to this frame.
    pub fn get_by_test_id(&self, id: impl Into<String>) -> BidiLocator {
        self.locator(Selector::test_id(id))
    }
}

/// Flatten the child frames of `info` into `out`, depth-first.
pub(crate) fn collect_frames(
    page: &BidiPage,
    info: &BrowsingContextInfo,
    out: &mut Vec<BidiFrame>,
) {
    for child in info.children.as_deref().unwrap_or_default() {
        out.push(BidiFrame::new(
            page.clone(),
            child.context.clone(),
            child.url.clone(),
        ));
        collect_frames(page, child, out);
    }
}
