use egui::{RichText, Ui};

use crate::engine::DawEngine;
use crate::model::Project;

pub struct TransportUi;

impl TransportUi {
    pub fn show(ui: &mut Ui, project: &mut Project, engine: &mut dyn DawEngine) {
        ui.horizontal(|ui| {
            let play_label = if engine.is_playing() { "Pause" } else { "Play" };
            if ui.button(play_label).clicked() {
                engine.toggle_playback();
            }
            if ui.button("Stop").clicked() {
                engine.stop();
                engine.seek_beats(0.0);
            }

            ui.separator();

            ui.label(RichText::new("BPM").strong());
            let mut bpm = project.bpm;
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .speed(0.5)
                        .range(40.0..=240.0),
                )
                .changed()
            {
                project.bpm = bpm;
                engine.set_beats_per_second(project.beats_per_second());
            }

            ui.separator();

            ui.label(RichText::new("Loop end (beats)").strong());
            let mut loop_end = project.loop_end_beats;
            if ui
                .add(
                    egui::DragValue::new(&mut loop_end)
                        .speed(0.25)
                        .range(4.0..=256.0),
                )
                .changed()
            {
                project.loop_end_beats = loop_end.max(4.0);
            }

            ui.separator();

            let bar = (engine.current_beats() / project.beats_per_bar).floor() as i32 + 1;
            let beat_in_bar = engine.current_beats().rem_euclid(project.beats_per_bar) + 1.0;
            ui.label(format!(
                "Position: bar {:.0} beat {:.2} ({:.2} beats)",
                bar,
                beat_in_bar,
                engine.current_beats()
            ));
        });
    }
}
