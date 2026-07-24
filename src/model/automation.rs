//! Per-track automation curve lanes (beat/value points modulating plugin params).

use serde::{Deserialize, Serialize};

/// Which plugin parameter an automation lane drives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationTarget {
    Instrument { param_id: u32 },
    Device { device_id: u64, param_id: u32 },
}

/// Interpolation from this point toward the next (evaluation is Phase B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    #[default]
    Linear,
    Hold,
    Smooth,
}

/// One automation breakpoint in beat / normalized-value space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub beat: f32,
    /// Normalized 0..1 (plugin-agnostic; mapped to native range at eval time).
    pub value: f32,
    #[serde(default)]
    pub curve: CurveKind,
}

fn default_enabled() -> bool {
    true
}

fn default_param_min() -> f64 {
    0.0
}

fn default_param_max() -> f64 {
    1.0
}

/// A drawable automation curve bound to one plugin parameter on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    pub id: u64,
    pub target: AutomationTarget,
    /// Cached for display when the plugin is not loaded.
    pub param_name: String,
    #[serde(default = "default_param_min")]
    pub param_min: f64,
    #[serde(default = "default_param_max")]
    pub param_max: f64,
    #[serde(default)]
    pub points: Vec<AutomationPoint>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}
