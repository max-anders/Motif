//! cpal-backed audio engine with UI-thread transport + edge-detect sequencing.
//! Per-track voices: built-in piano and/or CLAP/VST3 plugins (shared slots for GUI).
//! Plugin load/activate runs on a worker thread so the UI stays responsive.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use truce_rack::core::transport::TransportInfo;

use crate::model::{db_to_linear, Project, TrackInstrument};

use super::metronome::MetronomeRunner;
use super::piano::PianoSynth;
use super::plugins::{
    load_and_activate, CatalogEntry, HostedPlugin, PluginCatalog, PluginEditorHost, PluginRef,
};
use super::DawEngine;

const LOADING_STATUS: &str = "Loading plugin...";

struct VoiceLoadResult {
    track_id: u64,
    instrument: TrackInstrument,
    result: Result<HostedPlugin, String>,
}

/// Result of a background insert-FX device load. A device's plugin identity
/// never changes in place once created (only bypass/order/removal do — see
/// `Project::add_device`/`remove_device`), so unlike instruments there is no
/// identity to re-check here: a completed load for `(track_id, device_id)` is
/// always still valid unless the track or device itself was removed meanwhile
/// (checked in `poll_device_loads`).
struct DeviceLoadResult {
    track_id: u64,
    device_id: u64,
    result: Result<HostedPlugin, String>,
}

enum TrackVoice {
    Piano(PianoSynth),
    Plugin(Arc<Mutex<HostedPlugin>>),
    /// Plugin selected but failed to load/activate — stays silent.
    Silent,
}

/// One insert-FX slot in a track's serial chain (RT-side). Cheap to clone
/// (an `Arc` bump) since the UI thread rebuilds and resends the whole chain
/// on any add/remove/reorder/bypass change.
#[derive(Clone)]
struct FxSlot {
    plugin: Arc<Mutex<HostedPlugin>>,
    bypassed: bool,
}

#[derive(Clone, Copy)]
struct ChannelParams {
    gain: f32,
    pan_l: f32,
    pan_r: f32,
}

impl Default for ChannelParams {
    fn default() -> Self {
        // Equal-power center at unity.
        let center = std::f32::consts::FRAC_1_SQRT_2;
        Self {
            gain: 1.0,
            pan_l: center,
            pan_r: center,
        }
    }
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
    SetChannel {
        track_id: u64,
        gain: f32,
        pan_l: f32,
        pan_r: f32,
    },
    /// Replace a track's insert-FX chain wholesale (order + bypass baked in).
    /// Sent whenever the UI-side chain signature changes; cheap since slots
    /// are `Arc` clones, not new plugin instances.
    SetFxChain {
        track_id: u64,
        chain: Vec<FxSlot>,
    },
    RemoveFxChain {
        track_id: u64,
    },
    SetMasterGain {
        gain: f32,
    },
    SetTransport {
        transport: TransportInfo,
    },
    SetMetronome {
        enabled: bool,
        beats_per_bar: f32,
        loop_end_beats: f32,
    },
}

