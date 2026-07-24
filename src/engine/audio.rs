//! cpal-backed audio engine with UI-thread transport + edge-detect sequencing.
//! Per-track voices: built-in piano and/or headless CLAP/VST3 plugins.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use truce_rack::core::transport::TransportInfo;

use crate::model::{Project, TrackInstrument};

use super::piano::PianoSynth;
use super::plugins::{load_and_activate, HostedPlugin, PluginCatalog};
use super::DawEngine;

enum TrackVoice {
    Piano(PianoSynth),
    Plugin(HostedPlugin),
    /// Plugin selected but failed to load/activate — stays silent.
    Silent,
}

enum AudioCommand {
    NoteOn {
        track_id: u64,
        pitch: u8,
        velocity: u8,
    },
    NoteOff {
        track_id: u64,
        pitch: u8,
    },
    AllNotesOff,
    SetVoice {
        track_id: u64,
        voice: TrackVoice,
    },
    RemoveVoice {
        track_id: u64,
    },
    SetTransport {
        transport: TransportInfo,
    },
}

struct AudioCallbackState {
    voices: HashMap<u64, TrackVoice>,
    commands: Receiver<AudioCommand>,
    channels: usize,
    transport: TransportInfo,
        mix_l: Vec<f32>,
        mix_r: Vec<f32>,
    }

