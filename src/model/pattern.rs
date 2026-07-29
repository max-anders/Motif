use serde::{Deserialize, Serialize};

use super::project::Project;
use super::Note;

/// A MIDI note at absolute song beats after playlist + pattern resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedMidiNote {
    pub start_beats: f32,
    pub end_beats: f32,
    pub pitch: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternLane {
    pub id: u64,
    pub name: String,
    pub blocks: Vec<PatternBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternBlock {
    pub id: u64,
    pub name: String,
    pub start_beats: f32,
    pub length_beats: f32,
    #[serde(default)]
    pub solo: bool,
    pub tracks: Vec<PatternTrackContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternTrackContent {
    pub track_id: u64,
    pub notes: Vec<Note>,
    /// Explicit row editor mode. `None` = derive from `heuristic_row_mode` (a
    /// user can flip a row between step/melody in the row editor; missing on
    /// projects saved before Phase D2, so it defaults to the heuristic).
    #[serde(default)]
    pub row_mode: Option<PatternRowMode>,
}

/// Row editor surface: FL-style step buttons for drum drafting, or a slim
/// piano roll for melodic content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternRowMode {
    Step,
    Melody,
}

impl PatternTrackContent {
    /// Default row surface when the user has not explicitly picked one: rows
    /// with few distinct pitches (typical of a single drum hit) default to the
    /// step grid; anything more melodic defaults to the piano roll.
    pub fn heuristic_row_mode(notes: &[Note]) -> PatternRowMode {
        if notes.is_empty() {
            return PatternRowMode::Step;
        }
        let pitches: std::collections::HashSet<u8> = notes.iter().map(|note| note.pitch).collect();
        if pitches.len() <= 2 {
            PatternRowMode::Step
        } else {
            PatternRowMode::Melody
        }
    }

    /// The mode the row editor should actually show: explicit override first,
    /// else the heuristic derived from current notes.
    pub fn effective_row_mode(&self) -> PatternRowMode {
        self.row_mode
            .unwrap_or_else(|| Self::heuristic_row_mode(&self.notes))
    }
}

impl PatternBlock {
    pub fn end_beats(&self) -> f32 {
        self.start_beats + self.length_beats
    }

    pub fn track_content(&self, track_id: u64) -> Option<&PatternTrackContent> {
        self.tracks.iter().find(|row| row.track_id == track_id)
    }

    pub fn track_content_mut(&mut self, track_id: u64) -> Option<&mut PatternTrackContent> {
        self.tracks.iter_mut().find(|row| row.track_id == track_id)
    }
}

/// First solo block in lane order (top lane first, then block order within lane).
pub fn solo_pattern_block<'a>(lanes: &'a [PatternLane]) -> Option<&'a PatternBlock> {
    for lane in lanes {
        for block in &lane.blocks {
            if block.solo {
                return Some(block);
            }
        }
    }
    None
}

/// Flatten playlist MIDI clips for one track to absolute song beats.
pub fn playlist_midi_for_track(project: &Project, track_id: u64) -> Vec<ResolvedMidiNote> {
    let Some(track) = project.track(track_id) else {
        return Vec::new();
    };

    let mut notes = Vec::new();
    for clip in &track.clips {
        let Some(clip) = clip.as_midi() else {
            continue;
        };
        let clip_end = clip.end_beats();
        for note in &clip.notes {
            let start_beats = clip.start_beats + note.start_beats;
            let end_beats = (clip.start_beats + note.end_beats()).min(clip_end);
            if end_beats <= start_beats {
                continue;
            }
            notes.push(ResolvedMidiNote {
                start_beats,
                end_beats,
                pitch: note.pitch,
                velocity: note.velocity,
            });
        }
    }
    notes.sort_unstable_by(|a, b| a.start_beats.total_cmp(&b.start_beats));
    notes
}

