//! Per-track LFO / MSEG modulators that offset plugin parameters at audio rate.

use serde::{Deserialize, Serialize};

use super::automation::{AutomationPoint, AutomationTarget};

/// Waveform shape for an LFO modulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
    /// Drawable multi-segment looping envelope (`mseg_points`).
    Custom,
}

/// Rate of an LFO: tempo-synced beats or free-running Hz.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LfoRate {
    SyncBeats { beats: f32 },
    Hz { hz: f32 },
}

impl Default for LfoRate {
    fn default() -> Self {
        Self::SyncBeats { beats: 1.0 }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_depth() -> f32 {
    0.25
}

fn default_center() -> f32 {
    0.5
}

fn default_mseg_length_beats() -> f32 {
    1.0
}

/// Grid snap divisions for the MSEG editor (0 = freehand).
fn default_mseg_grid_divisions() -> u8 {
    16
}

/// Host-side modulator that offsets one plugin parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LfoModulator {
    pub id: u64,
    pub target: AutomationTarget,
    /// Cached for display when the plugin is not loaded.
    #[serde(default)]
    pub param_name: String,
    #[serde(default)]
    pub shape: LfoShape,
    #[serde(default)]
    pub rate: LfoRate,
    /// Modulation depth in normalized 0..1 units.
    #[serde(default = "default_depth")]
    pub depth: f32,
    /// Phase offset in cycles (0..1).
    #[serde(default)]
    pub phase: f32,
    /// When true, LFO output is -1..1; when false, 0..1.
    #[serde(default)]
    pub bipolar: bool,
    /// Base normalized value used when no automation lane drives the same target.
    #[serde(default = "default_center")]
    pub center: f32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Points for [`LfoShape::Custom`]. `beat` is cycle position 0..1; `value` is 0..1.
    #[serde(default)]
    pub mseg_points: Vec<AutomationPoint>,
    /// Legacy on-disk field from pre-normalized MSEG saves. Sync uses [`LfoRate`] now.
    #[serde(default = "default_mseg_length_beats")]
    pub mseg_length_beats: f32,
    /// Horizontal snap for the MSEG editor (e.g. 16 = 1/16 steps). 0 = off.
    #[serde(default = "default_mseg_grid_divisions")]
    pub mseg_grid_divisions: u8,
}

impl LfoModulator {
    pub fn new(id: u64, target: AutomationTarget) -> Self {
        Self {
            id,
            target,
            param_name: String::new(),
            shape: LfoShape::Sine,
            rate: LfoRate::default(),
            depth: default_depth(),
            phase: 0.0,
            bipolar: true,
            center: default_center(),
            enabled: true,
            mseg_points: Vec::new(),
            mseg_length_beats: default_mseg_length_beats(),
            mseg_grid_divisions: default_mseg_grid_divisions(),
        }
    }

    /// Returns true when points are stored in the legacy absolute-beat layout.
    pub fn mseg_needs_legacy_normalize(&self) -> bool {
        let length = self.mseg_length_beats.max(0.0625);
        length != 1.0 || self.mseg_points.iter().any(|p| p.beat > 1.0 + f32::EPSILON)
    }

    /// Legacy loop length in beats to preserve timing after normalizing X to 0..1.
    pub fn mseg_legacy_cycle_beats(&self) -> f32 {
        if self.mseg_needs_legacy_normalize() {
            self.mseg_length_beats.max(0.0625)
        } else {
            1.0
        }
    }
}
