use serde::{Deserialize, Serialize};

use super::Note;

pub const DEFAULT_BPM: f32 = 120.0;
pub const DEFAULT_BEATS_PER_BAR: f32 = 4.0;
pub const SNAP_BEATS: f32 = 0.25;
pub const MIN_PITCH: u8 = 48;
pub const MAX_PITCH: u8 = 84;
pub const DEFAULT_NOTE_DURATION_BEATS: f32 = 1.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub bpm: f32,
    pub beats_per_bar: f32,
    pub loop_end_beats: f32,
    pub notes: Vec<Note>,
    next_note_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            bpm: DEFAULT_BPM,
            beats_per_bar: DEFAULT_BEATS_PER_BAR,
            loop_end_beats: 16.0,
            notes: Vec::new(),
            next_note_id: 1,
        }
    }
}

impl Project {
    pub fn snap_beats(value: f32) -> f32 {
        (value / SNAP_BEATS).round() * SNAP_BEATS
    }

    pub fn clamp_pitch(pitch: i32) -> u8 {
        pitch.clamp(MIN_PITCH as i32, MAX_PITCH as i32) as u8
    }

    pub fn add_note(&mut self, pitch: u8, start_beats: f32, duration_beats: f32) -> Note {
        let note = Note {
            id: self.next_note_id,
            pitch,
            start_beats: Self::snap_beats(start_beats.max(0.0)),
            duration_beats: Self::snap_beats(duration_beats.max(SNAP_BEATS)),
            velocity: 100,
        };
        self.next_note_id += 1;
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

    pub fn beats_per_second(&self) -> f32 {
        self.bpm / 60.0
    }
}