/// Pattern notes for one track inside a block, shifted to absolute beats and clamped to the block.
fn pattern_notes_for_track_in_block(block: &PatternBlock, track_id: u64) -> Vec<ResolvedMidiNote> {
    let Some(content) = block.track_content(track_id) else {
        return Vec::new();
    };
    if content.notes.is_empty() {
        return Vec::new();
    }

    let block_start = block.start_beats;
    let block_end = block.end_beats();
    let mut notes = Vec::with_capacity(content.notes.len());
    for note in &content.notes {
        let start_beats = block_start + note.start_beats;
        let end_beats = (block_start + note.end_beats()).min(block_end);
        if end_beats <= start_beats {
            continue;
        }
        notes.push(ResolvedMidiNote {
            start_beats,
            end_beats,
            pitch: note.pitch,
            velocity: note.velocity,
        });
    }
    notes.sort_unstable_by(|a, b| a.start_beats.total_cmp(&b.start_beats));
    notes
}

/// Beat ranges where playlist MIDI is visually overridden for one track (ghost dimming).
pub fn override_windows_for_track(project: &Project, track_id: u64) -> Vec<(f32, f32)> {
    if let Some(solo_block) = solo_pattern_block(&project.pattern_lanes) {
        if solo_block
            .track_content(track_id)
            .is_some_and(|content| !content.notes.is_empty())
        {
            return vec![(solo_block.start_beats, solo_block.end_beats())];
        }
        return Vec::new();
    }

    let mut windows = Vec::new();
    let mut claimed: Vec<(f32, f32)> = Vec::new();

    for lane in &project.pattern_lanes {
        for block in &lane.blocks {
            let block_start = block.start_beats;
            let block_end = block.end_beats();
            let Some(content) = block.track_content(track_id) else {
                continue;
            };
            if content.notes.is_empty() {
                continue;
            }

            let active_windows = subtract_claimed(block_start, block_end, &claimed);
            for (win_start, win_end) in active_windows {
                windows.push((win_start, win_end));
                merge_claimed(&mut claimed, win_start, win_end);
            }
        }
    }

    windows
}

/// Apply pattern-lane overrides on top of playlist MIDI for one track.
pub fn resolve_midi_for_track(project: &Project, track_id: u64) -> Vec<ResolvedMidiNote> {
    if let Some(solo_block) = solo_pattern_block(&project.pattern_lanes) {
        return pattern_notes_for_track_in_block(solo_block, track_id);
    }

    let mut notes = playlist_midi_for_track(project, track_id);
    let mut claimed: Vec<(f32, f32)> = Vec::new();

    for lane in &project.pattern_lanes {
        for block in &lane.blocks {
            let block_start = block.start_beats;
            let block_end = block.end_beats();
            let Some(content) = block.track_content(track_id) else {
                continue;
            };
            if content.notes.is_empty() {
                continue;
            }

            let active_windows = subtract_claimed(block_start, block_end, &claimed);
            if active_windows.is_empty() {
                continue;
            }

            for (win_start, win_end) in &active_windows {
                trim_notes_outside_window(&mut notes, *win_start, *win_end);

                for note in &content.notes {
                    let abs_start = block_start + note.start_beats;
                    let abs_end = (block_start + note.end_beats()).min(block_end);
                    if abs_end <= abs_start {
                        continue;
                    }
                    if !Project::beat_ranges_overlap(abs_start, abs_end, *win_start, *win_end) {
                        continue;
                    }
                    let start_beats = abs_start.max(*win_start);
                    let end_beats = abs_end.min(*win_end);
                    if end_beats <= start_beats {
                        continue;
                    }
                    notes.push(ResolvedMidiNote {
                        start_beats,
                        end_beats,
                        pitch: note.pitch,
                        velocity: note.velocity,
                    });
                }
            }

            for (win_start, win_end) in active_windows {
                merge_claimed(&mut claimed, win_start, win_end);
            }
        }
    }

    notes.sort_unstable_by(|a, b| a.start_beats.total_cmp(&b.start_beats));
    notes
}

/// Keep only the portions of `notes` outside `[win_start, win_end)` (half-open).
fn trim_notes_outside_window(notes: &mut Vec<ResolvedMidiNote>, win_start: f32, win_end: f32) {
    if win_end <= win_start {
        return;
    }
    let mut trimmed = Vec::with_capacity(notes.len() + 1);
    for note in notes.drain(..) {
        let start = note.start_beats;
        let end = note.end_beats;
        if !Project::beat_ranges_overlap(start, end, win_start, win_end) {
            trimmed.push(note);
            continue;
        }
        if start < win_start {
            let end_beats = win_start.min(end);
            if end_beats > start {
                trimmed.push(ResolvedMidiNote {
                    start_beats: start,
                    end_beats,
                    pitch: note.pitch,
                    velocity: note.velocity,
                });
            }
        }
        if end > win_end {
            let start_beats = win_end.max(start);
            if end > start_beats {
                trimmed.push(ResolvedMidiNote {
                    start_beats,
                    end_beats: end,
                    pitch: note.pitch,
                    velocity: note.velocity,
                });
            }
        }
    }
    *notes = trimmed;
}

