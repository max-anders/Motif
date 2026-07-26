//! Sample-accurate MIDI sequencing on the audio thread.
//!
//! The UI thread owns the frames -> beats mapping but only paints when the
//! compositor asks it to. It therefore ships the *mapping* ([`RtClock`]) rather
//! than a position, and this sequencer evaluates it against the audio
//! callback's own frame counter. Note timing is then a function of rendered
//! frames alone: notes keep firing at the right sample while the window is
//! hidden, behind a modal plugin GUI, or during a multi-second paint stall.
//!
//! Two consequences worth knowing before touching this file:
//!
//! - The sequencer position deliberately does NOT subtract the output buffer
//!   the way `AudioEngine::advance` does. The callback renders audio that is
//!   heard one buffer later, so leading the ruler by exactly one buffer is what
//!   makes a note sound at the instant the ruler reaches it.
//! - Loop wrap happens here, not on the UI thread. A UI-side wrap would
//!   re-quantize note timing to the paint rate at every loop boundary.

use std::collections::HashMap;

/// A MIDI note flattened to absolute song beats for the audio thread.
///
/// The UI produces these per track, sorted ascending by `start_beats`; the
/// sorted order is what lets the sequencer binary-search a seek position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtNote {
    pub start_beats: f32,
    /// Already clamped to the owning clip's end.
    pub end_beats: f32,
    pub pitch: u8,
    pub velocity: u8,
}

/// The UI's frames -> beats mapping, shipped verbatim to the audio thread.
///
/// `epoch` is the discontinuity signal: it is bumped by `AudioEngine::reanchor`
/// and by nothing else, so a routine per-frame transport push carries an
/// identical clock and the sequencer keeps free-running. Without it the
/// callback would be dragged backwards every frame, because the position the
/// UI reports is one output buffer behind the position the callback renders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RtClock {
    pub epoch: u64,
    /// Beat the mapping is anchored to (`AudioEngine::clock_anchor_beats`).
    pub anchor_beats: f64,
    /// `samples_played` when the anchor was taken.
    pub anchor_samples: u64,
    pub beats_per_second: f64,
    pub playing: bool,
    pub loop_start_beats: f32,
    /// Not greater than `loop_start_beats` means "no active loop".
    pub loop_end_beats: f32,
}

impl Default for RtClock {
    fn default() -> Self {
        Self {
            epoch: 0,
            anchor_beats: 0.0,
            anchor_samples: 0,
            beats_per_second: 2.0,
            playing: false,
            loop_start_beats: 0.0,
            loop_end_beats: 0.0,
        }
    }
}

impl RtClock {
    /// `(start, end, span)` when a usable loop region is active.
    fn loop_region(&self) -> Option<(f64, f64, f64)> {
        let start = self.loop_start_beats as f64;
        let end = self.loop_end_beats as f64;
        if end > start {
            Some((start, end, end - start))
        } else {
            None
        }
    }
}

/// A note edge placed at a frame inside the current callback buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeqEvent {
    pub track_id: u64,
    /// Offset into the current buffer, in frames.
    pub frame: u32,
    pub pitch: u8,
    /// Zero for note offs.
    pub velocity: u8,
    pub on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ActiveNote {
    pitch: u8,
    velocity: u8,
    end_beats: f32,
}

/// Absorbs the f32-note / f64-clock rounding gap, so a beat that lands exactly
/// on a frame is not rounded up to the next one.
const FRAME_EPS: f64 = 1e-6;

/// Frames until `beats` have elapsed: the first frame at or after that beat.
fn frames_until(beats: f64, beats_per_frame: f64) -> f64 {
    (beats / beats_per_frame - FRAME_EPS).ceil().max(0.0)
}

#[derive(Default)]
struct TrackState {
    /// Index of the next note to consider for a note on.
    cursor: usize,
    active: Vec<ActiveNote>,
    /// Rebuild `cursor`/`active` from the position before emitting edges.
    needs_seed: bool,
}

