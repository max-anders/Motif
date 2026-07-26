//! Load and activate a CLAP/VST3 instrument for the audio thread.

use std::collections::HashSet;

use truce_rack::clap::ClapScanner;
use truce_rack::core::buffer::{AudioBuffer, BusRange};
use truce_rack::core::bus::BusLayout;
use truce_rack::core::editor::{PluginEditor, WindowHandle};
use truce_rack::core::events::{Event, EventBody, EventList, MidiData, TransportFlag};
use truce_rack::core::info::{ParameterFlags, PluginCategory, PluginInfo};
use truce_rack::core::plugin::{Plugin, PluginCore, ProcessContext};
use truce_rack::core::scanner::PluginScanner;
use truce_rack::core::state::{FormatId, StateEnvelope};
use truce_rack::core::transport::TransportInfo;
use truce_rack::vst3::Vst3Scanner;

use crate::model::PluginFormat;

use super::catalog::{CatalogEntry, EntryCategory};

/// Max frames we prepare plugin buffers for (cpal blocks are usually smaller).
pub const MAX_BLOCK_FRAMES: usize = 8192;

/// Lightweight, host-agnostic parameter metadata exposed to the UI/engine.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginParamInfo {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub step_count: u32,
    pub automatable: bool,
}

enum PluginInstance {
    Clap(truce_rack::clap::ClapPlugin),
    Vst3(truce_rack::vst3::Vst3Plugin),
}

/// Activated plugin with pre-sized planar buffers for real-time process.
pub struct HostedPlugin {
    instance: PluginInstance,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    events: EventList,
    out_events: EventList,
    /// Pitches currently held (sequencer + audition) so pause can NoteOff them.
    held_notes: HashSet<u8>,
    sample_rate: f64,
}

impl HostedPlugin {
    pub fn parameter_count(&self) -> usize {
        match &self.instance {
            PluginInstance::Clap(plugin) => plugin.parameter_count(),
            PluginInstance::Vst3(plugin) => plugin.parameter_count(),
        }
    }

