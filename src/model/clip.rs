use serde::{Deserialize, Serialize};

use super::audio_clip::AudioClip;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Clip {
    Midi(MidiClip),
    Audio(AudioClip),
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

    /// True if `[start, end)` overlaps any same-pitch note whose id is not in `ignore_ids`.
    /// Touching endpoints do not count (half-open), matching clip overlap.
    pub fn note_range_overlaps_any(
        &self,
        pitch: u8,
        start: f32,
        end: f32,
        ignore_ids: &[u64],
    ) -> bool {
        self.notes.iter().any(|note| {
            if note.pitch != pitch || ignore_ids.contains(&note.id) {
                return false;
            }
            Project::beat_ranges_overlap(start, end, note.start_beats, note.end_beats())
        })
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

impl Clip {
    pub fn id(&self) -> u64 {
        match self {
            Self::Midi(clip) => clip.id,
            Self::Audio(clip) => clip.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Midi(clip) => clip.name.as_str(),
            Self::Audio(clip) => clip.name.as_str(),
        }
    }

    pub fn start_beats(&self) -> f32 {
        match self {
            Self::Midi(clip) => clip.start_beats,
            Self::Audio(clip) => clip.start_beats,
        }
    }

    pub fn set_start_beats(&mut self, start: f32) {
        let start = Project::snap_beats(start.max(0.0));
        match self {
            Self::Midi(clip) => clip.start_beats = start,
            Self::Audio(clip) => clip.start_beats = start,
        }
    }

    pub fn length_beats(&self) -> f32 {
        match self {
            Self::Midi(clip) => clip.length_beats,
            Self::Audio(clip) => clip.length_beats,
        }
    }

    pub fn set_length_beats(&mut self, length: f32) {
        let length = Project::snap_beats(length.max(SNAP_BEATS));
        match self {
            Self::Midi(clip) => clip.length_beats = length,
            Self::Audio(clip) => clip.length_beats = length,
        }
    }

    pub fn end_beats(&self) -> f32 {
        self.start_beats() + self.length_beats()
    }

    pub fn as_midi(&self) -> Option<&MidiClip> {
        match self {
            Self::Midi(clip) => Some(clip),
            Self::Audio(_) => None,
        }
    }

    pub fn as_midi_mut(&mut self) -> Option<&mut MidiClip> {
        match self {
            Self::Midi(clip) => Some(clip),
            Self::Audio(_) => None,
        }
    }

    pub fn as_audio(&self) -> Option<&AudioClip> {
        match self {
            Self::Audio(clip) => Some(clip),
            Self::Midi(_) => None,
        }
    }

    pub fn as_audio_mut(&mut self) -> Option<&mut AudioClip> {
        match self {
            Self::Audio(clip) => Some(clip),
            Self::Midi(_) => None,
        }
    }
}