pub struct RtSequencer {
    sample_rate: f64,
    clock: RtClock,
    notes: HashMap<u64, Vec<RtNote>>,
    tracks: HashMap<u64, TrackState>,
    events: Vec<SeqEvent>,
    /// Loop spans already consumed at the last frame processed. A mismatch at a
    /// segment boundary is exactly one loop wrap, wherever it falls.
    wrap_index: i64,
    /// The clock epoch moved: re-seed every track on the next block.
    clock_dirty: bool,
    /// RT scratch, reused every block (never allocate on the steady path).
    seed_scratch: Vec<ActiveNote>,
    off_scratch: Vec<(u8, f32)>,
    track_id_scratch: Vec<u64>,
}

impl RtSequencer {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0) as f64,
            clock: RtClock::default(),
            notes: HashMap::new(),
            tracks: HashMap::new(),
            events: Vec::new(),
            wrap_index: 0,
            clock_dirty: false,
            seed_scratch: Vec::new(),
            off_scratch: Vec::new(),
            track_id_scratch: Vec::new(),
        }
    }

    pub fn beats_per_second(&self) -> f64 {
        self.clock.beats_per_second
    }

    pub fn playing(&self) -> bool {
        self.clock.playing
    }

    /// Apply a transport push. Only an epoch change re-seeds; everything else
    /// (loop region, tempo carried alongside an unchanged epoch, `playing`) is
    /// data the free-running clock picks up on the next block.
    pub fn set_clock(&mut self, clock: RtClock) {
        if clock.epoch != self.clock.epoch {
            self.clock_dirty = true;
        }
        self.clock = clock;
    }

    /// Replace a track's note list. `notes` must be sorted by `start_beats`.
    pub fn set_notes(&mut self, track_id: u64, notes: Vec<RtNote>) {
        self.notes.insert(track_id, notes);
        self.tracks.entry(track_id).or_default().needs_seed = true;
    }

    /// Drop a track's notes but keep its state, so the next seed can release
    /// whatever it left sounding.
    pub fn clear_notes(&mut self, track_id: u64) {
        self.notes.remove(&track_id);
        if let Some(state) = self.tracks.get_mut(&track_id) {
            state.needs_seed = true;
        }
    }

    /// Forget a track entirely (its voice is gone, so nothing can sound).
    pub fn remove_track(&mut self, track_id: u64) {
        self.notes.remove(&track_id);
        self.tracks.remove(&track_id);
    }

    /// A fresh voice took over this track. Whatever the sequencer already sent
    /// went to the old voice (or to nothing at all, while a plugin was still
    /// loading), so drop the sounding set and re-derive it from the position -
    /// otherwise an instrument that finishes loading mid-note stays silent
    /// until that note ends.
    pub fn reseed_track(&mut self, track_id: u64) {
        let state = self.tracks.entry(track_id).or_default();
        state.active.clear();
        state.cursor = 0;
        state.needs_seed = true;
    }

    /// The voices were silenced under us (UI `AllNotesOff`): forget what was
    /// sounding so the next seed re-triggers from scratch.
    pub fn forget_active(&mut self) {
        for state in self.tracks.values_mut() {
            state.active.clear();
            state.cursor = 0;
            state.needs_seed = true;
        }
    }

    pub fn reset(&mut self) {
        self.notes.clear();
        self.tracks.clear();
        self.events.clear();
        self.wrap_index = 0;
    }

    pub fn events(&self) -> &[SeqEvent] {
        &self.events
    }

    /// Wrapped song position at `samples`, without touching sequencer state.
    pub fn position_beats(&self, samples: u64) -> f64 {
        Self::wrap_pos(&self.clock, self.linear_beats(samples))
    }

    /// Sequence one callback buffer. `block_start_samples` is `samples_played`
    /// read at the top of the callback (it is bumped after the buffer is
    /// written, so it names the first frame of this block).
    ///
    /// Returns the wrapped song position at the first frame, for the transport
    /// snapshot the rest of the callback renders against.
    pub fn process_block(&mut self, block_start_samples: u64, frames: usize) -> f64 {
        self.events.clear();
        let linear_start = self.linear_beats(block_start_samples);

        if self.clock_dirty {
            self.clock_dirty = false;
            self.wrap_index = Self::wrap_index_of(&self.clock, linear_start);
            for state in self.tracks.values_mut() {
                state.needs_seed = true;
            }
        }

        let pos_start = Self::wrap_pos(&self.clock, linear_start);
        let beats_per_frame = self.clock.beats_per_second / self.sample_rate;
        if !self.clock.playing || frames == 0 || !(beats_per_frame > 0.0) {
            return pos_start;
        }

        let loop_region = self.clock.loop_region();
        self.track_id_scratch.clear();
        self.track_id_scratch.extend(self.tracks.keys().copied());

        let mut frame = 0_usize;
        while frame < frames {
            let linear = linear_start + frame as f64 * beats_per_frame;

            let index = Self::wrap_index_of(&self.clock, linear);
            if index != self.wrap_index {
                self.wrap_index = index;
                Self::emit_loop_wrap(
                    &self.track_id_scratch,
                    &mut self.tracks,
                    frame as u32,
                    &mut self.events,
                );
            }

            let pos = Self::wrap_pos(&self.clock, linear);
            let remaining = frames - frame;
            let seg_frames = match loop_region {
                // Stop the segment on the frame that reaches the loop end so
                // the wrap is handled at its own sample, not at block start.
                Some((_, end, _)) => (frames_until(end - pos, beats_per_frame) as i64)
                    .clamp(1, remaining as i64) as usize,
                None => remaining,
            };
            let mut seg_end = pos + seg_frames as f64 * beats_per_frame;
            if let Some((_, end, _)) = loop_region {
                seg_end = seg_end.min(end);
            }

            for slot in 0..self.track_id_scratch.len() {
                let track_id = self.track_id_scratch[slot];
                let Some(state) = self.tracks.get_mut(&track_id) else {
                    continue;
                };
                let notes = self
                    .notes
                    .get(&track_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                if state.needs_seed {
                    state.needs_seed = false;
                    Self::seed_track(
                        track_id,
                        notes,
                        state,
                        pos,
                        frame as u32,
                        &mut self.events,
                        &mut self.seed_scratch,
                    );
                }
                Self::sequence_segment(
                    SegmentSpan {
                        track_id,
                        pos,
                        end: seg_end,
                        start_frame: frame as u32,
                        frames: seg_frames,
                        beats_per_frame,
                    },
                    notes,
                    state,
                    &mut self.events,
                    &mut self.off_scratch,
                );
            }

            frame += seg_frames;
        }

        // Offs before ons at the same frame so a note butted against its
        // neighbour retriggers instead of being cut by the outgoing one.
        self.events
            .sort_unstable_by_key(|event| (event.track_id, event.frame, event.on));
        pos_start
    }

    fn linear_beats(&self, samples: u64) -> f64 {
        let elapsed = samples.saturating_sub(self.clock.anchor_samples) as f64;
        self.clock.anchor_beats + elapsed / self.sample_rate * self.clock.beats_per_second
    }

    fn wrap_pos(clock: &RtClock, linear: f64) -> f64 {
        match clock.loop_region() {
            Some((start, end, span)) if linear >= end => start + (linear - end).rem_euclid(span),
            _ => linear.max(0.0),
        }
    }

    fn wrap_index_of(clock: &RtClock, linear: f64) -> i64 {
        match clock.loop_region() {
            Some((_, end, span)) if linear >= end => ((linear - end) / span).floor() as i64 + 1,
            _ => 0,
        }
    }

    /// Release everything the sequencer started, then let the per-track seed
    /// re-trigger whatever sits on the loop start.
    fn emit_loop_wrap(
        track_ids: &[u64],
        tracks: &mut HashMap<u64, TrackState>,
        frame: u32,
        events: &mut Vec<SeqEvent>,
    ) {
        for track_id in track_ids {
            let Some(state) = tracks.get_mut(track_id) else {
                continue;
            };
            for i in 0..state.active.len() {
                let pitch = state.active[i].pitch;
                if state.active[..i].iter().any(|note| note.pitch == pitch) {
                    continue;
                }
                events.push(SeqEvent {
                    track_id: *track_id,
                    frame,
                    pitch,
                    velocity: 0,
                    on: false,
                });
            }
            state.active.clear();
            state.needs_seed = true;
        }
    }

    /// Rebuild cursor + sounding set at `pos` and emit only the difference, so
    /// a re-seed never retriggers a note that was already correctly sounding.
    fn seed_track(
        track_id: u64,
        notes: &[RtNote],
        state: &mut TrackState,
        pos: f64,
        frame: u32,
        events: &mut Vec<SeqEvent>,
        scratch: &mut Vec<ActiveNote>,
    ) {
        scratch.clear();
        for note in notes {
            if note.end_beats <= note.start_beats {
                continue;
            }
            if (note.start_beats as f64) <= pos && (note.end_beats as f64) > pos {
                scratch.push(ActiveNote {
                    pitch: note.pitch,
                    velocity: note.velocity,
                    end_beats: note.end_beats,
                });
            }
        }
        state.cursor = notes.partition_point(|note| (note.start_beats as f64) <= pos);

        for i in 0..state.active.len() {
            let pitch = state.active[i].pitch;
            if state.active[..i].iter().any(|note| note.pitch == pitch) {
                continue;
            }
            if scratch.iter().any(|note| note.pitch == pitch) {
                continue;
            }
            events.push(SeqEvent {
                track_id,
                frame,
                pitch,
                velocity: 0,
                on: false,
            });
        }
        for i in 0..scratch.len() {
            let note = scratch[i];
            if scratch[..i].iter().any(|other| other.pitch == note.pitch) {
                continue;
            }
            if state.active.iter().any(|other| other.pitch == note.pitch) {
                continue;
            }
            events.push(SeqEvent {
                track_id,
                frame,
                pitch: note.pitch,
                velocity: note.velocity,
                on: true,
            });
        }

        state.active.clear();
        state.active.extend_from_slice(scratch);
    }

    fn sequence_segment(
        span: SegmentSpan,
        notes: &[RtNote],
        state: &mut TrackState,
        events: &mut Vec<SeqEvent>,
        off_scratch: &mut Vec<(u8, f32)>,
    ) {
        let last_offset = span.frames.saturating_sub(1) as f64;
        let frame_of = |beat: f64| -> u32 {
            let offset =
                frames_until(beat - span.pos, span.beats_per_frame).clamp(0.0, last_offset);
            span.start_frame + offset as u32
        };

        Self::flush_note_offs(&span, state, events, off_scratch, &frame_of);

        while state.cursor < notes.len() {
            let note = notes[state.cursor];
            if (note.start_beats as f64) >= span.end {
                break;
            }
            state.cursor += 1;
            if note.end_beats <= note.start_beats || (note.end_beats as f64) <= span.pos {
                continue;
            }
            let start = (note.start_beats as f64).max(span.pos);
            events.push(SeqEvent {
                track_id: span.track_id,
                frame: frame_of(start),
                pitch: note.pitch,
                velocity: note.velocity,
                on: true,
            });
            state.active.push(ActiveNote {
                pitch: note.pitch,
                velocity: note.velocity,
                end_beats: note.end_beats,
            });
        }

        // Again, for notes that both started and ended inside this segment -
        // the first pass ran before they were in `active`.
        Self::flush_note_offs(&span, state, events, off_scratch, &frame_of);
    }

    /// Release every sounding note whose end falls inside this segment. A pitch
    /// only stops when no other sounding note still holds it, and then at the
    /// latest of the ends belonging to that pitch.
    fn flush_note_offs(
        span: &SegmentSpan,
        state: &mut TrackState,
        events: &mut Vec<SeqEvent>,
        off_scratch: &mut Vec<(u8, f32)>,
        frame_of: &impl Fn(f64) -> u32,
    ) {
        off_scratch.clear();
        for note in state.active.iter() {
            if (note.end_beats as f64) >= span.end {
                continue;
            }
            match off_scratch
                .iter_mut()
                .find(|(pitch, _)| *pitch == note.pitch)
            {
                Some(entry) => entry.1 = entry.1.max(note.end_beats),
                None => off_scratch.push((note.pitch, note.end_beats)),
            }
        }
        if off_scratch.is_empty() {
            return;
        }
        state.active.retain(|note| (note.end_beats as f64) >= span.end);
        off_scratch.retain(|(pitch, _)| !state.active.iter().any(|note| note.pitch == *pitch));
        for (pitch, end) in off_scratch.iter() {
            events.push(SeqEvent {
                track_id: span.track_id,
                frame: frame_of(*end as f64),
                pitch: *pitch,
                velocity: 0,
                on: false,
            });
        }
    }
}

