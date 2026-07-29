use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub id: u64,
    pub name: String,
    /// Arrangement position in beats.
    pub start_beats: f32,
    /// Visible length on the playlist timeline.
    pub length_beats: f32,
    /// Absolute or project-relative source file path.
    pub source: PathBuf,
    /// Per-clip pre-fader gain in dB (0 = unity).
    #[serde(default)]
    pub gain_db: f32,
    /// When true, this clip is silent during playback.
    #[serde(default)]
    pub muted: bool,
    /// Runtime-only state, set true when the source cannot be resolved.
    #[serde(skip)]
    pub missing: bool,
}
