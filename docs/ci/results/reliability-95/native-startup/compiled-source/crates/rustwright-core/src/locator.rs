//! Lazy, auto-waiting element locators.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
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

    /// Click after the element is visible, enabled, stable and receives pointer events.
    pub async fn click(&self) -> Result<()> {
        self.click_with_timeout(DEFAULT_TIMEOUT).await
    }

    /// Click with one deadline for frame resolution, actionability and dispatch.
    ///
    /// Actionability checks use two animation frames and recheck the target
    /// after mouse movement. Timeouts include the last reason for waiting.
    pub async fn click_with_timeout(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut reason = "element has not been checked".to_string();
        loop {
            self.page.ensure_open()?;
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout {
                    what: format!("element {} to be clickable; {reason}", self.describe()),
                    timeout,
                });
            }
            let attempt = async {
                let frame_id = self.frame_id().await?;
                self.page
                    .click_spec(
                        &self.spec(),
                        &self.describe(),
                        frame_id.as_deref(),
                        &mut reason,
                    )
                    .await
            };
            match tokio::time::timeout_at(deadline, attempt).await {
                Ok(Ok(None)) => return Ok(()),
                Ok(Ok(Some(observation))) => reason = observation,
                Ok(Err(error)) if is_transient_click_error(&error) => reason = error.to_string(),
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    return Err(Error::Timeout {
                        what: format!("element {} to be clickable; {reason}", self.describe()),
                        timeout,
                    })
                }
            }
            tokio::time::sleep_until(std::cmp::min(
                deadline,
                tokio::time::Instant::now() + Duration::from_millis(25),
            ))
            .await;
        }
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
        self.click().await
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
    ///
    /// Frame-scoped waits include iframe resolution and execution-context
    /// readiness in the same deadline, retrying if the frame navigates.
    pub async fn wait_for_with_timeout(&self, state: WaitState, timeout: Duration) -> Result<()> {
        if self.frame.is_some() {
            return self.wait_in_frame(state, timeout).await;
        }
        let wait = async {
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
        };
        tokio::time::timeout(timeout, wait)
            .await
            .unwrap_or_else(|_| {
                Err(Error::Timeout {
                    what: format!("element {} to be {}", self.describe(), state.as_str()),
                    timeout,
                })
            })
    }

    async fn wait_in_frame(&self, state: WaitState, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let timeout_error = || Error::Timeout {
            what: format!(
                "element {} in frame to be {}",
                self.describe(),
                state.as_str()
            ),
            timeout,
        };
        loop {
            self.page.ensure_open()?;
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(timeout_error());
            }
            let attempt = async {
                let frame_id = self.frame_id().await?;
                self.page
                    .wait_for_spec(
                        &self.spec(),
                        &self.describe(),
                        state,
                        remaining,
                        frame_id.as_deref(),
                    )
                    .await
            };
            match tokio::time::timeout_at(deadline, attempt).await {
                Ok(Ok(())) => return Ok(()),
                // The iframe can appear late, acquire a context after attachment,
                // or replace that context during a navigation. These are the
                // only transient failures of frame resolution and waiting.
                Ok(Err(error)) if is_transient_frame_error(&error) || error.is_timeout() => {}
                Ok(Err(error)) => return Err(error),
                Err(_) => return Err(timeout_error()),
            }
            tokio::time::sleep_until(std::cmp::min(
                deadline,
                tokio::time::Instant::now() + Duration::from_millis(25),
            ))
            .await;
        }
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

fn is_transient_frame_error(error: &Error) -> bool {
    match error {
        Error::ElementNotFound { .. } | Error::FrameNotReady { .. } => true,
        Error::Cdp(rustwright_cdp::CdpError::ContextDestroyed { method, .. }) => {
            matches!(method.as_str(), "Runtime.evaluate")
        }
        Error::Cdp(rustwright_cdp::CdpError::SessionDetached { method, .. }) => matches!(
            method.as_str(),
            "Runtime.evaluate" | "Runtime.callFunctionOn"
        ),
        Error::Cdp(rustwright_cdp::CdpError::Protocol {
            method,
            code: -32001,
            message,
            ..
        }) => {
            matches!(
                method.as_str(),
                "Runtime.evaluate" | "Runtime.callFunctionOn"
            ) && message.starts_with("Session with given id not found")
        }
        Error::Cdp(rustwright_cdp::CdpError::Protocol {
            method,
            code: -32000,
            message,
            ..
        }) => {
            matches!(
                method.as_str(),
                "Runtime.evaluate" | "Runtime.callFunctionOn"
            ) && (message.starts_with("Execution context was destroyed")
                || message == "Cannot find context with specified id"
                || message == "Could not find object with given id")
        }
        _ => false,
    }
}

fn is_transient_click_error(error: &Error) -> bool {
    if is_transient_frame_error(error) {
        return true;
    }
    match error {
        Error::Cdp(rustwright_cdp::CdpError::SessionDetached { method, .. }) => matches!(
            method.as_str(),
            "DOM.getFrameOwner"
                | "DOM.getBoxModel"
                | "DOM.getContentQuads"
                | "DOM.getNodeForLocation"
                | "DOM.resolveNode"
        ),
        Error::Cdp(rustwright_cdp::CdpError::Protocol {
            method,
            code: -32001,
            message,
            ..
        }) => {
            matches!(
                method.as_str(),
                "DOM.getFrameOwner"
                    | "DOM.getBoxModel"
                    | "DOM.getContentQuads"
                    | "DOM.getNodeForLocation"
                    | "DOM.resolveNode"
            ) && message.starts_with("Session with given id not found")
        }
        // These DOM probes run before mousePressed. A detached node or a
        // disappearing hit target is safe to resolve again without duplicating
        // a click. Errors from input dispatch are never retried.
        Error::Cdp(rustwright_cdp::CdpError::Protocol {
            method,
            code: -32000,
            message,
            ..
        }) => match method.as_str() {
            "DOM.getFrameOwner" => {
                message.starts_with("Frame with the given id was not found")
                    || message.starts_with("No frame for given id found")
            }
            "DOM.getBoxModel" => {
                message.starts_with("Could not compute box model")
                    || message == "Could not find object with given id"
            }
            "DOM.getContentQuads" => {
                message.starts_with("Could not compute content quads")
                    || message == "Could not find object with given id"
            }
            "DOM.getNodeForLocation" => message.starts_with("No node found at given location"),
            "DOM.resolveNode" => {
                message == "No node with given id found"
                    || message == "Node with given id does not belong to the document"
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_pre_input_probes_can_retry_but_input_never_retries() {
        for method in [
            "Runtime.evaluate",
            "Runtime.callFunctionOn",
            "DOM.getFrameOwner",
            "DOM.getBoxModel",
            "DOM.getContentQuads",
            "DOM.getNodeForLocation",
            "DOM.resolveNode",
        ] {
            let error = Error::Cdp(rustwright_cdp::CdpError::SessionDetached {
                session_id: "child".to_string(),
                method: method.to_string(),
            });
            assert!(is_transient_click_error(&error), "{method}");
        }
        for method in [
            "Input.dispatchMouseEvent",
            "Input.dispatchKeyEvent",
            "Input.insertText",
        ] {
            let error = Error::Cdp(rustwright_cdp::CdpError::SessionDetached {
                session_id: "child".to_string(),
                method: method.to_string(),
            });
            assert!(
                !is_transient_click_error(&error),
                "must not duplicate {method}"
            );
        }
        assert!(!is_transient_click_error(&Error::Cdp(
            rustwright_cdp::CdpError::Closed
        )));
    }
}