/// One contiguous slice of a callback buffer that maps linearly onto beats
/// (segments are split at loop wraps).
#[derive(Debug, Clone, Copy)]
struct SegmentSpan {
    track_id: u64,
    pos: f64,
    end: f64,
    start_frame: u32,
    frames: usize,
    beats_per_frame: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;
    /// 120 BPM: one beat = 24000 frames.
    const BPS: f64 = 2.0;

    fn note(pitch: u8, start: f32, end: f32) -> RtNote {
        RtNote {
            start_beats: start,
            end_beats: end,
            pitch,
            velocity: 100,
        }
    }

    fn clock(epoch: u64) -> RtClock {
        RtClock {
            epoch,
            anchor_beats: 0.0,
            anchor_samples: 0,
            beats_per_second: BPS,
            playing: true,
            loop_start_beats: 0.0,
            loop_end_beats: 0.0,
        }
    }

    fn sequencer(notes: Vec<RtNote>, clock: RtClock) -> RtSequencer {
        let mut seq = RtSequencer::new(SR);
        seq.set_clock(clock);
        seq.set_notes(1, notes);
        seq
    }

    /// Collect every event over `blocks` buffers as `(absolute_frame, pitch, on)`.
    fn run(seq: &mut RtSequencer, frames: usize, blocks: usize) -> Vec<(u64, u8, bool)> {
        let mut out = Vec::new();
        for block in 0..blocks {
            let start = (block * frames) as u64;
            seq.process_block(start, frames);
            for event in seq.events() {
                out.push((start + event.frame as u64, event.pitch, event.on));
            }
        }
        out
    }

