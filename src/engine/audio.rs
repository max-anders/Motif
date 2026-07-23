//! cpal-backed audio engine with UI-thread transport + edge-detect sequencing.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};

use crate::model::Project;

use super::piano::PianoSynth;
use super::DawEngine;

#[derive(Debug, Clone, Copy)]
enum AudioCommand {
    NoteOn { pitch: u8, velocity: u8 },
    NoteOff { pitch: u8 },
    AllNotesOff,
}

struct AudioCallbackState {
    synth: PianoSynth,
    commands: Receiver<AudioCommand>,
    channels: usize,
}

impl AudioCallbackState {
    fn process_commands(&mut self) {
        loop {
            match self.commands.try_recv() {
                Ok(AudioCommand::NoteOn { pitch, velocity }) => {
                    self.synth.note_on(pitch, velocity);
                }
                Ok(AudioCommand::NoteOff { pitch }) => {
                    self.synth.note_off(pitch);
                }
                Ok(AudioCommand::AllNotesOff) => {
                    self.synth.all_notes_off();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn write_f32(&mut self, data: &mut [f32]) {
        self.process_commands();
        if self.channels == 0 {
            return;
        }
        for frame in data.chunks_mut(self.channels) {
            let sample = self.synth.render_sample();
            for channel in frame.iter_mut() {
                *channel = sample;
            }
        }
    }
}

/// Real-time engine: UI advances the playhead; audio thread mixes the piano.
pub struct AudioEngine {
    playing: bool,
    current_beats: f32,
    previous_beats: f32,
    beats_per_second: f32,
    /// Note ids currently sounding from the sequencer (not keyboard audition).
    active_seq_notes: HashSet<u64>,
    command_tx: Option<SyncSender<AudioCommand>>,
    /// Kept alive for the lifetime of the engine.
    _stream: Option<Stream>,
    audio_available: bool,
    init_error: Option<String>,
}

impl AudioEngine {
    pub fn new(beats_per_second: f32) -> Self {
        match start_stream() {
            Ok((stream, tx)) => {
                if let Err(error) = stream.play() {
                    Self {
                        playing: false,
                        current_beats: 0.0,
                        previous_beats: 0.0,
                        beats_per_second,
                        active_seq_notes: HashSet::new(),
                        command_tx: Some(tx),
                        _stream: Some(stream),
                        audio_available: false,
                        init_error: Some(format!("Audio stream play failed: {error}")),
                    }
                } else {
                    Self {
                        playing: false,
                        current_beats: 0.0,
                        previous_beats: 0.0,
                        beats_per_second,
                        active_seq_notes: HashSet::new(),
                        command_tx: Some(tx),
                        _stream: Some(stream),
                        audio_available: true,
                        init_error: None,
                    }
                }
            }
            Err(error) => Self {
                playing: false,
                current_beats: 0.0,
                previous_beats: 0.0,
                beats_per_second,
                active_seq_notes: HashSet::new(),
                command_tx: None,
                _stream: None,
                audio_available: false,
                init_error: Some(error),
            },
        }
    }

    pub fn audio_available(&self) -> bool {
        self.audio_available
    }

    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }

    fn send(&self, command: AudioCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.try_send(command);
        }
    }

    fn silence_sequencer(&mut self) {
        self.active_seq_notes.clear();
        self.send(AudioCommand::AllNotesOff);
    }
}

impl DawEngine for AudioEngine {
    fn play(&mut self) {
        self.previous_beats = self.current_beats;
        self.playing = true;
    }

    fn pause(&mut self) {
        self.playing = false;
        self.silence_sequencer();
    }

    fn stop(&mut self) {
        self.playing = false;
        self.silence_sequencer();
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
        self.previous_beats = self.current_beats;
        self.silence_sequencer();
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

        self.previous_beats = self.current_beats;
        self.current_beats += delta_seconds * self.beats_per_second;

        if self.current_beats >= loop_end_beats {
            // Loop wrap: silence then restart edge detection from 0.
            self.current_beats %= loop_end_beats;
            self.previous_beats = self.current_beats - delta_seconds * self.beats_per_second;
            // Force a clean slate so notes at the loop start can retrigger.
            self.active_seq_notes.clear();
            self.send(AudioCommand::AllNotesOff);
            // Treat previous as just before 0 so notes at beat 0 fire.
            self.previous_beats = -0.0001;
        }
    }

    fn note_on(&mut self, pitch: u8, velocity: u8) {
        self.send(AudioCommand::NoteOn { pitch, velocity });
    }

    fn note_off(&mut self, pitch: u8) {
        self.send(AudioCommand::NoteOff { pitch });
    }

    fn all_notes_off(&mut self) {
        self.silence_sequencer();
    }

    fn schedule_project(&mut self, project: &Project) {
        if !self.playing {
            return;
        }

        let prev = self.previous_beats;
        let curr = self.current_beats;
        let mut should_be_active: HashSet<u64> = HashSet::new();

        for track in &project.tracks {
            for clip in &track.clips {
                let clip_start = clip.start_beats;
                let clip_end = clip.end_beats();

                for note in &clip.notes {
                    let abs_start = clip_start + note.start_beats;
                    let abs_end = (clip_start + note.end_beats()).min(clip_end);
                    if abs_end <= abs_start {
                        continue;
                    }

                    // Active if playhead is inside the note this frame.
                    let active_now = curr >= abs_start && curr < abs_end;
                    if active_now {
                        should_be_active.insert(note.id);
                    }

                    let crossed_start = prev < abs_start && curr >= abs_start && curr < abs_end;
                    let was_active = self.active_seq_notes.contains(&note.id);

                    if crossed_start || (active_now && !was_active) {
                        self.send(AudioCommand::NoteOn {
                            pitch: note.pitch,
                            velocity: note.velocity,
                        });
                    }
                }
            }
        }

        // Note-offs for notes that ended or left the active set.
        let ended: Vec<u64> = self
            .active_seq_notes
            .difference(&should_be_active)
            .copied()
            .collect();

        for note_id in ended {
            let Some((pitch, _)) = find_note_pitch(project, note_id) else {
                continue;
            };
            let pitch_still_held = should_be_active.iter().any(|id| {
                find_note_pitch(project, *id).is_some_and(|(p, _)| p == pitch)
            });
            if !pitch_still_held {
                self.send(AudioCommand::NoteOff { pitch });
            }
        }

        self.active_seq_notes = should_be_active;
        self.previous_beats = curr;
    }
}

fn find_note_pitch(project: &Project, note_id: u64) -> Option<(u8, u8)> {
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(note) = clip.note(note_id) {
                return Some((note.pitch, note.velocity));
            }
        }
    }
    None
}

