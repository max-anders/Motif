use serde::{Deserialize, Serialize};

use super::{Note, Project, SNAP_BEATS};

pub const DEFAULT_CLIP_LENGTH_BEATS: f32 = 4.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiClip {
    pub id: u64,
    pub name: String,
    /// Arrangement position in beats.
    pub start_beats: f32,
    /// Visible length on the playlist timeline.
    pub length_beats: f32,
    pub notes: Vec<Note>,
}

impl MidiClip {
    pub fn end_beats(&self) -> f32 {
        self.start_beats + self.length_beats
    }

    pub fn add_note_with_id(
        &mut self,
        id: u64,
        pitch: u8,
        start_beats: f32,
        duration_beats: f32,
    ) -> Note {
        let note = Note {
            id,
            pitch,
            start_beats: Project::snap_beats(start_beats.max(0.0)),
            duration_beats: Project::snap_beats(duration_beats.max(SNAP_BEATS)),
            velocity: 100,
        };
        self.notes.push(note);
        note
    }

    pub fn remove_note(&mut self, id: u64) {
        self.notes.retain(|note| note.id != id);
    }

    pub fn note_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|note| note.id == id)
    }

    pub fn note(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|note| note.id == id)
    }

    pub fn set_start_beats(&mut self, start: f32) {
        self.start_beats = Project::snap_beats(start.max(0.0));
    }

    pub fn set_length_beats(&mut self, length: f32) {
        self.length_beats = Project::snap_beats(length.max(SNAP_BEATS));
    }
}
