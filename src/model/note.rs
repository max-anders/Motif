use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub pitch: u8,
    /// Start time in beats from the beginning of the project.
    pub start_beats: f32,
    /// Length in beats.
    pub duration_beats: f32,
    pub velocity: u8,
}

impl Note {
    pub fn end_beats(&self) -> f32 {
        self.start_beats + self.duration_beats
    }

    pub fn contains_beat(&self, beat: f32) -> bool {
        beat >= self.start_beats && beat < self.end_beats()
    }
}