    pub fn parameters(&self) -> Vec<PluginParamInfo> {
        let count = self.parameter_count();
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let info = match &self.instance {
                PluginInstance::Clap(plugin) => plugin.parameter_info(index),
                PluginInstance::Vst3(plugin) => plugin.parameter_info(index),
            };
            let Ok(info) = info else {
                continue;
            };
            out.push(PluginParamInfo {
                id: info.id,
                name: info.name,
                min: info.min,
                max: info.max,
                step_count: info.step_count,
                automatable: info.flags.contains(ParameterFlags::AUTOMATABLE),
            });
        }
        out
    }

    /// Convert normalized `0..1` automation to this param's native plugin units.
    pub fn map_normalized_to_native(&self, param_id: u32, normalized_value: f64) -> Option<f64> {
        let info = self.parameters().into_iter().find(|param| param.id == param_id)?;
        let clamped = normalized_value.clamp(0.0, 1.0);
        let span = (info.max - info.min).max(0.0);
        let mut native = info.min + clamped * span;
        if info.step_count > 1 {
            let max_step = (info.step_count - 1) as f64;
            let step = (clamped * max_step).round().clamp(0.0, max_step);
            native = if max_step > 0.0 {
                info.min + (step / max_step) * span
            } else {
                info.min
            };
        } else if info.step_count == 1 {
            native = info.min;
        }
        Some(native)
    }

    /// Current param value as normalized `0..1` (for host UI sliders).
    pub fn get_param_normalized(&self, param_id: u32) -> Option<f32> {
        let (index, info) = self
            .parameters()
            .into_iter()
            .enumerate()
            .find(|(_, param)| param.id == param_id)?;
        let raw = match &self.instance {
            PluginInstance::Clap(plugin) => plugin.parameter_value(index).ok()?,
            // VST3 `parameter_value` is already normalized 0..1.
            PluginInstance::Vst3(plugin) => {
                return Some(plugin.parameter_value(index).ok()?.clamp(0.0, 1.0) as f32);
            }
        };
        let span = (info.max - info.min).max(f64::EPSILON);
        Some(((raw - info.min) / span).clamp(0.0, 1.0) as f32)
    }

    /// Set a param from normalized `0..1` (queues RT event + host-thread write for readback).
    pub fn set_param_normalized(&mut self, param_id: u32, normalized: f32) -> bool {
        let clamped = normalized.clamp(0.0, 1.0) as f64;
        let Some(native) = self.map_normalized_to_native(param_id, clamped) else {
            return false;
        };
        let Some(index) = self
            .parameters()
            .iter()
            .position(|param| param.id == param_id)
        else {
            return false;
        };
        // Host-thread write for immediate GUI / get_param_normalized readback.
        let _ = match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.set_parameter(index, native),
            PluginInstance::Vst3(plugin) => plugin.set_parameter(index, clamped),
        };
        // Audio-thread path (same as automation / macros).
        self.push_param(param_id, native, 0);
        true
    }

    /// Process one instrument block. Returns plugin-GUI param touches (for MRU).
    pub fn process_block(
        &mut self,
        frames: usize,
        transport: Option<TransportInfo>,
        mix_l: &mut [f32],
        mix_r: &mut [f32],
    ) -> Vec<u32> {
        if frames == 0 || frames > MAX_BLOCK_FRAMES {
            return Vec::new();
        }

        let motif_written = motif_written_param_ids(&self.events);

        let HostedPlugin {
            instance,
            in_l,
            in_r,
            out_l,
            out_r,
            events,
            out_events,
            sample_rate,
            held_notes: _,
        } = self;

        in_l[..frames].fill(0.0);
        in_r[..frames].fill(0.0);
        out_l[..frames].fill(0.0);
        out_r[..frames].fill(0.0);
        out_events.clear();

        let bus_in = [BusRange::new(0, 2)];
        let bus_out = [BusRange::new(0, 2)];
        let input_refs: [&[f32]; 2] = [&in_l[..frames], &in_r[..frames]];
        let mut output_refs: [&mut [f32]; 2] = [&mut out_l[..frames], &mut out_r[..frames]];

        let result = {
            let mut buffer =
                AudioBuffer::new(&input_refs, &mut output_refs, frames, &bus_in, &bus_out);
            let mut context = ProcessContext {
                sample_rate: *sample_rate,
                max_block_size: MAX_BLOCK_FRAMES,
                transport,
                output_events: out_events,
            };
            match instance {
                PluginInstance::Clap(plugin) => plugin.process(&mut buffer, events, &mut context),
                PluginInstance::Vst3(plugin) => plugin.process(&mut buffer, events, &mut context),
            }
        };

        events.clear();
        if result.is_err() {
            return Vec::new();
        }

        let touches = collect_gui_param_touches(out_events, &motif_written);

        let n = frames.min(mix_l.len()).min(mix_r.len());
        for i in 0..n {
            mix_l[i] += out_l[i];
            mix_r[i] += out_r[i];
        }
        touches
    }

    /// Insert-effect processing: feeds `buf_l`/`buf_r` as the plugin's input
    /// and **replaces** them with the plugin's output in place (unlike
    /// [`Self::process_block`], which is additive for instrument voices).
    /// On a processing error, or when frames are out of range, `buf_l`/`buf_r`
    /// are left untouched (passthrough) rather than silenced.
    ///
    /// Returns plugin-GUI param touches (for MRU).
    pub fn process_effect(
        &mut self,
        frames: usize,
        transport: Option<TransportInfo>,
        buf_l: &mut [f32],
        buf_r: &mut [f32],
    ) -> Vec<u32> {
        if frames == 0 || frames > MAX_BLOCK_FRAMES {
            return Vec::new();
        }
        let n = frames.min(buf_l.len()).min(buf_r.len());
        if n == 0 {
            return Vec::new();
        }

        let motif_written = motif_written_param_ids(&self.events);

        let HostedPlugin {
            instance,
            in_l,
            in_r,
            out_l,
            out_r,
            events,
            out_events,
            sample_rate,
            held_notes: _,
        } = self;

        in_l[..frames].fill(0.0);
        in_r[..frames].fill(0.0);
        in_l[..n].copy_from_slice(&buf_l[..n]);
        in_r[..n].copy_from_slice(&buf_r[..n]);
        out_l[..frames].fill(0.0);
        out_r[..frames].fill(0.0);
        out_events.clear();

        let bus_in = [BusRange::new(0, 2)];
        let bus_out = [BusRange::new(0, 2)];
        let input_refs: [&[f32]; 2] = [&in_l[..frames], &in_r[..frames]];
        let mut output_refs: [&mut [f32]; 2] = [&mut out_l[..frames], &mut out_r[..frames]];

        let result = {
            let mut buffer =
                AudioBuffer::new(&input_refs, &mut output_refs, frames, &bus_in, &bus_out);
            let mut context = ProcessContext {
                sample_rate: *sample_rate,
                max_block_size: MAX_BLOCK_FRAMES,
                transport,
                output_events: out_events,
            };
            match instance {
                PluginInstance::Clap(plugin) => plugin.process(&mut buffer, events, &mut context),
                PluginInstance::Vst3(plugin) => plugin.process(&mut buffer, events, &mut context),
            }
        };

        events.clear();
        if result.is_err() {
            return Vec::new();
        }

        let touches = collect_gui_param_touches(out_events, &motif_written);

        buf_l[..n].copy_from_slice(&out_l[..n]);
        buf_r[..n].copy_from_slice(&out_r[..n]);
        touches
    }

    pub fn push_note_on(&mut self, pitch: u8, velocity: u8) {
        let note = pitch.min(127);
        self.held_notes.insert(note);
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::NoteOn {
                channel: 0,
                note,
                velocity: velocity.min(127),
            }),
        });
    }

    pub fn push_note_off(&mut self, pitch: u8) {
        let note = pitch.min(127);
        self.held_notes.remove(&note);
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::NoteOff {
                channel: 0,
                note,
                velocity: 0,
            }),
        });
    }

    pub fn all_notes_off(&mut self) {
        // Explicit NoteOffs: many instruments ignore CC123 alone (Vital included).
        let held: Vec<u8> = self.held_notes.drain().collect();
        for note in held {
            self.events.push(Event {
                sample_offset: 0,
                body: EventBody::Midi(MidiData::NoteOff {
                    channel: 0,
                    note,
                    velocity: 0,
                }),
            });
        }
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::ControlChange {
                channel: 0,
                controller: 123,
                value: 0,
            }),
        });
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::TransportFlag(TransportFlag::PlayStop),
        });
    }

    pub fn push_param(&mut self, param_id: u32, native_value: f64, sample_offset: u32) {
        // truce-rack uses stable ParameterInfo.id here; CLAP consumes native
        // plugin units (not normalized 0..1) in EventBody::ParamValue.value.
        self.events.push(Event {
            sample_offset,
            body: EventBody::ParamValue {
                param_id,
                value: native_value,
            },
        });
    }

    /// Whether the loaded instance exposes a custom editor GUI.
    pub fn has_editor(&mut self) -> bool {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor().is_some(),
            PluginInstance::Vst3(plugin) => plugin.editor().is_some(),
        }
    }

    /// Open the plugin editor inside `parent` (UI thread only).
    pub fn open_editor(&mut self, parent: WindowHandle, scale: f64) -> Result<(), String> {
        let editor = match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor(),
            PluginInstance::Vst3(plugin) => plugin.editor(),
        };
        let Some(editor) = editor else {
            return Err(String::from("Plugin has no editor GUI"));
        };
        let editor: &mut dyn PluginEditor = editor;
        editor
            .open(parent, scale)
            .map_err(|error| format!("Open editor failed: {error}"))?;
        editor.show();
        Ok(())
    }

    /// Fully tear down the editor (used at plugin unload). For CLAP this hides
    /// the GUI (the real destroy/leak happens in the plugin's `Drop`); for VST3
    /// this detaches and releases the view.
    pub fn close_editor(&mut self) {
        let editor = match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor(),
            PluginInstance::Vst3(plugin) => plugin.editor(),
        };
        if let Some(editor) = editor {
            let editor: &mut dyn PluginEditor = editor;
            editor.close();
        }
    }

    /// Courtesy-hide the editor GUI without tearing it down (used when the user
    /// closes the editor window but the plugin stays loaded). CLAP hides its
    /// window; VST3 has no separate hide (the parent window is unmapped instead).
    pub fn hide_editor(&mut self) {
        let editor = match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor(),
            PluginInstance::Vst3(plugin) => plugin.editor(),
        };
        if let Some(editor) = editor {
            let editor: &mut dyn PluginEditor = editor;
            editor.hide();
        }
    }

    /// Re-show a previously hidden editor GUI (used when re-opening an editor
    /// whose parent window was kept alive). Counterpart to [`Self::hide_editor`].
    pub fn show_editor(&mut self) {
        let editor = match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor(),
            PluginInstance::Vst3(plugin) => plugin.editor(),
        };
        if let Some(editor) = editor {
            let editor: &mut dyn PluginEditor = editor;
            editor.show();
        }
    }

    /// Whether unloading this plugin must leak its editor's parent window
    /// instead of destroying it (LSP-Plugins run an un-joinable editor thread;
    /// destroying the window under it crashes the host). Only CLAP plugins with
    /// the known bug return true.
    pub fn editor_teardown_leaks_window(&self) -> bool {
        match &self.instance {
            PluginInstance::Clap(plugin) => plugin.editor_teardown_is_unsafe(),
            PluginInstance::Vst3(_) => false,
        }
    }

    pub fn editor_size(&mut self) -> Option<(u32, u32)> {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor().and_then(|e| e.size()),
            PluginInstance::Vst3(plugin) => plugin.editor().and_then(|e| e.size()),
        }
    }

    pub fn editor_is_resizable(&mut self) -> bool {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => {
                plugin.editor().map(|e| e.is_resizable()).unwrap_or(false)
            }
            PluginInstance::Vst3(plugin) => {
                plugin.editor().map(|e| e.is_resizable()).unwrap_or(false)
            }
        }
    }

    pub fn editor_set_size(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor().and_then(|e| e.set_size(width, height)),
            PluginInstance::Vst3(plugin) => plugin.editor().and_then(|e| e.set_size(width, height)),
        }
    }

    pub fn editor_on_idle(&mut self) {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => {
                if let Some(editor) = plugin.editor() {
                    editor.on_idle();
                }
            }
            PluginInstance::Vst3(plugin) => {
                if let Some(editor) = plugin.editor() {
                    editor.on_idle();
                }
            }
        }
    }

    /// Snapshot plugin state as an RKST envelope (for `project.json`).
    pub fn save_state_blob(&self, format: PluginFormat) -> Result<Vec<u8>, String> {
        let payload = match &self.instance {
            PluginInstance::Clap(plugin) => plugin
                .save_state()
                .map_err(|e| format!("CLAP save_state failed: {e}"))?,
            PluginInstance::Vst3(plugin) => plugin
                .save_state()
                .map_err(|e| format!("VST3 save_state failed: {e}"))?,
        };
        Ok(StateEnvelope {
            format: format_id(format),
            payload: &payload,
        }
        .encode())
    }
}

