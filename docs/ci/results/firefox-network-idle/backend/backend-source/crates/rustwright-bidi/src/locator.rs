//! Lazy, auto-waiting locators over WebDriver BiDi.
//!
//! These mirror the CDP-backed `Locator` from `rustwright-core`, reusing the
//! same [`Selector`] strategies and page-side helper script so behavior is
//! consistent across backends.

use std::time::Duration;

use rustwright_common::{Selector, WaitState};
use serde_json::Value;

use crate::browser::BidiPage;
use crate::error::{BidiError, BidiResult};

/// The default timeout for BiDi locator waits.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A lazy reference to one or more elements on a [`BidiPage`].
#[derive(Clone)]
pub struct BidiLocator {
    page: BidiPage,
    context: String,
    selector: Selector,
    nth: Option<i64>,
}

impl std::fmt::Debug for BidiLocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BidiLocator")
            .field("selector", &self.describe())
            .finish()
    }
}

impl BidiLocator {
    pub(crate) fn new(page: BidiPage, selector: Selector) -> Self {
        let context = page.context_id().to_string();
        Self {
            page,
            context,
            selector,
            nth: None,
        }
    }

    pub(crate) fn new_in_context(page: BidiPage, context: String, selector: Selector) -> Self {
        Self {
            page,
            context,
            selector,
            nth: None,
        }
    }

    /// The selector strategy used by this locator.
    pub fn selector(&self) -> &Selector {
        &self.selector
    }

    /// A human-readable description, including any index.
    pub fn describe(&self) -> String {
        match self.nth {
            Some(index) => format!("{} nth={index}", self.selector.describe()),
            None => self.selector.describe(),
        }
    }

    /// The first matching element.
    pub fn first(&self) -> BidiLocator {
        self.nth(0)
    }

    /// The last matching element.
    pub fn last(&self) -> BidiLocator {
        self.nth(-1)
    }

    /// The `n`-th matching element. Negative indices count from the end.
    pub fn nth(&self, index: i64) -> BidiLocator {
        let mut clone = self.clone();
        clone.nth = Some(index);
        clone
    }

    /// Every element matching the selector.
    pub async fn all(&self) -> BidiResult<Vec<BidiLocator>> {
        let count = self.count().await?;
        Ok((0..count as i64).map(|index| self.nth(index)).collect())
    }

    /// Click after the element is visible, enabled, stable and receives pointer events.
    pub async fn click(&self) -> BidiResult<()> {
        self.click_with_timeout(DEFAULT_TIMEOUT).await
    }