/// Portions of `[start, end)` not covered by any interval in `claimed` (half-open).
fn subtract_claimed(start: f32, end: f32, claimed: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if end <= start {
        return Vec::new();
    }
    let mut windows = vec![(start, end)];
    for &(c_start, c_end) in claimed {
        if c_end <= c_start {
            continue;
        }
        windows = windows
            .into_iter()
            .flat_map(|(ws, we)| {
                let mut out = Vec::new();
                if ws < c_start {
                    out.push((ws, c_start.min(we)));
                }
                if we > c_end {
                    out.push((c_end.max(ws), we));
                }
                out
            })
            .filter(|(ws, we)| we > ws)
            .collect();
        if windows.is_empty() {
            break;
        }
    }
    windows
}

/// True when a higher-priority pattern lane claims this track over overlapping
/// time (this block's row is silent during the overlap — rack dimming).
pub fn pattern_row_suppressed_by_higher_lane(
    project: &Project,
    block_id: u64,
    track_id: u64,
) -> bool {
    let Some((lane_index, block)) = project
        .pattern_lanes
        .iter()
        .enumerate()
        .find_map(|(index, lane)| {
            lane.blocks
                .iter()
                .find(|b| b.id == block_id)
                .map(|b| (index, b))
        })
    else {
        return false;
    };

    let block_start = block.start_beats;
    let block_end = block.end_beats();

    for lane in project.pattern_lanes.iter().take(lane_index) {
        for other in &lane.blocks {
            if !Project::beat_ranges_overlap(
                block_start,
                block_end,
                other.start_beats,
                other.end_beats(),
            ) {
                continue;
            }
            if other
                .track_content(track_id)
                .is_some_and(|content| !content.notes.is_empty())
            {
                return true;
            }
        }
    }
    false
}