/// Load from catalog metadata and activate at `sample_rate`.
/// When `state` is set (RKST envelope), restore it after activate.
pub fn load_and_activate(
    entry: &CatalogEntry,
    sample_rate: f64,
    state: Option<&[u8]>,
) -> Result<HostedPlugin, String> {
    if entry.path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("yabridge"))
    }) {
        return Err(String::from(
            "yabridge Windows VST3 cannot be loaded in-process (Wine bridge abort). Use a native Linux CLAP/VST3.",
        ));
    }

    let category = match entry.category {
        EntryCategory::Instrument => PluginCategory::Instrument,
        EntryCategory::Effect => PluginCategory::Effect,
    };
    let info = PluginInfo {
        name: entry.name.clone(),
        vendor: entry.vendor.clone(),
        version: 0,
        category,
        path: entry.path.clone(),
        unique_id: entry.unique_id.clone(),
        format: entry.format.as_str(),
        has_editor: entry.has_editor,
        accepts_midi: entry.accepts_midi,
    };

    let mut instance = match entry.format {
        PluginFormat::Clap => {
            let plugin = ClapScanner::new()
                .load(&info)
                .map_err(|e| format!("CLAP load failed: {e}"))?;
            PluginInstance::Clap(plugin)
        }
        PluginFormat::Vst3 => {
            let plugin = Vst3Scanner::new()
                .load(&info)
                .map_err(|e| format!("VST3 load failed: {e}"))?;
            PluginInstance::Vst3(plugin)
        }
    };

    let layout = match &instance {
        PluginInstance::Clap(p) => pick_layout(p.supported_layouts()),
        PluginInstance::Vst3(p) => pick_layout(p.supported_layouts()),
    };

    match &mut instance {
        PluginInstance::Clap(p) => p
            .activate(layout, sample_rate, MAX_BLOCK_FRAMES)
            .map_err(|e| format!("CLAP activate failed: {e}"))?,
        PluginInstance::Vst3(p) => p
            .activate(layout, sample_rate, MAX_BLOCK_FRAMES)
            .map_err(|e| format!("VST3 activate failed: {e}"))?,
    }

    if let Some(bytes) = state {
        // Best-effort: a corrupt / outdated blob must not silence the track.
        if let Err(error) = apply_state_blob(&mut instance, entry.format, bytes) {
            eprintln!(
                "motif: plugin state restore failed ({name}): {error}",
                name = entry.name
            );
        }
    }

    Ok(HostedPlugin {
        instance,
        in_l: vec![0.0; MAX_BLOCK_FRAMES],
        in_r: vec![0.0; MAX_BLOCK_FRAMES],
        out_l: vec![0.0; MAX_BLOCK_FRAMES],
        out_r: vec![0.0; MAX_BLOCK_FRAMES],
        events: EventList::new(),
        out_events: EventList::new(),
        held_notes: HashSet::new(),
        sample_rate,
    })
}

