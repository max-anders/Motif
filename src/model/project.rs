use serde::{Deserialize, Serialize};

use super::clip::MidiClip;
use super::clipboard::{ClipboardClip, ClipboardNote};
use super::instrument::TrackInstrument;
use super::track::{migrate_notes_to_clip, Track};
use super::Note;

pub const DEFAULT_BPM: f32 = 120.0;
pub const DEFAULT_BEATS_PER_BAR: f32 = 4.0;
pub const SNAP_BEATS: f32 = 0.25;
/// Full MIDI note range (C-1 .. G9), like a normal DAW piano roll.
pub const MIN_PITCH: u8 = 0;
pub const MAX_PITCH: u8 = 127;
pub const DEFAULT_NOTE_DURATION_BEATS: f32 = 1.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub bpm: f32,
    pub beats_per_bar: f32,
    pub loop_end_beats: f32,
    pub tracks: Vec<Track>,
    next_note_id: u64,
    next_clip_id: u64,
    next_track_id: u64,
}

/// Legacy on-disk format before tracks/clips.
#[derive(Debug, Deserialize)]
struct LegacyProject {
    bpm: f32,
    beats_per_bar: f32,
    loop_end_beats: f32,
    notes: Vec<super::Note>,
    next_note_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        let mut project = Self {
            bpm: DEFAULT_BPM,
            beats_per_bar: DEFAULT_BEATS_PER_BAR,
            loop_end_beats: 16.0,
            tracks: Vec::new(),
            next_note_id: 1,
            next_clip_id: 2,
            next_track_id: 2,
        };
        let track_id = project.add_track("Track 1", TrackInstrument::BuiltInPiano);
        project.add_clip_to_track(track_id, 0.0, 4.0);
        project
    }
}

impl Project {
    pub fn snap_beats(value: f32) -> f32 {
        (value / SNAP_BEATS).round() * SNAP_BEATS
    }

    pub fn clamp_pitch(pitch: i32) -> u8 {
        pitch.clamp(MIN_PITCH as i32, MAX_PITCH as i32) as u8
    }

    pub fn next_note_id(&self) -> u64 {
        self.next_note_id
    }

    pub fn bump_note_id(&mut self) {
        self.next_note_id += 1;
    }

    pub fn next_clip_id(&self) -> u64 {
        self.next_clip_id
    }

    pub fn bump_clip_id(&mut self) {
        self.next_clip_id += 1;
    }

    pub fn next_track_id(&self) -> u64 {
        self.next_track_id
    }

    pub fn bump_track_id(&mut self) {
        self.next_track_id += 1;
    }

    pub fn add_track(&mut self, name: &str, instrument: TrackInstrument) -> u64 {
        let id = self.next_track_id();
        self.bump_track_id();
        self.tracks.push(Track {
            id,
            name: name.to_string(),
            instrument,
            plugin_state: None,
            clips: Vec::new(),
        });
        id
    }

    pub fn add_clip_to_track(
        &mut self,
        track_id: u64,
        start_beats: f32,
        length_beats: f32,
    ) -> Option<u64> {
        let clip_number = self.track(track_id)?.clips.len() + 1;
        let clip_id = self.next_clip_id();
        self.bump_clip_id();
        let clip = MidiClip {
            id: clip_id,
            name: format!("Clip {clip_number}"),
            start_beats: Self::snap_beats(start_beats.max(0.0)),
            length_beats: Self::snap_beats(length_beats.max(SNAP_BEATS)),
            notes: Vec::new(),
        };
        self.track_mut(track_id)?.clips.push(clip);
        Some(clip_id)
    }

    pub fn add_note_to_clip(
        &mut self,
        clip_id: u64,
        pitch: u8,
        start_beats: f32,
        duration_beats: f32,
    ) -> Option<Note> {
        let id = self.next_note_id();
        self.bump_note_id();
        let note = self
            .clip_mut(clip_id)?
            .add_note_with_id(id, pitch, start_beats, duration_beats);
        Some(note)
    }

    pub fn remove_track(&mut self, track_id: u64) {
        self.tracks.retain(|track| track.id != track_id);
    }

