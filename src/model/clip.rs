use serde::{Deserialize, Deserializer, Serialize};

use super::audio_clip::AudioClip;
use super::{Note, Project, SNAP_BEATS};

pub const DEFAULT_CLIP_LENGTH_BEATS: f32 = 4.0;

/// One alternate note take for a MIDI clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiVariation {
    pub id: u64,
    pub name: String,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MidiClip {
    pub id: u64,
    pub name: String,
    /// Arrangement position in beats.
    pub start_beats: f32,
    /// Visible length on the playlist timeline.
    pub length_beats: f32,
    /// Alternate takes; always non-empty after normalize / constructors.
    pub variations: Vec<MidiVariation>,
    pub active_variation_id: u64,
}

#[derive(Debug, Deserialize)]
struct MidiClipDe {
    id: u64,
    name: String,
    start_beats: f32,
    length_beats: f32,
    #[serde(default)]
    notes: Option<Vec<Note>>,
    #[serde(default)]
    variations: Option<Vec<MidiVariation>>,
    #[serde(default)]
    active_variation_id: Option<u64>,
}

impl<'de> Deserialize<'de> for MidiClip {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = MidiClipDe::deserialize(deserializer)?;
        Ok(MidiClip::from_de(raw))
    }
}

impl MidiClip {
    fn from_de(raw: MidiClipDe) -> Self {
        let variations = match raw.variations {
            Some(v) if !v.is_empty() => v,
            _ => {
                let notes = raw.notes.unwrap_or_default();
                vec![MidiVariation {
                    // Temporary id; Project::normalize_clip_variations remaps globally.
                    id: 1,
                    name: String::from("A"),
                    notes,
                }]
            }
        };
        let active_variation_id = raw
            .active_variation_id
            .filter(|id| variations.iter().any(|v| v.id == *id))
            .unwrap_or_else(|| variations[0].id);
        Self {
            id: raw.id,
            name: raw.name,
            start_beats: raw.start_beats,
            length_beats: raw.length_beats,
            variations,
            active_variation_id,
        }
    }

    /// Build a clip with a single variation holding `notes`.
    pub fn with_single_variation(
        id: u64,
        name: String,
        start_beats: f32,
        length_beats: f32,
        variation_id: u64,
        notes: Vec<Note>,
    ) -> Self {
        Self {
            id,
            name,
            start_beats,
            length_beats,
            variations: vec![MidiVariation {
                id: variation_id,
                name: String::from("A"),
                notes,
            }],
            active_variation_id: variation_id,
        }
    }

    pub fn end_beats(&self) -> f32 {
        self.start_beats + self.length_beats
    }

    pub fn active_notes(&self) -> &[Note] {
        self.active_variation()
            .map(|v| v.notes.as_slice())
            .unwrap_or(&[])
    }

    pub fn active_notes_mut(&mut self) -> &mut Vec<Note> {
        if self.variations.is_empty() {
            self.variations.push(MidiVariation {
                id: 1,
                name: String::from("A"),
                notes: Vec::new(),
            });
            self.active_variation_id = 1;
        }
        let index = self
            .variations
            .iter()
            .position(|v| v.id == self.active_variation_id)
            .unwrap_or(0);
        self.active_variation_id = self.variations[index].id;
        &mut self.variations[index].notes
    }

    pub fn active_variation(&self) -> Option<&MidiVariation> {
        self.variations
            .iter()
            .find(|v| v.id == self.active_variation_id)
            .or_else(|| self.variations.first())
    }

    pub fn active_variation_mut(&mut self) -> Option<&mut MidiVariation> {
        let id = self.active_variation_id;
        if self.variations.iter().any(|v| v.id == id) {
            return self.variations.iter_mut().find(|v| v.id == id);
        }
        self.variations.first_mut()
    }

    pub fn variation(&self, variation_id: u64) -> Option<&MidiVariation> {
        self.variations.iter().find(|v| v.id == variation_id)
    }

    pub fn variation_mut(&mut self, variation_id: u64) -> Option<&mut MidiVariation> {
        self.variations.iter_mut().find(|v| v.id == variation_id)
    }

    /// Next default letter name (`A`, `B`, ...).
    pub fn next_variation_name(&self) -> String {
        variation_name_for_index(self.variations.len())
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
        self.active_notes_mut().push(note);
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
        self.active_notes().iter().any(|note| {
            if note.pitch != pitch || ignore_ids.contains(&note.id) {
                return false;
            }
            Project::beat_ranges_overlap(start, end, note.start_beats, note.end_beats())
        })
    }

    pub fn remove_note(&mut self, id: u64) {
        self.active_notes_mut().retain(|note| note.id != id);
    }

    pub fn note_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.active_notes_mut().iter_mut().find(|note| note.id == id)
    }

    pub fn note(&self, id: u64) -> Option<&Note> {
        self.active_notes().iter().find(|note| note.id == id)
    }

    pub fn set_start_beats(&mut self, start: f32) {
        self.start_beats = Project::snap_beats(start.max(0.0));
    }

    pub fn set_length_beats(&mut self, length: f32) {
        self.length_beats = Project::snap_beats(length.max(SNAP_BEATS));
    }
}

/// `0 -> "A"`, `25 -> "Z"`, `26 -> "AA"`, ...
pub fn variation_name_for_index(index: usize) -> String {
    let mut n = index;
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Clip {
    Midi(MidiClip),
    Audio(AudioClip),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_notes_field_deserializes_to_single_variation() {
        let json = r#"{
            "id": 1,
            "name": "Clip 1",
            "start_beats": 0.0,
            "length_beats": 4.0,
            "notes": [
                {"id": 1, "pitch": 60, "start_beats": 0.0, "duration_beats": 1.0, "velocity": 100}
            ]
        }"#;
        let clip: MidiClip = serde_json::from_str(json).expect("parse");
        assert_eq!(clip.variations.len(), 1);
        assert_eq!(clip.variations[0].name, "A");
        assert_eq!(clip.active_notes().len(), 1);
        assert_eq!(clip.active_notes()[0].pitch, 60);
    }

    #[test]
    fn variation_names_use_letters() {
        assert_eq!(variation_name_for_index(0), "A");
        assert_eq!(variation_name_for_index(1), "B");
        assert_eq!(variation_name_for_index(25), "Z");
        assert_eq!(variation_name_for_index(26), "AA");
    }
}
