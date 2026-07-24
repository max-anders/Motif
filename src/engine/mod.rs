mod audio;
#[allow(dead_code)]
mod mock;
mod metronome;
mod piano;
pub mod plugins;

pub use audio::AudioEngine;
// Kept for silent fallback / tests; not wired in the app UI path.
#[allow(unused_imports)]
pub use mock::MockEngine;
#[allow(unused_imports)] // EntryCategory: public catalog surface, consumed once the effect picker lands
pub use plugins::{
    CatalogEntry, EditorPoll, EntryCategory, HostX11, PluginCatalog, PluginRef, PLUGIN_CACHE_FILE,
};

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
    fn sync_instruments(&mut self, project: &Project, catalog: &Catalog) -> Vec<(u64, String)>;

    /// Copy live CLAP/VST3 state into each track's `plugin_state` before project save.
    fn capture_plugin_states(&mut self, project: &mut Project) {
        let _ = project;
    }

    /// Load/unload each track's insert-FX device chain to match `project`.
    /// Returns `(track_id, device_id, error)` triples for failed loads.
    fn sync_devices(&mut self, project: &Project, catalog: &Catalog) -> Vec<(u64, u64, String)> {
        let _ = (project, catalog);
        Vec::new()
    }

    /// Copy live CLAP/VST3 state into each device's `plugin_state` before project save.
    fn capture_device_states(&mut self, project: &mut Project) {
        let _ = project;
    }

    /// Drop cached voice identities so the next sync reloads everything.
    fn invalidate_instruments(&mut self);

    fn schedule_project(&mut self, project: &Project);

    fn set_metronome_enabled(&mut self, enabled: bool);
    fn metronome_enabled(&self) -> bool;

    /// Push per-track gain/pan (and master gain) from `project` to the audio thread.
    fn sync_channels(&mut self, project: &Project) {
        let _ = project;
    }

    /// Latest per-track peak meters as `(track_id, peak_l, peak_r)`. Cosmetic/UI only.
    fn meter_levels(&self) -> Vec<(u64, f32, f32)> {
        Vec::new()
    }

    /// Latest master-bus peak `(peak_l, peak_r)`.
    fn master_meter(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// True when an activated plugin instance is ready for this slot
    /// (a track's instrument, or one of its insert-FX devices).
    fn plugin_slot_ready(&self, target: PluginRef) -> bool {
        let _ = target;
        false
    }

    /// Open the native plugin editor for a slot (UI thread).
    /// `host_x11` should be Motif's Display + window so the editor parent shares the
    /// same X11 connection as winit (required for clickable GUIs under XWayland).
    /// `forward_transport` grabs Space so it drives Motif transport while the
    /// editor is focused, instead of going to the plugin.
    fn open_plugin_editor(
        &mut self,
        target: PluginRef,
        title: &str,
        host_x11: Option<crate::engine::plugins::HostX11>,
        forward_transport: bool,
    ) -> Result<(), String> {
        let _ = (target, title, host_x11, forward_transport);
        Err(String::from("Plugin editors not available"))
    }

    fn close_plugin_editor(&mut self, target: PluginRef) {
        let _ = target;
    }

    fn plugin_editor_is_open(&self, target: PluginRef) -> bool {
        let _ = target;
        false
    }

    /// Plugin refs + titles of currently open plugin editors.
    fn open_plugin_editors(&self) -> Vec<(PluginRef, String)> {
        Vec::new()
    }

    /// Live-toggle Space transport forwarding for one open editor.
    fn set_plugin_editor_transport(&mut self, target: PluginRef, forward: bool) {
        let _ = (target, forward);
    }

    /// Poll editor windows / idle callbacks. Returns aggregated outcome.
    fn poll_plugin_editors(&mut self) -> EditorPoll {
        EditorPoll::default()
    }
}
