mod audio;
#[allow(dead_code)]
mod mock;
mod piano;
pub mod plugins;

pub use audio::AudioEngine;
// Kept for silent fallback / tests; not wired in the app UI path.
#[allow(unused_imports)]
pub use mock::MockEngine;
pub use plugins::{CatalogEntry, PluginCatalog, PLUGIN_CACHE_FILE};

use crate::model::Project;

use self::plugins::PluginCatalog as Catalog;

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

    /// Audition / sequence note on for a specific track's instrument.
    fn note_on(&mut self, track_id: u64, pitch: u8, velocity: u8);
    fn note_off(&mut self, track_id: u64, pitch: u8);
    fn all_notes_off(&mut self);

    /// Load/unload per-track voices to match `project` instruments.
    /// Returns `(track_id, error)` pairs for failed loads.
    fn sync_instruments(
        &mut self,
        project: &Project,
        catalog: &Catalog,
    ) -> Vec<(u64, String)>;

    /// Drop cached voice identities so the next sync reloads everything.
    fn invalidate_instruments(&mut self);

    fn schedule_project(&mut self, project: &Project);
}
