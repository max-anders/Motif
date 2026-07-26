//! Sample-accurate metronome clicks mixed on the audio thread.

const CLICK_DURATION_SECS: f32 = 0.025;

/// Short sine burst with exponential decay; downbeat is louder and higher.
struct ClickVoice {
    phase: f32,
    phase_inc: f32,
    amplitude: f32,
    envelope: f32,
    decay: f32,
    active: bool,
}

impl ClickVoice {
    fn idle() -> Self {
        Self {
            phase: 0.0,
            phase_inc: 0.0,
            amplitude: 0.0,
            envelope: 0.0,
            decay: 0.0,
            active: false,
        }
    }

    fn start(sample_rate: f32, downbeat: bool) -> Self {
        let sr = sample_rate.max(1.0);
        let freq = if downbeat { 1_200.0 } else { 800.0 };
        let amplitude = if downbeat { 0.45 } else { 0.22 };
        Self {
            phase: 0.0,
            phase_inc: freq / sr,
            amplitude,
            envelope: 1.0,
            decay: (-1.0 / (CLICK_DURATION_SECS * sr)).exp(),
            active: true,
        }
    }

    fn render(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let sample = self.phase.sin() * self.amplitude * self.envelope;
        self.phase = (self.phase + self.phase_inc).rem_euclid(1.0);
        self.envelope *= self.decay;
        if self.envelope <= 1e-5 {
            self.active = false;
        }
        sample
    }
}

/// Renders up to two overlapping click bursts (enough at normal tempos).
pub struct MetronomeSynth {
    sample_rate: f32,
    voices: [ClickVoice; 2],
}

impl MetronomeSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            voices: [ClickVoice::idle(), ClickVoice::idle()],
        }
    }

    pub fn trigger(&mut self, downbeat: bool) {
        let voice = ClickVoice::start(self.sample_rate, downbeat);
        if !self.voices[0].active {
            self.voices[0] = voice;
        } else {
            self.voices[1] = voice;
        }
    }

    pub fn render_sample(&mut self) -> (f32, f32) {
        let mono = (self.voices[0].render() + self.voices[1].render()).clamp(-1.0, 1.0);
        (mono, mono)
    }
}

/// Beat scheduler + click synth; advances in wrapped beat space.
pub struct MetronomeRunner {
    synth: MetronomeSynth,
    enabled: bool,
    playing: bool,
    position_beats: f64,
    beats_per_second: f64,
    beats_per_bar: f32,
    loop_start_beats: f32,
    loop_end_beats: f32,
    sample_rate: f64,
    #[cfg(test)]
    triggers_fired: usize,
}