    #[test]
    fn note_on_lands_on_its_exact_sample_not_the_block_edge() {
        // Beat 1 == frame 24000, which is 46.875 blocks of 512 frames in.
        let mut seq = sequencer(vec![note(60, 1.0, 2.0)], clock(1));
        let events = run(&mut seq, 512, 120);
        assert_eq!(
            events,
            vec![(24_000, 60, true), (48_000, 60, false)],
            "note edges must land on the sample, not the buffer boundary"
        );
    }

    #[test]
    fn timing_is_independent_of_buffer_size() {
        let mut small = sequencer(vec![note(60, 0.5, 1.5)], clock(1));
        let mut large = sequencer(vec![note(60, 0.5, 1.5)], clock(1));
        assert_eq!(run(&mut small, 64, 800), run(&mut large, 2048, 25));
    }

    #[test]
    fn a_note_shorter_than_one_paint_frame_still_fires() {
        // 1/64 beat = 375 frames: the old UI sequencer could step straight over
        // this note between two paints and never sound it.
        let mut seq = sequencer(vec![note(72, 1.0, 1.015_625)], clock(1));
        let events = run(&mut seq, 4096, 20);
        assert_eq!(events, vec![(24_000, 72, true), (24_375, 72, false)]);
    }

    #[test]
    fn routine_transport_pushes_never_drag_the_position_backwards() {
        // The UI reports a position one output buffer behind what the callback
        // renders; re-seeding on every push would pin the sequencer in place.
        let mut seq = sequencer(vec![note(60, 4.0, 5.0)], clock(1));
        let frames = 512;
        let mut fired = None;
        for block in 0..400 {
            let start = (block * frames) as u64;
            // Same epoch, same anchor: exactly what `push_transport` sends.
            seq.set_clock(clock(1));
            seq.process_block(start, frames);
            for event in seq.events() {
                if event.on {
                    fired = Some(start + event.frame as u64);
                }
            }
        }
        assert_eq!(fired, Some(96_000), "beat 4 at 120 BPM is frame 96000");
    }

