mod audio;
#[allow(dead_code)]
mod mock;
mod piano;
pub mod plugins;

pub use audio::AudioEngine;
// Kept for silent fallback / tests; not wired in the app UI path.
#[allow(unused_imports)]
pub use mock::MockEngine;
pub use plugins::{CatalogEntry, HostX11, PluginCatalog, PLUGIN_CACHE_FILE};

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

    /// True when an activated plugin instance is ready for this track.
    fn plugin_slot_ready(&self, track_id: u64) -> bool {
        let _ = track_id;
        false
    }

    /// Open the native plugin editor for a track (UI thread).
    /// `host_x11` should be Motif's Display + window so the editor parent shares the
    /// same X11 connection as winit (required for clickable GUIs under XWayland).
    fn open_plugin_editor(
        &mut self,
        track_id: u64,
        title: &str,
        host_x11: Option<crate::engine::plugins::HostX11>,
    ) -> Result<(), String> {
        let _ = (track_id, title, host_x11);
        Err(String::from("Plugin editors not available"))
    }

    fn close_plugin_editor(&mut self, track_id: u64) {
        let _ = track_id;
    }

    fn plugin_editor_is_open(&self, track_id: u64) -> bool {
        let _ = track_id;
        false
    }

    /// Poll editor windows / idle callbacks. Returns true if any editor is open.
    fn poll_plugin_editors(&mut self) -> bool {
        false
    }
}