impl MetronomeRunner {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0) as f64;
        Self {
            synth: MetronomeSynth::new(sample_rate),
            enabled: true,
            playing: false,
            position_beats: 0.0,
            beats_per_second: 2.0,
            beats_per_bar: 4.0,
            loop_start_beats: 0.0,
            loop_end_beats: 16.0,
            sample_rate: sr,
            #[cfg(test)]
            triggers_fired: 0,
        }
    }

    #[cfg(test)]
    fn reset_trigger_count(&mut self) {
        self.triggers_fired = 0;
    }

    #[cfg(test)]
    fn trigger_count(&self) -> usize {
        self.triggers_fired
    }

    fn fire_click(&mut self, downbeat: bool) {
        #[cfg(test)]
        {
            self.triggers_fired += 1;
        }
        self.synth.trigger(downbeat);
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    pub fn set_beats_per_second(&mut self, beats_per_second: f64) {
        self.beats_per_second = beats_per_second.max(0.000_1);
    }

    pub fn set_beats_per_bar(&mut self, beats_per_bar: f32) {
        self.beats_per_bar = beats_per_bar.max(1.0);
    }

    pub fn set_loop_start_beats(&mut self, loop_start_beats: f32) {
        self.loop_start_beats = loop_start_beats;
    }

    pub fn set_loop_end_beats(&mut self, loop_end_beats: f32) {
        self.loop_end_beats = loop_end_beats;
    }

    /// Re-seat the click grid on a transport discontinuity. The audio thread
    /// drives this from the sequencer's position, so the two clocks cannot
    /// drift apart (see `AudioCallbackState::apply_transport`).
    pub fn sync_position_beats(&mut self, beats: f64) {
        self.position_beats = beats.max(0.0);
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    fn beat_is_downbeat(beat_index: i64, beats_per_bar: f32) -> bool {
        if beat_index <= 0 {
            return true;
        }
        (beat_index as f64).rem_euclid(beats_per_bar as f64) < 1e-6
    }

    fn advance_one_sample(&mut self) {
        let prev = self.position_beats;
        let delta = self.beats_per_second / self.sample_rate;
        let mut next = prev + delta;
        let loop_start = self.loop_start_beats as f64;
        let loop_end = self.loop_end_beats as f64;

        if loop_end > loop_start && next >= loop_end {
            let prev_int = prev.floor() as i64;
            let last_int = (loop_end - 1e-9).floor() as i64;
            for beat in (prev_int + 1)..=last_int {
                if beat as f64 > prev {
                    self.fire_click(Self::beat_is_downbeat(beat, self.beats_per_bar));
                }
            }
            self.fire_click(true);
            next = loop_start + (next - loop_end);
        } else {
            let prev_int = prev.floor() as i64;
            let next_int = next.floor() as i64;
            for beat in (prev_int + 1)..=next_int {
                self.fire_click(Self::beat_is_downbeat(beat, self.beats_per_bar));
            }
        }

        self.position_beats = next;
    }

    pub fn process_block(&mut self, frames: usize, mix_l: &mut [f32], mix_r: &mut [f32]) {
        if !self.enabled || !self.playing || frames == 0 {
            return;
        }

        for i in 0..frames {
            self.advance_one_sample();
            let (l, r) = self.synth.render_sample();
            if let Some(slot) = mix_l.get_mut(i) {
                *slot += l;
            }
            if let Some(slot) = mix_r.get_mut(i) {
                *slot += r;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_beats_per_second_produces_two_clicks_in_one_second() {
        let sr = 48_000.0;
        let mut runner = MetronomeRunner::new(sr);
        runner.set_enabled(true);
        runner.set_playing(true);
        runner.set_beats_per_second(2.0);
        runner.set_beats_per_bar(4.0);
        runner.set_loop_end_beats(16.0);
        runner.sync_position_beats(0.0);
        runner.reset_trigger_count();

        let frames = sr as usize;
        let mut mix_l = vec![0.0; frames];
        let mut mix_r = vec![0.0; frames];
        runner.process_block(frames, &mut mix_l, &mut mix_r);

        assert_eq!(
            runner.trigger_count(),
            2,
            "expected 2 quarter-note clicks at 120 BPM"
        );
    }

    #[test]
    fn downbeat_louder_than_offbeat() {
        let sr = 48_000.0;
        let mut down = MetronomeSynth::new(sr);
        down.trigger(true);
        let mut off = MetronomeSynth::new(sr);
        off.trigger(false);

        let down_peak = (0..512)
            .map(|_| down.render_sample().0)
            .fold(0.0f32, f32::max);
        let off_peak = (0..512)
            .map(|_| off.render_sample().0)
            .fold(0.0f32, f32::max);
        assert!(down_peak > off_peak);
    }

    #[test]
    fn loop_wrap_fires_downbeat_and_continues_grid() {
        let sr = 48_000.0;
        let bps = 2.0;
        let loop_end = 4.0;
        let mut runner = MetronomeRunner::new(sr);
        runner.set_enabled(true);
        runner.set_playing(true);
        runner.set_beats_per_second(bps);
        runner.set_beats_per_bar(4.0);
        runner.set_loop_end_beats(loop_end);
        runner.sync_position_beats(3.95);
        runner.reset_trigger_count();

        // 1.5 beats of audio: cross loop end once, then cross beat 1 after wrap.
        let frames = (f64::from(sr) / bps * 1.5) as usize;
        let mut mix_l = vec![0.0; frames];
        let mut mix_r = vec![0.0; frames];
        runner.process_block(frames, &mut mix_l, &mut mix_r);

        assert_eq!(
            runner.trigger_count(),
            2,
            "expected wrap downbeat plus beat 1 after loop"
        );
    }
}