fn apply_state_blob(
    instance: &mut PluginInstance,
    format: PluginFormat,
    bytes: &[u8],
) -> Result<(), String> {
    let envelope = StateEnvelope::decode(bytes).map_err(|e| format!("Bad plugin state: {e}"))?;
    let expected = format_id(format);
    if envelope.format != expected && envelope.format != FormatId::Unknown {
        return Err(format!(
            "Plugin state format mismatch: saved {:?}, track is {}",
            envelope.format,
            format.label()
        ));
    }
    match instance {
        PluginInstance::Clap(plugin) => plugin
            .load_state(envelope.payload)
            .map_err(|e| format!("CLAP load_state failed: {e}"))?,
        PluginInstance::Vst3(plugin) => plugin
            .load_state(envelope.payload)
            .map_err(|e| format!("VST3 load_state failed: {e}"))?,
    }
    Ok(())
}

fn format_id(format: PluginFormat) -> FormatId {
    match format {
        PluginFormat::Clap => FormatId::Clap,
        PluginFormat::Vst3 => FormatId::Vst3,
    }
}

fn pick_layout(layouts: &[BusLayout]) -> BusLayout {
    layouts
        .iter()
        .find(|layout| layout.total_output_channels() >= 2)
        .cloned()
        .or_else(|| layouts.first().cloned())
        .unwrap_or_else(BusLayout::stereo)
}

