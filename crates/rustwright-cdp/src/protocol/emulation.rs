//! `Emulation` domain.

use serde::Serialize;

/// Parameters of `Emulation.setDeviceMetricsOverride`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDeviceMetricsOverrideParams {
    /// Viewport width in CSS pixels.
    pub width: i64,
    /// Viewport height in CSS pixels.
    pub height: i64,
    /// Device pixel ratio.
    pub device_scale_factor: f64,
    /// Whether to emulate a mobile device.
    pub mobile: bool,
}
