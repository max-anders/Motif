//! cpal-backed audio engine with UI-thread transport + edge-detect sequencing.
//! Per-track voices: built-in piano and/or CLAP/VST3 plugins (shared slots for GUI).
//! Plugin load/activate is format-dependent: JUCE-based CLAP plugins bind their
//! message thread to the constructing thread, so every [main-thread] call
//! (init/activate/gui.*) must share it or gui.create deadlocks on
//! juce::MessageManager::Lock -- CLAP is therefore built on the UI (calling)
//! thread. VST3 has no such affinity and is built on a background worker so its
//! internal JUCE timers do not contend with our UI event loop.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use truce_rack::core::transport::TransportInfo;

use crate::model::{
    db_to_linear, AutomationPoint, AutomationTarget, CurveKind, LfoRate, LfoShape, PluginFormat,
    Project, TrackInstrument,
};

use super::metronome::MetronomeRunner;
use super::piano::PianoSynth;
use super::plugins::{
    load_and_activate, CatalogEntry, HostedPlugin, PluginCatalog, PluginEditorHost, PluginParamInfo,
    PluginRef,
};
use super::DecodedAudio;
use super::{DawEngine, EnginePerformance, LoopPlayback, TrackPerformance, TrackVoiceKind};

/// RT -> UI telemetry written only with Relaxed atomics (never locks).
struct AudioPerfShared {
    /// Latest callback load in tenths of a percent (123 = 12.3%).
    cpu_load_tenths: AtomicU32,
    buffer_frames: AtomicU32,
    xruns: AtomicU64,
    lock_skips: AtomicU64,
}

impl AudioPerfShared {
    fn new() -> Self {
        Self {
            cpu_load_tenths: AtomicU32::new(0),
            buffer_frames: AtomicU32::new(0),
            xruns: AtomicU64::new(0),
            lock_skips: AtomicU64::new(0),
        }
    }

    fn snapshot(&self, sample_rate_hz: u32) -> EnginePerformance {
        let buffer_frames = self.buffer_frames.load(Ordering::Relaxed);
        let cpu_percent = self.cpu_load_tenths.load(Ordering::Relaxed) as f32 / 10.0;
        let latency_ms = if sample_rate_hz > 0 && buffer_frames > 0 {
            (buffer_frames as f32 / sample_rate_hz as f32) * 1000.0
        } else {
            0.0
        };
        EnginePerformance {
            cpu_percent,
            buffer_frames,
            sample_rate_hz,
            latency_ms,
            xruns: self.xruns.load(Ordering::Relaxed),
            lock_skips: self.lock_skips.load(Ordering::Relaxed),
        }
    }
}

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

/// A plugin-bearing value retired from the audio callback. Hosted plugins must
/// never be *dropped* on the real-time audio thread: destroying a CLAP/VST3
/// plugin runs `terminate()` + native COM release, which is RT-unsafe and (for
/// VST3) must not happen off the main thread — doing so segfaults. Instead the
/// callback ships retired voices/chains here and the UI thread drops them.
///
/// The variant payloads are never read — they exist solely so their `Drop`
/// runs on the UI thread when the enum is dropped in `drain_retired`.
#[allow(dead_code)]
enum RetiredResource {
    Voice(TrackVoice),
    FxChain(Vec<FxSlot>),
}

/// One insert-FX slot in a track's serial chain (RT-side). Cheap to clone
/// (an `Arc` bump) since the UI thread rebuilds and resends the whole chain
/// on any add/remove/reorder/bypass change.
#[derive(Clone)]
struct FxSlot {
    device_id: u64,
    plugin: Arc<Mutex<HostedPlugin>>,
    bypassed: bool,
}

#[derive(Clone)]
struct SamplePlayback {
    clip_id: u64,
    start_beats: f32,
    length_beats: f32,
    gain: f32,
    buffer: Arc<DecodedAudio>,
}

#[derive(Clone, Copy)]
struct ChannelParams {
    gain: f32,
    pan_l: f32,
    pan_r: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RtAutomationTarget {
    Instrument { param_id: u32 },
    Device { device_id: u64, param_id: u32 },
}

#[derive(Debug, Clone, PartialEq)]
struct RtAutomationLane {
    target: RtAutomationTarget,
    points: Vec<AutomationPoint>,
    min: f64,
    max: f64,
    step_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RtLfoRate {
    SyncBeats { beats: f32 },
    Hz { hz: f32 },
}

#[derive(Debug, Clone, PartialEq)]
struct RtModulator {
    id: u64,
    target: RtAutomationTarget,
    shape: LfoShape,
    rate: RtLfoRate,
    depth: f32,
    phase: f32,
    bipolar: bool,
    center: f32,
    mseg_points: Vec<AutomationPoint>,
    min: f64,
    max: f64,
    step_count: u32,
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
    SetAutomation {
        track_id: u64,
        lanes: Vec<RtAutomationLane>,
    },
    SetModulators {
        track_id: u64,
        modulators: Vec<RtModulator>,
    },
    SetTrackSamples {
        track_id: u64,
        clips: Vec<SamplePlayback>,
    },
    ClearTrackSamples {
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
        loop_start_beats: f32,
        loop_end_beats: f32,
    },
}

struct AudioCallbackState {
    voices: HashMap<u64, TrackVoice>,
    channel_params: HashMap<u64, ChannelParams>,
    /// Per-track serial insert-FX chain, processed after the voice and
    /// before gain/pan. Absent key == empty chain (passthrough).
    fx_chains: HashMap<u64, Vec<FxSlot>>,
    /// Per-track audio clip list (already decoded/resampled).
    sample_clips: HashMap<u64, Vec<SamplePlayback>>,
    /// Per-track block-rate automation lanes for plugin params.
    automation: HashMap<u64, Vec<RtAutomationLane>>,
    /// Per-track LFO / MSEG modulators for plugin params.
    modulators: HashMap<u64, Vec<RtModulator>>,
    /// Free-running Hz LFO phase (cycles 0..1) keyed by `(track_id, modulator_id)`.
    lfo_phases: HashMap<(u64, u64), f64>,
    master_gain: f32,
    commands: Receiver<AudioCommand>,
    channels: usize,
    sample_rate: f32,
    transport: TransportInfo,
    metronome: MetronomeRunner,
    mix_l: Vec<f32>,
    mix_r: Vec<f32>,
    tmp_l: Vec<f32>,
    tmp_r: Vec<f32>,
    /// Shared with UI: `(track_id, peak_l, peak_r)`.
    track_meters: Arc<Mutex<Vec<(u64, f32, f32)>>>,
    master_meter: Arc<Mutex<(f32, f32)>>,
    /// Shared with UI: per-track DSP timing for the latest callback.
    track_perf: Arc<Mutex<Vec<TrackPerformance>>>,
    perf: Arc<AudioPerfShared>,
    /// Retired plugin voices/chains handed back to the UI thread to drop
    /// (never destroy a hosted plugin on the RT thread — see [`RetiredResource`]).
    retire_tx: Sender<RetiredResource>,
}

fn normalize_mseg_points(points: &mut Vec<AutomationPoint>, legacy_length: f32) {
    let length = legacy_length.max(0.0625);
    if length == 1.0 && !points.iter().any(|p| p.beat > 1.0 + f32::EPSILON) {
        return;
    }
    for point in &mut *points {
        point.beat = (point.beat / length).clamp(0.0, 1.0);
    }
    points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
}

impl AudioCallbackState {
    /// Ship a retired voice to the UI thread for dropping. Piano/Silent voices
    /// carry no native plugin and are safe to drop here.
    fn retire_voice(&self, voice: TrackVoice) {
        if matches!(voice, TrackVoice::Plugin(_)) {
            let _ = self.retire_tx.send(RetiredResource::Voice(voice));
        }
    }

