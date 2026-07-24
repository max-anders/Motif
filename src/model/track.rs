use serde::{Deserialize, Serialize};

use super::clip::{MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
use super::instrument::TrackInstrument;
use super::{Note, Project};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub instrument: TrackInstrument,
    pub clips: Vec<MidiClip>,
}

impl Track {
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
