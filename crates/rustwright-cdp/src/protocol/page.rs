//! `Page` domain.

use serde::{Deserialize, Serialize};

/// Parameters of `Page.navigate`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateParams {
    /// Destination URL.
    pub url: String,
    /// Referrer URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    /// Frame id; defaults to the main frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

/// Response of `Page.navigate`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateResult {
    /// The frame that will be navigated.
    #[serde(default)]
    pub frame_id: String,
    /// The loader id of the committed navigation.
    #[serde(default)]
    pub loader_id: Option<String>,
    /// Non-fatal error text, for example when the load was aborted.
    #[serde(default)]
    pub error_text: Option<String>,
}

/// Parameters of `Page.reload`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadParams {
    /// Bypass the cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_cache: Option<bool>,
}

/// Parameters of `Page.captureScreenshot`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureScreenshotParams {
    /// Image format (`png`, `jpeg`, `webp`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Compression quality for lossy formats (0-100).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<i64>,
    /// Capture beyond the viewport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_beyond_viewport: Option<bool>,
    /// Render from the surface rather than the view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_surface: Option<bool>,
}

/// Response of `Page.captureScreenshot`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureScreenshotResult {
    /// Base64-encoded image data.
    pub data: String,
}

/// Parameters of `Page.setLifecycleEventsEnabled`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLifecycleEventsEnabledParams {
    /// Whether lifecycle events are emitted.
    pub enabled: bool,
}

/// Parameters of `Page.addScriptToEvaluateOnNewDocument`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddScriptParams {
    /// The script source.
    pub source: String,
}

/// Response of `Page.addScriptToEvaluateOnNewDocument`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddScriptResult {
    /// Identifier that can be used to remove the script.
    pub identifier: String,
}

/// Parameters of the `Page.lifecycleEvent` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleEventParams {
    /// Frame id.
    pub frame_id: String,
    /// Loader id.
    #[serde(default)]
    pub loader_id: String,
    /// Lifecycle name (`init`, `DOMContentLoaded`, `load`, `networkIdle`).
    pub name: String,
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
}

/// A frame as reported by `Page.frameNavigated`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Frame id.
    pub id: String,
    /// Parent frame id for subframes.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// The loader id for the current document.
    #[serde(default)]
    pub loader_id: Option<String>,
    /// Current URL.
    #[serde(default)]
    pub url: String,
    /// Frame name.
    #[serde(default)]
    pub name: String,
    /// Security origin.
    #[serde(default)]
    pub security_origin: String,
    /// Frame mime type.
    #[serde(default)]
    pub mime_type: String,
}

/// A frame tree as returned by `Page.getFrameTree`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameTree {
    /// The frame.
    pub frame: Frame,
    /// Child frames.
    #[serde(default)]
    pub child_frames: Vec<FrameTree>,
}

/// Response of `Page.getFrameTree`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFrameTreeResult {
    /// The frame tree rooted at the main frame.
    pub frame_tree: FrameTree,
}

/// Parameters of the `Page.frameAttached` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameAttachedParams {
    /// The attached frame id.
    pub frame_id: String,
    /// The parent frame id.
    #[serde(default)]
    pub parent_frame_id: String,
}

/// Parameters of the `Page.frameDetached` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDetachedParams {
    /// The detached frame id.
    pub frame_id: String,
    /// Detach reason.
    #[serde(default)]
    pub reason: String,
}

/// Parameters of the `Page.frameNavigated` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameNavigatedParams {
    /// The navigated frame.
    pub frame: Frame,
    /// Frame type (`outermostFrame`, `subframe`, ...).
    #[serde(rename = "type", default)]
    pub frame_type: String,
}

/// Parameters of the `Page.loadEventFired` and `Page.domContentEventFired`
/// events.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampParams {
    /// Monotonic timestamp in seconds.
    #[serde(default)]
    pub timestamp: f64,
}

/// Parameters of `Page.handleJavaScriptDialog`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandleJavaScriptDialogParams {
    /// Whether to accept the dialog.
    pub accept: bool,
    /// Prompt text when accepting a `prompt` dialog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
}

/// Parameters of the `Page.javascriptDialogOpening` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavascriptDialogOpeningParams {
    /// The page URL that opened the dialog.
    #[serde(default)]
    pub url: String,
    /// The dialog message.
    #[serde(default)]
    pub message: String,
    /// The dialog type (`alert`, `confirm`, `prompt`, `beforeunload`).
    #[serde(rename = "type", default)]
    pub dialog_type: String,
    /// Whether the browser has a default handler.
    #[serde(default)]
    pub has_browser_handler: bool,
    /// Default prompt text.
    #[serde(default)]
    pub default_prompt: Option<String>,
}

/// A navigation history entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationEntry {
    /// Unique entry id.
    pub id: i64,
    /// The URL.
    #[serde(default)]
    pub url: String,
    /// The title.
    #[serde(default)]
    pub title: String,
}

/// Response of `Page.getNavigationHistory`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNavigationHistoryResult {
    /// Index of the current entry.
    #[serde(default)]
    pub current_index: i64,
    /// All entries.
    #[serde(default)]
    pub entries: Vec<NavigationEntry>,
}

/// Parameters of `Page.navigateToHistoryEntry`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateToHistoryEntryParams {
    /// The entry id to navigate to.
    pub entry_id: i64,
}
