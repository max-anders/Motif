use serde::{Deserialize, Serialize};

use super::clip::{MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
use super::instrument::TrackInstrument;
use super::mixer::{db_to_linear, pan_gains, Device, Macro, Send};
use super::serde_b64;
use super::{Note, Project};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    /// When true, track is silent unless another track is soloed (solo overrides mute).
    #[serde(default)]
    pub muted: bool,
    /// When any track is soloed, only soloed tracks are audible.
    #[serde(default)]
    pub solo: bool,
    /// Channel fader in dB (0 = unity). Applied after the instrument voice.
    #[serde(default)]
    pub gain_db: f32,
    /// Stereo pan in -1 (left) .. +1 (right).
    #[serde(default)]
    pub pan: f32,
    /// Aux sends (serialized; not processed yet).
    #[serde(default)]
    pub sends: Vec<Send>,
    /// Serial insert FX chain (hosted CLAP/VST3 plugins), processed after the
    /// instrument voice and before gain/pan. Vec order is the chain order.
    #[serde(default)]
    pub devices: Vec<Device>,
    /// Macro knobs (serialized; modulation not wired yet).
    #[serde(default)]
    pub macros: Vec<Macro>,
    #[serde(default)]
    pub instrument: TrackInstrument,
    /// Opaque CLAP/VST3 state (RKST envelope). Restored after plugin activate.
    /// Kept off `TrackInstrument` so identity sync does not reload on every save.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_b64")]
    pub plugin_state: Option<Vec<u8>>,
    pub clips: Vec<MidiClip>,
}

impl Track {
    pub fn gain_linear(&self) -> f32 {
        db_to_linear(self.gain_db)
    }

    pub fn pan_gains(&self) -> (f32, f32) {
        pan_gains(self.pan)
    }

    pub fn remove_clip(&mut self, clip_id: u64) {
        self.clips.retain(|clip| clip.id != clip_id);
    }

    pub fn clip_mut(&mut self, clip_id: u64) -> Option<&mut MidiClip> {
        self.clips.iter_mut().find(|clip| clip.id == clip_id)
    }

    pub fn clip(&self, clip_id: u64) -> Option<&MidiClip> {
        self.clips.iter().find(|clip| clip.id == clip_id)
    }
}

pub fn migrate_notes_to_clip(notes: Vec<Note>, loop_end_beats: f32) -> MidiClip {
    let content_end = notes
        .iter()
        .map(|note| note.end_beats())
        .fold(0.0_f32, f32::max);
    let length = Project::snap_beats(
        loop_end_beats
            .max(content_end)
            .max(DEFAULT_CLIP_LENGTH_BEATS),
    );

    MidiClip {
        id: 1,
        name: String::from("Clip 1"),
        start_beats: 0.0,
        length_beats: length,
        notes,
    }
}