fn motif_written_param_ids(events: &EventList) -> HashSet<u32> {
    events
        .iter()
        .filter_map(|event| match event.body {
            EventBody::ParamValue { param_id, .. } => Some(param_id),
            _ => None,
        })
        .collect()
}

/// Collect unique param ids touched by the plugin GUI this block.
/// Prefers gesture-begin; also includes outbound ParamValue not written by Motif.
fn collect_gui_param_touches(out_events: &EventList, motif_written: &HashSet<u32>) -> Vec<u32> {
    let mut touches = Vec::new();
    let mut seen = HashSet::new();
    for event in out_events.iter() {
        let param_id = match event.body {
            EventBody::ParamGesture {
                param_id,
                active: true,
            } => param_id,
            EventBody::ParamValue { param_id, .. } if !motif_written.contains(&param_id) => {
                param_id
            }
            _ => continue,
        };
        if seen.insert(param_id) {
            touches.push(param_id);
        }
    }
    touches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_gui_touches_prefers_gesture_filters_motif_writes() {
        let mut out = EventList::new();
        out.push(Event {
            sample_offset: 0,
            body: EventBody::ParamGesture {
                param_id: 1,
                active: true,
            },
        });
        out.push(Event {
            sample_offset: 1,
            body: EventBody::ParamValue {
                param_id: 2,
                value: 0.5,
            },
        });
        out.push(Event {
            sample_offset: 2,
            body: EventBody::ParamValue {
                param_id: 3,
                value: 0.25,
            },
        });
        let motif_written = HashSet::from([3]);
        let touches = collect_gui_param_touches(&out, &motif_written);
        assert_eq!(touches, vec![1, 2]);
    }
}
