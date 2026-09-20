//! `Browser` domain download configuration.

use serde::Serialize;

/// Parameters of `Browser.setDownloadBehavior`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDownloadBehaviorParams {
    /// Behavior: `deny`, `allow`, or `allowAndName`.
    pub behavior: String,
    /// Restrict to a browser context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_context_id: Option<String>,
    /// Directory to write downloads to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_path: Option<String>,
    /// Whether to emit download progress events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_enabled: Option<bool>,
}