    pub fn track_mut(&mut self, track_id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == track_id)
    }

    pub fn track(&self, track_id: u64) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    pub fn clip_mut(&mut self, clip_id: u64) -> Option<&mut MidiClip> {
        for track in &mut self.tracks {
            if let Some(clip) = track.clip_mut(clip_id) {
                return Some(clip);
            }
        }
        None
    }

    pub fn clip(&self, clip_id: u64) -> Option<&MidiClip> {
        for track in &self.tracks {
            if let Some(clip) = track.clip(clip_id) {
                return Some(clip);
            }
        }
        None
    }

    pub fn remove_clip(&mut self, clip_id: u64) {
        for track in &mut self.tracks {
            track.remove_clip(clip_id);
        }
    }

    /// Duplicate notes inside a clip. Returns new note ids in the same order as `note_ids`
    /// (skipping missing ids). Pitch/start offsets are applied after cloning.
    pub fn duplicate_notes_in_clip(
        &mut self,
        clip_id: u64,
        note_ids: &[u64],
        delta_beats: f32,
        delta_pitch: i32,
    ) -> Vec<u64> {
        let Some(clip) = self.clip(clip_id) else {
            return Vec::new();
        };
        let templates: Vec<Note> = note_ids
            .iter()
            .filter_map(|id| clip.note(*id).copied())
            .collect();
        if templates.is_empty() {
            return Vec::new();
        }

        let mut new_ids = Vec::with_capacity(templates.len());
        for template in templates {
            let id = self.next_note_id();
            self.bump_note_id();
            let pitch = Self::clamp_pitch(template.pitch as i32 + delta_pitch);
            let start = (template.start_beats + delta_beats).max(0.0);
            if let Some(clip) = self.clip_mut(clip_id) {
                clip.add_note_with_id(id, pitch, start, template.duration_beats);
                if let Some(note) = clip.note_mut(id) {
                    note.velocity = template.velocity;
                }
                new_ids.push(id);
            }
        }
        new_ids
    }

    /// Collect notes by id for clipboard (order follows `note_ids`, skipping missing).
    pub fn notes_for_clipboard(&self, clip_id: u64, note_ids: &[u64]) -> Vec<Note> {
        let Some(clip) = self.clip(clip_id) else {
            return Vec::new();
        };
        note_ids
            .iter()
            .filter_map(|id| clip.note(*id).copied())
            .collect()
    }

    /// Paste clipboard notes into a clip. `origin_beats` is clip-local; entry starts are
    /// relative to that origin. Returns new note ids.
    pub fn paste_notes_into_clip(
        &mut self,
        clip_id: u64,
        notes: &[ClipboardNote],
        origin_beats: f32,
    ) -> Vec<u64> {
        if notes.is_empty() || self.clip(clip_id).is_none() {
            return Vec::new();
        }
        let origin = Self::snap_beats(origin_beats.max(0.0));
        let mut new_ids = Vec::with_capacity(notes.len());
        for template in notes {
            let id = self.next_note_id();
            self.bump_note_id();
            let start = (origin + template.start_beats).max(0.0);
            if let Some(clip) = self.clip_mut(clip_id) {
                clip.add_note_with_id(id, template.pitch, start, template.duration_beats);
                if let Some(note) = clip.note_mut(id) {
                    note.velocity = template.velocity;
                }
                new_ids.push(id);
            }
        }
        new_ids
    }

    /// Build clip clipboard entries (arrangement starts relative to earliest selected clip).
    pub fn clips_for_clipboard(&self, clip_ids: &[u64]) -> Vec<ClipboardClip> {
        let mut templates = Vec::new();
        for clip_id in clip_ids {
            let Some(track_id) = self.track_id_for_clip(*clip_id) else {
                continue;
            };
            let Some(clip) = self.clip(*clip_id) else {
                continue;
            };
            templates.push(ClipboardClip {
                track_id,
                name: clip.name.clone(),
                start_beats: clip.start_beats,
                length_beats: clip.length_beats,
                notes: clip
                    .notes
                    .iter()
                    .map(|note| ClipboardNote {
                        pitch: note.pitch,
                        start_beats: note.start_beats,
                        duration_beats: note.duration_beats,
                        velocity: note.velocity,
                    })
                    .collect(),
            });
        }
        if templates.is_empty() {
            return templates;
        }
        let origin = templates
            .iter()
            .map(|clip| clip.start_beats)
            .fold(f32::INFINITY, f32::min);
        for clip in &mut templates {
            clip.start_beats = (clip.start_beats - origin).max(0.0);
        }
        templates
    }

    /// Paste clipboard clips at an arrangement origin. Keeps each entry's track when present;
    /// otherwise skips. Returns new clip ids.
    pub fn paste_clips(&mut self, clips: &[ClipboardClip], origin_beats: f32) -> Vec<u64> {
        if clips.is_empty() {
            return Vec::new();
        }
        let origin = Self::snap_beats(origin_beats.max(0.0));
        let mut new_ids = Vec::with_capacity(clips.len());
        for template in clips {
            if self.track(template.track_id).is_none() {
                continue;
            }
            let clip_id = self.next_clip_id();
            self.bump_clip_id();
            let mut notes = Vec::with_capacity(template.notes.len());
            for note in &template.notes {
                let id = self.next_note_id();
                self.bump_note_id();
                notes.push(Note {
                    id,
                    pitch: note.pitch,
                    start_beats: Self::snap_beats(note.start_beats.max(0.0)),
                    duration_beats: Self::snap_beats(note.duration_beats.max(SNAP_BEATS)),
                    velocity: note.velocity,
                });
            }
            let clip = MidiClip {
                id: clip_id,
                name: format!("{} copy", template.name),
                start_beats: Self::snap_beats((origin + template.start_beats).max(0.0)),
                length_beats: template.length_beats,
                notes,
            };
            if let Some(track) = self.track_mut(template.track_id) {
                track.clips.push(clip);
                new_ids.push(clip_id);
            }
        }
        new_ids
    }

    /// Deep-copy clips (new clip + note ids) onto the same tracks, offset in time.
    /// Returns new clip ids in input order (skipping missing ids).
    pub fn duplicate_clips(&mut self, clip_ids: &[u64], delta_beats: f32) -> Vec<u64> {
        #[derive(Clone)]
        struct ClipTemplate {
            track_id: u64,
            name: String,
            start_beats: f32,
            length_beats: f32,
            notes: Vec<Note>,
        }

        let mut templates = Vec::with_capacity(clip_ids.len());
        for &clip_id in clip_ids {
            let Some(track_id) = self.track_id_for_clip(clip_id) else {
                continue;
            };
            let Some(clip) = self.clip(clip_id) else {
                continue;
            };
            templates.push(ClipTemplate {
                track_id,
                name: clip.name.clone(),
                start_beats: clip.start_beats,
                length_beats: clip.length_beats,
                notes: clip.notes.clone(),
            });
        }

        let mut new_ids = Vec::with_capacity(templates.len());
        for template in templates {
            let clip_id = self.next_clip_id();
            self.bump_clip_id();
            let mut notes = Vec::with_capacity(template.notes.len());
            for note in template.notes {
                let id = self.next_note_id();
                self.bump_note_id();
                notes.push(Note {
                    id,
                    pitch: note.pitch,
                    start_beats: note.start_beats,
                    duration_beats: note.duration_beats,
                    velocity: note.velocity,
                });
            }
            let clip = MidiClip {
                id: clip_id,
                name: format!("{} copy", template.name),
                start_beats: Self::snap_beats((template.start_beats + delta_beats).max(0.0)),
                length_beats: template.length_beats,
                notes,
            };
            if let Some(track) = self.track_mut(template.track_id) {
                track.clips.push(clip);
                new_ids.push(clip_id);
            }
        }
        new_ids
    }

    /// Beat span of a time selection: `max(end) - min(start)`, snapped, at least one grid step.
    pub fn selection_span_beats(ranges: impl IntoIterator<Item = (f32, f32)>) -> f32 {
        let mut min_start = f32::INFINITY;
        let mut max_end = f32::NEG_INFINITY;
        let mut any = false;
        for (start, end) in ranges {
            any = true;
            min_start = min_start.min(start);
            max_end = max_end.max(end);
        }
        if !any {
            return SNAP_BEATS;
        }
        Self::snap_beats((max_end - min_start).max(SNAP_BEATS))
    }

    pub fn track_id_for_clip(&self, clip_id: u64) -> Option<u64> {
        for track in &self.tracks {
            if track.clip(clip_id).is_some() {
                return Some(track.id);
            }
        }
        None
    }

    pub fn beats_per_second(&self) -> f32 {
        self.bpm / 60.0
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        // Versioned envelope (current on-disk format for .motif files).
        if let Ok(envelope) = serde_json::from_str::<super::persistence::ProjectEnvelope>(json) {
            return Ok(envelope.project);
        }

        // Bare Project (pre-envelope project.json / early saves).
        if let Ok(project) = serde_json::from_str::<Self>(json) {
            return Ok(project);
        }

        // Flat-notes legacy before tracks/clips.
        let legacy: LegacyProject = serde_json::from_str(json)?;
        let clip = migrate_notes_to_clip(legacy.notes, legacy.loop_end_beats);
        let track = Track {
            id: 1,
            name: String::from("Track 1"),
            instrument: TrackInstrument::BuiltInPiano,
            plugin_state: None,
            clips: vec![clip],
        };

        Ok(Self {
            bpm: legacy.bpm,
            beats_per_bar: legacy.beats_per_bar,
            loop_end_beats: legacy.loop_end_beats,
            tracks: vec![track],
            next_note_id: legacy.next_note_id,
            next_clip_id: 2,
            next_track_id: 2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClipboardNote, EditClipboard};

    #[test]
    fn paste_notes_into_another_clip_at_origin() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id;
        let dst = project
            .add_clip_to_track(track_id, 4.0, 4.0)
            .expect("dst clip");

        let a = project
            .add_note_to_clip(src, 60, 1.0, 1.0)
            .expect("note a");
        let b = project
            .add_note_to_clip(src, 64, 2.0, 0.5)
            .expect("note b");
        if let Some(note) = project.clip_mut(src).and_then(|c| c.note_mut(b.id)) {
            note.velocity = 77;
        }

        let clipboard = EditClipboard::from_notes(&project.notes_for_clipboard(src, &[a.id, b.id]));
        let EditClipboard::Notes(entries) = clipboard else {
            panic!("notes");
        };
        let new_ids = project.paste_notes_into_clip(dst, &entries, 0.5);
        assert_eq!(new_ids.len(), 2);

        let dst_clip = project.clip(dst).expect("dst");
        let n0 = dst_clip.note(new_ids[0]).expect("n0");
        let n1 = dst_clip.note(new_ids[1]).expect("n1");
        assert_eq!(n0.pitch, 60);
        assert_eq!(n0.start_beats, 0.5);
        assert_eq!(n1.pitch, 64);
        assert_eq!(n1.start_beats, 1.5);
        assert_eq!(n1.velocity, 77);
        assert_eq!(project.clip(src).map(|c| c.notes.len()), Some(2));
    }

    #[test]
    fn paste_clips_at_playhead_keeps_notes() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id;
        project.add_note_to_clip(src, 60, 0.0, 1.0).expect("note");

        let entries = project.clips_for_clipboard(&[src]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].start_beats, 0.0);

        let new_ids = project.paste_clips(&entries, 8.0);
        assert_eq!(new_ids.len(), 1);
        let pasted = project.clip(new_ids[0]).expect("pasted");
        assert_eq!(pasted.start_beats, 8.0);
        assert_eq!(pasted.notes.len(), 1);
        assert_eq!(pasted.notes[0].pitch, 60);
        assert_eq!(project.track(track_id).map(|t| t.clips.len()), Some(2));
    }

    #[test]
    fn clipboard_note_round_trip_relative_starts() {
        let entry = ClipboardNote {
            pitch: 72,
            start_beats: 1.25,
            duration_beats: 0.25,
            velocity: 90,
        };
        let mut project = Project::default();
        let clip_id = project.tracks[0].clips[0].id;
        let ids = project.paste_notes_into_clip(clip_id, &[entry], 2.0);
        let note = project.clip(clip_id).unwrap().note(ids[0]).unwrap();
        assert_eq!(note.start_beats, 3.25);
        assert_eq!(note.duration_beats, 0.25);
        assert_eq!(note.velocity, 90);
    }
}