    #[test]
    fn epoch_bump_reseeds_and_starts_a_note_the_playhead_landed_inside() {
        let mut seq = sequencer(vec![note(60, 10.0, 20.0)], clock(1));
        seq.process_block(0, 512);
        assert!(seq.events().is_empty());

        // Seek to beat 12, mid-note: anchor moves, epoch bumps.
        seq.set_clock(RtClock {
            epoch: 2,
            anchor_beats: 12.0,
            anchor_samples: 512,
            ..clock(2)
        });
        seq.process_block(512, 512);
        assert_eq!(
            seq.events(),
            [SeqEvent {
                track_id: 1,
                frame: 0,
                pitch: 60,
                velocity: 100,
                on: true
            }]
        );
    }

    #[test]
    fn reseed_leaves_an_already_sounding_note_alone() {
        let mut seq = sequencer(vec![note(60, 0.0, 100.0)], clock(1));
        seq.process_block(0, 512);
        assert_eq!(seq.events().len(), 1, "note starts");

        // Tempo change: same position, new anchor, new epoch.
        seq.set_clock(RtClock {
            epoch: 2,
            anchor_beats: 512.0 * BPS / SR as f64,
            anchor_samples: 512,
            beats_per_second: 3.0,
            ..clock(2)
        });
        seq.process_block(512, 512);
        assert!(
            seq.events().is_empty(),
            "a held note must not retrigger across a re-anchor"
        );
    }

