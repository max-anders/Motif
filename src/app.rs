use std::fs;
use std::path::PathBuf;

use eframe::egui;

use crate::engine::{AudioEngine, DawEngine};
use crate::model::Project;
use crate::ui::{
    Action, AppSettings, PianoRollUi, PlaylistUi, PollFilter, SettingsAction, SettingsUi,
    TransportUi, SETTINGS_FILE,
};

const PROJECT_FILE: &str = "project.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterView {
    Playlist,
    PianoRoll { clip_id: u64 },
    Settings,
}

pub struct DawApp {
    project: Project,
    engine: AudioEngine,
    playlist: PlaylistUi,
    piano_roll: PianoRollUi,
    settings_ui: SettingsUi,
    center_view: CenterView,
    /// View to restore when leaving Settings (playlist or piano roll).
    settings_return: CenterView,
    settings: AppSettings,
    status_message: String,
}

impl DawApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let project = load_project().unwrap_or_default();
        let engine = AudioEngine::new(project.beats_per_second());
        let settings = AppSettings::load_or_defaults(&Self::settings_path());
        let status_message = if engine.audio_available() {
            String::from(
                "Playlist: click empty lane to add clip, double-click clip to open piano roll. Wheel=scroll, Ctrl+Wheel=zoom H.",
            )
        } else {
            let detail = engine.init_error().unwrap_or("unknown error");
            format!("Audio unavailable ({detail}). Transport still works silently.")
        };

        Self {
            project,
            engine,
            playlist: PlaylistUi::default(),
            piano_roll: PianoRollUi::default(),
            settings_ui: SettingsUi::default(),
            center_view: CenterView::Playlist,
            settings_return: CenterView::Playlist,
            settings,
            status_message,
        }
    }

    fn project_path() -> PathBuf {
        PathBuf::from(PROJECT_FILE)
    }

    fn settings_path() -> PathBuf {
        PathBuf::from(SETTINGS_FILE)
    }

    fn save_settings(&mut self) {
        match self.settings.save_to_path(&Self::settings_path()) {
            Ok(()) => {
                self.status_message = format!("Settings saved to {SETTINGS_FILE}");
            }
            Err(error) => {
                self.status_message = format!("Settings save failed: {error}");
            }
        }
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
                self.engine.stop();
                self.engine.all_notes_off();
                self.engine
                    .set_beats_per_second(project.beats_per_second());
                self.project = project;
                self.center_view = CenterView::Playlist;
                self.settings_return = CenterView::Playlist;
                self.playlist.clear_selection();
                self.piano_roll.release_audition(&mut self.engine);
                self.piano_roll.clear_selection();
                self.settings_ui.clear_capture();
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
            if matches!(self.settings_return, CenterView::PianoRoll { clip_id } if clip_id == id)
            {
                self.settings_return = CenterView::Playlist;
            }
        }
        self.playlist.clear_selection();
    }

    fn duplicate_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self.piano_roll.selected_note_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let span = {
            let Some(clip) = self.project.clip(clip_id) else {
                return;
            };
            Project::selection_span_beats(ids.iter().filter_map(|id| {
                clip.note(*id)
                    .map(|note| (note.start_beats, note.end_beats()))
            }))
        };
        let new_ids = self
            .project
            .duplicate_notes_in_clip(clip_id, &ids, span, 0);
        if !new_ids.is_empty() {
            self.piano_roll.set_selection(new_ids);
        }
    }

    fn duplicate_selected_clips(&mut self) {
        let ids: Vec<u64> = self.playlist.selected_clip_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let span = Project::selection_span_beats(ids.iter().filter_map(|id| {
            self.project
                .clip(*id)
                .map(|clip| (clip.start_beats, clip.end_beats()))
        }));
        let new_ids = self.project.duplicate_clips(&ids, span);
        if !new_ids.is_empty() {
            self.playlist.set_selection(new_ids);
        }
    }

    fn open_clip(&mut self, clip_id: u64) {
        if self.project.clip(clip_id).is_some() {
            self.piano_roll.clear_selection();
            self.center_view = CenterView::PianoRoll { clip_id };
        }
    }

    fn back_to_playlist(&mut self) {
        self.piano_roll.release_audition(&mut self.engine);
        self.piano_roll.clear_selection();
        self.center_view = CenterView::Playlist;
    }

    fn open_settings(&mut self) {
        if matches!(self.center_view, CenterView::Settings) {
            return;
        }
        self.settings_return = self.center_view;
        self.settings_ui.clear_capture();
        self.center_view = CenterView::Settings;
    }

    fn close_settings(&mut self) {
        self.settings_ui.clear_capture();
        let return_to = self.settings_return;
        self.center_view = match return_to {
            CenterView::Settings => CenterView::Playlist,
            other => other,
        };
    }

    fn dispatch_action(&mut self, action: Action) {
        match action {
            Action::TogglePlayback => self.engine.toggle_playback(),
            Action::DeleteSelection => match self.center_view {
                CenterView::Playlist => self.delete_selected_clips(),
                CenterView::PianoRoll { .. } => self.delete_selected_notes(),
                CenterView::Settings => {}
            },
            Action::DuplicateSelection => match self.center_view {
                CenterView::Playlist => self.duplicate_selected_clips(),
                CenterView::PianoRoll { .. } => self.duplicate_selected_notes(),
                CenterView::Settings => {}
            },
            Action::SaveProject => self.save_project(),
            Action::LoadProject => self.load_project_from_disk(),
            Action::BackToPlaylist => match self.center_view {
                CenterView::Settings => self.close_settings(),
                CenterView::PianoRoll { .. } => self.back_to_playlist(),
                CenterView::Playlist => {}
            },
        }
    }
}

