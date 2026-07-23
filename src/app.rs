use std::fs;
use std::path::PathBuf;

use eframe::egui;

use crate::engine::{DawEngine, MockEngine};
use crate::model::Project;
use crate::ui::{PianoRollUi, PlaylistUi, TransportUi};

const PROJECT_FILE: &str = "project.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterView {
    Playlist,
    PianoRoll { clip_id: u64 },
}

pub struct DawApp {
    project: Project,
    engine: MockEngine,
    playlist: PlaylistUi,
    piano_roll: PianoRollUi,
    center_view: CenterView,
    status_message: String,
}

impl DawApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let project = load_project().unwrap_or_default();
        let engine = MockEngine::new(project.beats_per_second());

        Self {
            project,
            engine,
            playlist: PlaylistUi::default(),
            piano_roll: PianoRollUi::default(),
            center_view: CenterView::Playlist,
            status_message: String::from(
                "Playlist: click empty lane to add clip, click clip to open piano roll. Wheel=scroll, Ctrl+Wheel=zoom H.",
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
                self.center_view = CenterView::Playlist;
                self.playlist.clear_selection();
                self.piano_roll.clear_selection();
                self.status_message = format!("Loaded {PROJECT_FILE}");
            }
            Err(error) => self.status_message = format!("Load failed: {error}"),
        }
    }

    fn delete_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self.piano_roll.selected_note_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        if let Some(clip) = self.project.clip_mut(clip_id) {
            for id in ids {
                clip.remove_note(id);
            }
        }
        self.piano_roll.clear_selection();
    }

    fn delete_selected_clips(&mut self) {
        let ids: Vec<u64> = self.playlist.selected_clip_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        for id in ids {
            self.project.remove_clip(id);
            if matches!(self.center_view, CenterView::PianoRoll { clip_id } if clip_id == id) {
                self.center_view = CenterView::Playlist;
                self.piano_roll.clear_selection();
            }
        }
        self.playlist.clear_selection();
    }

    fn open_clip(&mut self, clip_id: u64) {
        if self.project.clip(clip_id).is_some() {
            self.piano_roll.clear_selection();
            self.center_view = CenterView::PianoRoll { clip_id };
        }
    }

    fn back_to_playlist(&mut self) {
        self.piano_roll.clear_selection();
        self.center_view = CenterView::Playlist;
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
            let cut_pressed = input.key_pressed(egui::Key::X)
                && (input.modifiers.ctrl || input.modifiers.command || input.modifiers.mac_cmd);
            if cut_pressed
                || input.key_pressed(egui::Key::Delete)
                || input.key_pressed(egui::Key::Backspace)
            {
                match self.center_view {
                    CenterView::Playlist => self.delete_selected_clips(),
                    CenterView::PianoRoll { .. } => self.delete_selected_notes(),
                }
            }
        });

        egui::TopBottomPanel::top("transport_panel").show(ctx, |ui| {
            ui.heading("Motif");
            ui.add_space(4.0);
            TransportUi::show(ui, &mut self.project, &mut self.engine);

            ui.separator();
            ui.horizontal(|ui| {
                if matches!(self.center_view, CenterView::PianoRoll { .. }) {
                    if ui.button("Back to playlist").clicked() {
                        self.back_to_playlist();
                    }
                    ui.separator();
                }
                if ui.button("Save project").clicked() {
                    self.save_project();
                }
                if ui.button("Load project").clicked() {
                    self.load_project_from_disk();
                }
                if let CenterView::PianoRoll { clip_id } = self.center_view {
                    if let Some(clip) = self.project.clip(clip_id) {
                        ui.label(format!("Editing: {}", clip.name));
                    }
                }
                ui.label(&self.status_message);
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                match self.center_view {
                    CenterView::Playlist => {
                        self.playlist
                            .show(ui, &mut self.project, &mut self.engine);
                        if let Some(clip_id) = self.playlist.take_open_clip_request() {
                            self.open_clip(clip_id);
                        }
                    }
                    CenterView::PianoRoll { clip_id } => {
                        if self.project.clip(clip_id).is_some() {
                            self.piano_roll
                                .show(ui, clip_id, &mut self.project, &mut self.engine);
                        } else {
                            self.back_to_playlist();
                        }
                    }
                }
            });

        ctx.request_repaint();
    }
}

fn load_project() -> Result<Project, Box<dyn std::error::Error>> {
    let json = fs::read_to_string(DawApp::project_path())?;
    Ok(Project::from_json(&json)?)
}
