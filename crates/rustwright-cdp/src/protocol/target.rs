//! `Browser` and `Target` domains.

use serde::{Deserialize, Serialize};

/// Response of `Browser.getVersion`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetVersionResult {
    /// Protocol version, for example `1.3`.
    pub protocol_version: String,
    /// Product string, for example `Chrome/150.0.0.0`.
    pub product: String,
    /// Source revision.
    #[serde(default)]
    pub revision: String,
    /// Default user agent.
    #[serde(default)]
    pub user_agent: String,
    /// V8 version.
    #[serde(default)]
    pub js_version: String,
}

/// Parameters of `Target.createTarget`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetParams {
    /// Initial URL for the new tab.
    pub url: String,
    /// Browser context to create the target in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_context_id: Option<String>,
    /// Open in a new window instead of a tab.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_window: Option<bool>,
    /// Create the target in the background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
}

/// Response of `Target.createTarget`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTargetResult {
    /// The created target id.
    pub target_id: String,
}

/// Parameters of `Target.attachToTarget`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachToTargetParams {
    /// The target to attach to.
    pub target_id: String,
    /// Use flat session multiplexing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flatten: Option<bool>,
}

/// Response of `Target.attachToTarget`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachToTargetResult {
    /// The new session id.
    pub session_id: String,
}

/// Parameters of `Target.closeTarget`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTargetParams {
    /// The target to close.
    pub target_id: String,
}

/// Response of `Target.closeTarget`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseTargetResult {
    /// Whether the target was closed.
    #[serde(default)]
    pub success: bool,
}

/// Response of `Target.getTargets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetTargetsResult {
    /// All known targets.
    pub target_infos: Vec<TargetInfo>,
}

/// A CDP target (tab, page, worker, ...).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    /// Target id.
    pub target_id: String,
    /// Target type, for example `page`.
    #[serde(rename = "type")]
    pub target_type: String,
    /// Current title.
    #[serde(default)]
    pub title: String,
    /// Current URL.
    #[serde(default)]
    pub url: String,
    /// Whether a debugger session is attached.
    #[serde(default)]
    pub attached: bool,
    /// The browser context the target belongs to.
    #[serde(default)]
    pub browser_context_id: Option<String>,
    /// The target that opened this one, if any.
    #[serde(default)]
    pub opener_id: Option<String>,
}

/// Response of `Target.createBrowserContext`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBrowserContextResult {
    /// The new browser context id.
    pub browser_context_id: String,
}

/// Parameters of `Target.disposeBrowserContext`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisposeBrowserContextParams {
    /// The context to dispose.
    pub browser_context_id: String,
}

/// Parameters of `Target.setDiscoverTargets`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDiscoverTargetsParams {
    /// Whether discovery is enabled.
    pub discover: bool,
}

/// Parameters of the `Target.attachedToTarget` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedToTargetParams {
    /// The session id.
    pub session_id: String,
    /// Information about the attached target.
    pub target_info: TargetInfo,
    /// Whether the target is waiting for debugging.
    #[serde(default)]
    pub waiting_for_debugger: bool,
}

/// Parameters of the `Target.detachedFromTarget` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachedFromTargetParams {
    /// The detached session id.
    pub session_id: String,
    /// The target id that was detached.
    #[serde(default)]
    pub target_id: Option<String>,
}

/// Parameters of the `Target.targetCreated` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetCreatedParams {
    /// Information about the created target.
    pub target_info: TargetInfo,
}