struct AudioCallbackState {
    voices: HashMap<u64, TrackVoice>,
    channel_params: HashMap<u64, ChannelParams>,
    /// Per-track serial insert-FX chain, processed after the voice and
    /// before gain/pan. Absent key == empty chain (passthrough).
    fx_chains: HashMap<u64, Vec<FxSlot>>,
    master_gain: f32,
    commands: Receiver<AudioCommand>,
    channels: usize,
    transport: TransportInfo,
    metronome: MetronomeRunner,
    mix_l: Vec<f32>,
    mix_r: Vec<f32>,
    tmp_l: Vec<f32>,
    tmp_r: Vec<f32>,
    /// Shared with UI: `(track_id, peak_l, peak_r)`.
    track_meters: Arc<Mutex<Vec<(u64, f32, f32)>>>,
    master_meter: Arc<Mutex<(f32, f32)>>,
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
                    Some(TrackVoice::Plugin(plugin)) => {
                        if let Ok(mut guard) = plugin.try_lock() {
                            guard.push_note_on(pitch, velocity);
                        }
                    }
                    Some(TrackVoice::Silent) | None => {}
                },
                Ok(AudioCommand::NoteOff { track_id, pitch }) => {
                    match self.voices.get_mut(&track_id) {
                        Some(TrackVoice::Piano(synth)) => synth.note_off(pitch),
                        Some(TrackVoice::Plugin(plugin)) => {
                            if let Ok(mut guard) = plugin.try_lock() {
                                guard.push_note_off(pitch);
                            }
                        }
                        Some(TrackVoice::Silent) | None => {}
                    }
                }
                Ok(AudioCommand::AllNotesOff) => {
                    for voice in self.voices.values_mut() {
                        match voice {
                            TrackVoice::Piano(synth) => synth.all_notes_off(),
                            TrackVoice::Plugin(plugin) => {
                                if let Ok(mut guard) = plugin.try_lock() {
                                    guard.all_notes_off();
                                }
                            }
                            TrackVoice::Silent => {}
                        }
                    }
                }
                Ok(AudioCommand::SetVoice { track_id, voice }) => {
                    self.voices.insert(track_id, voice);
                }
                Ok(AudioCommand::RemoveVoice { track_id }) => {
                    self.voices.remove(&track_id);
                    self.channel_params.remove(&track_id);
                    self.fx_chains.remove(&track_id);
                }
                Ok(AudioCommand::SetChannel {
                    track_id,
                    gain,
                    pan_l,
                    pan_r,
                }) => {
                    self.channel_params.insert(
                        track_id,
                        ChannelParams {
                            gain,
                            pan_l,
                            pan_r,
                        },
                    );
                }
                Ok(AudioCommand::SetFxChain { track_id, chain }) => {
                    self.fx_chains.insert(track_id, chain);
                }
                Ok(AudioCommand::RemoveFxChain { track_id }) => {
                    self.fx_chains.remove(&track_id);
                }
                Ok(AudioCommand::SetMasterGain { gain }) => {
                    self.master_gain = gain;
                }
                Ok(AudioCommand::SetTransport { transport }) => {
                    self.apply_transport(transport);
                }
                Ok(AudioCommand::SetMetronome {
                    enabled,
                    beats_per_bar,
                    loop_end_beats,
                }) => {
                    self.metronome.set_enabled(enabled);
                    self.metronome.set_beats_per_bar(beats_per_bar);
                    self.metronome.set_loop_end_beats(loop_end_beats);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn apply_transport(&mut self, transport: TransportInfo) {
        let was_playing = self.metronome.playing();
        self.transport = transport;
        self.metronome.set_playing(transport.playing);
        if let Some(bpm) = transport.tempo_bpm {
            self.metronome.set_beats_per_second(bpm / 60.0);
        }
        if let Some((numerator, _)) = transport.time_signature {
            self.metronome.set_beats_per_bar(numerator as f32);
        }

        let should_sync = !was_playing && transport.playing
            || was_playing && !transport.playing
            || transport
                .song_position_beats
                .is_some_and(|beats| (beats - self.metronome.position_beats()).abs() > 0.05);

        if should_sync {
            if let Some(samples) = transport.song_position_samples {
                let bps = transport
                    .tempo_bpm
                    .map(|bpm| bpm / 60.0)
                    .unwrap_or(self.metronome.beats_per_second());
                self.metronome.sync_position_samples(samples, bps);
            } else if let Some(beats) = transport.song_position_beats {
                self.metronome.sync_position_beats(beats);
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
        if self.tmp_l.len() < frames {
            self.tmp_l.resize(frames, 0.0);
            self.tmp_r.resize(frames, 0.0);
        }
        self.mix_l[..frames].fill(0.0);
        self.mix_r[..frames].fill(0.0);

        let transport = Some(self.transport);
        let default_channel = ChannelParams::default();
        let mut meter_scratch: Vec<(u64, f32, f32)> = Vec::with_capacity(self.voices.len());

        let track_ids: Vec<u64> = self.voices.keys().copied().collect();
        for track_id in track_ids {
            let params = self
                .channel_params
                .get(&track_id)
                .copied()
                .unwrap_or(default_channel);
            self.tmp_l[..frames].fill(0.0);
            self.tmp_r[..frames].fill(0.0);

            match self.voices.get_mut(&track_id) {
                Some(TrackVoice::Piano(synth)) => {
                    for i in 0..frames {
                        let sample = synth.render_sample();
                        self.tmp_l[i] = sample;
                        self.tmp_r[i] = sample;
                    }
                }
                Some(TrackVoice::Plugin(plugin)) => {
                    if let Ok(mut guard) = plugin.try_lock() {
                        guard.process_block(
                            frames,
                            transport,
                            &mut self.tmp_l[..frames],
                            &mut self.tmp_r[..frames],
                        );
                    }
                }
                Some(TrackVoice::Silent) | None => {}
            }

            // Serial insert-FX chain, pre-fader: each non-bypassed device
            // replaces tmp_l/tmp_r in place before gain/pan is applied below.
            if let Some(chain) = self.fx_chains.get(&track_id) {
                for slot in chain {
                    if slot.bypassed {
                        continue;
                    }
                    if let Ok(mut guard) = slot.plugin.try_lock() {
                        guard.process_effect(
                            frames,
                            transport,
                            &mut self.tmp_l[..frames],
                            &mut self.tmp_r[..frames],
                        );
                    }
                }
            }

            let mut peak_l = 0.0_f32;
            let mut peak_r = 0.0_f32;
            let gl = params.gain * params.pan_l;
            let gr = params.gain * params.pan_r;
            for i in 0..frames {
                let l = self.tmp_l[i] * gl;
                let r = self.tmp_r[i] * gr;
                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
                self.mix_l[i] += l;
                self.mix_r[i] += r;
            }
            meter_scratch.push((track_id, peak_l, peak_r));
        }

        let master = self.master_gain;
        let mut master_peak_l = 0.0_f32;
        let mut master_peak_r = 0.0_f32;
        for i in 0..frames {
            if (master - 1.0).abs() > 1e-6 {
                self.mix_l[i] *= master;
                self.mix_r[i] *= master;
            }
            master_peak_l = master_peak_l.max(self.mix_l[i].abs());
            master_peak_r = master_peak_r.max(self.mix_r[i].abs());
        }

        // Cosmetic meters: skip on contention so the RT thread never blocks.
        if let Ok(mut meters) = self.track_meters.try_lock() {
            *meters = meter_scratch;
        }
        if let Ok(mut master_m) = self.master_meter.try_lock() {
            *master_m = (master_peak_l, master_peak_r);
        }

        self.metronome
            .process_block(frames, &mut self.mix_l[..frames], &mut self.mix_r[..frames]);
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
    /// Last `(gain_db, pan)` pushed per track (UI-side).
    synced_channels: HashMap<u64, (f32, f32)>,
    /// Last master gain_db pushed.
    synced_master_gain_db: Option<f32>,
    /// In-flight background plugin loads (track -> desired instrument).
    pending_loads: HashMap<u64, TrackInstrument>,
    load_tx: Option<SyncSender<VoiceLoadResult>>,
    load_rx: Receiver<VoiceLoadResult>,
    /// UI-side handles to hosted insert-FX devices, keyed by `(track_id, device_id)`.
    device_slots: HashMap<(u64, u64), Arc<Mutex<HostedPlugin>>>,
    /// Last chain signature `(device_id, bypassed)` in order sent to the audio
    /// thread per track — diffed against the live project each sync.
    device_chain_sig: HashMap<u64, Vec<(u64, bool)>>,
    /// Tracks whose chain needs resending even though its signature hasn't
    /// changed (a background device load just resolved this frame).
    device_chain_dirty: HashSet<u64>,
    /// In-flight background device loads.
    pending_device_loads: HashSet<(u64, u64)>,
    device_load_tx: Option<SyncSender<DeviceLoadResult>>,
    device_load_rx: Receiver<DeviceLoadResult>,
    /// Commands waiting for audio-thread channel space (never block UI / never drop MIDI).
    pending_cmds: VecDeque<AudioCommand>,
    /// Latest transport when the channel is full (coalesced; only one pending).
    pending_transport: Option<TransportInfo>,
    /// Latest metronome config when the channel is full (coalesced).
    pending_metronome: Option<(bool, f32, f32)>,
    command_tx: Option<SyncSender<AudioCommand>>,
    sample_rate: f32,
    /// UI-side handles to the same plugin instances the audio thread mixes.
    plugin_slots: HashMap<u64, Arc<Mutex<HostedPlugin>>>,
    /// Open native plugin editor windows (UI thread).
    editor_host: PluginEditorHost,
    track_meters: Arc<Mutex<Vec<(u64, f32, f32)>>>,
    master_meter: Arc<Mutex<(f32, f32)>>,
    /// Kept alive for the lifetime of the engine.
    _stream: Option<Stream>,
    audio_available: bool,
    init_error: Option<String>,
    metronome_enabled: bool,
}

impl AudioEngine {
    pub fn new(beats_per_second: f32) -> Self {
        let (load_tx, load_rx) = mpsc::sync_channel::<VoiceLoadResult>(8);
        let (device_load_tx, device_load_rx) = mpsc::sync_channel::<DeviceLoadResult>(8);
        let track_meters = Arc::new(Mutex::new(Vec::new()));
        let master_meter = Arc::new(Mutex::new((0.0_f32, 0.0_f32)));
        match start_stream(Arc::clone(&track_meters), Arc::clone(&master_meter)) {
            Ok((stream, tx, sample_rate)) => {
                let play_error = stream
                    .play()
                    .err()
                    .map(|e| format!("Audio stream play failed: {e}"));
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
                    synced_channels: HashMap::new(),
                    synced_master_gain_db: None,
                    pending_loads: HashMap::new(),
                    load_tx: Some(load_tx),
                    load_rx,
                    device_slots: HashMap::new(),
                    device_chain_sig: HashMap::new(),
                    device_chain_dirty: HashSet::new(),
                    pending_device_loads: HashSet::new(),
                    device_load_tx: Some(device_load_tx),
                    device_load_rx,
                    pending_cmds: VecDeque::new(),
                    pending_transport: None,
                    pending_metronome: None,
                    command_tx: Some(tx),
                    sample_rate,
                    plugin_slots: HashMap::new(),
                    editor_host: PluginEditorHost::default(),
                    track_meters,
                    master_meter,
                    _stream: Some(stream),
                    audio_available,
                    init_error: play_error,
                    metronome_enabled: true,
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
                synced_channels: HashMap::new(),
                synced_master_gain_db: None,
                pending_loads: HashMap::new(),
                load_tx: Some(load_tx),
                load_rx,
                device_slots: HashMap::new(),
                device_chain_sig: HashMap::new(),
                device_chain_dirty: HashSet::new(),
                pending_device_loads: HashSet::new(),
                device_load_tx: Some(device_load_tx),
                device_load_rx,
                pending_cmds: VecDeque::new(),
                pending_transport: None,
                pending_metronome: None,
                command_tx: None,
                sample_rate: 48_000.0,
                plugin_slots: HashMap::new(),
                editor_host: PluginEditorHost::default(),
                track_meters,
                master_meter,
                _stream: None,
                audio_available: false,
                init_error: Some(error),
                metronome_enabled: true,
            },
        }
    }

    pub fn audio_available(&self) -> bool {
        self.audio_available
    }

    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }

    fn send(&mut self, command: AudioCommand) {
        if self.command_tx.is_none() {
            return;
        }
        // Transport updates every UI frame while playing — keep only the latest.
        if let AudioCommand::SetTransport { transport } = command {
            self.pending_transport = Some(transport);
            self.flush_pending_cmds();
            return;
        }
        if let AudioCommand::SetMetronome {
            enabled,
            beats_per_bar,
            loop_end_beats,
        } = command
        {
            self.pending_metronome = Some((enabled, beats_per_bar, loop_end_beats));
            self.flush_pending_cmds();
            return;
        }
        self.pending_cmds.push_back(command);
        self.flush_pending_cmds();
    }

    fn push_metronome_config(&mut self) {
        self.send(AudioCommand::SetMetronome {
            enabled: self.metronome_enabled,
            beats_per_bar: self.beats_per_bar,
            loop_end_beats: self.loop_end_beats,
        });
    }

    fn flush_pending_cmds(&mut self) {
        let Some(tx) = &self.command_tx else {
            self.pending_cmds.clear();
            self.pending_transport = None;
            self.pending_metronome = None;
            return;
        };
        while let Some(command) = self.pending_cmds.pop_front() {
            match tx.try_send(command) {
                Ok(()) => {}
                Err(TrySendError::Full(command)) => {
                    self.pending_cmds.push_front(command);
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.pending_cmds.clear();
                    self.pending_transport = None;
                    self.pending_metronome = None;
                    return;
                }
            }
        }
        if let Some(transport) = self.pending_transport.take() {
            match tx.try_send(AudioCommand::SetTransport { transport }) {
                Ok(()) => {}
                Err(TrySendError::Full(AudioCommand::SetTransport { transport })) => {
                    self.pending_transport = Some(transport);
                }
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    self.pending_cmds.clear();
                    self.pending_transport = None;
                    self.pending_metronome = None;
                }
            }
        }
        if let Some((enabled, beats_per_bar, loop_end_beats)) = self.pending_metronome.take() {
            match tx.try_send(AudioCommand::SetMetronome {
                enabled,
                beats_per_bar,
                loop_end_beats,
            }) {
                Ok(()) => {}
                Err(TrySendError::Full(AudioCommand::SetMetronome {
                    enabled,
                    beats_per_bar,
                    loop_end_beats,
                })) => {
                    self.pending_metronome = Some((enabled, beats_per_bar, loop_end_beats));
                }
                Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    self.pending_cmds.clear();
                    self.pending_transport = None;
                    self.pending_metronome = None;
                }
            }
        }
    }

    fn silence_sequencer(&mut self) {
        self.active_seq_notes.clear();
        self.send(AudioCommand::AllNotesOff);
        self.flush_pending_cmds();
    }

    fn push_transport(&mut self) {
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
                (self.current_beats / self.beats_per_bar).floor() as f64
                    * self.beats_per_bar as f64,
            ),
            playing: self.playing,
            recording: false,
            loop_active: self.loop_end_beats > 0.0,
        };
        self.send(AudioCommand::SetTransport { transport });
        self.push_metronome_config();
    }

    fn spawn_plugin_load(
        &mut self,
        track_id: u64,
        instrument: TrackInstrument,
        entry: CatalogEntry,
        state: Option<Vec<u8>>,
    ) {
        let Some(load_tx) = self.load_tx.clone() else {
            return;
        };
        let sample_rate = self.sample_rate as f64;
        let name = match &instrument {
            TrackInstrument::Plugin { name, .. } => name.clone(),
            TrackInstrument::BuiltInPiano => String::from("Piano"),
        };
        self.pending_loads.insert(track_id, instrument.clone());
        // Mute the lane until the worker finishes (keeps old wrong plugin from playing).
        self.drop_plugin_slot(track_id);
        self.send(AudioCommand::SetVoice {
            track_id,
            voice: TrackVoice::Silent,
        });
        thread::spawn(move || {
            let result = match load_and_activate(&entry, sample_rate, state.as_deref()) {
                Ok(plugin) => Ok(plugin),
                Err(error) => Err(format!("{name}: {error}")),
            };
            let _ = load_tx.send(VoiceLoadResult {
                track_id,
                instrument,
                result,
            });
        });
    }

    fn drop_plugin_slot(&mut self, track_id: u64) {
        self.editor_host.close(PluginRef::instrument(track_id));
        self.plugin_slots.remove(&track_id);
    }

    fn drop_device_slot(&mut self, track_id: u64, device_id: u64) {
        self.editor_host.close(PluginRef::device(track_id, device_id));
        self.device_slots.remove(&(track_id, device_id));
    }

    fn spawn_device_load(
        &mut self,
        track_id: u64,
        device_id: u64,
        entry: CatalogEntry,
        state: Option<Vec<u8>>,
    ) {
        let Some(load_tx) = self.device_load_tx.clone() else {
            return;
        };
        let sample_rate = self.sample_rate as f64;
        self.pending_device_loads.insert((track_id, device_id));
        thread::spawn(move || {
            let result = load_and_activate(&entry, sample_rate, state.as_deref());
            let _ = load_tx.send(DeviceLoadResult {
                track_id,
                device_id,
                result,
            });
        });
    }

    /// Drains completed device loads. A track's chain is marked dirty
    /// (`device_chain_dirty`) whenever a slot's readiness changes, so
    /// `sync_devices` resends the chain even if identity/order/bypass
    /// (its signature) didn't change this frame.
    fn poll_device_loads(&mut self, project: &Project) -> Vec<(u64, u64, String)> {
        let mut errors = Vec::new();
        loop {
            match self.device_load_rx.try_recv() {
                Ok(DeviceLoadResult {
                    track_id,
                    device_id,
                    result,
                }) => {
                    self.pending_device_loads.remove(&(track_id, device_id));
                    let still_present = project
                        .track(track_id)
                        .is_some_and(|track| track.devices.iter().any(|d| d.id == device_id));
                    if !still_present {
                        // Stale load (track/device removed while loading) — drop it.
                        continue;
                    }
                    match result {
                        Ok(plugin) => {
                            self.device_slots
                                .insert((track_id, device_id), Arc::new(Mutex::new(plugin)));
                            errors.push((track_id, device_id, String::new()));
                        }
                        Err(error) => {
                            // No slot inserted: the device stays a no-op passthrough
                            // rather than muting the rest of the track's chain.
                            errors.push((track_id, device_id, error));
                        }
                    }
                    self.device_chain_dirty.insert(track_id);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        errors
    }

    fn poll_plugin_loads(&mut self, project: &Project) -> Vec<(u64, String)> {
        let mut updates = Vec::new();
        loop {
            match self.load_rx.try_recv() {
                Ok(VoiceLoadResult {
                    track_id,
                    instrument,
                    result,
                }) => {
                    let still_wanted = project
                        .tracks
                        .iter()
                        .any(|track| track.id == track_id && track.instrument == instrument);
                    let still_pending = self
                        .pending_loads
                        .get(&track_id)
                        .is_some_and(|pending| pending == &instrument);

                    if !still_wanted || !still_pending {
                        // Stale load (track removed / instrument changed) — drop voice.
                        continue;
                    }

                    self.pending_loads.remove(&track_id);
                    match result {
                        Ok(plugin) => {
                            let slot = Arc::new(Mutex::new(plugin));
                            self.drop_plugin_slot(track_id);
                            self.plugin_slots.insert(track_id, Arc::clone(&slot));
                            self.synced_instruments.insert(track_id, instrument);
                            updates.push((track_id, String::new()));
                            self.send(AudioCommand::SetVoice {
                                track_id,
                                voice: TrackVoice::Plugin(slot),
                            });
                        }
                        Err(error) => {
                            self.drop_plugin_slot(track_id);
                            self.synced_instruments.insert(track_id, instrument);
                            updates.push((track_id, error));
                            self.send(AudioCommand::SetVoice {
                                track_id,
                                voice: TrackVoice::Silent,
                            });
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        updates
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
        self.flush_pending_cmds();
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
        self.flush_pending_cmds();

        let mut errors = self.poll_plugin_loads(project);
        let live_ids: HashSet<u64> = project.tracks.iter().map(|t| t.id).collect();

        let stale: Vec<u64> = self
            .synced_instruments
            .keys()
            .copied()
            .filter(|id| !live_ids.contains(id))
            .collect();
        for track_id in stale {
            self.synced_instruments.remove(&track_id);
            self.pending_loads.remove(&track_id);
            self.drop_plugin_slot(track_id);
            self.send(AudioCommand::RemoveVoice { track_id });
        }

        let stale_pending: Vec<u64> = self
            .pending_loads
            .keys()
            .copied()
            .filter(|id| !live_ids.contains(id))
            .collect();
        for track_id in stale_pending {
            self.pending_loads.remove(&track_id);
        }

        for track in &project.tracks {
            if self
                .pending_loads
                .get(&track.id)
                .is_some_and(|pending| pending == &track.instrument)
            {
                errors.push((track.id, String::from(LOADING_STATUS)));
                continue;
            }

            let needs_sync = self
                .synced_instruments
                .get(&track.id)
                .map(|current| current != &track.instrument)
                .unwrap_or(true);
            if !needs_sync {
                continue;
            }

            match &track.instrument {
                TrackInstrument::BuiltInPiano => {
                    let voice = TrackVoice::Piano(PianoSynth::new(self.sample_rate));
                    self.pending_loads.remove(&track.id);
                    self.drop_plugin_slot(track.id);
                    self.synced_instruments
                        .insert(track.id, track.instrument.clone());
                    errors.push((track.id, String::new()));
                    self.send(AudioCommand::SetVoice {
                        track_id: track.id,
                        voice,
                    });
                }
                TrackInstrument::Plugin {
                    format,
                    unique_id,
                    name,
                } => {
                    let Some(entry) = catalog.find(*format, unique_id).cloned() else {
                        let error = format!("Plugin not in catalog: {name} ({})", format.label());
                        self.pending_loads.remove(&track.id);
                        self.drop_plugin_slot(track.id);
                        self.synced_instruments
                            .insert(track.id, track.instrument.clone());
                        errors.push((track.id, error));
                        self.send(AudioCommand::SetVoice {
                            track_id: track.id,
                            voice: TrackVoice::Silent,
                        });
                        continue;
                    };
                    self.spawn_plugin_load(
                        track.id,
                        track.instrument.clone(),
                        entry,
                        track.plugin_state.clone(),
                    );
                    errors.push((track.id, String::from(LOADING_STATUS)));
                }
            }
        }

        self.flush_pending_cmds();
        self.push_metronome_config();
        errors
    }

    fn capture_plugin_states(&mut self, project: &mut Project) {
        self.flush_pending_cmds();
        for track in &mut project.tracks {
            let format = match &track.instrument {
                TrackInstrument::Plugin { format, .. } => *format,
                TrackInstrument::BuiltInPiano => {
                    track.plugin_state = None;
                    continue;
                }
            };
            let Some(slot) = self.plugin_slots.get(&track.id) else {
                // Keep last saved blob while the plugin is still loading / missing.
                continue;
            };
            let Ok(guard) = slot.lock() else {
                continue;
            };
            match guard.save_state_blob(format) {
                Ok(blob) => track.plugin_state = Some(blob),
                Err(_) => {
                    // Leave previous project blob; avoid wiping a good save on a flaky getState.
                }
            }
        }
    }

    fn sync_devices(&mut self, project: &Project, catalog: &PluginCatalog) -> Vec<(u64, u64, String)> {
        let mut errors = self.poll_device_loads(project);
        let live_track_ids: HashSet<u64> = project.tracks.iter().map(|t| t.id).collect();

        let stale_tracks: Vec<u64> = self
            .device_chain_sig
            .keys()
            .copied()
            .filter(|id| !live_track_ids.contains(id))
            .collect();
        for track_id in stale_tracks {
            self.device_chain_sig.remove(&track_id);
            self.device_chain_dirty.remove(&track_id);
            let device_ids: Vec<u64> = self
                .device_slots
                .keys()
                .filter(|(tid, _)| *tid == track_id)
                .map(|(_, did)| *did)
                .collect();
            for device_id in device_ids {
                self.drop_device_slot(track_id, device_id);
            }
            self.pending_device_loads
                .retain(|(tid, _)| *tid != track_id);
            self.send(AudioCommand::RemoveFxChain { track_id });
        }

        for track in &project.tracks {
            let live_device_ids: HashSet<u64> = track.devices.iter().map(|d| d.id).collect();

            let stale_devices: Vec<u64> = self
                .device_slots
                .keys()
                .filter(|(tid, did)| *tid == track.id && !live_device_ids.contains(did))
                .map(|(_, did)| *did)
                .collect();
            for device_id in stale_devices {
                self.drop_device_slot(track.id, device_id);
            }
            self.pending_device_loads
                .retain(|(tid, did)| *tid != track.id || live_device_ids.contains(did));

            for device in &track.devices {
                if device.unique_id.is_empty() {
                    // Legacy placeholder device (pre-Phase-2): no identity, never hosted.
                    continue;
                }
                let key = (track.id, device.id);
                if self.device_slots.contains_key(&key)
                    || self.pending_device_loads.contains(&key)
                {
                    continue;
                }
                let Some(entry) = catalog.find(device.format, &device.unique_id).cloned() else {
                    errors.push((
                        track.id,
                        device.id,
                        format!(
                            "Plugin not in catalog: {} ({})",
                            device.name,
                            device.format.label()
                        ),
                    ));
                    continue;
                };
                self.spawn_device_load(track.id, device.id, entry, device.plugin_state.clone());
                errors.push((track.id, device.id, String::from(LOADING_STATUS)));
            }

            let signature: Vec<(u64, bool)> =
                track.devices.iter().map(|d| (d.id, d.bypassed)).collect();
            let was_dirty = self.device_chain_dirty.remove(&track.id);
            let needs_resend =
                was_dirty || self.device_chain_sig.get(&track.id) != Some(&signature);
            if !needs_resend {
                continue;
            }
            self.device_chain_sig.insert(track.id, signature);

            let chain: Vec<FxSlot> = track
                .devices
                .iter()
                .filter_map(|device| {
                    let plugin = self.device_slots.get(&(track.id, device.id))?.clone();
                    Some(FxSlot {
                        plugin,
                        bypassed: device.bypassed,
                    })
                })
                .collect();
            if chain.is_empty() {
                self.send(AudioCommand::RemoveFxChain { track_id: track.id });
            } else {
                self.send(AudioCommand::SetFxChain {
                    track_id: track.id,
                    chain,
                });
            }
        }

        self.flush_pending_cmds();
        errors
    }

    fn capture_device_states(&mut self, project: &mut Project) {
        self.flush_pending_cmds();
        for track in &mut project.tracks {
            for device in &mut track.devices {
                let Some(slot) = self.device_slots.get(&(track.id, device.id)) else {
                    // Keep last saved blob while loading / missing.
                    continue;
                };
                let Ok(guard) = slot.lock() else {
                    continue;
                };
                match guard.save_state_blob(device.format) {
                    Ok(blob) => device.plugin_state = Some(blob),
                    Err(_) => {
                        // Leave previous project blob; avoid wiping a good save on a flaky getState.
                    }
                }
            }
        }
    }

    fn invalidate_instruments(&mut self) {
        self.editor_host.close_all();
        self.plugin_slots.clear();
        self.synced_instruments.clear();
        self.pending_loads.clear();
        self.device_slots.clear();
        self.device_chain_sig.clear();
        self.device_chain_dirty.clear();
        self.pending_device_loads.clear();
    }

    fn plugin_slot_ready(&self, target: PluginRef) -> bool {
        match target.device_id {
            None => self.plugin_slots.contains_key(&target.track_id),
            Some(device_id) => self.device_slots.contains_key(&(target.track_id, device_id)),
        }
    }

    fn open_plugin_editor(
        &mut self,
        target: PluginRef,
        title: &str,
        host_x11: Option<super::plugins::HostX11>,
        forward_transport: bool,
    ) -> Result<(), String> {
        let slot = match target.device_id {
            None => self.plugin_slots.get(&target.track_id).cloned(),
            Some(device_id) => self
                .device_slots
                .get(&(target.track_id, device_id))
                .cloned(),
        };
        let Some(slot) = slot else {
            return Err(String::from("Plugin not loaded yet"));
        };
        self.editor_host
            .open(target, slot, title, host_x11, forward_transport)
    }

    fn close_plugin_editor(&mut self, target: PluginRef) {
        self.editor_host.close(target);
    }

    fn plugin_editor_is_open(&self, target: PluginRef) -> bool {
        self.editor_host.is_open(target)
    }

    fn open_plugin_editors(&self) -> Vec<(PluginRef, String)> {
        self.editor_host.open_editors()
    }

    fn set_plugin_editor_transport(&mut self, target: PluginRef, forward: bool) {
        self.editor_host.set_forward_transport(target, forward);
    }

    fn poll_plugin_editors(&mut self) -> super::EditorPoll {
        self.editor_host.poll()
    }

    fn schedule_project(&mut self, project: &Project) {
        self.flush_pending_cmds();
        if !self.playing {
            return;
        }

        let prev = self.previous_beats;
        let curr = self.current_beats;
        let mut should_be_active: HashSet<u64> = HashSet::new();

        for track in &project.tracks {
            if !project.track_audible(track) {
                continue;
            }
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

    fn set_metronome_enabled(&mut self, enabled: bool) {
        self.metronome_enabled = enabled;
        self.push_metronome_config();
    }

    fn metronome_enabled(&self) -> bool {
        self.metronome_enabled
    }

    fn sync_channels(&mut self, project: &Project) {
        self.flush_pending_cmds();
        let live_ids: HashSet<u64> = project.tracks.iter().map(|t| t.id).collect();
        let stale: Vec<u64> = self
            .synced_channels
            .keys()
            .copied()
            .filter(|id| !live_ids.contains(id))
            .collect();
        for track_id in stale {
            self.synced_channels.remove(&track_id);
        }

        if self.synced_master_gain_db != Some(project.master_gain_db) {
            self.synced_master_gain_db = Some(project.master_gain_db);
            self.send(AudioCommand::SetMasterGain {
                gain: db_to_linear(project.master_gain_db),
            });
        }

        for track in &project.tracks {
            let key = (track.gain_db, track.pan);
            let needs = self
                .synced_channels
                .get(&track.id)
                .map(|current| current != &key)
                .unwrap_or(true);
            if !needs {
                continue;
            }
            let (pan_l, pan_r) = track.pan_gains();
            self.synced_channels.insert(track.id, key);
            self.send(AudioCommand::SetChannel {
                track_id: track.id,
                gain: track.gain_linear(),
                pan_l,
                pan_r,
            });
        }
        self.flush_pending_cmds();
    }

    fn meter_levels(&self) -> Vec<(u64, f32, f32)> {
        self.track_meters
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn master_meter(&self) -> (f32, f32) {
        self.master_meter
            .lock()
            .map(|guard| *guard)
            .unwrap_or((0.0, 0.0))
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

fn start_stream(
    track_meters: Arc<Mutex<Vec<(u64, f32, f32)>>>,
    master_meter: Arc<Mutex<(f32, f32)>>,
) -> Result<(Stream, SyncSender<AudioCommand>, f32), String> {
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
        channel_params: HashMap::new(),
        fx_chains: HashMap::new(),
        master_gain: 1.0,
        commands: rx,
        channels,
        transport: TransportInfo::default(),
        metronome: MetronomeRunner::new(sample_rate),
        mix_l: vec![0.0; 4096],
        mix_r: vec![0.0; 4096],
        tmp_l: vec![0.0; 4096],
        tmp_r: vec![0.0; 4096],
        track_meters,
        master_meter,
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