fn merge_claimed(claimed: &mut Vec<(f32, f32)>, start: f32, end: f32) {
    if end <= start {
        return;
    }
    claimed.push((start, end));
    claimed.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f32, f32)> = Vec::new();
    for (s, e) in claimed.drain(..) {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    *claimed = merged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::clip::Clip;
    use crate::model::instrument::TrackInstrument;
    use crate::model::track::Track;

    fn note(id: u64, pitch: u8, start: f32, dur: f32) -> Note {
        Note {
            id,
            pitch,
            start_beats: start,
            duration_beats: dur,
            velocity: 100,
        }
    }

    fn track_with_clip(track_id: u64, clip_start: f32, clip_len: f32, notes: Vec<Note>) -> Track {
        Track {
            id: track_id,
            name: format!("Track {track_id}"),
            muted: false,
            solo: false,
            gain_db: 0.0,
            pan: 0.0,
            sends: Vec::new(),
            devices: Vec::new(),
            macros: Vec::new(),
            automation_lanes: Vec::new(),
            modulators: Vec::new(),
            instrument: TrackInstrument::BuiltInPiano,
            plugin_state: None,
            clips: vec![Clip::Midi(crate::model::MidiClip {
                id: track_id * 100,
                name: String::from("Clip"),
                start_beats: clip_start,
                length_beats: clip_len,
                notes,
            })],
        }
    }

    fn empty_project_with_tracks(tracks: Vec<Track>) -> Project {
        let mut project = Project::default();
        project.tracks = tracks;
        project
    }

    #[test]
    fn empty_pattern_lanes_match_playlist_only() {
        let project = empty_project_with_tracks(vec![track_with_clip(
            1,
            0.0,
            8.0,
            vec![note(1, 60, 0.0, 1.0), note(2, 64, 2.0, 1.0)],
        )]);

        let resolved = resolve_midi_for_track(&project, 1);
        let playlist = playlist_midi_for_track(&project, 1);
        assert_eq!(resolved, playlist);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].pitch, 60);
        assert_eq!(resolved[1].pitch, 64);
    }

    #[test]
    fn pattern_block_replaces_playlist_inside_window() {
        let project = empty_project_with_tracks(vec![track_with_clip(
            1,
            0.0,
            16.0,
            vec![note(1, 60, 0.0, 5.0), note(2, 62, 8.0, 2.0)],
        )]);
        let mut project = project;
        project.pattern_lanes = vec![PatternLane {
            id: 1,
            name: String::from("Lane 1"),
            blocks: vec![PatternBlock {
                id: 1,
                name: String::from("Verse"),
                start_beats: 4.0,
                length_beats: 4.0,
                solo: false,
                tracks: vec![PatternTrackContent {
                    track_id: 1,
                    notes: vec![note(10, 72, 0.0, 2.0)],
                    row_mode: None,
                }],
            }],
        }];

        let resolved = resolve_midi_for_track(&project, 1);
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].pitch, 60);
        assert_eq!(resolved[0].start_beats, 0.0);
        assert_eq!(resolved[0].end_beats, 4.0);
        assert_eq!(resolved[1].pitch, 72);
        assert_eq!(resolved[1].start_beats, 4.0);
        assert_eq!(resolved[1].end_beats, 6.0);
        assert_eq!(resolved[2].pitch, 62);
        assert_eq!(resolved[2].start_beats, 8.0);
    }

    #[test]
    fn empty_pattern_row_does_not_claim() {
        let project = empty_project_with_tracks(vec![track_with_clip(
            1,
            0.0,
            8.0,
            vec![note(1, 60, 2.0, 2.0)],
        )]);
        let mut project = project;
        project.pattern_lanes = vec![PatternLane {
            id: 1,
            name: String::from("Lane 1"),
            blocks: vec![PatternBlock {
                id: 1,
                name: String::from("Empty row"),
                start_beats: 0.0,
                length_beats: 8.0,
                solo: false,
                tracks: vec![PatternTrackContent {
                    track_id: 1,
                    notes: Vec::new(),
                    row_mode: None,
                }],
            }],
        }];

        let resolved = resolve_midi_for_track(&project, 1);
        let playlist = playlist_midi_for_track(&project, 1);
        assert_eq!(resolved, playlist);
    }

    #[test]
    fn top_lane_wins_on_same_track_overlap() {
        let project = empty_project_with_tracks(vec![track_with_clip(
            1,
            0.0,
            16.0,
            Vec::new(),
        )]);
        let mut project = project;
        project.pattern_lanes = vec![
            PatternLane {
                id: 1,
                name: String::from("Top"),
                blocks: vec![PatternBlock {
                    id: 1,
                    name: String::from("Top block"),
                    start_beats: 0.0,
                    length_beats: 8.0,
                    solo: false,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(10, 72, 0.0, 8.0)],
                        row_mode: None,
                    }],
                }],
            },
            PatternLane {
                id: 2,
                name: String::from("Bottom"),
                blocks: vec![PatternBlock {
                    id: 2,
                    name: String::from("Bottom block"),
                    start_beats: 4.0,
                    length_beats: 8.0,
                    solo: false,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(20, 48, 0.0, 8.0)],
                        row_mode: None,
                    }],
                }],
            },
        ];

        let resolved = resolve_midi_for_track(&project, 1);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].pitch, 72);
        assert_eq!(resolved[0].start_beats, 0.0);
        assert_eq!(resolved[0].end_beats, 8.0);
        assert_eq!(resolved[1].pitch, 48);
        assert_eq!(resolved[1].start_beats, 8.0);
        assert_eq!(resolved[1].end_beats, 12.0);
    }

    #[test]
    fn solo_block_ignores_playlist_and_other_patterns() {
        let project = empty_project_with_tracks(vec![
            track_with_clip(1, 0.0, 16.0, vec![note(1, 60, 0.0, 16.0)]),
            track_with_clip(2, 0.0, 16.0, vec![note(2, 50, 0.0, 4.0)]),
        ]);
        let mut project = project;
        project.pattern_lanes = vec![PatternLane {
            id: 1,
            name: String::from("Lane 1"),
            blocks: vec![
                PatternBlock {
                    id: 1,
                    name: String::from("Other"),
                    start_beats: 0.0,
                    length_beats: 8.0,
                    solo: false,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(10, 80, 0.0, 4.0)],
                        row_mode: None,
                    }],
                },
                PatternBlock {
                    id: 2,
                    name: String::from("Solo"),
                    start_beats: 4.0,
                    length_beats: 4.0,
                    solo: true,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(11, 72, 0.0, 2.0)],
                        row_mode: None,
                    }],
                },
            ],
        }];

        let track1 = resolve_midi_for_track(&project, 1);
        assert_eq!(track1.len(), 1);
        assert_eq!(track1[0].pitch, 72);
        assert_eq!(track1[0].start_beats, 4.0);

        let track2 = resolve_midi_for_track(&project, 2);
        assert!(track2.is_empty());
    }

    #[test]
    fn old_project_json_without_pattern_fields_loads_defaults() {
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
        assert!(project.pattern_lanes.is_empty());
        assert_eq!(project.next_pattern_lane_id(), 1);
        assert_eq!(project.next_pattern_block_id(), 1);
    }

    #[test]
    fn heuristic_row_mode_prefers_step_for_empty_or_sparse_pitches() {
        assert_eq!(
            PatternTrackContent::heuristic_row_mode(&[]),
            PatternRowMode::Step
        );
        // A single repeated pitch (typical kick/snare row) stays Step.
        let one_pitch = vec![note(1, 36, 0.0, 0.25), note(2, 36, 1.0, 0.25)];
        assert_eq!(
            PatternTrackContent::heuristic_row_mode(&one_pitch),
            PatternRowMode::Step
        );
        // Two distinct pitches (e.g. kick + hat sharing a row) is still Step.
        let two_pitches = vec![note(1, 36, 0.0, 0.25), note(2, 42, 0.5, 0.25)];
        assert_eq!(
            PatternTrackContent::heuristic_row_mode(&two_pitches),
            PatternRowMode::Step
        );
    }

    #[test]
    fn heuristic_row_mode_prefers_melody_for_three_or_more_pitches() {
        let melodic = vec![
            note(1, 60, 0.0, 1.0),
            note(2, 62, 1.0, 1.0),
            note(3, 64, 2.0, 1.0),
        ];
        assert_eq!(
            PatternTrackContent::heuristic_row_mode(&melodic),
            PatternRowMode::Melody
        );
    }

    #[test]
    fn pattern_row_suppressed_when_higher_lane_wins() {
        let project = empty_project_with_tracks(vec![track_with_clip(
            1,
            0.0,
            16.0,
            Vec::new(),
        )]);
        let mut project = project;
        project.pattern_lanes = vec![
            PatternLane {
                id: 1,
                name: String::from("Top"),
                blocks: vec![PatternBlock {
                    id: 1,
                    name: String::from("Top block"),
                    start_beats: 0.0,
                    length_beats: 8.0,
                    solo: false,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(10, 72, 0.0, 8.0)],
                        row_mode: None,
                    }],
                }],
            },
            PatternLane {
                id: 2,
                name: String::from("Bottom"),
                blocks: vec![PatternBlock {
                    id: 2,
                    name: String::from("Bottom block"),
                    start_beats: 4.0,
                    length_beats: 8.0,
                    solo: false,
                    tracks: vec![PatternTrackContent {
                        track_id: 1,
                        notes: vec![note(20, 48, 0.0, 8.0)],
                        row_mode: None,
                    }],
                }],
            },
        ];

        assert!(!pattern_row_suppressed_by_higher_lane(&project, 1, 1));
        assert!(pattern_row_suppressed_by_higher_lane(&project, 2, 1));
    }

    #[test]
    fn effective_row_mode_prefers_explicit_override_over_heuristic() {
        let melodic_notes = vec![
            note(1, 60, 0.0, 1.0),
            note(2, 62, 1.0, 1.0),
            note(3, 64, 2.0, 1.0),
        ];
        let mut content = PatternTrackContent {
            track_id: 1,
            notes: melodic_notes,
            row_mode: None,
        };
        assert_eq!(content.effective_row_mode(), PatternRowMode::Melody);

        content.row_mode = Some(PatternRowMode::Step);
        assert_eq!(content.effective_row_mode(), PatternRowMode::Step);
    }
}
