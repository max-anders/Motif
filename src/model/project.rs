use serde::{Deserialize, Serialize};

use super::audio_clip::AudioClip;
use super::automation::{AutomationLane, AutomationTarget};
use super::clip::{Clip, MidiClip};
use super::clipboard::{ClipboardClip, ClipboardNote};
use super::instrument::{PluginFormat, TrackInstrument};
use super::mixer::Device;
use super::track::{migrate_notes_to_clip, Track};
use super::Note;

pub const DEFAULT_BPM: f32 = 120.0;
pub const DEFAULT_BEATS_PER_BAR: f32 = 4.0;
pub const SNAP_BEATS: f32 = 0.25;
/// Minimum visible arrangement length (in beats) even for an empty project.
pub const DEFAULT_ARRANGEMENT_MIN_BEATS: f32 = 16.0;
/// Empty bars kept past the last clip so the grid always extends beyond content.
pub const ARRANGEMENT_HEADROOM_BARS: f32 = 4.0;
/// Smallest loop region the UI allows (one beat).
pub const MIN_LOOP_SPAN_BEATS: f32 = 1.0;
/// Full MIDI note range (C-1 .. G9), like a normal DAW piano roll.
pub const MIN_PITCH: u8 = 0;
pub const MAX_PITCH: u8 = 127;
pub const DEFAULT_NOTE_DURATION_BEATS: f32 = 1.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub bpm: f32,
    pub beats_per_bar: f32,
    /// Loop region end (beats). Only affects playback when `loop_enabled`.
    pub loop_end_beats: f32,
    /// Loop region start (beats). Missing on pre-loop-region saves; starts at 0.
    #[serde(default)]
    pub loop_start_beats: f32,
    /// When true, playback cycles inside `[loop_start_beats, loop_end_beats]`.
    /// When false, playback runs straight through and stops at the end of content.
    #[serde(default)]
    pub loop_enabled: bool,
    /// Master bus fader in dB (0 = unity).
    #[serde(default)]
    pub master_gain_db: f32,
    pub tracks: Vec<Track>,
    next_note_id: u64,
    next_clip_id: u64,
    next_track_id: u64,
    /// Missing on projects saved before insert FX (Phase 2); starts at 1.
    #[serde(default = "default_next_device_id")]
    next_device_id: u64,
    /// Missing on projects saved before automation lanes; starts at 1.
    #[serde(default = "default_next_automation_lane_id")]
    next_automation_lane_id: u64,
}

fn default_next_device_id() -> u64 {
    1
}

fn default_next_automation_lane_id() -> u64 {
    1
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
            loop_start_beats: 0.0,
            loop_enabled: false,
            master_gain_db: 0.0,
            tracks: Vec::new(),
            next_note_id: 1,
            next_clip_id: 2,
            next_track_id: 2,
            next_device_id: 1,
            next_automation_lane_id: 1,
        };
        let track_id = project.add_track("Track 1", TrackInstrument::BuiltInPiano);
        project.add_clip_to_track(track_id, 0.0, 4.0);
        project
    }
}

