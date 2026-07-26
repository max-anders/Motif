mod audio;
#[allow(dead_code)]
mod mock;
mod metronome;
mod piano;
pub mod plugins;
mod sample;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use audio::{AudioEngine, ParamTouchEvent};
// Kept for silent fallback / tests; not wired in the app UI path.
#[allow(unused_imports)]
pub use mock::MockEngine;

/// Live audio-thread telemetry for the transport bar / Performance view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnginePerformance {
    /// Recent callback load as percent of the buffer period (can exceed 100).
    pub cpu_percent: f32,
    /// Frames delivered in the latest callback (device period).
    pub buffer_frames: u32,
    pub sample_rate_hz: u32,
    /// Estimated one-buffer output latency in milliseconds.
    pub latency_ms: f32,
    /// Callbacks that exceeded their buffer budget (underrun risk).
    pub xruns: u64,
    /// Plugin `try_lock` failures skipped on the RT thread (dropout risk).
    pub lock_skips: u64,
}

impl Default for EnginePerformance {
    fn default() -> Self {
        Self {
            cpu_percent: 0.0,
            buffer_frames: 0,
            sample_rate_hz: 0,
            latency_ms: 0.0,
            xruns: 0,
            lock_skips: 0,
        }
    }
}

/// Kind of voice currently mixed for a track (audio-thread view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackVoiceKind {
    #[default]
    None,
    Piano,
    Plugin,
    Silent,
}

impl TrackVoiceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "-",
            Self::Piano => "Piano",
            Self::Plugin => "Plugin",
            Self::Silent => "Silent",
        }
    }
}

/// Per-track DSP timing for the latest audio callback (cosmetic / UI only).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TrackPerformance {
    pub track_id: u64,
    pub voice_kind: TrackVoiceKind,
    /// Instrument / piano voice DSP (ms).
    pub voice_ms: f32,
    /// Insert-FX chain DSP (ms).
    pub fx_ms: f32,
    /// Sample-clip mix DSP (ms).
    pub samples_ms: f32,
    /// voice + fx + samples for this track (ms).
    pub total_ms: f32,
    /// `try_lock` skips on this track during the latest callback.
    pub lock_skips: u32,
    /// Active piano voices (0 for plugin / silent).
    pub active_voices: u32,
}
#[allow(unused_imports)] // EntryCategory: public catalog surface, consumed once the effect picker lands
pub use plugins::{
    CatalogEntry, EditorCloseBinding, EditorPoll, EntryCategory, HostX11, PluginCatalog,
    PluginParamInfo, PluginRef, PLUGIN_CACHE_FILE,
};
pub use sample::{decode_audio_file, DecodedAudio};

use crate::model::Project;

use self::plugins::PluginCatalog as Catalog;

/// Per-frame transport bounds passed to the engine: the loop region (when the
/// user enabled cycling) plus the end of arranged content (used to stop
/// playback at the end of the song when not looping).
#[derive(Debug, Clone, Copy, Default)]
pub struct LoopPlayback {
    pub enabled: bool,
    pub start_beats: f32,
    pub end_beats: f32,
    pub content_end_beats: f32,
}

pub trait DawEngine {
    fn play(&mut self);
    /// Pause and restore the playhead to where playback started.
    fn pause(&mut self);
    /// Pause, leave the playhead where it is, and move the return-anchor there.
    fn pause_in_place(&mut self);
    fn stop(&mut self);
    fn toggle_playback(&mut self);
    fn is_playing(&self) -> bool;
    fn seek_beats(&mut self, beats: f32);
    fn current_beats(&self) -> f32;
    /// Beat where Pause will return the playhead (set on Play / seek / pause-in-place).
    fn playback_anchor_beats(&self) -> f32;
    fn set_beats_per_second(&mut self, beats_per_second: f32);
    fn advance(&mut self, delta_seconds: f32, playback: LoopPlayback);

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

    /// Push decoded audio clips per track to the audio thread.
    fn sync_samples(&mut self, project: &Project, decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>) {
        let _ = (project, decoded_audio);
    }

    /// Latest per-track peak meters as `(track_id, peak_l, peak_r)`. Cosmetic/UI only.
    fn meter_levels(&self) -> Vec<(u64, f32, f32)> {
        Vec::new()
    }

    /// Free-running Hz LFO phase in cycles `0..1` for `(track_id, modulator_id)`.
    /// Does not include the modulator's phase offset. Tempo-synced modulators
    /// are not stored here — derive those from [`Self::current_beats`].
    fn free_lfo_phase(&self, track_id: u64, modulator_id: u64) -> Option<f32> {
        let _ = (track_id, modulator_id);
        None
    }

    /// Latest master-bus peak `(peak_l, peak_r)`.
    fn master_meter(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Latest audio-thread performance snapshot (CPU load, buffer, xruns).
    fn performance(&self) -> EnginePerformance {
        EnginePerformance::default()
    }

    /// Per-track DSP timing from the latest callback (empty when unavailable).
    fn track_performance(&self) -> Vec<TrackPerformance> {
        Vec::new()
    }

    /// Output device name reported by cpal at stream open.
    fn audio_device_name(&self) -> Option<String> {
        None
    }

    /// In-flight background plugin loads: `(instruments, insert_fx)`.
    fn pending_plugin_loads(&self) -> (usize, usize) {
        (0, 0)
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

    fn set_plugin_editor_close_binding(
        &mut self,
        _close_binding: crate::engine::plugins::EditorCloseBinding,
    ) {
    }

    /// Poll editor windows / idle callbacks. Returns aggregated outcome.
    fn poll_plugin_editors(&mut self) -> EditorPoll {
        EditorPoll::default()
    }

    /// Enumerate automatable parameters for a track instrument (`device_id: None`)
    /// or one insert-FX device.
    fn plugin_parameters(&self, track_id: u64, device_id: Option<u64>) -> Vec<PluginParamInfo> {
        let _ = (track_id, device_id);
        Vec::new()
    }

    /// Current normalized `0..1` value for a plugin param (`None` if unloaded / unknown).
    fn plugin_param_normalized(
        &self,
        track_id: u64,
        device_id: Option<u64>,
        param_id: u32,
    ) -> Option<f32> {
        let _ = (track_id, device_id, param_id);
        None
    }

    /// Set a plugin param from normalized `0..1`. Returns false if the slot is unavailable.
    fn set_plugin_param_normalized(
        &mut self,
        track_id: u64,
        device_id: Option<u64>,
        param_id: u32,
        normalized: f32,
    ) -> bool {
        let _ = (track_id, device_id, param_id, normalized);
        false
    }
}