fn start_stream() -> Result<(Stream, SyncSender<AudioCommand>), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| String::from("No default audio output device"))?;

    let supported = device
        .default_output_config()
        .map_err(|error| format!("Default output config failed: {error}"))?;

    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;

    let (tx, rx) = mpsc::sync_channel::<AudioCommand>(256);
    let state = AudioCallbackState {
        synth: PianoSynth::new(sample_rate),
        commands: rx,
        channels,
    };

    let err_fn = |error| eprintln!("Motif audio stream error: {error}");

    let stream = match sample_format {
        SampleFormat::F32 => {
            let mut state = state;
            device
                .build_output_stream(
                    &config,
                    move |data: &mut [f32], _| state.write_f32(data),
                    err_fn,
                    None,
                )
                .map_err(|error| format!("Build f32 stream failed: {error}"))?
        }
        SampleFormat::I16 => {
            let mut state = state;
            device
                .build_output_stream(
                    &config,
                    move |data: &mut [i16], _| {
                        state.process_commands();
                        if state.channels == 0 {
                            return;
                        }
                        for frame in data.chunks_mut(state.channels) {
                            let sample = state.synth.render_sample();
                            let quantized = (sample * i16::MAX as f32) as i16;
                            for channel in frame.iter_mut() {
                                *channel = quantized;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|error| format!("Build i16 stream failed: {error}"))?
        }
        SampleFormat::U16 => {
            let mut state = state;
            device
                .build_output_stream(
                    &config,
                    move |data: &mut [u16], _| {
                        state.process_commands();
                        if state.channels == 0 {
                            return;
                        }
                        for frame in data.chunks_mut(state.channels) {
                            let sample = state.synth.render_sample();
                            let quantized =
                                ((sample * 0.5 + 0.5) * u16::MAX as f32) as u16;
                            for channel in frame.iter_mut() {
                                *channel = quantized;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|error| format!("Build u16 stream failed: {error}"))?
        }
        other => {
            return Err(format!("Unsupported sample format: {other:?}"));
        }
    };

    Ok((stream, tx))
}