    #[test]
    fn loop_wrap_is_sample_accurate_and_retriggers_the_downbeat() {
        // Loop 0..2 beats == 48000 frames, which is 93.75 buffers of 512: the
        // wrap has to land mid-block.
        let mut seq = sequencer(
            vec![note(60, 0.0, 2.0)],
            RtClock {
                loop_end_beats: 2.0,
                ..clock(1)
            },
        );
        let events = run(&mut seq, 512, 200);
        assert_eq!(
            events,
            vec![
                (0, 60, true),
                (48_000, 60, false),
                (48_000, 60, true),
                (96_000, 60, false),
                (96_000, 60, true),
            ],
            "wrap edges land on the loop-end sample, off before on"
        );
    }

    #[test]
    fn loop_wrap_falling_on_a_block_boundary_is_not_lost() {
        // An 8-beat loop is 192000 frames == exactly 375 buffers of 512, so the
        // wrap lands at frame 0 of a block instead of inside one.
        let mut seq = sequencer(
            vec![note(60, 0.0, 8.0)],
            RtClock {
                loop_end_beats: 8.0,
                ..clock(1)
            },
        );
        let events = run(&mut seq, 512, 400);
        assert_eq!(
            events,
            vec![(0, 60, true), (192_000, 60, false), (192_000, 60, true)]
        );
    }

