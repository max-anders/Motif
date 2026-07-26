//! Per-track mixer facets: gain/pan math plus sends/devices/macros.
//!
//! Sends are serialized project data today but not yet processed by the audio
//! engine. Devices and macros bind to live engine / track data; Routing remains
//! future scaffolding on these same fields.

use serde::{Deserialize, Serialize};

use super::instrument::PluginFormat;
use super::serde_b64;

/// Soft mute floor for the channel fader (linear ~0).
pub const MIN_GAIN_DB: f32 = -60.0;
/// Practical headroom above unity.
pub const MAX_GAIN_DB: f32 = 12.0;

/// Aux send slot (routing not processed yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Send {
    /// Destination track id when set; `None` means unassigned.
    #[serde(default)]
    pub target_track: Option<u64>,
    #[serde(default)]
    pub level_db: f32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for Send {
    fn default() -> Self {
        Self {
            target_track: None,
            level_db: 0.0,
            enabled: true,
        }
    }
}

/// Insert effect: a hosted CLAP/VST3 plugin in a track's serial FX chain.
/// Processed after the instrument voice and before gain/pan (see `engine::audio`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: u64,
    /// Display name (defaults to the plugin's catalog name).
    pub name: String,
    /// Plugin format. Defaults to CLAP for legacy placeholder devices that
    /// predate plugin identity (empty `unique_id`, never hosted).
    #[serde(default)]
    pub format: PluginFormat,
    /// Catalog unique id. Empty for legacy placeholder devices.
    #[serde(default)]
    pub unique_id: String,
    #[serde(default)]
    pub bypassed: bool,
    /// Opaque CLAP/VST3 state (RKST envelope). Same encoding as `Track.plugin_state`.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_b64")]
    pub plugin_state: Option<Vec<u8>>,
}

impl Device {
    /// Build a device from a plugin identity (catalog selection). Fresh slot,
    /// not bypassed, no saved state.
    pub fn new_plugin(
        id: u64,
        format: PluginFormat,
        unique_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            format,
            unique_id: unique_id.into(),
            bypassed: false,
            plugin_state: None,
        }
    }

    pub fn format_badge(&self) -> &'static str {
        self.format.label()
    }
}

/// Where a macro knob writes its scaled value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroTarget {
    Instrument { param_id: u32 },
    Device { device_id: u64, param_id: u32 },
    ModulatorRate { modulator_id: u64 },
    ModulatorDepth { modulator_id: u64 },
}

fn default_mapping_min() -> f32 {
    0.0
}

fn default_mapping_max() -> f32 {
    1.0
}

/// One destination of a macro, with a normalized output range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroMapping {
    pub target: MacroTarget,
    /// Cached for display when the plugin is not loaded.
    #[serde(default)]
    pub param_name: String,
    #[serde(default = "default_mapping_min")]
    pub min: f32,
    #[serde(default = "default_mapping_max")]
    pub max: f32,
}

impl MacroMapping {
    pub fn new(target: MacroTarget) -> Self {
        Self {
            target,
            param_name: String::new(),
            min: default_mapping_min(),
            max: default_mapping_max(),
        }
    }

    /// Destination value for a macro knob in 0..1.
    pub fn mapped_value(&self, macro_value: f32) -> f32 {
        let t = macro_value.clamp(0.0, 1.0);
        self.min + (self.max - self.min) * t
    }
}

/// Named host macro knob (0..1) with zero or more destinations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    /// Stable id. Missing on legacy saves; assigned on load via [`Project::ensure_macro_ids`].
    #[serde(default)]
    pub id: u64,
    pub name: String,
    /// Normalized 0..1.
    #[serde(default)]
    pub value: f32,
    #[serde(default)]
    pub mappings: Vec<MacroMapping>,
}

impl Default for Macro {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::from("Macro"),
            value: 0.0,
            mappings: Vec::new(),
        }
    }
}

impl Macro {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            value: 0.0,
            mappings: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Convert dB to a linear amplitude multiplier. Floors at [`MIN_GAIN_DB`].
pub fn db_to_linear(db: f32) -> f32 {
    let clamped = db.clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    if clamped <= MIN_GAIN_DB {
        return 0.0;
    }
    10.0_f32.powf(clamped / 20.0)
}

/// Equal-power stereo pan. `pan` in -1 (full left) .. +1 (full right).
/// Returns `(left_gain, right_gain)` multipliers (each in 0..1 at the edges).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    // Map -1..1 -> 0..1 then equal-power.
    let t = (p + 1.0) * 0.5;
    let angle = t * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_to_linear_unity_and_mute() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-5);
        assert_eq!(db_to_linear(MIN_GAIN_DB), 0.0);
        assert_eq!(db_to_linear(MIN_GAIN_DB - 10.0), 0.0);
        let half = db_to_linear(-6.0);
        assert!((half - 0.501187).abs() < 0.01);
    }

    #[test]
    fn pan_gains_center_and_edges() {
        let (l, r) = pan_gains(0.0);
        let center = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l - center).abs() < 1e-5);
        assert!((r - center).abs() < 1e-5);

        let (l, r) = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-5);
        assert!(r.abs() < 1e-5);

        let (l, r) = pan_gains(1.0);
        assert!(l.abs() < 1e-5);
        assert!((r - 1.0).abs() < 1e-5);
    }

    #[test]
    fn macro_mapping_lerps_range() {
        let mapping = MacroMapping {
            target: MacroTarget::Instrument { param_id: 1 },
            param_name: String::from("Cut"),
            min: 0.25,
            max: 0.75,
        };
        assert!((mapping.mapped_value(0.0) - 0.25).abs() < 1e-5);
        assert!((mapping.mapped_value(1.0) - 0.75).abs() < 1e-5);
        assert!((mapping.mapped_value(0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn legacy_macro_json_loads_defaults() {
        let json = r#"{"name":"A","value":0.25}"#;
        let m: Macro = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, 0);
        assert_eq!(m.name, "A");
        assert_eq!(m.value, 0.25);
        assert!(m.mappings.is_empty());
    }
}