    /// Ship a retired insert-FX chain to the UI thread for dropping.
    fn retire_chain(&self, chain: Vec<FxSlot>) {
        if !chain.is_empty() {
            let _ = self.retire_tx.send(RetiredResource::FxChain(chain));
        }
    }

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
                        } else {
                            self.perf.lock_skips.fetch_add(1, Ordering::Relaxed);
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
                            } else {
                                self.perf.lock_skips.fetch_add(1, Ordering::Relaxed);
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
                                } else {
                                    self.perf.lock_skips.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            TrackVoice::Silent => {}
                        }
                    }
                }
                Ok(AudioCommand::SetVoice { track_id, voice }) => {
                    if let Some(old) = self.voices.insert(track_id, voice) {
                        self.retire_voice(old);
                    }
                }
                Ok(AudioCommand::RemoveVoice { track_id }) => {
                    if let Some(old) = self.voices.remove(&track_id) {
                        self.retire_voice(old);
                    }
                    self.channel_params.remove(&track_id);
                    if let Some(chain) = self.fx_chains.remove(&track_id) {
                        self.retire_chain(chain);
                    }
                    self.sample_clips.remove(&track_id);
                    self.automation.remove(&track_id);
                    self.modulators.remove(&track_id);
                    self.lfo_phases.retain(|&(tid, _), _| tid != track_id);
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
                    if let Some(old) = self.fx_chains.insert(track_id, chain) {
                        self.retire_chain(old);
                    }
                }
                Ok(AudioCommand::RemoveFxChain { track_id }) => {
                    if let Some(old) = self.fx_chains.remove(&track_id) {
                        self.retire_chain(old);
                    }
                    self.automation.remove(&track_id);
                    self.modulators.remove(&track_id);
                    self.lfo_phases.retain(|&(tid, _), _| tid != track_id);
                }
                Ok(AudioCommand::SetAutomation { track_id, lanes }) => {
                    if lanes.is_empty() {
                        self.automation.remove(&track_id);
                    } else {
                        self.automation.insert(track_id, lanes);
                    }
                }
                Ok(AudioCommand::SetModulators {
                    track_id,
                    modulators,
                }) => {
                    if modulators.is_empty() {
                        self.modulators.remove(&track_id);
                        self.lfo_phases.retain(|&(tid, _), _| tid != track_id);
                    } else {
                        let live_ids: HashSet<u64> =
                            modulators.iter().map(|modulator| modulator.id).collect();
                        self.lfo_phases
                            .retain(|&(tid, mid), _| tid != track_id || live_ids.contains(&mid));
                        self.modulators.insert(track_id, modulators);
                    }
                }
                Ok(AudioCommand::SetTrackSamples { track_id, clips }) => {
                    self.sample_clips.insert(track_id, clips);
                }
                Ok(AudioCommand::ClearTrackSamples { track_id }) => {
                    self.sample_clips.remove(&track_id);
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
                    loop_start_beats,
                    loop_end_beats,
                }) => {
                    self.metronome.set_enabled(enabled);
                    self.metronome.set_beats_per_bar(beats_per_bar);
                    self.metronome.set_loop_start_beats(loop_start_beats);
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

    fn evaluate_lane_value(points: &[AutomationPoint], beat: f32) -> Option<f64> {
        if points.is_empty() {
            return None;
        }
        if points.len() == 1 || beat <= points[0].beat {
            return Some(points[0].value as f64);
        }
        if beat >= points[points.len() - 1].beat {
            return Some(points[points.len() - 1].value as f64);
        }

        for pair in points.windows(2) {
            let left = &pair[0];
            let right = &pair[1];
            if beat < left.beat || beat > right.beat {
                continue;
            }
            if matches!(left.curve, CurveKind::Hold) || (right.beat - left.beat).abs() <= f32::EPSILON {
                return Some(left.value as f64);
            }
            let t = ((beat - left.beat) / (right.beat - left.beat)).clamp(0.0, 1.0) as f64;
            let left_v = left.value as f64;
            let right_v = right.value as f64;
            return Some(left_v + (right_v - left_v) * t);
        }

        Some(points[points.len() - 1].value as f64)
    }

    fn normalized_to_native(normalized: f64, min: f64, max: f64, step_count: u32) -> f64 {
        let clamped = normalized.clamp(0.0, 1.0);
        let span = (max - min).max(0.0);
        let mut native = min + clamped * span;
        if step_count > 1 {
            let max_step = (step_count - 1) as f64;
            let step = (clamped * max_step).round().clamp(0.0, max_step);
            native = if max_step > 0.0 {
                min + (step / max_step) * span
            } else {
                min
            };
        } else if step_count == 1 {
            native = min;
        }
        native
    }

    fn lfo_wave(shape: LfoShape, phase01: f64) -> f64 {
        let p = phase01.rem_euclid(1.0);
        match shape {
            LfoShape::Sine => (p * std::f64::consts::TAU).sin(),
            LfoShape::Triangle => {
                if p < 0.25 {
                    p * 4.0
                } else if p < 0.75 {
                    2.0 - p * 4.0
                } else {
                    p * 4.0 - 4.0
                }
            }
            LfoShape::Saw => 2.0 * p - 1.0,
            LfoShape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::Custom => 0.0,
        }
    }

    fn evaluate_mseg_at_phase(points: &[AutomationPoint], phase01: f64) -> f64 {
        let local = phase01.rem_euclid(1.0) as f32;
        Self::evaluate_lane_value(points, local).unwrap_or(0.0)
    }

    fn modulator_cycle_phase(
        modulator: &RtModulator,
        beat: f32,
        free_phase: f64,
    ) -> f64 {
        match modulator.rate {
            RtLfoRate::SyncBeats { beats } => {
                let period = beats.max(0.0625) as f64;
                ((beat as f64) / period + modulator.phase as f64).rem_euclid(1.0)
            }
            RtLfoRate::Hz { .. } => (free_phase + modulator.phase as f64).rem_euclid(1.0),
        }
    }

    fn modulator_signal(
        modulator: &RtModulator,
        beat: f32,
        free_phase: f64,
    ) -> f64 {
        let phase = Self::modulator_cycle_phase(modulator, beat, free_phase);
        match modulator.shape {
            LfoShape::Custom => {
                let unipolar = Self::evaluate_mseg_at_phase(&modulator.mseg_points, phase);
                if modulator.bipolar {
                    unipolar * 2.0 - 1.0
                } else {
                    unipolar
                }
            }
            shape => {
                let bipolar = Self::lfo_wave(shape, phase);
                if modulator.bipolar {
                    bipolar
                } else {
                    (bipolar + 1.0) * 0.5
                }
            }
        }
    }

    fn advance_lfo_phases(&mut self, track_id: u64, modulators: &[RtModulator], frames: usize) {
        let dt = frames as f64 / self.sample_rate.max(1.0) as f64;
        for modulator in modulators {
            let RtLfoRate::Hz { hz } = modulator.rate else {
                continue;
            };
            let key = (track_id, modulator.id);
            let phase = self.lfo_phases.entry(key).or_insert(0.0);
            *phase = (*phase + hz.max(0.0) as f64 * dt).rem_euclid(1.0);
        }
    }

    fn push_params_for_instrument(
        track_id: u64,
        lanes: &[RtAutomationLane],
        modulators: &[RtModulator],
        lfo_phases: &HashMap<(u64, u64), f64>,
        beat: f32,
        apply_automation: bool,
        apply_modulation: bool,
        plugin: &mut HostedPlugin,
    ) {
        let mut param_ids: HashSet<u32> = HashSet::new();
        if apply_automation {
            for lane in lanes {
                if let RtAutomationTarget::Instrument { param_id } = lane.target {
                    param_ids.insert(param_id);
                }
            }
        }
        if apply_modulation {
            for modulator in modulators {
                if let RtAutomationTarget::Instrument { param_id } = modulator.target {
                    param_ids.insert(param_id);
                }
            }
        }

        for param_id in param_ids {
            let lane = lanes.iter().find(|lane| {
                matches!(
                    lane.target,
                    RtAutomationTarget::Instrument { param_id: id } if id == param_id
                )
            });
            let mods: Vec<&RtModulator> = modulators
                .iter()
                .filter(|modulator| {
                    matches!(
                        modulator.target,
                        RtAutomationTarget::Instrument { param_id: id } if id == param_id
                    )
                })
                .collect();

            let (mut normalized, min, max, step_count) = if apply_automation {
                if let Some(lane) = lane {
                    let Some(value) = Self::evaluate_lane_value(&lane.points, beat) else {
                        continue;
                    };
                    (value, lane.min, lane.max, lane.step_count)
                } else if let Some(first) = mods.first() {
                    (
                        first.center as f64,
                        first.min,
                        first.max,
                        first.step_count,
                    )
                } else {
                    continue;
                }
            } else if let Some(first) = mods.first() {
                (
                    first.center as f64,
                    first.min,
                    first.max,
                    first.step_count,
                )
            } else {
                continue;
            };

            if apply_modulation {
                for modulator in mods {
                    let free_phase = lfo_phases
                        .get(&(track_id, modulator.id))
                        .copied()
                        .unwrap_or(0.0);
                    let signal = Self::modulator_signal(modulator, beat, free_phase);
                    normalized += (modulator.depth as f64) * signal;
                }
            }

            let native = Self::normalized_to_native(normalized, min, max, step_count);
            plugin.push_param(param_id, native, 0);
        }
    }

    fn push_params_for_device(
        track_id: u64,
        lanes: &[RtAutomationLane],
        modulators: &[RtModulator],
        lfo_phases: &HashMap<(u64, u64), f64>,
        beat: f32,
        device_id: u64,
        apply_automation: bool,
        apply_modulation: bool,
        plugin: &mut HostedPlugin,
    ) {
        let mut param_ids: HashSet<u32> = HashSet::new();
        if apply_automation {
            for lane in lanes {
                if let RtAutomationTarget::Device {
                    device_id: lane_device_id,
                    param_id,
                } = lane.target
                {
                    if lane_device_id == device_id {
                        param_ids.insert(param_id);
                    }
                }
            }
        }
        if apply_modulation {
            for modulator in modulators {
                if let RtAutomationTarget::Device {
                    device_id: mod_device_id,
                    param_id,
                } = modulator.target
                {
                    if mod_device_id == device_id {
                        param_ids.insert(param_id);
                    }
                }
            }
        }

        for param_id in param_ids {
            let lane = lanes.iter().find(|lane| {
                matches!(
                    lane.target,
                    RtAutomationTarget::Device {
                        device_id: did,
                        param_id: pid,
                    } if did == device_id && pid == param_id
                )
            });
            let mods: Vec<&RtModulator> = modulators
                .iter()
                .filter(|modulator| {
                    matches!(
                        modulator.target,
                        RtAutomationTarget::Device {
                            device_id: did,
                            param_id: pid,
                        } if did == device_id && pid == param_id
                    )
                })
                .collect();

            let (mut normalized, min, max, step_count) = if apply_automation {
                if let Some(lane) = lane {
                    let Some(value) = Self::evaluate_lane_value(&lane.points, beat) else {
                        continue;
                    };
                    (value, lane.min, lane.max, lane.step_count)
                } else if let Some(first) = mods.first() {
                    (
                        first.center as f64,
                        first.min,
                        first.max,
                        first.step_count,
                    )
                } else {
                    continue;
                }
            } else if let Some(first) = mods.first() {
                (
                    first.center as f64,
                    first.min,
                    first.max,
                    first.step_count,
                )
            } else {
                continue;
            };

            if apply_modulation {
                for modulator in mods {
                    let free_phase = lfo_phases
                        .get(&(track_id, modulator.id))
                        .copied()
                        .unwrap_or(0.0);
                    let signal = Self::modulator_signal(modulator, beat, free_phase);
                    normalized += (modulator.depth as f64) * signal;
                }
            }

            let native = Self::normalized_to_native(normalized, min, max, step_count);
            plugin.push_param(param_id, native, 0);
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
        let mut track_perf_scratch: Vec<TrackPerformance> =
            Vec::with_capacity(self.voices.len());
        let block_start_beat = self
            .transport
            .song_position_beats
            .map(|beats| beats as f32)
            .unwrap_or(0.0);
        let apply_automation = self.transport.playing;
        let apply_modulation = self.transport.playing;

        let track_ids: Vec<u64> = self.voices.keys().copied().collect();
        for track_id in track_ids {
            let params = self
                .channel_params
                .get(&track_id)
                .copied()
                .unwrap_or(default_channel);
            self.tmp_l[..frames].fill(0.0);
            self.tmp_r[..frames].fill(0.0);
            let lanes = self
                .automation
                .get(&track_id)
                .cloned()
                .unwrap_or_default();
            let modulators = self
                .modulators
                .get(&track_id)
                .cloned()
                .unwrap_or_default();
            if apply_modulation {
                self.advance_lfo_phases(track_id, &modulators, frames);
            }
            // Snapshot phases before borrowing voices mutably.
            let mut track_phases: HashMap<(u64, u64), f64> = HashMap::new();
            for modulator in &modulators {
                if let Some(phase) = self.lfo_phases.get(&(track_id, modulator.id)) {
                    track_phases.insert((track_id, modulator.id), *phase);
                }
            }

            let mut voice_kind = TrackVoiceKind::None;
            let mut active_voices = 0_u32;
            let mut lock_skips = 0_u32;

            let voice_started = Instant::now();
            match self.voices.get_mut(&track_id) {
                Some(TrackVoice::Piano(synth)) => {
                    voice_kind = TrackVoiceKind::Piano;
                    for i in 0..frames {
                        let sample = synth.render_sample();
                        self.tmp_l[i] = sample;
                        self.tmp_r[i] = sample;
                    }
                    active_voices = synth.active_voice_count();
                }
                Some(TrackVoice::Plugin(plugin)) => {
                    voice_kind = TrackVoiceKind::Plugin;
                    if let Ok(mut guard) = plugin.try_lock() {
                        if apply_automation || apply_modulation {
                            Self::push_params_for_instrument(
                                track_id,
                                &lanes,
                                &modulators,
                                &track_phases,
                                block_start_beat,
                                apply_automation,
                                apply_modulation,
                                &mut guard,
                            );
                        }
                        guard.process_block(
                            frames,
                            transport,
                            &mut self.tmp_l[..frames],
                            &mut self.tmp_r[..frames],
                        );
                    } else {
                        lock_skips += 1;
                        self.perf.lock_skips.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Some(TrackVoice::Silent) => {
                    voice_kind = TrackVoiceKind::Silent;
                }
                None => {}
            }
            let voice_ms = voice_started.elapsed().as_secs_f64() as f32 * 1000.0;

            let samples_started = Instant::now();
            if self.transport.playing {
                if let (Some(song_pos_samples), Some(tempo_bpm)) =
                    (self.transport.song_position_samples, self.transport.tempo_bpm)
                {
                    let bps = (tempo_bpm as f32 / 60.0).max(0.0001);
                    if let Some(clips) = self.sample_clips.get(&track_id) {
                        for clip in clips {
                            let sample_rate = clip.buffer.device_sample_rate as f32;
                            let clip_start_samples =
                                ((clip.start_beats / bps) * sample_rate).round() as i64;
                            let clip_length_samples =
                                ((clip.length_beats / bps) * sample_rate).round() as i64;
                            if clip_length_samples <= 0 {
                                continue;
                            }
                            for i in 0..frames {
                                let song_sample = song_pos_samples + i as i64;
                                let clip_sample = song_sample - clip_start_samples;
                                if clip_sample < 0
                                    || clip_sample >= clip_length_samples
                                    || clip_sample as usize >= clip.buffer.frames
                                {
                                    continue;
                                }
                                let idx = clip_sample as usize;
                                let l = clip.buffer.left.get(idx).copied().unwrap_or(0.0) * clip.gain;
                                let r = clip.buffer.right.get(idx).copied().unwrap_or(0.0) * clip.gain;
                                self.tmp_l[i] += l;
                                self.tmp_r[i] += r;
                            }
                        }
                    }
                }
            }
            let samples_ms = samples_started.elapsed().as_secs_f64() as f32 * 1000.0;

            // Serial insert-FX chain, pre-fader: each non-bypassed device
            // replaces tmp_l/tmp_r in place before gain/pan is applied below.
            let fx_started = Instant::now();
            if let Some(chain) = self.fx_chains.get(&track_id) {
                for slot in chain {
                    if slot.bypassed {
                        continue;
                    }
                    if let Ok(mut guard) = slot.plugin.try_lock() {
                        if apply_automation || apply_modulation {
                            Self::push_params_for_device(
                                track_id,
                                &lanes,
                                &modulators,
                                &track_phases,
                                block_start_beat,
                                slot.device_id,
                                apply_automation,
                                apply_modulation,
                                &mut guard,
                            );
                        }
                        guard.process_effect(
                            frames,
                            transport,
                            &mut self.tmp_l[..frames],
                            &mut self.tmp_r[..frames],
                        );
                    } else {
                        lock_skips += 1;
                        self.perf.lock_skips.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            let fx_ms = fx_started.elapsed().as_secs_f64() as f32 * 1000.0;

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
            track_perf_scratch.push(TrackPerformance {
                track_id,
                voice_kind,
                voice_ms,
                fx_ms,
                samples_ms,
                total_ms: voice_ms + fx_ms + samples_ms,
                lock_skips,
                active_voices,
            });
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

        // Cosmetic meters / perf: skip on contention so the RT thread never blocks.
        if let Ok(mut meters) = self.track_meters.try_lock() {
            *meters = meter_scratch;
        }
        if let Ok(mut master_m) = self.master_meter.try_lock() {
            *master_m = (master_peak_l, master_peak_r);
        }
        if let Ok(mut tracks) = self.track_perf.try_lock() {
            *tracks = track_perf_scratch;
        }

        self.metronome
            .process_block(frames, &mut self.mix_l[..frames], &mut self.mix_r[..frames]);
    }

    /// Record callback load vs buffer budget. RT-safe (atomics only).
    fn record_callback_perf(&self, frames: usize, started: Instant) {
        if frames == 0 || self.sample_rate <= 0.0 {
            return;
        }
        let elapsed = started.elapsed().as_secs_f64();
        let budget = frames as f64 / self.sample_rate as f64;
        if budget <= 0.0 {
            return;
        }
        let load_pct = (elapsed / budget) * 100.0;
        let tenths = (load_pct * 10.0).round().clamp(0.0, 999_990.0) as u32;
        self.perf
            .cpu_load_tenths
            .store(tenths, Ordering::Relaxed);
        self.perf
            .buffer_frames
            .store(frames as u32, Ordering::Relaxed);
        if elapsed > budget {
            self.perf.xruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn write_f32(&mut self, data: &mut [f32]) {
        let started = Instant::now();
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
        self.record_callback_perf(frames, started);
    }

    fn write_i16(&mut self, data: &mut [i16]) {
        let started = Instant::now();
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
        self.record_callback_perf(frames, started);
    }

    fn write_u16(&mut self, data: &mut [u16]) {
        let started = Instant::now();
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
        self.record_callback_perf(frames, started);
    }
}

/// Real-time engine: UI advances the playhead; audio thread mixes track voices.
pub struct AudioEngine {
    playing: bool,
    current_beats: f32,
    previous_beats: f32,
    beats_per_second: f32,
    beats_per_bar: f32,
    loop_enabled: bool,
    loop_start_beats: f32,
    loop_end_beats: f32,
    /// End of arranged content; playback stops here when not looping.
    content_end_beats: f32,
    /// Note ids currently sounding from the sequencer (not keyboard audition).
    active_seq_notes: HashSet<u64>,
    /// Last instrument identity synced per track (UI-side).
    synced_instruments: HashMap<u64, TrackInstrument>,
    /// Last `(gain_db, pan)` pushed per track (UI-side).
    synced_channels: HashMap<u64, (f32, f32)>,
    /// Last master gain_db pushed.
    synced_master_gain_db: Option<f32>,
    /// Last automation payload sent to the audio thread per track.
    synced_automation: HashMap<u64, Vec<RtAutomationLane>>,
    /// Last modulator payload sent to the audio thread per track.
    synced_modulators: HashMap<u64, Vec<RtModulator>>,
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
    /// Latest metronome config when the channel is full (coalesced):
    /// `(enabled, beats_per_bar, loop_start_beats, loop_end_beats)`.
    pending_metronome: Option<(bool, f32, f32, f32)>,
    command_tx: Option<SyncSender<AudioCommand>>,
    sample_rate: f32,
    /// UI-side handles to the same plugin instances the audio thread mixes.
    plugin_slots: HashMap<u64, Arc<Mutex<HostedPlugin>>>,
    /// Parameter metadata enumerated once at load time. Cached so per-frame
    /// automation sync never has to take the RT-shared plugin mutex (a blocking
    /// `lock()` there starves the audio callback's `try_lock()` -> dropouts).
    plugin_params: HashMap<u64, Arc<Vec<PluginParamInfo>>>,
    /// Same, for insert-FX devices keyed by `(track_id, device_id)`.
    device_params: HashMap<(u64, u64), Arc<Vec<PluginParamInfo>>>,
    /// Open native plugin editor windows (UI thread).
    editor_host: PluginEditorHost,
    track_meters: Arc<Mutex<Vec<(u64, f32, f32)>>>,
    master_meter: Arc<Mutex<(f32, f32)>>,
    track_perf: Arc<Mutex<Vec<TrackPerformance>>>,
    perf: Arc<AudioPerfShared>,
    /// Plugin voices/chains retired by the audio callback, drained and dropped
    /// on the UI thread (never destroy a hosted plugin on the RT thread).
    retire_rx: Receiver<RetiredResource>,
    /// Kept alive for the lifetime of the engine.
    _stream: Option<Stream>,
    audio_available: bool,
    init_error: Option<String>,
    device_name: Option<String>,
    metronome_enabled: bool,
}

impl AudioEngine {
    pub fn new(beats_per_second: f32) -> Self {
        let (load_tx, load_rx) = mpsc::sync_channel::<VoiceLoadResult>(8);
        let (device_load_tx, device_load_rx) = mpsc::sync_channel::<DeviceLoadResult>(8);
        let (retire_tx, retire_rx) = mpsc::channel::<RetiredResource>();
        let track_meters = Arc::new(Mutex::new(Vec::new()));
        let master_meter = Arc::new(Mutex::new((0.0_f32, 0.0_f32)));
        let track_perf = Arc::new(Mutex::new(Vec::new()));
        let perf = Arc::new(AudioPerfShared::new());
        match start_stream(
            Arc::clone(&track_meters),
            Arc::clone(&master_meter),
            Arc::clone(&track_perf),
            Arc::clone(&perf),
            retire_tx,
        ) {
            Ok((stream, tx, sample_rate, device_name)) => {
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
                    loop_enabled: false,
                    loop_start_beats: 0.0,
                    loop_end_beats: 16.0,
                    content_end_beats: 0.0,
                    active_seq_notes: HashSet::new(),
                    synced_instruments: HashMap::new(),
                    synced_channels: HashMap::new(),
                    synced_master_gain_db: None,
                    synced_automation: HashMap::new(),
                    synced_modulators: HashMap::new(),
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
                    plugin_params: HashMap::new(),
                    device_params: HashMap::new(),
                    editor_host: PluginEditorHost::default(),
                    track_meters,
                    master_meter,
                    track_perf,
                    perf,
                    retire_rx,
                    _stream: Some(stream),
                    audio_available,
                    init_error: play_error,
                    device_name: Some(device_name),
                    metronome_enabled: true,
                }
            }
            Err(error) => Self {
                playing: false,
                current_beats: 0.0,
                previous_beats: 0.0,
                beats_per_second,
                beats_per_bar: 4.0,
                loop_enabled: false,
                loop_start_beats: 0.0,
                loop_end_beats: 16.0,
                content_end_beats: 0.0,
                active_seq_notes: HashSet::new(),
                synced_instruments: HashMap::new(),
                synced_channels: HashMap::new(),
                synced_master_gain_db: None,
                synced_automation: HashMap::new(),
                synced_modulators: HashMap::new(),
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
                plugin_params: HashMap::new(),
                device_params: HashMap::new(),
                editor_host: PluginEditorHost::default(),
                track_meters,
                master_meter,
                track_perf,
                perf,
                retire_rx,
                _stream: None,
                audio_available: false,
                init_error: Some(error),
                device_name: None,
                metronome_enabled: true,
            },
        }
    }

    /// Drop plugin voices/chains retired by the audio callback here on the UI
    /// thread. Destroying a hosted CLAP/VST3 plugin on the RT thread segfaults.
    fn drain_retired(&mut self) {
        while let Ok(resource) = self.retire_rx.try_recv() {
            drop(resource);
        }
    }

    pub fn audio_available(&self) -> bool {
        self.audio_available
    }

    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate.round().max(1.0) as u32
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
            loop_start_beats,
            loop_end_beats,
        } = command
        {
            self.pending_metronome =
                Some((enabled, beats_per_bar, loop_start_beats, loop_end_beats));
            self.flush_pending_cmds();
            return;
        }
        self.pending_cmds.push_back(command);
        self.flush_pending_cmds();
    }

    fn push_metronome_config(&mut self) {
        // Only wrap the click grid when a valid loop is active; otherwise the
        // metronome counts straight through (0,0 disables the wrap).
        let (loop_start, loop_end) =
            if self.loop_enabled && self.loop_end_beats > self.loop_start_beats {
                (self.loop_start_beats, self.loop_end_beats)
            } else {
                (0.0, 0.0)
            };
        self.send(AudioCommand::SetMetronome {
            enabled: self.metronome_enabled,
            beats_per_bar: self.beats_per_bar,
            loop_start_beats: loop_start,
            loop_end_beats: loop_end,
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
        if let Some((enabled, beats_per_bar, loop_start_beats, loop_end_beats)) =
            self.pending_metronome.take()
        {
            match tx.try_send(AudioCommand::SetMetronome {
                enabled,
                beats_per_bar,
                loop_start_beats,
                loop_end_beats,
            }) {
                Ok(()) => {}
                Err(TrySendError::Full(AudioCommand::SetMetronome {
                    enabled,
                    beats_per_bar,
                    loop_start_beats,
                    loop_end_beats,
                })) => {
                    self.pending_metronome =
                        Some((enabled, beats_per_bar, loop_start_beats, loop_end_beats));
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
            loop_active: self.loop_enabled && self.loop_end_beats > self.loop_start_beats,
        };
        self.send(AudioCommand::SetTransport { transport });
        self.push_metronome_config();
    }

    fn load_plugin_now(
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
        // Mute the lane until the load finishes (keeps old wrong plugin from playing).
        self.drop_plugin_slot(track_id);
        self.send(AudioCommand::SetVoice {
            track_id,
            voice: TrackVoice::Silent,
        });

        let format = entry.format;
        // Route the result through load_tx either way, so poll_plugin_loads owns
        // the stale-check and slot bookkeeping regardless of which thread built it.
        let build = move || {
            let result = match load_and_activate(&entry, sample_rate, state.as_deref()) {
                Ok(plugin) => Ok(plugin),
                Err(error) => Err(format!("{name}: {error}")),
            };
            let _ = load_tx.send(VoiceLoadResult {
                track_id,
                instrument,
                result,
            });
        };

        // CLAP must be constructed on the UI (calling) thread: JUCE-based CLAP
        // plugins (e.g. Vital via clap-juce-extensions) bind their "message
        // thread" to the constructing thread, and per the CLAP spec init/activate
        // and every gui.* call are [main-thread]. Building on a worker while
        // opening the editor on the UI thread deadlocked gui.create inside
        // juce::MessageManager::Lock::tryAcquire. VST3 has no such affinity, and
        // building it on the UI thread makes its JUCE timer thread contend with
        // our event loop (visible UI lag), so keep VST3 on a background worker.
        match format {
            PluginFormat::Clap => build(),
            PluginFormat::Vst3 => {
                thread::spawn(build);
            }
        }
    }

    fn drop_plugin_slot(&mut self, track_id: u64) {
        self.editor_host.remove(PluginRef::instrument(track_id));
        self.plugin_slots.remove(&track_id);
        self.plugin_params.remove(&track_id);
    }

    fn drop_device_slot(&mut self, track_id: u64, device_id: u64) {
        self.editor_host
            .remove(PluginRef::device(track_id, device_id));
        self.device_slots.remove(&(track_id, device_id));
        self.device_params.remove(&(track_id, device_id));
    }

    fn load_device_now(
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

        let format = entry.format;
        let build = move || {
            let result = load_and_activate(&entry, sample_rate, state.as_deref());
            let _ = load_tx.send(DeviceLoadResult {
                track_id,
                device_id,
                result,
            });
        };
        // See load_plugin_now: CLAP is built on the UI thread (JUCE message-thread
        // affinity), VST3 on a worker to avoid UI-thread timer contention/lag.
        match format {
            PluginFormat::Clap => build(),
            PluginFormat::Vst3 => {
                thread::spawn(build);
            }
        }
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
                            // Enumerate params once, before sharing with the RT thread.
                            let params = Arc::new(plugin.parameters());
                            self.device_slots
                                .insert((track_id, device_id), Arc::new(Mutex::new(plugin)));
                            self.device_params.insert((track_id, device_id), params);
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
                            // Enumerate params once, before sharing with the RT thread.
                            let params = Arc::new(plugin.parameters());
                            let slot = Arc::new(Mutex::new(plugin));
                            self.drop_plugin_slot(track_id);
                            self.plugin_slots.insert(track_id, Arc::clone(&slot));
                            self.plugin_params.insert(track_id, params);
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

    /// Cached parameter metadata for a slot (enumerated once at load time).
    /// Never touches the RT-shared plugin mutex, so it is safe to call from
    /// per-frame UI paths.
    fn cached_params(&self, track_id: u64, device_id: Option<u64>) -> Option<&Arc<Vec<PluginParamInfo>>> {
        match device_id {
            None => self.plugin_params.get(&track_id),
            Some(device_id) => self.device_params.get(&(track_id, device_id)),
        }
    }

    pub fn plugin_parameters(&self, track_id: u64, device_id: Option<u64>) -> Vec<PluginParamInfo> {
        self.cached_params(track_id, device_id)
            .map(|params| params.as_ref().clone())
            .unwrap_or_default()
    }

    fn param_info_for_target(
        &self,
        track_id: u64,
        target: &AutomationTarget,
    ) -> Option<PluginParamInfo> {
        let (device_id, param_id) = match target {
            AutomationTarget::Instrument { param_id } => (None, *param_id),
            AutomationTarget::Device {
                device_id,
                param_id,
            } => (Some(*device_id), *param_id),
        };
        self.cached_params(track_id, device_id)?
            .iter()
            .find(|param| param.id == param_id)
            .cloned()
    }

    pub fn sync_automation(&mut self, project: &Project) {
        self.flush_pending_cmds();

        let live_ids: HashSet<u64> = project.tracks.iter().map(|track| track.id).collect();
        let stale: Vec<u64> = self
            .synced_automation
            .keys()
            .copied()
            .filter(|track_id| !live_ids.contains(track_id))
            .collect();
        for track_id in stale {
            self.synced_automation.remove(&track_id);
            self.send(AudioCommand::SetAutomation {
                track_id,
                lanes: Vec::new(),
            });
        }

        for track in &project.tracks {
            let mut lanes = Vec::new();
            for lane in &track.automation_lanes {
                if !lane.enabled {
                    continue;
                }
                let target = match &lane.target {
                    AutomationTarget::Instrument { param_id } => RtAutomationTarget::Instrument {
                        param_id: *param_id,
                    },
                    AutomationTarget::Device {
                        device_id,
                        param_id,
                    } => RtAutomationTarget::Device {
                        device_id: *device_id,
                        param_id: *param_id,
                    },
                };
                let Some(param_info) = self.param_info_for_target(track.id, &lane.target) else {
                    continue;
                };
                if !param_info.automatable {
                    continue;
                }
                let mut points = lane.points.clone();
                points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
                if points.is_empty() {
                    continue;
                }
                lanes.push(RtAutomationLane {
                    target,
                    points,
                    min: param_info.min,
                    max: param_info.max,
                    step_count: param_info.step_count,
                });
            }

            let changed = self
                .synced_automation
                .get(&track.id)
                .map(|prev| prev != &lanes)
                .unwrap_or(true);
            if !changed {
                continue;
            }

            if lanes.is_empty() {
                self.synced_automation.remove(&track.id);
            } else {
                self.synced_automation.insert(track.id, lanes.clone());
            }
            self.send(AudioCommand::SetAutomation {
                track_id: track.id,
                lanes,
            });
        }

        self.flush_pending_cmds();
    }

    pub fn sync_modulators(&mut self, project: &Project) {
        self.flush_pending_cmds();

        let live_ids: HashSet<u64> = project.tracks.iter().map(|track| track.id).collect();
        let stale: Vec<u64> = self
            .synced_modulators
            .keys()
            .copied()
            .filter(|track_id| !live_ids.contains(track_id))
            .collect();
        for track_id in stale {
            self.synced_modulators.remove(&track_id);
            self.send(AudioCommand::SetModulators {
                track_id,
                modulators: Vec::new(),
            });
        }

        for track in &project.tracks {
            let mut modulators = Vec::new();
            for modulator in &track.modulators {
                if !modulator.enabled {
                    continue;
                }
                let target = match &modulator.target {
                    AutomationTarget::Instrument { param_id } => RtAutomationTarget::Instrument {
                        param_id: *param_id,
                    },
                    AutomationTarget::Device {
                        device_id,
                        param_id,
                    } => RtAutomationTarget::Device {
                        device_id: *device_id,
                        param_id: *param_id,
                    },
                };
                let Some(param_info) = self.param_info_for_target(track.id, &modulator.target)
                else {
                    continue;
                };
                if !param_info.automatable {
                    continue;
                }
                let legacy_cycle = if matches!(modulator.shape, LfoShape::Custom) {
                    modulator.mseg_legacy_cycle_beats()
                } else {
                    1.0
                };
                let rate = match modulator.rate {
                    LfoRate::SyncBeats { beats } => RtLfoRate::SyncBeats {
                        beats: (beats * legacy_cycle).max(0.0625),
                    },
                    LfoRate::Hz { hz } => RtLfoRate::Hz { hz: hz.max(0.0) },
                };
                let mut mseg_points = modulator.mseg_points.clone();
                mseg_points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
                if matches!(modulator.shape, LfoShape::Custom) {
                    normalize_mseg_points(&mut mseg_points, legacy_cycle);
                }
                if matches!(modulator.shape, LfoShape::Custom) && mseg_points.is_empty() {
                    continue;
                }
                modulators.push(RtModulator {
                    id: modulator.id,
                    target,
                    shape: modulator.shape,
                    rate,
                    depth: modulator.depth.clamp(0.0, 1.0),
                    phase: modulator.phase.rem_euclid(1.0),
                    bipolar: modulator.bipolar,
                    center: modulator.center.clamp(0.0, 1.0),
                    mseg_points,
                    min: param_info.min,
                    max: param_info.max,
                    step_count: param_info.step_count,
                });
            }

            let changed = self
                .synced_modulators
                .get(&track.id)
                .map(|prev| prev != &modulators)
                .unwrap_or(true);
            if !changed {
                continue;
            }

            if modulators.is_empty() {
                self.synced_modulators.remove(&track.id);
            } else {
                self.synced_modulators
                    .insert(track.id, modulators.clone());
            }
            self.send(AudioCommand::SetModulators {
                track_id: track.id,
                modulators,
            });
        }

        self.flush_pending_cmds();
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

    fn advance(&mut self, delta_seconds: f32, playback: LoopPlayback) {
        self.loop_enabled = playback.enabled;
        self.loop_start_beats = playback.start_beats;
        self.loop_end_beats = playback.end_beats;
        self.content_end_beats = playback.content_end_beats;
        self.flush_pending_cmds();
        if !self.playing {
            return;
        }

        self.previous_beats = self.current_beats;
        self.current_beats += delta_seconds * self.beats_per_second;

        let loop_active = playback.enabled && playback.end_beats > playback.start_beats;
        if loop_active {
            if self.current_beats >= playback.end_beats {
                let span = playback.end_beats - playback.start_beats;
                let overshoot = (self.current_beats - playback.end_beats).rem_euclid(span);
                self.current_beats = playback.start_beats + overshoot;
                self.active_seq_notes.clear();
                self.send(AudioCommand::AllNotesOff);
                // Re-trigger notes sitting on the loop start next schedule pass.
                self.previous_beats = playback.start_beats - 0.0001;
            }
        } else if playback.content_end_beats > 0.0
            && self.current_beats >= playback.content_end_beats
        {
            // Not looping: play through, then stop at the end of the arrangement.
            self.current_beats = playback.content_end_beats;
            self.playing = false;
            self.active_seq_notes.clear();
            self.send(AudioCommand::AllNotesOff);
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
        self.loop_enabled = project.loop_enabled;
        self.loop_start_beats = project.loop_start_beats;
        self.loop_end_beats = project.loop_end_beats;
        self.content_end_beats = project.content_end_beats();
        self.flush_pending_cmds();
        self.drain_retired();

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
                    self.load_plugin_now(
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
                self.load_device_now(track.id, device.id, entry, device.plugin_state.clone());
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
                        device_id: device.id,
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
        self.plugin_params.clear();
        self.synced_instruments.clear();
        self.pending_loads.clear();
        self.device_slots.clear();
        self.device_params.clear();
        self.device_chain_sig.clear();
        self.device_chain_dirty.clear();
        self.pending_device_loads.clear();
        self.synced_automation.clear();
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

    fn set_plugin_editor_close_binding(&mut self, close_binding: super::plugins::EditorCloseBinding) {
        self.editor_host.set_close_binding(close_binding);
    }

    fn poll_plugin_editors(&mut self) -> super::EditorPoll {
        self.editor_host.poll()
    }

    fn plugin_parameters(&self, track_id: u64, device_id: Option<u64>) -> Vec<PluginParamInfo> {
        AudioEngine::plugin_parameters(self, track_id, device_id)
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
                let Some(clip) = clip.as_midi() else {
                    continue;
                };
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

    fn performance(&self) -> EnginePerformance {
        self.perf.snapshot(self.sample_rate_hz())
    }

    fn track_performance(&self) -> Vec<TrackPerformance> {
        self.track_perf
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    fn audio_device_name(&self) -> Option<String> {
        self.device_name.clone()
    }

    fn pending_plugin_loads(&self) -> (usize, usize) {
        (
            self.pending_loads.len(),
            self.pending_device_loads.len(),
        )
    }

    fn sync_samples(
        &mut self,
        project: &Project,
        decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
    ) {
        self.flush_pending_cmds();
        let live_ids: HashSet<u64> = project.tracks.iter().map(|t| t.id).collect();
        for track_id in self.synced_channels.keys().copied().collect::<Vec<_>>() {
            if !live_ids.contains(&track_id) {
                self.send(AudioCommand::ClearTrackSamples { track_id });
            }
        }

        for track in &project.tracks {
            let mut clips = Vec::new();
            for clip in &track.clips {
                let Some(audio) = clip.as_audio() else {
                    continue;
                };
                let Some(decoded) = decoded_audio.get(&audio.source) else {
                    continue;
                };
                clips.push(SamplePlayback {
                    clip_id: audio.id,
                    start_beats: audio.start_beats,
                    length_beats: audio.length_beats,
                    gain: db_to_linear(audio.gain_db),
                    buffer: Arc::clone(decoded),
                });
            }
            if clips.is_empty() {
                self.send(AudioCommand::ClearTrackSamples { track_id: track.id });
            } else {
                self.send(AudioCommand::SetTrackSamples {
                    track_id: track.id,
                    clips,
                });
            }
        }
        self.flush_pending_cmds();
    }
}

fn find_note(project: &Project, note_id: u64) -> Option<(u64, u8, u8)> {
    for track in &project.tracks {
        for clip in &track.clips {
            let Some(clip) = clip.as_midi() else {
                continue;
            };
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
    track_perf: Arc<Mutex<Vec<TrackPerformance>>>,
    perf: Arc<AudioPerfShared>,
    retire_tx: Sender<RetiredResource>,
) -> Result<(Stream, SyncSender<AudioCommand>, f32, String), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| String::from("No default audio output device"))?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| String::from("Unknown output"));

    let supported = device
        .default_output_config()
        .map_err(|error| format!("Default output config failed: {error}"))?;

    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    if let cpal::BufferSize::Fixed(frames) = config.buffer_size {
        perf.buffer_frames.store(frames, Ordering::Relaxed);
    }

    let (tx, rx) = mpsc::sync_channel::<AudioCommand>(64);
    let state = AudioCallbackState {
        voices: HashMap::new(),
        channel_params: HashMap::new(),
        fx_chains: HashMap::new(),
        sample_clips: HashMap::new(),
        automation: HashMap::new(),
        modulators: HashMap::new(),
        lfo_phases: HashMap::new(),
        master_gain: 1.0,
        commands: rx,
        channels,
        sample_rate,
        transport: TransportInfo::default(),
        metronome: MetronomeRunner::new(sample_rate),
        mix_l: vec![0.0; 4096],
        mix_r: vec![0.0; 4096],
        tmp_l: vec![0.0; 4096],
        tmp_r: vec![0.0; 4096],
        track_meters,
        master_meter,
        track_perf,
        perf,
        retire_tx,
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

    Ok((stream, tx, sample_rate, device_name))
}
