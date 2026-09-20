//! `Tracing` domain: Chrome trace collection.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Trace configuration passed to `Tracing.start`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceConfig {
    /// Categories to include.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included_categories: Option<Vec<String>>,
    /// Categories to exclude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_categories: Option<Vec<String>>,
    /// Record mode (`recordUntilFull`, `recordContinuously`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_mode: Option<String>,
}

/// Parameters of `Tracing.start`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartParams {
    /// Comma-separated category filters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<String>,
    /// Legacy options string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    /// Buffer usage reporting interval in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_usage_reporting_interval: Option<f64>,
    /// How data is transferred (`ReportEvents` or `ReturnAsStream`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_mode: Option<String>,
    /// Stream format (`json` or `proto`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_format: Option<String>,
    /// Structured trace configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_config: Option<TraceConfig>,
}

/// Parameters of the `Tracing.dataCollected` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataCollectedParams {
    /// A batch of trace events.
    pub value: Vec<Value>,
}

/// Parameters of the `Tracing.tracingComplete` event.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracingCompleteParams {
    /// Whether some data was lost.
    #[serde(default)]
    pub data_loss_occurred: bool,
}
