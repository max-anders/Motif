use std::fs;
use std::path::PathBuf;

use eframe::egui;

use crate::engine::{DawEngine, MockEngine};
use crate::model::Project;
use crate::ui::{PianoRollUi, TransportUi};

const PROJECT_FILE: &str = "project.json";

pub struct DawApp {
    project: Project,
    engine: MockEngine,
    piano_roll: PianoRollUi,
    status_message: String,
}

impl DawApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let project = load_project().unwrap_or_default();
        let engine = MockEngine::new(project.beats_per_second());

        Self {
            project,
            engine,
            piano_roll: PianoRollUi::default(),
            status_message: String::from(
                "Ready. Click = add/select. Drag empty = marquee select. Shift/right empty = seek.",
            ),
        }
    }

    fn project_path() -> PathBuf {
        PathBuf::from(PROJECT_FILE)
    }

    fn save_project(&mut self) {
        match serde_json::to_string_pretty(&self.project) {
            Ok(json) => match fs::write(Self::project_path(), json) {
                Ok(()) => self.status_message = format!("Saved to {PROJECT_FILE}"),
                Err(error) => self.status_message = format!("Save failed: {error}"),
            },
            Err(error) => self.status_message = format!("Serialize failed: {error}"),
        }
    }

    fn load_project_from_disk(&mut self) {
        match load_project() {
            Ok(project) => {
                self.engine
                    .set_beats_per_second(project.beats_per_second());
                self.project = project;
                self.status_message = format!("Loaded {PROJECT_FILE}");
            }
            Err(error) => self.status_message = format!("Load failed: {error}"),
        }
    }
}

impl eframe::App for DawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let delta_seconds = ctx.input(|input| input.unstable_dt);
        let loop_end = self.project.loop_end_beats;
        self.engine.advance(delta_seconds, loop_end);

        ctx.input(|input| {
            if input.key_pressed(egui::Key::Space) {
                self.engine.toggle_playback();
            }
            if input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace) {
                let ids: Vec<u64> = self.piano_roll.selected_note_ids().iter().copied().collect();
                if !ids.is_empty() {
                    for id in ids {
                        self.project.remove_note(id);
                    }
                    self.piano_roll.clear_selection();
                }
            }
        });

        egui::TopBottomPanel::top("transport_panel").show(ctx, |ui| {
            ui.heading("Motif");
            ui.add_space(4.0);
            TransportUi::show(ui, &mut self.project, &mut self.engine);

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Save project").clicked() {
                    self.save_project();
                }
                if ui.button("Load project").clicked() {
                    self.load_project_from_disk();
                }
                ui.label(&self.status_message);
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.piano_roll
                    .show(ui, &mut self.project, &mut self.engine);
            });

        ctx.request_repaint();
    }
}

fn load_project() -> Result<Project, Box<dyn std::error::Error>> {
    let json = fs::read_to_string(DawApp::project_path())?;
    Ok(serde_json::from_str(&json)?)
}
