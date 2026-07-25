use crate::model::Project;

use super::plugins::{PluginCatalog, PluginRef};
use super::{DawEngine, LoopPlayback};

pub struct MockEngine {
    playing: bool,
    current_beats: f32,
    playback_anchor_beats: f32,
    beats_per_second: f32,
    metronome_enabled: bool,
}

impl MockEngine {
    pub fn new(beats_per_second: f32) -> Self {
        Self {
            playing: false,
            current_beats: 0.0,
            playback_anchor_beats: 0.0,
            beats_per_second,
            metronome_enabled: true,
        }
    }

    pub fn set_beats_per_second(&mut self, beats_per_second: f32) {
        self.beats_per_second = beats_per_second;
    }
}

impl DawEngine for MockEngine {
    fn play(&mut self) {
        self.playback_anchor_beats = self.current_beats;
        self.playing = true;
    }

    fn pause(&mut self) {
        if self.playing {
            self.current_beats = self.playback_anchor_beats;
        }
        self.playing = false;
    }

    fn pause_in_place(&mut self) {
        if !self.playing {
            return;
        }
        self.playback_anchor_beats = self.current_beats;
        self.playing = false;
    }

    fn stop(&mut self) {
        self.playing = false;
    }

    fn toggle_playback(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    fn is_playing(&self) -> bool {
        self.playing
    }

    fn seek_beats(&mut self, beats: f32) {
        self.current_beats = beats.max(0.0);
        self.playback_anchor_beats = self.current_beats;
    }

    fn current_beats(&self) -> f32 {
        self.current_beats
    }

    fn playback_anchor_beats(&self) -> f32 {
        self.playback_anchor_beats
    }

    fn set_beats_per_second(&mut self, beats_per_second: f32) {
        self.beats_per_second = beats_per_second;
    }

    fn advance(&mut self, delta_seconds: f32, playback: LoopPlayback) {
        if !self.playing {
            return;
        }

        self.current_beats += delta_seconds * self.beats_per_second;

        if playback.enabled && playback.end_beats > playback.start_beats {
            if self.current_beats >= playback.end_beats {
                let span = playback.end_beats - playback.start_beats;
                let overshoot = (self.current_beats - playback.end_beats).rem_euclid(span);
                self.current_beats = playback.start_beats + overshoot;
            }
        } else if playback.content_end_beats > 0.0
            && self.current_beats >= playback.content_end_beats
        {
            self.current_beats = playback.content_end_beats;
            self.playing = false;
        }
    }

    fn note_on(&mut self, _track_id: u64, _pitch: u8, _velocity: u8) {}

    fn note_off(&mut self, _track_id: u64, _pitch: u8) {}

    fn all_notes_off(&mut self) {}

    fn sync_instruments(
        &mut self,
        _project: &Project,
        _catalog: &PluginCatalog,
    ) -> Vec<(u64, String)> {
        Vec::new()
    }

    fn invalidate_instruments(&mut self) {}

    fn schedule_project(&mut self, _project: &Project) {}

    fn set_metronome_enabled(&mut self, enabled: bool) {
        self.metronome_enabled = enabled;
    }

    fn metronome_enabled(&self) -> bool {
        self.metronome_enabled
    }

    fn open_plugin_editor(
        &mut self,
        _target: PluginRef,
        _title: &str,
        _host_x11: Option<super::plugins::HostX11>,
        _forward_transport: bool,
    ) -> Result<(), String> {
        Err(String::from("MockEngine has no plugins"))
    }
}
