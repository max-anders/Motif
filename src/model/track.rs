use serde::{Deserialize, Serialize};

use super::automation::AutomationLane;
use super::clip::{Clip, MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
use super::instrument::TrackInstrument;
use super::mixer::{db_to_linear, pan_gains, Device, Macro, Send};
use super::modulator::LfoModulator;
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
    /// Host macro knobs that drive plugin params and/or modulator controls.
    #[serde(default)]
    pub macros: Vec<Macro>,
    /// Beat/value automation curves for instrument or insert-FX parameters.
    #[serde(default)]
    pub automation_lanes: Vec<AutomationLane>,
    /// Host-side LFO / MSEG modulators for instrument or insert-FX parameters.
    #[serde(default)]
    pub modulators: Vec<LfoModulator>,
    #[serde(default)]
    pub instrument: TrackInstrument,
    /// Opaque CLAP/VST3 state (RKST envelope). Restored after plugin activate.
    /// Kept off `TrackInstrument` so identity sync does not reload on every save.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_b64")]
    pub plugin_state: Option<Vec<u8>>,
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn gain_linear(&self) -> f32 {
        db_to_linear(self.gain_db)
    }

    pub fn pan_gains(&self) -> (f32, f32) {
        pan_gains(self.pan)
    }

    pub fn remove_clip(&mut self, clip_id: u64) {
        self.clips.retain(|clip| clip.id() != clip_id);
    }

    pub fn clip_mut(&mut self, clip_id: u64) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|clip| clip.id() == clip_id)
    }

    pub fn clip(&self, clip_id: u64) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id() == clip_id)
    }

    pub fn midi_clip_mut(&mut self, clip_id: u64) -> Option<&mut MidiClip> {
        self.clip_mut(clip_id).and_then(Clip::as_midi_mut)
    }

    pub fn midi_clip(&self, clip_id: u64) -> Option<&MidiClip> {
        self.clip(clip_id).and_then(Clip::as_midi)
    }

    /// True if `[start, end)` overlaps any clip whose id is not in `ignore_ids`.
    pub fn range_overlaps_any(&self, start: f32, end: f32, ignore_ids: &[u64]) -> bool {
        self.clips.iter().any(|clip| {
            if ignore_ids.contains(&clip.id()) {
                return false;
            }
            Project::beat_ranges_overlap(start, end, clip.start_beats(), clip.end_beats())
        })
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

    MidiClip::with_single_variation(
        1,
        String::from("Clip 1"),
        0.0,
        length,
        1,
        notes,
    )
}