impl AudioCallbackState {
    fn process_commands(&mut self) {
        loop {
            match self.commands.try_recv() {
                Ok(AudioCommand::NoteOn {
                    track_id,
                    pitch,
                    velocity,
                }) => match self.voices.get_mut(&track_id) {
                    Some(TrackVoice::Piano(synth)) => synth.note_on(pitch, velocity),
                    Some(TrackVoice::Plugin(plugin)) => plugin.push_note_on(pitch, velocity),
                    Some(TrackVoice::Silent) | None => {}
                },
                Ok(AudioCommand::NoteOff { track_id, pitch }) => match self.voices.get_mut(&track_id)
                {
                    Some(TrackVoice::Piano(synth)) => synth.note_off(pitch),
                    Some(TrackVoice::Plugin(plugin)) => plugin.push_note_off(pitch),
                    Some(TrackVoice::Silent) | None => {}
                },
                Ok(AudioCommand::AllNotesOff) => {
                    for voice in self.voices.values_mut() {
                        match voice {
                            TrackVoice::Piano(synth) => synth.all_notes_off(),
                            TrackVoice::Plugin(plugin) => plugin.all_notes_off(),
                            TrackVoice::Silent => {}
                        }
                    }
                }
                Ok(AudioCommand::SetVoice { track_id, voice }) => {
                    self.voices.insert(track_id, voice);
                }
                Ok(AudioCommand::RemoveVoice { track_id }) => {
                    self.voices.remove(&track_id);
                }
                Ok(AudioCommand::SetTransport { transport }) => {
                    self.transport = transport;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn render_stereo(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        if self.mix_l.len() < frames {
            self.mix_l.resize(frames, 0.0);
            self.mix_r.resize(frames, 0.0);
        }
        let mix_l = &mut self.mix_l[..frames];
        let mix_r = &mut self.mix_r[..frames];
        mix_l.fill(0.0);
        mix_r.fill(0.0);

        let transport = Some(self.transport);

        for voice in self.voices.values_mut() {
            match voice {
                TrackVoice::Piano(synth) => {
                    for i in 0..frames {
                        let sample = synth.render_sample();
                        mix_l[i] += sample;
                        mix_r[i] += sample;
                    }
                }
                TrackVoice::Plugin(plugin) => {
                    plugin.process_block(frames, transport, mix_l, mix_r);
                }
                TrackVoice::Silent => {}
            }
        }
    }

    fn write_f32(&mut self, data: &mut [f32]) {
        self.process_commands();
        if self.channels == 0 {
            return;
        }
        let frames = data.len() / self.channels;
        self.render_stereo(frames);
        for (frame_index, frame) in data.chunks_mut(self.channels).enumerate() {
            let l = self.mix_l.get(frame_index).copied().unwrap_or(0.0);
            let r = self.mix_r.get(frame_index).copied().unwrap_or(l);
            if frame.len() >= 2 {
                frame[0] = l;
                frame[1] = r;
                for channel in frame.iter_mut().skip(2) {
                    *channel = 0.0;
                }
            } else if let Some(ch0) = frame.first_mut() {
                *ch0 = 0.5 * (l + r);
            }
        }
    }

    fn write_i16(&mut self, data: &mut [i16]) {
        self.process_commands();
        if self.channels == 0 {
            return;
        }
        let frames = data.len() / self.channels;
        self.render_stereo(frames);
        for (frame_index, frame) in data.chunks_mut(self.channels).enumerate() {
            let l = self.mix_l.get(frame_index).copied().unwrap_or(0.0);
            let r = self.mix_r.get(frame_index).copied().unwrap_or(l);
            if frame.len() >= 2 {
                frame[0] = (l * i16::MAX as f32) as i16;
                frame[1] = (r * i16::MAX as f32) as i16;
                for channel in frame.iter_mut().skip(2) {
                    *channel = 0;
                }
            } else if let Some(ch0) = frame.first_mut() {
                *ch0 = (0.5 * (l + r) * i16::MAX as f32) as i16;
            }
        }
    }

    fn write_u16(&mut self, data: &mut [u16]) {
        self.process_commands();
        if self.channels == 0 {
            return;
        }
        let frames = data.len() / self.channels;
        self.render_stereo(frames);
        for (frame_index, frame) in data.chunks_mut(self.channels).enumerate() {
            let l = self.mix_l.get(frame_index).copied().unwrap_or(0.0);
            let r = self.mix_r.get(frame_index).copied().unwrap_or(l);
            let ql = ((l * 0.5 + 0.5) * u16::MAX as f32) as u16;
            let qr = ((r * 0.5 + 0.5) * u16::MAX as f32) as u16;
            if frame.len() >= 2 {
                frame[0] = ql;
                frame[1] = qr;
                for channel in frame.iter_mut().skip(2) {
                    *channel = u16::MAX / 2;
                }
            } else if let Some(ch0) = frame.first_mut() {
                let m = 0.5 * (l + r);
                *ch0 = ((m * 0.5 + 0.5) * u16::MAX as f32) as u16;
            }
        }
    }
}

/// Real-time engine: UI advances the playhead; audio thread mixes track voices.
pub struct AudioEngine {
    playing: bool,
    current_beats: f32,
    previous_beats: f32,
    beats_per_second: f32,
    beats_per_bar: f32,
    loop_end_beats: f32,
    /// Note ids currently sounding from the sequencer (not keyboard audition).
    active_seq_notes: HashSet<u64>,
    /// Last instrument identity synced per track (UI-side).
    synced_instruments: HashMap<u64, TrackInstrument>,
    command_tx: Option<SyncSender<AudioCommand>>,
    sample_rate: f32,
    /// Kept alive for the lifetime of the engine.
    _stream: Option<Stream>,
    audio_available: bool,
    init_error: Option<String>,
}

impl AudioEngine {
    pub fn new(beats_per_second: f32) -> Self {
        match start_stream() {
            Ok((stream, tx, sample_rate)) => {
                let play_error = stream.play().err().map(|e| format!("Audio stream play failed: {e}"));
                let audio_available = play_error.is_none();
                Self {
                    playing: false,
                    current_beats: 0.0,
                    previous_beats: 0.0,
                    beats_per_second,
                    beats_per_bar: 4.0,
                    loop_end_beats: 16.0,
                    active_seq_notes: HashSet::new(),
                    synced_instruments: HashMap::new(),
                    command_tx: Some(tx),
                    sample_rate,
                    _stream: Some(stream),
                    audio_available,
                    init_error: play_error,
                }
            }
            Err(error) => Self {
                playing: false,
                current_beats: 0.0,
                previous_beats: 0.0,
                beats_per_second,
                beats_per_bar: 4.0,
                loop_end_beats: 16.0,
                active_seq_notes: HashSet::new(),
                synced_instruments: HashMap::new(),
                command_tx: None,
                sample_rate: 48_000.0,
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
            // Voice swaps may block briefly on the UI thread; note events use try_send.
            let is_voice = matches!(
                command,
                AudioCommand::SetVoice { .. } | AudioCommand::RemoveVoice { .. }
            );
            if is_voice {
                let _ = tx.send(command);
            } else {
                let _ = tx.try_send(command);
            }
        }
    }

    fn silence_sequencer(&mut self) {
        self.active_seq_notes.clear();
        self.send(AudioCommand::AllNotesOff);
    }

    fn push_transport(&self) {
        let tempo_bpm = (self.beats_per_second * 60.0) as f64;
        let beats_per_bar = self.beats_per_bar.max(1.0) as u32;
        let transport = TransportInfo {
            tempo_bpm: Some(tempo_bpm),
            time_signature: Some((beats_per_bar, 4)),
            song_position_beats: Some(self.current_beats as f64),
            song_position_samples: Some(
                (self.current_beats as f64 / self.beats_per_second.max(0.0001) as f64
                    * self.sample_rate as f64) as i64,
            ),
            bar_start_beats: Some(
                (self.current_beats / self.beats_per_bar).floor() as f64 * self.beats_per_bar as f64,
            ),
            playing: self.playing,
            recording: false,
            loop_active: self.loop_end_beats > 0.0,
        };
        self.send(AudioCommand::SetTransport { transport });
    }

    fn build_voice(
        &self,
        instrument: &TrackInstrument,
        catalog: &PluginCatalog,
    ) -> Result<TrackVoice, String> {
        match instrument {
            TrackInstrument::BuiltInPiano => Ok(TrackVoice::Piano(PianoSynth::new(self.sample_rate))),
            TrackInstrument::Plugin {
                format,
                unique_id,
                name,
            } => {
                let Some(entry) = catalog.find(*format, unique_id) else {
                    return Err(format!(
                        "Plugin not in catalog: {name} ({})",
                        format.label()
                    ));
                };
                match load_and_activate(entry, self.sample_rate as f64) {
                    Ok(plugin) => Ok(TrackVoice::Plugin(plugin)),
                    Err(error) => Err(format!("{name}: {error}")),
                }
            }
        }
    }
}

impl DawEngine for AudioEngine {
    fn play(&mut self) {
        self.previous_beats = self.current_beats;
        self.playing = true;
        self.push_transport();
    }

    fn pause(&mut self) {
        self.playing = false;
        self.silence_sequencer();
        self.push_transport();
    }

    fn stop(&mut self) {
        self.playing = false;
        self.silence_sequencer();
        self.push_transport();
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
        self.push_transport();
    }

    fn current_beats(&self) -> f32 {
        self.current_beats
    }

    fn set_beats_per_second(&mut self, beats_per_second: f32) {
        self.beats_per_second = beats_per_second;
        self.push_transport();
    }

    fn advance(&mut self, delta_seconds: f32, loop_end_beats: f32) {
        self.loop_end_beats = loop_end_beats;
        if !self.playing || loop_end_beats <= 0.0 {
            return;
        }

        self.previous_beats = self.current_beats;
        self.current_beats += delta_seconds * self.beats_per_second;

        if self.current_beats >= loop_end_beats {
            self.current_beats %= loop_end_beats;
            self.previous_beats = self.current_beats - delta_seconds * self.beats_per_second;
            self.active_seq_notes.clear();
            self.send(AudioCommand::AllNotesOff);
            self.previous_beats = -0.0001;
        }
        self.push_transport();
    }

    fn note_on(&mut self, track_id: u64, pitch: u8, velocity: u8) {
        self.send(AudioCommand::NoteOn {
            track_id,
            pitch,
            velocity,
        });
    }

    fn note_off(&mut self, track_id: u64, pitch: u8) {
        self.send(AudioCommand::NoteOff { track_id, pitch });
    }

    fn all_notes_off(&mut self) {
        self.silence_sequencer();
    }

    fn sync_instruments(
        &mut self,
        project: &Project,
        catalog: &PluginCatalog,
    ) -> Vec<(u64, String)> {
        self.beats_per_bar = project.beats_per_bar;
        self.loop_end_beats = project.loop_end_beats;

        let mut errors = Vec::new();
        let live_ids: HashSet<u64> = project.tracks.iter().map(|t| t.id).collect();

        let stale: Vec<u64> = self
            .synced_instruments
            .keys()
            .copied()
            .filter(|id| !live_ids.contains(id))
            .collect();
        for track_id in stale {
            self.synced_instruments.remove(&track_id);
            self.send(AudioCommand::RemoveVoice { track_id });
        }

        for track in &project.tracks {
            let needs_sync = self
                .synced_instruments
                .get(&track.id)
                .map(|current| current != &track.instrument)
                .unwrap_or(true);
            if !needs_sync {
                continue;
            }

            match self.build_voice(&track.instrument, catalog) {
                Ok(voice) => {
                    self.synced_instruments
                        .insert(track.id, track.instrument.clone());
                    // Empty error string means success for this track (app clears banner).
                    errors.push((track.id, String::new()));
                    self.send(AudioCommand::SetVoice {
                        track_id: track.id,
                        voice,
                    });
                }
                Err(error) => {
                    errors.push((track.id, error));
                    self.synced_instruments
                        .insert(track.id, track.instrument.clone());
                    self.send(AudioCommand::SetVoice {
                        track_id: track.id,
                        voice: TrackVoice::Silent,
                    });
                }
            }
        }

        errors
    }

    fn invalidate_instruments(&mut self) {
        self.synced_instruments.clear();
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

                    let active_now = curr >= abs_start && curr < abs_end;
                    if active_now {
                        should_be_active.insert(note.id);
                    }

                    let crossed_start = prev < abs_start && curr >= abs_start && curr < abs_end;
                    let was_active = self.active_seq_notes.contains(&note.id);

                    if crossed_start || (active_now && !was_active) {
                        self.send(AudioCommand::NoteOn {
                            track_id: track.id,
                            pitch: note.pitch,
                            velocity: note.velocity,
                        });
                    }
                }
            }
        }

        let ended: Vec<u64> = self
            .active_seq_notes
            .difference(&should_be_active)
            .copied()
            .collect();

        for note_id in ended {
            let Some((track_id, pitch, _)) = find_note(project, note_id) else {
                continue;
            };
            let pitch_still_held = should_be_active.iter().any(|id| {
                find_note(project, *id).is_some_and(|(tid, p, _)| tid == track_id && p == pitch)
            });
            if !pitch_still_held {
                self.send(AudioCommand::NoteOff { track_id, pitch });
            }
        }

        self.active_seq_notes = should_be_active;
        self.previous_beats = curr;
    }
}

fn find_note(project: &Project, note_id: u64) -> Option<(u64, u8, u8)> {
    for track in &project.tracks {
        for clip in &track.clips {
            if let Some(note) = clip.note(note_id) {
                return Some((track.id, note.pitch, note.velocity));
            }
        }
    }
    None
}

fn start_stream() -> Result<(Stream, SyncSender<AudioCommand>, f32), String> {
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

    let (tx, rx) = mpsc::sync_channel::<AudioCommand>(64);
    let state = AudioCallbackState {
        voices: HashMap::new(),
        commands: rx,
        channels,
        transport: TransportInfo::default(),
        mix_l: vec![0.0; 4096],
        mix_r: vec![0.0; 4096],
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
                    move |data: &mut [i16], _| state.write_i16(data),
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
                    move |data: &mut [u16], _| state.write_u16(data),
                    err_fn,
                    None,
                )
                .map_err(|error| format!("Build u16 stream failed: {error}"))?
        }
        other => {
            return Err(format!("Unsupported sample format: {other:?}"));
        }
    };

    Ok((stream, tx, sample_rate))
}
