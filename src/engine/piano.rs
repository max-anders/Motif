//! Soft additive piano voice mixer for the audio callback.

const MAX_VOICES: usize = 32;
const HARMONICS: [(f32, f32); 4] = [
    (1.0, 1.0),
    (2.0, 0.45),
    (3.0, 0.22),
    (4.0, 0.10),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceStage {
    Attack,
    Decay,
    Release,
    Idle,
}

#[derive(Debug, Clone)]
struct Voice {
    pitch: u8,
    stage: VoiceStage,
    phase: f32,
    phase_inc: f32,
    amplitude: f32,
    envelope: f32,
    age: u64,
}

impl Voice {
    fn idle() -> Self {
        Self {
            pitch: 0,
            stage: VoiceStage::Idle,
            phase: 0.0,
            phase_inc: 0.0,
            amplitude: 0.0,
            envelope: 0.0,
            age: 0,
        }
    }

    fn is_active(&self) -> bool {
        self.stage != VoiceStage::Idle
    }
}

pub struct PianoSynth {
    voices: [Voice; MAX_VOICES],
    sample_rate: f32,
    next_age: u64,
    attack_per_sample: f32,
    decay_coeff: f32,
    release_coeff: f32,
}

impl PianoSynth {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        Self {
            voices: std::array::from_fn(|_| Voice::idle()),
            sample_rate: sr,
            next_age: 1,
            // ~5 ms attack
            attack_per_sample: 1.0 / (0.005 * sr),
            // exponential decay toward sustain floor
            decay_coeff: (-3.0 / sr).exp(),
            // ~80 ms release
            release_coeff: (-1.0 / (0.08 * sr)).exp(),
        }
    }

    pub fn note_on(&mut self, pitch: u8, velocity: u8) {
        let freq = midi_to_hz(pitch);
        let amplitude = (velocity as f32 / 127.0).clamp(0.05, 1.0) * 0.22;
        let phase_inc = freq / self.sample_rate;

        // Retrigger existing voice for this pitch if present.
        if let Some(voice) = self.voices.iter_mut().find(|v| v.is_active() && v.pitch == pitch) {
            voice.stage = VoiceStage::Attack;
            voice.phase = 0.0;
            voice.phase_inc = phase_inc;
            voice.amplitude = amplitude;
            voice.envelope = 0.0;
            voice.age = self.next_age;
            self.next_age = self.next_age.wrapping_add(1);
            return;
        }

        let slot = self
            .voices
            .iter()
            .position(|v| !v.is_active())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });

        self.voices[slot] = Voice {
            pitch,
            stage: VoiceStage::Attack,
            phase: 0.0,
            phase_inc,
            amplitude,
            envelope: 0.0,
            age: self.next_age,
        };
        self.next_age = self.next_age.wrapping_add(1);
    }

    pub fn note_off(&mut self, pitch: u8) {
        for voice in &mut self.voices {
            if voice.is_active() && voice.pitch == pitch && voice.stage != VoiceStage::Release {
                voice.stage = VoiceStage::Release;
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.stage = VoiceStage::Release;
            }
        }
    }

    /// Render one mono sample and advance envelopes/phases.
    pub fn render_sample(&mut self) -> f32 {
        let mut mix = 0.0_f32;

        for voice in &mut self.voices {
            if !voice.is_active() {
                continue;
            }

            match voice.stage {
                VoiceStage::Attack => {
                    voice.envelope += self.attack_per_sample;
                    if voice.envelope >= 1.0 {
                        voice.envelope = 1.0;
                        voice.stage = VoiceStage::Decay;
                    }
                }
                VoiceStage::Decay => {
                    voice.envelope *= self.decay_coeff;
                    // Keep a quiet sustain tail while key is held / note scheduled.
                    if voice.envelope < 0.02 {
                        voice.envelope = 0.02;
                    }
                }
                VoiceStage::Release => {
                    voice.envelope *= self.release_coeff;
                    if voice.envelope < 0.0005 {
                        *voice = Voice::idle();
                        continue;
                    }
                }
                VoiceStage::Idle => continue,
            }

            let mut sample = 0.0_f32;
            for &(harmonic, gain) in &HARMONICS {
                let phase = (voice.phase * harmonic).fract();
                sample += (phase * std::f32::consts::TAU).sin() * gain;
            }

            mix += sample * voice.envelope * voice.amplitude;
            voice.phase = (voice.phase + voice.phase_inc).fract();
        }

        mix.clamp(-1.0, 1.0)
    }
}

fn midi_to_hz(pitch: u8) -> f32 {
    440.0 * 2.0_f32.powf((pitch as f32 - 69.0) / 12.0)
}
