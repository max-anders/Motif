//! Per-track sound source selection (built-in piano or hosted plugin).

use serde::{Deserialize, Serialize};

/// Wire format for a CLAP or VST3 plugin identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    Clap,
    Vst3,
}

impl Default for PluginFormat {
    /// Arbitrary but stable default so `#[serde(default)]` can restore legacy
    /// device placeholders that predate plugin identity (never hosted, so the
    /// exact format is inert until a real plugin is assigned).
    fn default() -> Self {
        Self::Clap
    }
}

impl PluginFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clap => "clap",
            Self::Vst3 => "vst3",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
        }
    }

    pub fn from_rack_format(format: &str) -> Option<Self> {
        match format {
            "clap" => Some(Self::Clap),
            "vst3" => Some(Self::Vst3),
            _ => None,
        }
    }
}

/// Instrument assigned to a track. Default is the built-in soft piano.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrackInstrument {
    BuiltInPiano,
    Plugin {
        format: PluginFormat,
        unique_id: String,
        name: String,
    },
}

impl Default for TrackInstrument {
    fn default() -> Self {
        Self::BuiltInPiano
    }
}

impl TrackInstrument {
    pub fn display_name(&self) -> &str {
        match self {
            Self::BuiltInPiano => "Built-in Piano",
            Self::Plugin { name, .. } => name.as_str(),
        }
    }

    pub fn format_badge(&self) -> Option<&'static str> {
        match self {
            Self::BuiltInPiano => None,
            Self::Plugin { format, .. } => Some(format.label()),
        }
    }
}