    /// Click with one deadline for actionability checks and trusted input dispatch.
    ///
    /// Waiting retries only happen before button dispatch. An input error is
    /// returned without retrying, because the browser may already have clicked.
    pub async fn click_with_timeout(&self, timeout: Duration) -> BidiResult<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut reason = "element has not been checked".to_string();
        loop {
            let attempt = async {
                let point = self.prepare_click().await?;
                if let Some(observation) = point.get("reason").and_then(Value::as_str) {
                    return Ok(Some(observation.to_string()));
                }
                let x = point.get("x").and_then(Value::as_f64).ok_or_else(|| {
                    BidiError::Unexpected(format!("click point missing x: {point}"))
                })?;
                let y = point.get("y").and_then(Value::as_f64).ok_or_else(|| {
                    BidiError::Unexpected(format!("click point missing y: {point}"))
                })?;
                self.page.pointer_move(&self.context, x, y).await?;
                let checked = self.check_click(&point).await?;
                if let Some(observation) = checked.as_str() {
                    return Ok(Some(observation.to_string()));
                }
                // No retry follows this command, including ambiguous transport
                // failures after pointerDown reached the remote end.
                self.page.pointer_click(&self.context).await?;
                Ok::<_, BidiError>(None)
            };
            match tokio::time::timeout_at(deadline, attempt).await {
                Ok(Ok(None)) => return Ok(()),
                Ok(Ok(Some(observation))) => reason = observation,
                Ok(Err(error)) => return Err(error),
                Err(_) => return Err(self.click_timeout(&reason)),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(self.click_timeout(&reason));
            }
            tokio::time::sleep_until(std::cmp::min(
                deadline,
                tokio::time::Instant::now() + Duration::from_millis(25),
            ))
            .await;
        }
    }

    fn click_timeout(&self, reason: &str) -> BidiError {
        BidiError::WaitTimeout(format!(
            "element {} to be clickable; {reason}",
            self.describe()
        ))
    }

    async fn prepare_click(&self) -> BidiResult<Value> {
        let expression = format!(
            r#"(async () => {{
                const helper = window.__rustwright;
                const el = helper.resolve({spec});
                if (!el) return {{reason: 'element was not found'}};
                const geometry = await helper.prepareClick(el);
                if (geometry.reason) return geometry;
                for (const rect of el.getClientRects()) {{
                    const left = Math.max(geometry.clip[0], rect.left);
                    const top = Math.max(geometry.clip[1], rect.top);
                    const right = Math.min(geometry.clip[2], rect.right);
                    const bottom = Math.min(geometry.clip[3], rect.bottom);
                    if (right <= left || bottom <= top) continue;
                    const insetX = Math.min(1, (right - left) / 2);
                    const insetY = Math.min(1, (bottom - top) / 2);
                    const points = [
                        [(left + right) / 2, (top + bottom) / 2],
                        [left + insetX, top + insetY],
                        [right - insetX, top + insetY],
                        [left + insetX, bottom - insetY],
                        [right - insetX, bottom - insetY]
                    ];
                    for (const point of points) {{
                        const x = Math.floor(point[0]), y = Math.floor(point[1]);
                        let hit = document.elementFromPoint(x, y);
                        while (hit && hit.shadowRoot) {{
                            const nested = hit.shadowRoot.elementFromPoint(x, y);
                            if (!nested || nested === hit) break;
                            hit = nested;
                        }}
                        if (helper.includesHit(el, hit)) return {{geometry, x, y}};
                    }}
                }}
                return {{reason: 'another element intercepts pointer events at candidate points'}};
            }})()"#,
            spec = self.spec_json()
        );
        self.evaluate(&expression).await
    }

    async fn check_click(&self, point: &Value) -> BidiResult<Value> {
        let expression = format!(
            r#"(() => {{
                const helper = window.__rustwright;
                const spec = {spec}, point = {point};
                const el = helper.resolve(spec);
                if (!el) return 'element was detached';
                const reason = helper.checkClick(el, spec, point.geometry);
                if (reason) return reason;
                let hit = document.elementFromPoint(point.x, point.y);
                while (hit && hit.shadowRoot) {{
                    const nested = hit.shadowRoot.elementFromPoint(point.x, point.y);
                    if (!nested || nested === hit) break;
                    hit = nested;
                }}
                return helper.includesHit(el, hit) ? null : 'another element intercepts pointer events';
            }})()"#,
            spec = self.spec_json(),
            point = point,
        );
        self.evaluate(&expression).await
    }

    /// Move the mouse over the element.
    pub async fn hover(&self) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        let (x, y) = self.center().await?;
        self.page.pointer_move(&self.context, x, y).await
    }

    /// Replace the element's value, waiting for it to become visible first.
    pub async fn fill(&self, text: impl AsRef<str>) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        let text = serde_json::to_string(text.as_ref())?;
        let body = format!(
            "const tag = el.tagName; \
             let proto = null; \
             if (tag === 'TEXTAREA') {{ proto = HTMLTextAreaElement.prototype; }} \
             else if (tag === 'INPUT') {{ proto = HTMLInputElement.prototype; }} \
             if (proto) {{ \
               const descriptor = Object.getOwnPropertyDescriptor(proto, 'value'); \
               if (descriptor && descriptor.set) {{ descriptor.set.call(el, {text}); }} \
               else {{ el.value = {text}; }} \
             }} else if (el.isContentEditable) {{ el.textContent = {text}; }} \
             else {{ el.value = {text}; }} \
             el.dispatchEvent(new Event('input', {{ bubbles: true, cancelable: true }})); \
             el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
             return true;"
        );
        if self.eval_with_element(&body).await?.as_bool() != Some(true) {
            return Err(BidiError::ElementNotFound(self.describe()));
        }
        Ok(())
    }

    /// Focus the element and insert text.
    pub async fn type_text(&self, text: impl AsRef<str>) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        self.focus().await?;
        self.page.type_text(&self.context, text.as_ref()).await
    }

    /// Focus the element and press a key.
    pub async fn press(&self, key: &str) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        self.focus().await?;
        self.page.press_key(&self.context, key).await
    }

    /// The element's rendered (inner) text, trimmed.
    pub async fn text(&self) -> BidiResult<String> {
        self.wait_for(WaitState::Attached).await?;
        let value = self
            .eval_with_element("return el.innerText != null ? el.innerText : el.textContent;")
            .await?;
        Ok(value.as_str().unwrap_or_default().trim().to_string())
    }

    /// The element's `textContent`, trimmed.
    pub async fn text_content(&self) -> BidiResult<String> {
        self.wait_for(WaitState::Attached).await?;
        let value = self.eval_with_element("return el.textContent;").await?;
        Ok(value.as_str().unwrap_or_default().trim().to_string())
    }

    /// Whether the element exists and is visible.
    pub async fn is_visible(&self) -> BidiResult<bool> {
        let expression = format!(
            "(window.__rustwright ? window.__rustwright.isVisible({}) : false)",
            self.spec_json()
        );
        Ok(self.evaluate(&expression).await?.as_bool().unwrap_or(false))
    }

    /// Whether the element is absent or not visible.
    pub async fn is_hidden(&self) -> BidiResult<bool> {
        Ok(!self.is_visible().await?)
    }

    /// Whether the element is enabled.
    pub async fn is_enabled(&self) -> BidiResult<bool> {
        let expression = format!(
            "(window.__rustwright ? window.__rustwright.isEnabled({}) : false)",
            self.spec_json()
        );
        Ok(self.evaluate(&expression).await?.as_bool().unwrap_or(false))
    }

    /// Whether a checkbox/radio is checked.
    pub async fn is_checked(&self) -> BidiResult<bool> {
        self.wait_for(WaitState::Attached).await?;
        Ok(self
            .eval_with_element("return el.checked === true;")
            .await?
            .as_bool()
            .unwrap_or(false))
    }

    /// The number of elements matching the selector.
    pub async fn count(&self) -> BidiResult<usize> {
        let expression = format!(
            "(window.__rustwright ? window.__rustwright.count({}) : 0)",
            self.spec_json()
        );
        Ok(self.evaluate(&expression).await?.as_u64().unwrap_or(0) as usize)
    }

    /// The value of an attribute, or `None` if absent.
    pub async fn get_attribute(&self, name: &str) -> BidiResult<Option<String>> {
        self.wait_for(WaitState::Attached).await?;
        let name = serde_json::to_string(name)?;
        let value = self
            .eval_with_element(&format!("return el.getAttribute({name});"))
            .await?;
        Ok(value.as_str().map(str::to_string))
    }

    /// Select an option in a `<select>` by value.
    pub async fn select_option(&self, value: impl AsRef<str>) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        let value = serde_json::to_string(value.as_ref())?;
        let body = format!(
            "if (el.tagName !== 'SELECT') {{ return false; }} \
             el.value = {value}; \
             el.dispatchEvent(new Event('input', {{ bubbles: true }})); \
             el.dispatchEvent(new Event('change', {{ bubbles: true }})); \
             return true;"
        );
        if self.eval_with_element(&body).await?.as_bool() != Some(true) {
            return Err(BidiError::ElementNotFound(format!(
                "{} (not a <select>)",
                self.describe()
            )));
        }
        Ok(())
    }

    /// Ensure a checkbox/radio is checked.
    pub async fn check(&self) -> BidiResult<()> {
        self.set_checked(true).await
    }

    /// Ensure a checkbox/radio is unchecked.
    pub async fn uncheck(&self) -> BidiResult<()> {
        self.set_checked(false).await
    }

    /// Scroll the element into the viewport.
    pub async fn scroll_into_view_if_needed(&self) -> BidiResult<()> {
        self.wait_for(WaitState::Attached).await?;
        let body = "el.scrollIntoView({ block: 'center', inline: 'center' }); return true;";
        if self.eval_with_element(body).await?.as_bool() != Some(true) {
            return Err(BidiError::ElementNotFound(self.describe()));
        }
        Ok(())
    }

    /// Wait for the element to reach `state`, using the default timeout.
    pub async fn wait_for(&self, state: WaitState) -> BidiResult<()> {
        self.wait_for_with_timeout(state, DEFAULT_TIMEOUT).await
    }

    /// Wait for the element to reach `state` with an explicit timeout.
    pub async fn wait_for_with_timeout(
        &self,
        state: WaitState,
        timeout: Duration,
    ) -> BidiResult<()> {
        let millis = timeout.as_millis();
        let expression = format!(
            "(window.__rustwright ? window.__rustwright.waitFor({}, \"{}\", {millis}) : false)",
            self.spec_json(),
            state.as_str()
        );
        let satisfied = tokio::time::timeout(timeout, self.evaluate(&expression))
            .await
            .map_err(|_| {
                BidiError::WaitTimeout(format!(
                    "element {} to be {}",
                    self.describe(),
                    state.as_str(),
                ))
            })??
            .as_bool()
            .unwrap_or(false);
        if !satisfied {
            return Err(BidiError::WaitTimeout(format!(
                "element {} to be {}",
                self.describe(),
                state.as_str()
            )));
        }
        Ok(())
    }

    async fn set_checked(&self, checked: bool) -> BidiResult<()> {
        self.wait_for(WaitState::Visible).await?;
        if self.is_checked().await? == checked {
            return Ok(());
        }
        self.click().await
    }

    async fn focus(&self) -> BidiResult<()> {
        let body = "if (el.focus) { el.focus(); } return true;";
        if self.eval_with_element(body).await?.as_bool() != Some(true) {
            return Err(BidiError::ElementNotFound(self.describe()));
        }
        Ok(())
    }

    async fn center(&self) -> BidiResult<(f64, f64)> {
        let expression = format!(
            "(() => {{ const el = (window.__rustwright ? window.__rustwright.resolve({}) : null); \
             if (!el) return null; \
             el.scrollIntoView({{ block: 'center', inline: 'center' }}); \
             const rect = el.getBoundingClientRect(); \
             return {{ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }}; }})()",
            self.spec_json()
        );
        let value = self.evaluate(&expression).await?;
        if value.is_null() {
            return Err(BidiError::ElementNotFound(self.describe()));
        }
        let x = value
            .get("x")
            .and_then(Value::as_f64)
            .ok_or_else(|| BidiError::ElementNotFound(self.describe()))?;
        let y = value
            .get("y")
            .and_then(Value::as_f64)
            .ok_or_else(|| BidiError::ElementNotFound(self.describe()))?;
        Ok((x, y))
    }

    async fn eval_with_element(&self, body: &str) -> BidiResult<Value> {
        let expression = format!(
            "(() => {{ const el = (window.__rustwright ? window.__rustwright.resolve({}) : null); \
             if (!el) return null; {body} }})()",
            self.spec_json()
        );
        self.evaluate(&expression).await
    }

    async fn evaluate(&self, expression: &str) -> BidiResult<Value> {
        self.page.ensure_helper_in(&self.context).await?;
        self.page.evaluate_in(&self.context, expression).await
    }

    fn spec_json(&self) -> String {
        serde_json::to_string(&self.spec()).expect("selector spec is always serializable")
    }

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