impl Project {
    fn migrate_clip_kinds(value: &mut serde_json::Value) {
        let Some(tracks) = value
            .get_mut("tracks")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return;
        };
        for track in tracks {
            let Some(clips) = track
                .get_mut("clips")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for clip in clips {
                let is_tagged = clip.get("kind").is_some();
                let is_midi = clip.get("notes").is_some();
                let is_audio = clip.get("source").is_some();
                if is_tagged {
                    continue;
                }
                if is_audio {
                    if let Some(obj) = clip.as_object_mut() {
                        obj.insert(
                            "kind".to_string(),
                            serde_json::Value::String("audio".to_string()),
                        );
                    }
                } else if is_midi {
                    if let Some(obj) = clip.as_object_mut() {
                        obj.insert(
                            "kind".to_string(),
                            serde_json::Value::String("midi".to_string()),
                        );
                    }
                }
            }
        }
    }

    pub fn snap_beats(value: f32) -> f32 {
        (value / SNAP_BEATS).round() * SNAP_BEATS
    }

    /// Half-open beat ranges `[a_start, a_end)` and `[b_start, b_end)`.
    /// Touching endpoints (adjacent clips) do not count as overlap.
    pub fn beat_ranges_overlap(a_start: f32, a_end: f32, b_start: f32, b_end: f32) -> bool {
        a_start < b_end && b_start < a_end
    }

    /// Whether `[start_beats, start_beats + length_beats)` is free on the track.
    pub fn clip_range_free(
        &self,
        track_id: u64,
        start_beats: f32,
        length_beats: f32,
        ignore_ids: &[u64],
    ) -> bool {
        let Some(track) = self.track(track_id) else {
            return false;
        };
        let start = Self::snap_beats(start_beats.max(0.0));
        let length = Self::snap_beats(length_beats.max(SNAP_BEATS));
        !track.range_overlaps_any(start, start + length, ignore_ids)
    }

    /// Clamp a multi-clip move delta so no mover overlaps a non-moving clip on its track.
    /// `ignore_ids` are additional clips movers may overlap (Shift+drag sources).
    pub fn clamp_clip_move_delta(
        &self,
        originals: &[(u64, f32, f32)],
        mut delta: f32,
        ignore_ids: &[u64],
    ) -> f32 {
        if originals.is_empty() {
            return delta;
        }
        let moving: std::collections::HashSet<u64> =
            originals.iter().map(|(id, _, _)| *id).collect();
        let min_start = originals
            .iter()
            .map(|(_, start, _)| *start)
            .fold(f32::INFINITY, f32::min);
        if min_start + delta < 0.0 {
            delta = -min_start;
        }

        for &(clip_id, start, length) in originals {
            let Some(track_id) = self.track_id_for_clip(clip_id) else {
                continue;
            };
            let Some(track) = self.track(track_id) else {
                continue;
            };
            let m_end = start + length;
            for other in &track.clips {
                if moving.contains(&other.id()) || ignore_ids.contains(&other.id()) {
                    continue;
                }
                let o_start = other.start_beats();
                let o_end = other.end_beats();
                if delta >= 0.0 {
                    if m_end <= o_start {
                        delta = delta.min(o_start - m_end);
                    }
                } else if start >= o_end {
                    delta = delta.max(o_end - start);
                }
            }
        }
        delta
    }

    /// Left edge a resize-start drag may not cross (neighbor end, or 0).
    pub fn clip_resize_start_bound(&self, clip_id: u64, original_start: f32) -> f32 {
        let Some(track_id) = self.track_id_for_clip(clip_id) else {
            return 0.0;
        };
        let Some(track) = self.track(track_id) else {
            return 0.0;
        };
        track
            .clips
            .iter()
            .filter(|clip| clip.id() != clip_id && clip.end_beats() <= original_start)
            .map(Clip::end_beats)
            .fold(0.0_f32, f32::max)
    }

    /// Right edge a resize-end drag may not cross (`f32::INFINITY` if none).
    pub fn clip_resize_end_bound(&self, clip_id: u64, original_end: f32) -> f32 {
        let Some(track_id) = self.track_id_for_clip(clip_id) else {
            return f32::INFINITY;
        };
        let Some(track) = self.track(track_id) else {
            return f32::INFINITY;
        };
        track
            .clips
            .iter()
            .filter(|clip| clip.id() != clip_id && clip.start_beats() >= original_end)
            .map(Clip::start_beats)
            .fold(f32::INFINITY, f32::min)
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

    pub fn next_device_id(&self) -> u64 {
        self.next_device_id
    }

    pub fn bump_device_id(&mut self) {
        self.next_device_id += 1;
    }

    pub fn next_automation_lane_id(&self) -> u64 {
        self.next_automation_lane_id
    }

    pub fn bump_automation_lane_id(&mut self) {
        self.next_automation_lane_id += 1;
    }

    /// Append an automation lane to a track. Returns the new lane id.
    pub fn add_automation_lane(
        &mut self,
        track_id: u64,
        target: AutomationTarget,
        param_name: impl Into<String>,
        param_min: f64,
        param_max: f64,
    ) -> Option<u64> {
        let id = self.next_automation_lane_id();
        self.bump_automation_lane_id();
        let lane = AutomationLane {
            id,
            target,
            param_name: param_name.into(),
            param_min,
            param_max,
            points: Vec::new(),
            enabled: true,
        };
        self.track_mut(track_id)?.automation_lanes.push(lane);
        Some(id)
    }

    /// Remove an automation lane from a track. Returns `false` if the track or
    /// lane id is unknown (project unchanged).
    pub fn remove_automation_lane(&mut self, track_id: u64, lane_id: u64) -> bool {
        let Some(track) = self.track_mut(track_id) else {
            return false;
        };
        let before = track.automation_lanes.len();
        track.automation_lanes.retain(|lane| lane.id != lane_id);
        track.automation_lanes.len() != before
    }

    pub fn automation_lane(&self, track_id: u64, lane_id: u64) -> Option<&AutomationLane> {
        self.track(track_id)?
            .automation_lanes
            .iter()
            .find(|lane| lane.id == lane_id)
    }

    pub fn automation_lane_mut(
        &mut self,
        track_id: u64,
        lane_id: u64,
    ) -> Option<&mut AutomationLane> {
        self.track_mut(track_id)?
            .automation_lanes
            .iter_mut()
            .find(|lane| lane.id == lane_id)
    }

    /// Append a plugin effect to a track's insert chain. Returns the new device id.
    pub fn add_device(
        &mut self,
        track_id: u64,
        format: PluginFormat,
        unique_id: &str,
        name: &str,
    ) -> Option<u64> {
        let id = self.next_device_id();
        self.bump_device_id();
        let device = Device::new_plugin(id, format, unique_id, name);
        self.track_mut(track_id)?.devices.push(device);
        Some(id)
    }

    /// Remove a device from a track's insert chain. Returns `false` if the
    /// track or device id is unknown (project unchanged).
    pub fn remove_device(&mut self, track_id: u64, device_id: u64) -> bool {
        let Some(track) = self.track_mut(track_id) else {
            return false;
        };
        let before = track.devices.len();
        track.devices.retain(|device| device.id != device_id);
        track.devices.len() != before
    }

    /// Reorder a device within its track's chain. Returns `false` if the
    /// track is unknown or either index is out of range (project unchanged).
    pub fn move_device(&mut self, track_id: u64, from_index: usize, to_index: usize) -> bool {
        let Some(track) = self.track_mut(track_id) else {
            return false;
        };
        if from_index == to_index
            || from_index >= track.devices.len()
            || to_index >= track.devices.len()
        {
            return false;
        }
        let device = track.devices.remove(from_index);
        track.devices.insert(to_index, device);
        true
    }

    /// Set a device's bypass flag. Returns `false` if the track or device id
    /// is unknown (project unchanged).
    pub fn set_device_bypass(&mut self, track_id: u64, device_id: u64, bypassed: bool) -> bool {
        let Some(track) = self.track_mut(track_id) else {
            return false;
        };
        let Some(device) = track.devices.iter_mut().find(|d| d.id == device_id) else {
            return false;
        };
        device.bypassed = bypassed;
        true
    }

    pub fn add_track(&mut self, name: &str, instrument: TrackInstrument) -> u64 {
        let id = self.next_track_id();
        self.bump_track_id();
        self.tracks.push(Track {
            id,
            name: name.to_string(),
            muted: false,
            solo: false,
            gain_db: 0.0,
            pan: 0.0,
            sends: Vec::new(),
            devices: Vec::new(),
            macros: Vec::new(),
            automation_lanes: Vec::new(),
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
        let start = Self::snap_beats(start_beats.max(0.0));
        let length = Self::snap_beats(length_beats.max(SNAP_BEATS));
        if !self.clip_range_free(track_id, start, length, &[]) {
            return None;
        }
        let clip_number = self.track(track_id)?.clips.len() + 1;
        let clip_id = self.next_clip_id();
        self.bump_clip_id();
        let clip = MidiClip {
            id: clip_id,
            name: format!("Clip {clip_number}"),
            start_beats: start,
            length_beats: length,
            notes: Vec::new(),
        };
        self.track_mut(track_id)?.clips.push(Clip::Midi(clip));
        Some(clip_id)
    }

    pub fn add_audio_clip_to_track(
        &mut self,
        track_id: u64,
        source: std::path::PathBuf,
        name: String,
        start_beats: f32,
        length_beats: f32,
    ) -> Option<u64> {
        let start = Self::snap_beats(start_beats.max(0.0));
        let length = Self::snap_beats(length_beats.max(SNAP_BEATS));
        if !self.clip_range_free(track_id, start, length, &[]) {
            return None;
        }
        let clip_id = self.next_clip_id();
        self.bump_clip_id();
        let clip = AudioClip {
            id: clip_id,
            name,
            start_beats: start,
            length_beats: length,
            source,
            gain_db: 0.0,
            missing: false,
        };
        self.track_mut(track_id)?.clips.push(Clip::Audio(clip));
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
            .midi_clip_mut(clip_id)?
            .add_note_with_id(id, pitch, start_beats, duration_beats);
        Some(note)
    }

    /// At least one track must remain; deleting the last track is a no-op.
    pub fn can_remove_track(&self) -> bool {
        self.tracks.len() > 1
    }

    /// Removes a track and its clips. Returns `false` if this is the last track
    /// or the id is unknown (project unchanged).
    pub fn remove_track(&mut self, track_id: u64) -> bool {
        if !self.can_remove_track() {
            return false;
        }
        let before = self.tracks.len();
        self.tracks.retain(|track| track.id != track_id);
        self.tracks.len() != before
    }

    pub fn any_track_soloed(&self) -> bool {
        self.tracks.iter().any(|track| track.solo)
    }

    /// Arrangement playback: solo wins when any track is soloed; otherwise respect mute.
    pub fn track_audible(&self, track: &Track) -> bool {
        if self.any_track_soloed() {
            track.solo
        } else {
            !track.muted
        }
    }

    pub fn track_mut(&mut self, track_id: u64) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == track_id)
    }

    pub fn track(&self, track_id: u64) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    pub fn clip_mut(&mut self, clip_id: u64) -> Option<&mut Clip> {
        for track in &mut self.tracks {
            if let Some(clip) = track.clip_mut(clip_id) {
                return Some(clip);
            }
        }
        None
    }

    pub fn clip(&self, clip_id: u64) -> Option<&Clip> {
        for track in &self.tracks {
            if let Some(clip) = track.clip(clip_id) {
                return Some(clip);
            }
        }
        None
    }

    pub fn midi_clip_mut(&mut self, clip_id: u64) -> Option<&mut MidiClip> {
        self.clip_mut(clip_id).and_then(Clip::as_midi_mut)
    }

    pub fn midi_clip(&self, clip_id: u64) -> Option<&MidiClip> {
        self.clip(clip_id).and_then(Clip::as_midi)
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
        let Some(clip) = self.midi_clip(clip_id) else {
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
            if let Some(clip) = self.midi_clip_mut(clip_id) {
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
        let Some(clip) = self.midi_clip(clip_id) else {
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
        if notes.is_empty() || self.midi_clip(clip_id).is_none() {
            return Vec::new();
        }
        let origin = Self::snap_beats(origin_beats.max(0.0));
        let mut new_ids = Vec::with_capacity(notes.len());
        for template in notes {
            let id = self.next_note_id();
            self.bump_note_id();
            let start = (origin + template.start_beats).max(0.0);
            if let Some(clip) = self.midi_clip_mut(clip_id) {
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
            let Some(clip) = self.midi_clip(*clip_id) else {
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
    /// otherwise skips. Skips placements that would overlap an existing or earlier-pasted clip.
    /// Returns new clip ids.
    pub fn paste_clips(&mut self, clips: &[ClipboardClip], origin_beats: f32) -> Vec<u64> {
        if clips.is_empty() {
            return Vec::new();
        }
        let origin = Self::snap_beats(origin_beats.max(0.0));
        let mut new_ids = Vec::with_capacity(clips.len());
        let mut placed: Vec<(u64, f32, f32)> = Vec::new();
        for template in clips {
            if self.track(template.track_id).is_none() {
                continue;
            }
            let start = Self::snap_beats((origin + template.start_beats).max(0.0));
            let length = Self::snap_beats(template.length_beats.max(SNAP_BEATS));
            let end = start + length;
            if !self.clip_range_free(template.track_id, start, length, &[]) {
                continue;
            }
            if placed.iter().any(|(track_id, p_start, p_end)| {
                *track_id == template.track_id && Self::beat_ranges_overlap(start, end, *p_start, *p_end)
            }) {
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
            let clip = Clip::Midi(MidiClip {
                id: clip_id,
                name: format!("{} copy", template.name),
                start_beats: start,
                length_beats: length,
                notes,
            });
            if let Some(track) = self.track_mut(template.track_id) {
                track.clips.push(clip);
                placed.push((template.track_id, start, end));
                new_ids.push(clip_id);
            }
        }
        new_ids
    }

    /// Deep-copy clips (new clip + note ids) onto the same tracks, offset in time.
    /// Returns `(source_id, new_id)` pairs for successful placements only.
    ///
    /// When `allow_overlap_sources` is true (Shift+drag), copies may start on top of the
    /// clips being duplicated; they still cannot overlap any other clip.
    pub fn duplicate_clips(
        &mut self,
        clip_ids: &[u64],
        delta_beats: f32,
        allow_overlap_sources: bool,
    ) -> Vec<(u64, u64)> {
        #[derive(Clone)]
        struct ClipTemplate {
            source_id: u64,
            track_id: u64,
            name: String,
            start_beats: f32,
            length_beats: f32,
            clip: Clip,
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
                source_id: clip_id,
                track_id,
                name: clip.name().to_string(),
                start_beats: clip.start_beats(),
                length_beats: clip.length_beats(),
                clip: clip.clone(),
            });
        }

        let source_ignore: Vec<u64> = if allow_overlap_sources {
            templates.iter().map(|t| t.source_id).collect()
        } else {
            Vec::new()
        };

        let mut created = Vec::with_capacity(templates.len());
        let mut placed: Vec<(u64, f32, f32)> = Vec::new();
        for template in templates {
            let start = Self::snap_beats((template.start_beats + delta_beats).max(0.0));
            let length = Self::snap_beats(template.length_beats.max(SNAP_BEATS));
            let end = start + length;
            if !self.clip_range_free(template.track_id, start, length, &source_ignore) {
                continue;
            }
            if placed.iter().any(|(track_id, p_start, p_end)| {
                *track_id == template.track_id
                    && Self::beat_ranges_overlap(start, end, *p_start, *p_end)
            }) {
                continue;
            }
            let clip_id = self.next_clip_id();
            self.bump_clip_id();
            let mut clip = template.clip;
            clip.set_start_beats(start);
            clip.set_length_beats(length);
            match &mut clip {
                Clip::Midi(midi) => {
                    midi.id = clip_id;
                    midi.name = format!("{} copy", template.name);
                    for note in &mut midi.notes {
                        let id = self.next_note_id();
                        self.bump_note_id();
                        note.id = id;
                    }
                }
                Clip::Audio(audio) => {
                    audio.id = clip_id;
                    audio.name = format!("{} copy", template.name);
                }
            }
            if let Some(track) = self.track_mut(template.track_id) {
                track.clips.push(clip);
                placed.push((template.track_id, start, end));
                created.push((template.source_id, clip_id));
            }
        }
        created
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

    /// End (in beats) of the last clip across all tracks; 0 when the project is empty.
    pub fn content_end_beats(&self) -> f32 {
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter())
            .map(Clip::end_beats)
            .fold(0.0_f32, f32::max)
    }

    /// Grid extent for the arrangement view. Grows with content (rounded up to a
    /// bar plus a few empty bars of headroom) and is independent of the loop
    /// region, except that it always stays wide enough to show an active loop.
    pub fn arrangement_length_beats(&self) -> f32 {
        let bar = self.beats_per_bar.max(1.0);
        let content_bars_end = (self.content_end_beats() / bar).ceil() * bar;
        let mut length =
            (content_bars_end + bar * ARRANGEMENT_HEADROOM_BARS).max(DEFAULT_ARRANGEMENT_MIN_BEATS);
        if self.loop_enabled {
            length = length.max(self.loop_end_beats + bar);
        }
        length
    }

    /// The active loop span `(start, end)` when looping is enabled and valid.
    pub fn loop_span(&self) -> Option<(f32, f32)> {
        if self.loop_enabled && self.loop_end_beats > self.loop_start_beats {
            Some((self.loop_start_beats, self.loop_end_beats))
        } else {
            None
        }
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        // Versioned envelope (current on-disk format for .motif files).
        if let Ok(mut envelope_json) = serde_json::from_str::<serde_json::Value>(json) {
            if let Some(project_json) = envelope_json.get_mut("project") {
                Self::migrate_clip_kinds(project_json);
                if let Ok(project) = serde_json::from_value::<Self>(project_json.clone()) {
                    return Ok(project);
                }
            }
        }

        // Bare Project (pre-envelope project.json / early saves).
        if let Ok(mut project_json) = serde_json::from_str::<serde_json::Value>(json) {
            Self::migrate_clip_kinds(&mut project_json);
            if let Ok(project) = serde_json::from_value::<Self>(project_json) {
                return Ok(project);
            }
        }

        // Flat-notes legacy before tracks/clips.
        let legacy: LegacyProject = serde_json::from_str(json)?;
        let clip = migrate_notes_to_clip(legacy.notes, legacy.loop_end_beats);
        let track = Track {
            id: 1,
            name: String::from("Track 1"),
            muted: false,
            solo: false,
            gain_db: 0.0,
            pan: 0.0,
            sends: Vec::new(),
            devices: Vec::new(),
            macros: Vec::new(),
            automation_lanes: Vec::new(),
            instrument: TrackInstrument::BuiltInPiano,
            plugin_state: None,
            clips: vec![Clip::Midi(clip)],
        };

        Ok(Self {
            bpm: legacy.bpm,
            beats_per_bar: legacy.beats_per_bar,
            loop_end_beats: legacy.loop_end_beats,
            loop_start_beats: 0.0,
            loop_enabled: false,
            master_gain_db: 0.0,
            tracks: vec![track],
            next_note_id: legacy.next_note_id,
            next_clip_id: 2,
            next_track_id: 2,
            next_device_id: 1,
            next_automation_lane_id: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClipboardNote, EditClipboard, EditHistory};
    use std::path::PathBuf;

    #[test]
    fn paste_notes_into_another_clip_at_origin() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id();
        let dst = project
            .add_clip_to_track(track_id, 4.0, 4.0)
            .expect("dst clip");

        let a = project
            .add_note_to_clip(src, 60, 1.0, 1.0)
            .expect("note a");
        let b = project
            .add_note_to_clip(src, 64, 2.0, 0.5)
            .expect("note b");
        if let Some(note) = project.midi_clip_mut(src).and_then(|c| c.note_mut(b.id)) {
            note.velocity = 77;
        }

        let clipboard = EditClipboard::from_notes(&project.notes_for_clipboard(src, &[a.id, b.id]));
        let EditClipboard::Notes(entries) = clipboard else {
            panic!("notes");
        };
        let new_ids = project.paste_notes_into_clip(dst, &entries, 0.5);
        assert_eq!(new_ids.len(), 2);

        let dst_clip = project.midi_clip(dst).expect("dst");
        let n0 = dst_clip.note(new_ids[0]).expect("n0");
        let n1 = dst_clip.note(new_ids[1]).expect("n1");
        assert_eq!(n0.pitch, 60);
        assert_eq!(n0.start_beats, 0.5);
        assert_eq!(n1.pitch, 64);
        assert_eq!(n1.start_beats, 1.5);
        assert_eq!(n1.velocity, 77);
        assert_eq!(project.midi_clip(src).map(|c| c.notes.len()), Some(2));
    }

    #[test]
    fn paste_clips_at_playhead_keeps_notes() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id();
        project.add_note_to_clip(src, 60, 0.0, 1.0).expect("note");

        let entries = project.clips_for_clipboard(&[src]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].start_beats, 0.0);

        let new_ids = project.paste_clips(&entries, 8.0);
        assert_eq!(new_ids.len(), 1);
        let pasted = project.midi_clip(new_ids[0]).expect("pasted");
        assert_eq!(pasted.start_beats, 8.0);
        assert_eq!(pasted.notes.len(), 1);
        assert_eq!(pasted.notes[0].pitch, 60);
        assert_eq!(project.track(track_id).map(|t| t.clips.len()), Some(2));
    }

    #[test]
    fn arrangement_length_grows_with_content_not_loop() {
        let mut project = Project::default();
        // Default project has one 4-beat clip at 0: content rounded to a bar (4)
        // plus the headroom bars, never below the minimum.
        assert!(project.arrangement_length_beats() >= DEFAULT_ARRANGEMENT_MIN_BEATS);
        let empty_len = project.arrangement_length_beats();

        // A far-out clip extends the grid past the (unchanged) loop end.
        let track_id = project.tracks[0].id;
        project
            .add_clip_to_track(track_id, 40.0, 8.0)
            .expect("clip");
        let length = project.arrangement_length_beats();
        assert!(
            length > empty_len,
            "grid should grow when content is added, got {length}"
        );
        assert!(
            length >= 48.0 + project.beats_per_bar,
            "grid should extend past the last clip end, got {length}"
        );
        // Grid extent is independent of the (default 16) loop end.
        assert!(length > project.loop_end_beats);
    }

    #[test]
    fn loop_span_only_when_enabled_and_valid() {
        let mut project = Project::default();
        assert!(!project.loop_enabled);
        assert_eq!(project.loop_span(), None);

        project.loop_enabled = true;
        project.loop_start_beats = 4.0;
        project.loop_end_beats = 12.0;
        assert_eq!(project.loop_span(), Some((4.0, 12.0)));

        // Invalid (end <= start) yields no loop even when enabled.
        project.loop_end_beats = 4.0;
        assert_eq!(project.loop_span(), None);
    }

    #[test]
    fn arrangement_length_stays_wide_enough_for_active_loop() {
        let mut project = Project::default();
        project.loop_enabled = true;
        project.loop_end_beats = 200.0;
        let length = project.arrangement_length_beats();
        assert!(length >= 200.0 + project.beats_per_bar);
    }

    #[test]
    fn remove_track_refuses_last_track() {
        let mut project = Project::default();
        assert_eq!(project.tracks.len(), 1);
        assert!(!project.can_remove_track());
        let id = project.tracks[0].id;
        assert!(!project.remove_track(id));
        assert_eq!(project.tracks.len(), 1);
        assert!(project.track(id).is_some());
    }

    #[test]
    fn remove_track_drops_clips() {
        let mut project = Project::default();
        let keep = project.tracks[0].id;
        let remove = project.add_track("Track 2", TrackInstrument::BuiltInPiano);
        let clip_id = project
            .add_clip_to_track(remove, 2.0, 4.0)
            .expect("clip");
        project
            .add_note_to_clip(clip_id, 60, 0.0, 1.0)
            .expect("note");
        assert!(project.can_remove_track());
        assert!(project.remove_track(remove));
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.tracks[0].id, keep);
        assert!(project.clip(clip_id).is_none());
        assert!(project.track(remove).is_none());
    }

    #[test]
    fn track_audible_solo_overrides_mute() {
        let mut project = Project::default();
        let a = project.tracks[0].id;
        let b = project.add_track("Track 2", TrackInstrument::BuiltInPiano);
        project.track_mut(a).expect("a").muted = true;
        project.track_mut(b).expect("b").solo = true;
        assert!(!project.track_audible(project.track(a).expect("a")));
        assert!(project.track_audible(project.track(b).expect("b")));
    }

    #[test]
    fn track_audible_respects_mute_without_solo() {
        let mut project = Project::default();
        let a = project.tracks[0].id;
        project.track_mut(a).expect("a").muted = true;
        assert!(!project.any_track_soloed());
        assert!(!project.track_audible(project.track(a).expect("a")));
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
        let clip_id = project.tracks[0].clips[0].id();
        let ids = project.paste_notes_into_clip(clip_id, &[entry], 2.0);
        let note = project.midi_clip(clip_id).unwrap().note(ids[0]).unwrap();
        assert_eq!(note.start_beats, 3.25);
        assert_eq!(note.duration_beats, 0.25);
        assert_eq!(note.velocity, 90);
    }

    #[test]
    fn old_json_without_mixer_fields_loads_defaults() {
        // Pre-mixer bare Project (no master_gain_db / track gain/pan/sends/devices/macros).
        let json = r#"{
            "bpm": 120.0,
            "beats_per_bar": 4.0,
            "loop_end_beats": 16.0,
            "tracks": [{
                "id": 1,
                "name": "Track 1",
                "muted": false,
                "solo": false,
                "instrument": { "type": "built_in_piano" },
                "clips": [{
                    "id": 1,
                    "name": "Clip 1",
                    "start_beats": 0.0,
                    "length_beats": 4.0,
                    "notes": []
                }]
            }],
            "next_note_id": 1,
            "next_clip_id": 2,
            "next_track_id": 2
        }"#;
        let project = Project::from_json(json).expect("parse");
        assert_eq!(project.master_gain_db, 0.0);
        let track = &project.tracks[0];
        assert_eq!(track.gain_db, 0.0);
        assert_eq!(track.pan, 0.0);
        assert!(track.sends.is_empty());
        assert!(track.devices.is_empty());
        assert!(track.macros.is_empty());
        assert!((track.gain_linear() - 1.0).abs() < 1e-5);
        let (l, r) = track.pan_gains();
        let center = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l - center).abs() < 1e-5);
        assert!((r - center).abs() < 1e-5);
    }

    #[test]
    fn old_clip_json_without_kind_migrates_to_midi() {
        let json = r#"{
            "bpm": 120.0,
            "beats_per_bar": 4.0,
            "loop_end_beats": 16.0,
            "tracks": [{
                "id": 1,
                "name": "Track 1",
                "muted": false,
                "solo": false,
                "instrument": { "type": "built_in_piano" },
                "clips": [{
                    "id": 1,
                    "name": "Clip 1",
                    "start_beats": 0.0,
                    "length_beats": 4.0,
                    "notes": []
                }]
            }],
            "next_note_id": 1,
            "next_clip_id": 2,
            "next_track_id": 2
        }"#;
        let project = Project::from_json(json).expect("parse");
        assert!(matches!(project.tracks[0].clips[0], Clip::Midi(_)));
    }

    #[test]
    fn audio_clip_round_trip_in_project_json() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        // Default project already has a MIDI clip at [0, 4); place audio after it.
        let clip_id = project
            .add_audio_clip_to_track(
                track_id,
                PathBuf::from("/tmp/kick.wav"),
                String::from("kick"),
                4.0,
                8.0,
            )
            .expect("audio clip");
        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).expect("reload");
        let clip = loaded.clip(clip_id).expect("clip");
        let audio = clip.as_audio().expect("audio");
        assert_eq!(audio.name, "kick");
        assert_eq!(audio.source, PathBuf::from("/tmp/kick.wav"));
        assert_eq!(audio.length_beats, 8.0);
    }

    #[test]
    fn adjacent_clips_do_not_overlap_half_open() {
        assert!(!Project::beat_ranges_overlap(0.0, 4.0, 4.0, 8.0));
        assert!(Project::beat_ranges_overlap(0.0, 4.0, 3.75, 8.0));
    }

    #[test]
    fn add_clip_rejects_same_track_overlap() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        assert!(project.add_clip_to_track(track_id, 2.0, 4.0).is_none());
        assert!(project.add_clip_to_track(track_id, 4.0, 4.0).is_some());
    }

    #[test]
    fn clamp_move_stops_at_neighbor() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let left_id = project.tracks[0].clips[0].id();
        let right_id = project
            .add_clip_to_track(track_id, 8.0, 4.0)
            .expect("right");
        // Moving left clip right by 10 beats should stop adjacent to right ([4, 8)).
        let delta = project.clamp_clip_move_delta(&[(left_id, 0.0, 4.0)], 10.0, &[]);
        assert!((delta - 4.0).abs() < 1e-5);
        // Moving right clip left should stop adjacent to left.
        let delta_left = project.clamp_clip_move_delta(&[(right_id, 8.0, 4.0)], -10.0, &[]);
        assert!((delta_left - (-4.0)).abs() < 1e-5);
    }

    #[test]
    fn duplicate_without_space_is_skipped() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id();
        project
            .add_clip_to_track(track_id, 4.0, 4.0)
            .expect("blocker");
        let created = project.duplicate_clips(&[src], 4.0, false);
        assert!(created.is_empty());
    }

    #[test]
    fn duplicate_stacked_may_overlap_sources_only() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let src = project.tracks[0].clips[0].id();
        project
            .add_clip_to_track(track_id, 4.0, 4.0)
            .expect("neighbor");
        let created = project.duplicate_clips(&[src], 0.0, true);
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].0, src);
        let copy = project.clip(created[0].1).expect("copy");
        assert_eq!(copy.start_beats(), 0.0);
    }

    #[test]
    fn resize_bounds_respect_neighbors() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let left_id = project.tracks[0].clips[0].id();
        let right_id = project
            .add_clip_to_track(track_id, 8.0, 4.0)
            .expect("right");
        assert!((project.clip_resize_end_bound(left_id, 4.0) - 8.0).abs() < 1e-5);
        assert!((project.clip_resize_start_bound(right_id, 8.0) - 4.0).abs() < 1e-5);
    }

    #[test]
    fn mixer_fields_round_trip_in_envelope() {
        let mut project = Project::default();
        project.master_gain_db = -3.0;
        {
            let track = &mut project.tracks[0];
            track.gain_db = -6.0;
            track.pan = 0.5;
            track.sends.push(crate::model::Send {
                target_track: None,
                level_db: -12.0,
                enabled: true,
            });
            track.devices.push(crate::model::Device::new_plugin(
                1,
                crate::model::PluginFormat::Clap,
                "com.example.reverb",
                "Placeholder",
            ));
            track.macros.push(crate::model::Macro {
                name: String::from("A"),
                value: 0.25,
            });
        }
        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert_eq!(loaded.master_gain_db, -3.0);
        assert_eq!(loaded.tracks[0].gain_db, -6.0);
        assert_eq!(loaded.tracks[0].pan, 0.5);
        assert_eq!(loaded.tracks[0].sends.len(), 1);
        assert_eq!(loaded.tracks[0].devices[0].name, "Placeholder");
        assert_eq!(loaded.tracks[0].macros[0].value, 0.25);
    }

    #[test]
    fn device_plugin_state_round_trips_through_envelope() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let device_id = project
            .add_device(
                track_id,
                crate::model::PluginFormat::Vst3,
                "vendor.eq",
                "Channel EQ",
            )
            .expect("device added");
        {
            let track = project.track_mut(track_id).expect("track");
            let device = track
                .devices
                .iter_mut()
                .find(|d| d.id == device_id)
                .expect("device");
            device.bypassed = true;
            device.plugin_state = Some(vec![1, 2, 3, 4, 5]);
        }

        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        let device = &loaded.tracks[0].devices[0];
        assert_eq!(device.id, device_id);
        assert_eq!(device.format, crate::model::PluginFormat::Vst3);
        assert_eq!(device.unique_id, "vendor.eq");
        assert_eq!(device.name, "Channel EQ");
        assert!(device.bypassed);
        assert_eq!(device.plugin_state, Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn old_device_json_without_plugin_fields_loads_defaults() {
        // Pre-Phase-2 placeholder device: only {id, name, bypassed}, no format/
        // unique_id/plugin_state, and the project predates next_device_id.
        let json = r#"{
            "bpm": 120.0,
            "beats_per_bar": 4.0,
            "loop_end_beats": 16.0,
            "tracks": [{
                "id": 1,
                "name": "Track 1",
                "instrument": { "type": "built_in_piano" },
                "devices": [{
                    "id": 7,
                    "name": "Placeholder",
                    "bypassed": true
                }],
                "clips": []
            }],
            "next_note_id": 1,
            "next_clip_id": 2,
            "next_track_id": 2
        }"#;
        let project = Project::from_json(json).expect("parse");
        let device = &project.tracks[0].devices[0];
        assert_eq!(device.id, 7);
        assert_eq!(device.name, "Placeholder");
        assert!(device.bypassed);
        assert_eq!(device.format, crate::model::PluginFormat::Clap);
        assert_eq!(device.unique_id, "");
        assert_eq!(device.plugin_state, None);
        // Missing next_device_id defaults to 1, independent of legacy device ids.
        assert_eq!(project.next_device_id(), 1);
    }

    #[test]
    fn next_device_id_increments_and_survives_round_trip() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        assert_eq!(project.next_device_id(), 1);
        let first = project
            .add_device(track_id, crate::model::PluginFormat::Clap, "a", "A")
            .expect("first device");
        let second = project
            .add_device(track_id, crate::model::PluginFormat::Clap, "b", "B")
            .expect("second device");
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(project.next_device_id(), 3);

        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert_eq!(loaded.next_device_id(), 3);
    }

    #[test]
    fn device_chain_add_remove_move_bypass() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let a = project
            .add_device(track_id, crate::model::PluginFormat::Clap, "a", "A")
            .expect("a");
        let b = project
            .add_device(track_id, crate::model::PluginFormat::Clap, "b", "B")
            .expect("b");
        let c = project
            .add_device(track_id, crate::model::PluginFormat::Clap, "c", "C")
            .expect("c");
        assert_eq!(
            project.track(track_id).unwrap().devices.len(),
            3
        );

        assert!(project.set_device_bypass(track_id, b, true));
        assert!(project.track(track_id).unwrap().devices[1].bypassed);

        // Move C (index 2) to the front.
        assert!(project.move_device(track_id, 2, 0));
        let ids: Vec<u64> = project
            .track(track_id)
            .unwrap()
            .devices
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec![c, a, b]);

        assert!(project.remove_device(track_id, a));
        let ids: Vec<u64> = project
            .track(track_id)
            .unwrap()
            .devices
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec![c, b]);

        assert!(!project.remove_device(track_id, a));
        assert!(!project.move_device(track_id, 5, 0));
        assert!(!project.set_device_bypass(999, b, false));
    }

    #[test]
    fn old_json_without_automation_fields_loads_defaults() {
        let json = r#"{
            "bpm": 120.0,
            "beats_per_bar": 4.0,
            "loop_end_beats": 16.0,
            "tracks": [{
                "id": 1,
                "name": "Track 1",
                "instrument": { "type": "built_in_piano" },
                "clips": []
            }],
            "next_note_id": 1,
            "next_clip_id": 2,
            "next_track_id": 2
        }"#;
        let project = Project::from_json(json).expect("parse");
        assert!(project.tracks[0].automation_lanes.is_empty());
        assert_eq!(project.next_automation_lane_id(), 1);
    }

    #[test]
    fn automation_lane_round_trip_in_envelope() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let lane_id = project
            .add_automation_lane(
                track_id,
                AutomationTarget::Instrument { param_id: 42 },
                "Cutoff",
                20.0,
                20_000.0,
            )
            .expect("lane added");
        {
            let lane = project
                .automation_lane_mut(track_id, lane_id)
                .expect("lane");
            lane.points.push(crate::model::AutomationPoint {
                beat: 0.0,
                value: 0.25,
                curve: crate::model::CurveKind::Hold,
            });
            lane.points.push(crate::model::AutomationPoint {
                beat: 4.0,
                value: 0.75,
                curve: crate::model::CurveKind::Linear,
            });
        }

        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert_eq!(loaded.next_automation_lane_id(), 2);
        let lane = loaded
            .automation_lane(track_id, lane_id)
            .expect("lane reloaded");
        assert_eq!(lane.param_name, "Cutoff");
        assert_eq!(lane.param_min, 20.0);
        assert_eq!(lane.param_max, 20_000.0);
        assert!(lane.enabled);
        assert_eq!(lane.points.len(), 2);
        assert_eq!(lane.points[0].curve, crate::model::CurveKind::Hold);
        assert_eq!(lane.points[1].value, 0.75);
    }

    #[test]
    fn next_automation_lane_id_increments_and_survives_round_trip() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        assert_eq!(project.next_automation_lane_id(), 1);
        let first = project
            .add_automation_lane(
                track_id,
                AutomationTarget::Instrument { param_id: 1 },
                "A",
                0.0,
                1.0,
            )
            .expect("first lane");
        let second = project
            .add_automation_lane(
                track_id,
                AutomationTarget::Device {
                    device_id: 7,
                    param_id: 2,
                },
                "B",
                0.0,
                1.0,
            )
            .expect("second lane");
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(project.next_automation_lane_id(), 3);

        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert_eq!(loaded.next_automation_lane_id(), 3);
    }

    #[test]
    fn automation_lane_add_remove_mut() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let lane_id = project
            .add_automation_lane(
                track_id,
                AutomationTarget::Instrument { param_id: 5 },
                "Gain",
                0.0,
                1.0,
            )
            .expect("lane");
        assert_eq!(project.track(track_id).unwrap().automation_lanes.len(), 1);

        project
            .automation_lane_mut(track_id, lane_id)
            .expect("lane")
            .enabled = false;
        assert!(!project.automation_lane(track_id, lane_id).unwrap().enabled);

        assert!(project.remove_automation_lane(track_id, lane_id));
        assert!(project.track(track_id).unwrap().automation_lanes.is_empty());
        assert!(!project.remove_automation_lane(track_id, lane_id));
        assert!(project.automation_lane_mut(track_id, lane_id).is_none());
    }

    #[test]
    fn undo_snapshot_includes_automation_lanes_via_clone() {
        let mut project = Project::default();
        let track_id = project.tracks[0].id;
        let lane_id = project
            .add_automation_lane(
                track_id,
                AutomationTarget::Instrument { param_id: 9 },
                "Resonance",
                0.0,
                1.0,
            )
            .expect("lane");
        let before = project.clone();
        project
            .automation_lane_mut(track_id, lane_id)
            .expect("lane")
            .points
            .push(crate::model::AutomationPoint {
                beat: 2.0,
                value: 0.5,
                curve: crate::model::CurveKind::Linear,
            });

        let mut history = EditHistory::new(8);
        history.push_before(before);
        assert!(history.undo(&mut project));
        let lane = &project.tracks[0].automation_lanes[0];
        assert_eq!(lane.id, lane_id);
        assert!(lane.points.is_empty());
    }
}
