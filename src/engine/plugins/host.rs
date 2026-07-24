//! Load and activate a CLAP/VST3 instrument for the audio thread.

use truce_rack::clap::ClapScanner;
use truce_rack::core::buffer::{AudioBuffer, BusRange};
use truce_rack::core::bus::BusLayout;
use truce_rack::core::events::{Event, EventBody, EventList, MidiData};
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
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::NoteOn {
                channel: 0,
                note: pitch.min(127),
                velocity: velocity.min(127),
            }),
        });
    }

    pub fn push_note_off(&mut self, pitch: u8) {
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::NoteOff {
                channel: 0,
                note: pitch.min(127),
                velocity: 0,
            }),
        });
    }

    pub fn all_notes_off(&mut self) {
        self.events.push(Event {
            sample_offset: 0,
            body: EventBody::Midi(MidiData::ControlChange {
                channel: 0,
                controller: 123,
                value: 0,
            }),
        });
    }
}

/// Load from catalog metadata and activate at `sample_rate`.
pub fn load_and_activate(
    entry: &CatalogEntry,
    sample_rate: f64,
) -> Result<HostedPlugin, String> {
    let info = PluginInfo {
        name: entry.name.clone(),
        vendor: entry.vendor.clone(),
        version: 0,
        category: PluginCategory::Instrument,
        path: entry.path.clone(),
        unique_id: entry.unique_id.clone(),
        format: entry.format.as_str(),
        has_editor: false,
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
