//! Session clipboard for notes and clips (not persisted).

use super::Note;

/// Note payload relative to the selection's earliest start.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardNote {
    pub pitch: u8,
    pub start_beats: f32,
    pub duration_beats: f32,
    pub velocity: u8,
}

/// One variation payload for a clipboard MIDI clip.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardVariation {
    pub name: String,
    /// Note times stay clip-local (unchanged on paste).
    pub notes: Vec<ClipboardNote>,
}

/// Clip payload relative to the selection's earliest arrangement start.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipboardClip {
    pub track_id: u64,
    pub name: String,
    pub start_beats: f32,
    pub length_beats: f32,
    /// All takes; paste remaps note/variation ids. Index of the active take.
    pub variations: Vec<ClipboardVariation>,
    pub active_variation_index: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum EditClipboard {
    #[default]
    Empty,
    Notes(Vec<ClipboardNote>),
    Clips(Vec<ClipboardClip>),
}

impl EditClipboard {
    pub fn from_notes(notes: &[Note]) -> Self {
        if notes.is_empty() {
            return Self::Empty;
        }
        let origin = notes
            .iter()
            .map(|note| note.start_beats)
            .fold(f32::INFINITY, f32::min);
        let entries = notes
            .iter()
            .map(|note| ClipboardNote {
                pitch: note.pitch,
                start_beats: (note.start_beats - origin).max(0.0),
                duration_beats: note.duration_beats,
                velocity: note.velocity,
            })
            .collect();
        Self::Notes(entries)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Notes(notes) => notes.is_empty(),
            Self::Clips(clips) => clips.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_clipboard_normalizes_to_earliest_start() {
        let notes = [
            Note {
                id: 1,
                pitch: 60,
                start_beats: 2.0,
                duration_beats: 1.0,
                velocity: 100,
            },
            Note {
                id: 2,
                pitch: 64,
                start_beats: 3.5,
                duration_beats: 0.5,
                velocity: 80,
            },
        ];
        let EditClipboard::Notes(entries) = EditClipboard::from_notes(&notes) else {
            panic!("expected notes clipboard");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].start_beats, 0.0);
        assert_eq!(entries[1].start_beats, 1.5);
        assert_eq!(entries[1].velocity, 80);
    }
}