impl eframe::App for DawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let delta_seconds = ctx.input(|input| input.unstable_dt);
        let loop_end = self.project.loop_end_beats;
        self.engine.advance(delta_seconds, loop_end);
        self.engine.schedule_project(&self.project);

        let poll_filter = if self.settings_ui.is_capturing() {
            PollFilter::None
        } else if matches!(self.center_view, CenterView::Settings) {
            PollFilter::NavigationOnly
        } else {
            PollFilter::All
        };
        for action in self.settings.shortcuts.poll(ctx, poll_filter) {
            self.dispatch_action(action);
        }

        self.settings.themes.colors().apply_to_context(ctx);

        egui::TopBottomPanel::top("transport_panel").show(ctx, |ui| {
            ui.heading("Motif");
            ui.add_space(4.0);
            TransportUi::show(ui, &mut self.project, &mut self.engine);

            ui.separator();
            ui.horizontal(|ui| {
                match self.center_view {
                    CenterView::Settings => {
                        if ui.button("Back").clicked() {
                            self.close_settings();
                        }
                        ui.separator();
                    }
                    CenterView::PianoRoll { .. } => {
                        if ui.button("Back to playlist").clicked() {
                            self.back_to_playlist();
                        }
                        ui.separator();
                    }
                    CenterView::Playlist => {}
                }
                if ui.button("Save project").clicked() {
                    self.save_project();
                }
                if ui.button("Load project").clicked() {
                    self.load_project_from_disk();
                }
                if !matches!(self.center_view, CenterView::Settings)
                    && ui.button("Settings").clicked()
                {
                    self.open_settings();
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
                        let open_clip = {
                            let DawApp {
                                playlist,
                                project,
                                engine,
                                settings,
                                ..
                            } = self;
                            playlist.show(ui, project, engine, settings.themes.colors());
                            playlist.take_open_clip_request()
                        };
                        if let Some(clip_id) = open_clip {
                            self.open_clip(clip_id);
                        }
                    }
                    CenterView::PianoRoll { clip_id } => {
                        if self.project.clip(clip_id).is_some() {
                            let DawApp {
                                piano_roll,
                                project,
                                engine,
                                settings,
                                ..
                            } = self;
                            piano_roll.show(
                                ui,
                                clip_id,
                                project,
                                engine,
                                settings.themes.colors(),
                            );
                        } else {
                            self.back_to_playlist();
                        }
                    }
                    CenterView::Settings => {
                        ui.add_space(8.0);
                        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                            match self.settings_ui.show(
                                ui,
                                &mut self.settings.shortcuts,
                                &mut self.settings.themes,
                            ) {
                                Some(SettingsAction::Back) => self.close_settings(),
                                Some(SettingsAction::ShortcutsChanged)
                                | Some(SettingsAction::ThemeChanged) => self.save_settings(),
                                None => {}
                            }
                        });
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
