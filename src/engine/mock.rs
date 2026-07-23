use super::DawEngine;

pub struct MockEngine {
    playing: bool,
    current_beats: f32,
    playback_anchor_beats: f32,
    beats_per_second: f32,
}

impl MockEngine {
    pub fn new(beats_per_second: f32) -> Self {
        Self {
            playing: false,
            current_beats: 0.0,
            playback_anchor_beats: 0.0,
            beats_per_second,
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

    fn set_beats_per_second(&mut self, beats_per_second: f32) {
        self.beats_per_second = beats_per_second;
    }

    fn advance(&mut self, delta_seconds: f32, loop_end_beats: f32) {
        if !self.playing || loop_end_beats <= 0.0 {
            return;
        }

        self.current_beats += delta_seconds * self.beats_per_second;

        if self.current_beats >= loop_end_beats {
            self.current_beats %= loop_end_beats;
        }
    }
}
