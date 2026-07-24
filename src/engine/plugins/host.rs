//! Load and activate a CLAP/VST3 instrument for the audio thread.

use std::collections::HashSet;

use truce_rack::clap::ClapScanner;
use truce_rack::core::buffer::{AudioBuffer, BusRange};
use truce_rack::core::bus::BusLayout;
use truce_rack::core::editor::{PluginEditor, WindowHandle};
use truce_rack::core::events::{Event, EventBody, EventList, MidiData, TransportFlag};
use truce_rack::core::info::{PluginCategory, PluginInfo};
use truce_rack::core::plugin::{Plugin, PluginCore, ProcessContext};
use truce_rack::core::scanner::PluginScanner;
use truce_rack::core::transport::TransportInfo;
use truce_rack::vst3::Vst3Scanner;

use crate::model::PluginFormat;

use super::catalog::CatalogEntry;

/// Max frames we prepare plugin buffers for (cpal blocks are usually smaller).
pub const MAX_BLOCK_FRAMES: usize = 8192;

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
    pub fn process_block(
        &mut self,
        frames: usize,
        transport: Option<TransportInfo>,
        mix_l: &mut [f32],
        mix_r: &mut [f32],
    ) {
        if frames == 0 || frames > MAX_BLOCK_FRAMES {
            return;
        }

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
            let mut buffer = AudioBuffer::new(
                &input_refs,
                &mut output_refs,
                frames,
                &bus_in,
                &bus_out,
            );
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
            return;
        }

        let n = frames.min(mix_l.len()).min(mix_r.len());
        for i in 0..n {
            mix_l[i] += out_l[i];
            mix_r[i] += out_r[i];
        }
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

    pub fn editor_size(&mut self) -> Option<(u32, u32)> {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin.editor().and_then(|e| e.size()),
            PluginInstance::Vst3(plugin) => plugin.editor().and_then(|e| e.size()),
        }
    }

    pub fn editor_is_resizable(&mut self) -> bool {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => plugin
                .editor()
                .map(|e| e.is_resizable())
                .unwrap_or(false),
            PluginInstance::Vst3(plugin) => plugin
                .editor()
                .map(|e| e.is_resizable())
                .unwrap_or(false),
        }
    }

    pub fn editor_set_size(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        match &mut self.instance {
            PluginInstance::Clap(plugin) => {
                plugin.editor().and_then(|e| e.set_size(width, height))
            }
            PluginInstance::Vst3(plugin) => {
                plugin.editor().and_then(|e| e.set_size(width, height))
            }
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
}

/// Load from catalog metadata and activate at `sample_rate`.
pub fn load_and_activate(
    entry: &CatalogEntry,
    sample_rate: f64,
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

    let info = PluginInfo {
        name: entry.name.clone(),
        vendor: entry.vendor.clone(),
        version: 0,
        category: PluginCategory::Instrument,
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

fn pick_layout(layouts: &[BusLayout]) -> BusLayout {
    layouts
        .iter()
        .find(|layout| layout.total_output_channels() >= 2)
        .cloned()
        .or_else(|| layouts.first().cloned())
        .unwrap_or_else(BusLayout::stereo)
}