    #[test]
    fn overlapping_same_pitch_notes_hold_until_the_later_end() {
        let mut seq = sequencer(vec![note(60, 0.0, 2.0), note(60, 1.0, 3.0)], clock(1));
        let events = run(&mut seq, 512, 200);
        assert_eq!(
            events,
            vec![(0, 60, true), (24_000, 60, true), (72_000, 60, false)],
            "the pitch must not be released at the first note's end"
        );
    }

    #[test]
    fn editing_a_track_reseeds_without_disturbing_a_held_note() {
        let mut seq = sequencer(vec![note(60, 0.0, 8.0)], clock(1));
        seq.process_block(0, 512);
        assert_eq!(seq.events().len(), 1);

        seq.set_notes(1, vec![note(60, 0.0, 8.0), note(64, 0.0, 8.0)]);
        seq.process_block(512, 512);
        assert_eq!(
            seq.events(),
            [SeqEvent {
                track_id: 1,
                frame: 0,
                pitch: 64,
                velocity: 100,
                on: true
            }],
            "only the added pitch should sound"
        );
    }

    #[test]
    fn clearing_a_track_releases_what_it_left_sounding() {
        let mut seq = sequencer(vec![note(60, 0.0, 8.0)], clock(1));
        seq.process_block(0, 512);
        seq.clear_notes(1);
        seq.process_block(512, 512);
        assert_eq!(
            seq.events(),
            [SeqEvent {
                track_id: 1,
                frame: 0,
                pitch: 60,
                velocity: 0,
                on: false
            }]
        );
    }

    #[test]
    fn a_stopped_transport_emits_nothing() {
        let mut seq = sequencer(
            vec![note(60, 0.0, 8.0)],
            RtClock {
                playing: false,
                ..clock(1)
            },
        );
        seq.process_block(0, 512);
        assert!(seq.events().is_empty());
    }

    #[test]
    fn the_ruler_and_the_sequencer_describe_the_same_line_across_loop_wraps() {
        // The load-bearing invariant of the split: `AudioEngine::advance`
        // subtracts one output buffer and slides its anchor down one span per
        // wrap, while the sequencer applies `rem_euclid` to an anchor it never
        // touches. If these ever disagree, notes fire off the ruler.
        let (loop_start, loop_end) = (1.0_f64, 5.0_f64);
        let span = loop_end - loop_start;
        let buffer = 512_u64;
        let seq = sequencer(
            vec![],
            RtClock {
                anchor_beats: loop_start,
                loop_start_beats: loop_start as f32,
                loop_end_beats: loop_end as f32,
                ..clock(1)
            },
        );

        let mut anchor = loop_start;
        let frames = 512_u64;
        for block in 1..2_000_u64 {
            let samples = block * frames;
            let elapsed = samples.saturating_sub(buffer) as f64;
            let mut ruler = (anchor + elapsed / SR as f64 * BPS).max(anchor);
            if ruler >= loop_end {
                ruler = loop_start + (ruler - loop_end).rem_euclid(span);
                anchor -= span;
            }
            let rendered = seq.position_beats(samples.saturating_sub(buffer));
            assert!(
                (ruler - rendered).abs() < 1e-9,
                "block {block}: ruler {ruler}, sequencer {rendered}"
            );
        }
    }

    #[test]
    fn position_leads_the_ui_ruler_by_the_render_offset_only() {
        // The sequencer reads the same anchor as `AudioEngine::advance` but
        // without its buffer subtraction, so it is exactly one buffer ahead.
        let seq = sequencer(vec![], clock(1));
        let buffer = 512_u64;
        let rt = seq.position_beats(48_000);
        let ui = seq.position_beats(48_000 - buffer);
        assert!((rt - 2.0).abs() < 1e-9);
        assert!((rt - ui - buffer as f64 * BPS / SR as f64).abs() < 1e-9);
    }
}
