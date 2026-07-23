mod audio;
#[allow(dead_code)]
mod mock;
mod piano;

pub use audio::AudioEngine;
// Kept for silent fallback / tests; not wired in the app UI path.
#[allow(unused_imports)]
pub use mock::MockEngine;

use crate::model::Project;

pub trait DawEngine {
    fn play(&mut self);
    fn pause(&mut self);
    fn stop(&mut self);
    fn toggle_playback(&mut self);
    fn is_playing(&self) -> bool;
    fn seek_beats(&mut self, beats: f32);
    fn current_beats(&self) -> f32;
    fn set_beats_per_second(&mut self, beats_per_second: f32);
    fn advance(&mut self, delta_seconds: f32, loop_end_beats: f32);

    fn note_on(&mut self, pitch: u8, velocity: u8);
    fn note_off(&mut self, pitch: u8);
    fn all_notes_off(&mut self);
    fn schedule_project(&mut self, project: &Project);
}
