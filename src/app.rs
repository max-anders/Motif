use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::engine::{AudioEngine, DawEngine, PluginCatalog, PLUGIN_CACHE_FILE};
use crate::model::{
    clear_recovery, ensure_motif_extension, format_unix_time, legacy_project_path,
    load_project_from, load_recovery_meta, load_recovery_project, project_display_name,
    projects_dir, push_recent, save_project_to, write_recovery, EditClipboard, EditHistory,
    Project, PROJECT_EXTENSION, RecoveryMeta,
};
use crate::ui::{
    Action, AppSettings, PianoRollUi, PlaylistUi, PluginEditorRequest, PollFilter,
    ProjectBrowserAction, ProjectBrowserUi, SettingsAction, SettingsUi, TransportUi, SETTINGS_FILE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterView {
    Playlist,
    PianoRoll { clip_id: u64 },
    Settings,
}

pub struct DawApp {
    project: Project,
    /// Last explicitly saved (or loaded) project state for dirty detection.
    saved_snapshot: Project,
    current_path: Option<PathBuf>,
    project_name: String,
    engine: AudioEngine,
    playlist: PlaylistUi,
    piano_roll: PianoRollUi,
    settings_ui: SettingsUi,
    project_browser: ProjectBrowserUi,
    center_view: CenterView,
    /// View to restore when leaving Settings (playlist or piano roll).
    settings_return: CenterView,
    settings: AppSettings,
    catalog: PluginCatalog,
    /// Per-track instrument load errors for playlist headers.
    instrument_errors: HashMap<u64, String>,
    /// Session clipboard for notes/clips (Ctrl/Cmd+C/X/V).
    clipboard: EditClipboard,
    /// Snapshot undo/redo for clip and note edits.
    history: EditHistory,
    status_message: String,
    autosave_accum: f32,
    pending_recovery: Option<RecoveryMeta>,
    show_project_browser: bool,
    /// Confirm discard when New is requested while dirty.
    confirm_new_discard: bool,
    /// Force dirty (e.g. after restoring a recovery backup that has no clean disk match).
    dirty_forced: bool,
}

impl DawApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load_or_defaults(&Self::settings_path());
        let mut catalog = PluginCatalog::load_or_defaults(&Self::plugin_cache_path());
        catalog.extra_paths = settings.plugin_extra_paths.clone();

        let pending_recovery = load_recovery_meta();
        let (project, current_path, show_browser, status_message) =
            Self::startup_project(&settings, pending_recovery.is_some());

        let engine = AudioEngine::new(project.beats_per_second());
        let status_message = if !engine.audio_available() {
            let detail = engine.init_error().unwrap_or("unknown error");
            format!("Audio unavailable ({detail}). Transport still works silently.")
        } else if pending_recovery.is_some() {
            String::from("Unsaved recovery found — choose Restore or Discard.")
        } else {
            status_message
        };

        let project_name = current_path
            .as_ref()
            .map(|p| project_display_name(p))
            .unwrap_or_else(|| String::from("Untitled"));
        let saved_snapshot = project.clone();
        let undo_limit = settings.undo_limit;

        let mut app = Self {
            project,
            saved_snapshot,
            current_path,
            project_name,
            engine,
            playlist: PlaylistUi::default(),
            piano_roll: PianoRollUi::default(),
            settings_ui: SettingsUi::default(),
            project_browser: ProjectBrowserUi::default(),
            center_view: CenterView::Playlist,
            settings_return: CenterView::Playlist,
            settings,
            catalog,
            instrument_errors: HashMap::new(),
            clipboard: EditClipboard::Empty,
            history: EditHistory::new(undo_limit),
            status_message,
            autosave_accum: 0.0,
            pending_recovery,
            show_project_browser: show_browser,
            confirm_new_discard: false,
            dirty_forced: false,
        };
        app.sync_instruments();
        app
    }

    /// Choose initial project: recovery deferral, recent, legacy CWD, or empty + browser.
    fn startup_project(
        settings: &AppSettings,
        has_recovery: bool,
    ) -> (Project, Option<PathBuf>, bool, String) {
        if has_recovery {
            // Keep a blank session until the user restores or discards.
            return (
                Project::default(),
                None,
                false,
                String::from("Recovery pending"),
            );
        }

        if let Some(path) = settings.recent_projects.first() {
            if path.exists() {
                match load_project_from(path) {
                    Ok(project) => {
                        return (
                            project,
                            Some(path.clone()),
                            false,
                            format!("Opened {}", path.display()),
                        );
                    }
                    Err(error) => {
                        return (
                            Project::default(),
                            None,
                            true,
                            format!("Recent open failed: {error}"),
                        );
                    }
                }
            }
        }

        let legacy = legacy_project_path();
        if legacy.exists() {
            match load_project_from(&legacy) {
                Ok(project) => {
                    return (
                        project,
                        Some(legacy),
                        false,
                        String::from("Loaded legacy project.json"),
                    );
                }
                Err(error) => {
                    return (
                        Project::default(),
                        None,
                        true,
                        format!("Legacy load failed: {error}"),
                    );
                }
            }
        }

        (
            Project::default(),
            None,
            true,
            String::from(
                "New project. Use File -> Save As to choose a .motif path, or open a recent project.",
            ),
        )
    }

    fn settings_path() -> PathBuf {
        PathBuf::from(SETTINGS_FILE)
    }

    fn plugin_cache_path() -> PathBuf {
        PathBuf::from(PLUGIN_CACHE_FILE)
    }

    fn dirty(&self) -> bool {
        self.dirty_forced || self.project != self.saved_snapshot
    }

    fn sync_instruments(&mut self) {
        let updates = self.engine.sync_instruments(&self.project, &self.catalog);
        let mut dirty = false;
        for (track_id, error) in updates {
            dirty = true;
            if error.is_empty() {
                self.instrument_errors.remove(&track_id);
            } else {
                self.instrument_errors.insert(track_id, error);
            }
        }
        let before_len = self.instrument_errors.len();
        self.instrument_errors
            .retain(|track_id, _| self.project.tracks.iter().any(|t| t.id == *track_id));
        if self.instrument_errors.len() != before_len {
            dirty = true;
        }
        if dirty {
            self.playlist
                .set_instrument_errors(self.instrument_errors.clone());
        }
    }

    fn save_plugin_cache(&mut self) {
        if let Err(error) = self.catalog.save_to_path(&Self::plugin_cache_path()) {
            self.status_message = format!("Plugin cache save failed: {error}");
        }
    }

    fn save_settings(&mut self) {
        match self.settings.save_to_path(&Self::settings_path()) {
            Ok(()) => {
                // Quiet success for frequent autosave-related writes; only announce explicit saves.
            }
            Err(error) => {
                self.status_message = format!("Settings save failed: {error}");
            }
        }
    }

    fn remember_recent(&mut self, path: PathBuf) {
        push_recent(&mut self.settings.recent_projects, path);
        self.save_settings();
    }

    fn mark_clean(&mut self) {
        self.saved_snapshot = self.project.clone();
        self.dirty_forced = false;
        self.autosave_accum = 0.0;
        let _ = clear_recovery();
    }

    fn apply_loaded_project(&mut self, project: Project, path: Option<PathBuf>) {
        self.engine.stop();
        self.engine.all_notes_off();
        self.engine.set_beats_per_second(project.beats_per_second());
        self.project = project;
        self.saved_snapshot = self.project.clone();
        self.dirty_forced = false;
        self.history.clear();
        self.center_view = CenterView::Playlist;
        self.settings_return = CenterView::Playlist;
        self.playlist.clear_selection();
        self.piano_roll.release_audition(&mut self.engine);
        self.piano_roll.clear_selection();
        self.settings_ui.clear_capture();
        self.current_path = path.clone();
        self.project_name = path
            .as_ref()
            .map(|p| project_display_name(p))
            .unwrap_or_else(|| String::from("Untitled"));
        self.autosave_accum = 0.0;
        self.confirm_new_discard = false;
        self.sync_instruments();
    }

    fn save(&mut self) {
        if self.current_path.is_some() {
            self.save_to_current_path();
        } else {
            self.save_as();
        }
    }

    fn save_to_current_path(&mut self) {
        let Some(path) = self.current_path.clone() else {
            self.save_as();
            return;
        };
        self.engine.capture_plugin_states(&mut self.project);
        match save_project_to(&path, &self.project) {
            Ok(()) => {
                self.project_name = project_display_name(&path);
                self.mark_clean();
                self.remember_recent(path.clone());
                self.status_message = format!("Saved {}", path.display());
            }
            Err(error) => self.status_message = format!("Save failed: {error}"),
        }
    }

    fn save_as(&mut self) {
        let start_dir = projects_dir().ok();
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Motif project", &[PROJECT_EXTENSION])
            .set_file_name(format!("{}.{}", self.project_name, PROJECT_EXTENSION));
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            self.status_message = String::from("Save As cancelled");
            return;
        };
        let path = ensure_motif_extension(path);
        self.engine.capture_plugin_states(&mut self.project);
        match save_project_to(&path, &self.project) {
            Ok(()) => {
                self.current_path = Some(path.clone());
                self.project_name = project_display_name(&path);
                self.mark_clean();
                self.remember_recent(path.clone());
                self.status_message = format!("Saved {}", path.display());
            }
            Err(error) => self.status_message = format!("Save As failed: {error}"),
        }
    }

    fn open_dialog(&mut self) {
        let start_dir = projects_dir().ok();
        let mut dialog =
            rfd::FileDialog::new().add_filter("Motif project", &[PROJECT_EXTENSION, "json"]);
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else {
            self.status_message = String::from("Open cancelled");
            return;
        };
        self.open_path(&path);
    }

    fn open_path(&mut self, path: &Path) {
        match load_project_from(path) {
            Ok(project) => {
                self.apply_loaded_project(project, Some(path.to_path_buf()));
                self.remember_recent(path.to_path_buf());
                let _ = clear_recovery();
                self.pending_recovery = None;
                self.status_message = format!("Opened {}", path.display());
            }
            Err(error) => self.status_message = format!("Open failed: {error}"),
        }
    }

    fn request_new_project(&mut self) {
        if self.dirty() {
            self.confirm_new_discard = true;
        } else {
            self.new_project();
        }
    }

    fn new_project(&mut self) {
        self.apply_loaded_project(Project::default(), None);
        let _ = clear_recovery();
        self.pending_recovery = None;
        self.status_message = String::from("New project");
    }

    fn write_recovery_backup(&mut self) {
        self.engine.capture_plugin_states(&mut self.project);
        match write_recovery(
            &self.project,
            self.current_path.as_deref(),
            &self.project_name,
        ) {
            Ok(()) => self.status_message = String::from("Recovery saved"),
            Err(error) => self.status_message = format!("Recovery save failed: {error}"),
        }
    }

    fn tick_autosave(&mut self, delta_seconds: f32) {
        if !self.settings.autosave_enabled || !self.dirty() {
            self.autosave_accum = 0.0;
            return;
        }
        self.autosave_accum += delta_seconds;
        let interval = self.settings.autosave_interval_secs.max(30) as f32;
        if self.autosave_accum >= interval {
            self.autosave_accum = 0.0;
            self.write_recovery_backup();
        }
    }

    fn restore_recovery(&mut self) {
        let meta = match self.pending_recovery.take() {
            Some(meta) => meta,
            None => return,
        };
        match load_recovery_project() {
            Ok(project) => {
                let path = meta.original_path.filter(|p| p.exists());
                self.apply_loaded_project(project, path);
                // Restored content is unsaved until the user Saves.
                self.dirty_forced = true;
                self.project_name = meta.project_name;
                self.status_message = String::from("Restored recovery - save to keep");
            }
            Err(error) => {
                self.pending_recovery = Some(meta);
                self.status_message = format!("Restore failed: {error}");
            }
        }
    }

    fn discard_recovery(&mut self) {
        let _ = clear_recovery();
        self.pending_recovery = None;
        // After discard, offer the normal startup path (recent / empty + browser).
        let (project, path, show_browser, status) =
            Self::startup_project(&self.settings, false);
        self.apply_loaded_project(project, path);
        self.show_project_browser = show_browser;
        self.status_message = if status.starts_with("Opened") || status.starts_with("Loaded") {
            format!("Discarded recovery. {status}")
        } else {
            String::from("Discarded recovery")
        };
    }

    fn update_window_title(&self, ctx: &egui::Context) {
        let dirty_mark = if self.dirty() { " *" } else { "" };
        let title = format!("Motif - {}{dirty_mark}", self.project_name);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn show_recovery_modal(&mut self, ctx: &egui::Context) {
        let Some(meta) = self.pending_recovery.clone() else {
            return;
        };
        let when = format_unix_time(meta.saved_at_unix);
        let name = meta.project_name.clone();
        let mut restore = false;
        let mut discard = false;

        egui::Window::new("Recover unsaved project")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Motif found unsaved changes from {when} ({name})."
                ));
                ui.label("Restore them, or discard the recovery backup?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Restore").clicked() {
                        restore = true;
                    }
                    if ui.button("Discard").clicked() {
                        discard = true;
                    }
                });
            });

        if restore {
            self.restore_recovery();
        } else if discard {
            self.discard_recovery();
        }
    }

    fn show_new_discard_modal(&mut self, ctx: &egui::Context) {
        if !self.confirm_new_discard {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new("Discard unsaved changes?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("The current project has unsaved changes. Start a new project anyway?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Discard and new").clicked() {
                        confirm = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if confirm {
            self.confirm_new_discard = false;
            self.new_project();
        } else if cancel {
            self.confirm_new_discard = false;
        }
    }

    fn handle_project_browser_action(&mut self, action: ProjectBrowserAction) {
        match action {
            ProjectBrowserAction::New => self.request_new_project(),
            ProjectBrowserAction::OpenPath(path) => self.open_path(&path),
            ProjectBrowserAction::OpenDialog => self.open_dialog(),
            ProjectBrowserAction::RemoveRecent(path) => {
                self.settings.recent_projects.retain(|p| p != &path);
                self.save_settings();
                self.status_message = format!("Removed {} from recent", path.display());
            }
            ProjectBrowserAction::Close => {}
        }
    }

    fn show_file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                self.request_new_project();
                ui.close_menu();
            }
            if ui.button("Open...").clicked() {
                self.open_dialog();
                ui.close_menu();
            }
            ui.menu_button("Open Recent", |ui| {
                if self.settings.recent_projects.is_empty() {
                    ui.weak("No recent projects");
                } else {
                    let recent = self.settings.recent_projects.clone();
                    for path in recent {
                        let label = format!(
                            "{} — {}",
                            project_display_name(&path),
                            path.display()
                        );
                        let enabled = path.exists();
                        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                            self.open_path(&path);
                            ui.close_menu();
                        }
                    }
                }
            });
            if ui.button("Projects...").clicked() {
                self.show_project_browser = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Save").clicked() {
                self.save();
                ui.close_menu();
            }
            if ui.button("Save As...").clicked() {
                self.save_as();
                ui.close_menu();
            }
        });
    }

    fn prune_ui_after_history(&mut self) {
        self.engine.set_beats_per_second(self.project.beats_per_second());
        self.playlist.prune_selection(&self.project);
        if let CenterView::PianoRoll { clip_id } = self.center_view {
            if self.project.clip(clip_id).is_none() {
                self.back_to_playlist();
            } else {
                self.piano_roll.prune_selection(clip_id, &self.project);
            }
        }
        if matches!(self.settings_return, CenterView::PianoRoll { clip_id } if self.project.clip(clip_id).is_none())
        {
            self.settings_return = CenterView::Playlist;
        }
        self.sync_instruments();
    }

    fn undo_edit(&mut self) {
        if !self.history.can_undo() {
            self.status_message = String::from("Nothing to undo");
            return;
        }
        if !self.history.undo(&mut self.project) {
            return;
        }
        self.prune_ui_after_history();
        self.status_message = String::from("Undo");
    }

    fn redo_edit(&mut self) {
        if !self.history.can_redo() {
            self.status_message = String::from("Nothing to redo");
            return;
        }
        if !self.history.redo(&mut self.project) {
            return;
        }
        self.prune_ui_after_history();
        self.status_message = String::from("Redo");
    }

    fn delete_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self
            .piano_roll
            .selected_note_ids()
            .iter()
            .copied()
            .collect();
        if ids.is_empty() {
            return;
        }
        self.history.push_before(self.project.clone());
        self.remove_notes_from_clip(clip_id, &ids);
        self.piano_roll.clear_selection();
    }

    fn remove_notes_from_clip(&mut self, clip_id: u64, ids: &[u64]) {
        if let Some(clip) = self.project.clip_mut(clip_id) {
            for id in ids {
                clip.remove_note(*id);
            }
        }
    }

    fn delete_selected_clips(&mut self) {
        let ids: Vec<u64> = self.playlist.selected_clip_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        self.history.push_before(self.project.clone());
        self.remove_clips(&ids);
        self.playlist.clear_selection();
    }

    fn remove_clips(&mut self, ids: &[u64]) {
        for id in ids {
            self.project.remove_clip(*id);
            if matches!(self.center_view, CenterView::PianoRoll { clip_id } if clip_id == *id) {
                self.center_view = CenterView::Playlist;
                self.piano_roll.clear_selection();
            }
            if matches!(self.settings_return, CenterView::PianoRoll { clip_id } if clip_id == *id) {
                self.settings_return = CenterView::Playlist;
            }
        }
    }

    fn duplicate_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self
            .piano_roll
            .selected_note_ids()
            .iter()
            .copied()
            .collect();
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
        self.history.push_before(self.project.clone());
        let new_ids = self.project.duplicate_notes_in_clip(clip_id, &ids, span, 0);
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
        self.history.push_before(self.project.clone());
        let new_ids = self.project.duplicate_clips(&ids, span);
        if !new_ids.is_empty() {
            self.playlist.set_selection(new_ids);
        }
    }

    fn copy_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self
            .piano_roll
            .selected_note_ids()
            .iter()
            .copied()
            .collect();
        if ids.is_empty() {
            return;
        }
        let notes = self.project.notes_for_clipboard(clip_id, &ids);
        self.clipboard = EditClipboard::from_notes(&notes);
        if !self.clipboard.is_empty() {
            self.status_message = format!("Copied {} note(s)", notes.len());
        }
    }

    fn copy_selected_clips(&mut self) {
        let ids: Vec<u64> = self.playlist.selected_clip_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let clips = self.project.clips_for_clipboard(&ids);
        if clips.is_empty() {
            return;
        }
        let count = clips.len();
        self.clipboard = EditClipboard::Clips(clips);
        self.status_message = format!("Copied {count} clip(s)");
    }

    fn paste_notes_at_playhead(&mut self, clip_id: u64) {
        let EditClipboard::Notes(notes) = &self.clipboard else {
            if self.clipboard.is_empty() {
                self.status_message = String::from("Clipboard empty");
            } else {
                self.status_message = String::from("Clipboard has clips - paste in playlist");
            }
            return;
        };
        let notes = notes.clone();
        let clip_start = self
            .project
            .clip(clip_id)
            .map(|clip| clip.start_beats)
            .unwrap_or(0.0);
        let origin = (self.engine.current_beats() - clip_start).max(0.0);
        let before = self.project.clone();
        let new_ids = self.project.paste_notes_into_clip(clip_id, &notes, origin);
        if new_ids.is_empty() {
            self.project = before;
            return;
        }
        self.history.push_before(before);
        self.piano_roll.set_selection(new_ids);
        self.status_message = format!("Pasted {} note(s)", notes.len());
    }

    fn paste_clips_at_playhead(&mut self) {
        let EditClipboard::Clips(clips) = &self.clipboard else {
            if self.clipboard.is_empty() {
                self.status_message = String::from("Clipboard empty");
            } else {
                self.status_message = String::from("Clipboard has notes - paste in piano roll");
            }
            return;
        };
        let clips = clips.clone();
        let origin = self.engine.current_beats();
        let before = self.project.clone();
        let new_ids = self.project.paste_clips(&clips, origin);
        if new_ids.is_empty() {
            self.project = before;
            self.status_message = String::from("Paste failed (missing track?)");
            return;
        }
        self.history.push_before(before);
        self.playlist.set_selection(new_ids);
        self.status_message = format!("Pasted {} clip(s)", clips.len());
    }

    fn cut_selected_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        let ids: Vec<u64> = self
            .piano_roll
            .selected_note_ids()
            .iter()
            .copied()
            .collect();
        if ids.is_empty() {
            return;
        }
        let notes = self.project.notes_for_clipboard(clip_id, &ids);
        let count = notes.len();
        if count == 0 {
            return;
        }
        self.history.push_before(self.project.clone());
        self.clipboard = EditClipboard::from_notes(&notes);
        self.remove_notes_from_clip(clip_id, &ids);
        self.piano_roll.clear_selection();
        self.status_message = format!("Cut {count} note(s)");
    }

    fn cut_selected_clips(&mut self) {
        let ids: Vec<u64> = self.playlist.selected_clip_ids().iter().copied().collect();
        if ids.is_empty() {
            return;
        }
        let clips = self.project.clips_for_clipboard(&ids);
        let count = clips.len();
        if count == 0 {
            return;
        }
        self.history.push_before(self.project.clone());
        self.clipboard = EditClipboard::Clips(clips);
        self.remove_clips(&ids);
        self.playlist.clear_selection();
        self.status_message = format!("Cut {count} clip(s)");
    }

    fn open_clip(&mut self, clip_id: u64) {
        if self.project.clip(clip_id).is_some() {
            self.piano_roll.clear_selection();
            self.center_view = CenterView::PianoRoll { clip_id };
        }
    }

    fn handle_plugin_editor_request(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        request: PluginEditorRequest,
    ) {
        match request {
            PluginEditorRequest::Open { track_id, title } => {
                let host_x11 = host_x11_from_frame(frame);
                let forward = self.plugin_forward_transport(track_id);
                match self
                    .engine
                    .open_plugin_editor(track_id, &title, host_x11, forward)
                {
                    Ok(()) => {
                        self.status_message = format!("Opened plugin editor: {title}");
                        ctx.request_repaint();
                    }
                    Err(error) => {
                        self.status_message = format!("Plugin editor: {error}");
                    }
                }
            }
            PluginEditorRequest::Close { track_id } => {
                self.engine.close_plugin_editor(track_id);
                self.status_message = String::from("Closed plugin editor");
            }
        }
    }

    /// Plugin `unique_id` for a track, if it hosts a plugin instrument.
    fn track_plugin_unique_id(&self, track_id: u64) -> Option<String> {
        self.project
            .tracks
            .iter()
            .find(|track| track.id == track_id)
            .and_then(|track| match &track.instrument {
                crate::model::TrackInstrument::Plugin { unique_id, .. } => Some(unique_id.clone()),
                crate::model::TrackInstrument::BuiltInPiano => None,
            })
    }

    /// Effective "forward Space to Motif" setting for a track's plugin.
    fn plugin_forward_transport(&self, track_id: u64) -> bool {
        match self.track_plugin_unique_id(track_id) {
            Some(unique_id) => self
                .settings
                .plugin_keys
                .forward_transport_for(&unique_id),
            None => self.settings.plugin_keys.forward_transport_default,
        }
    }

    /// Toggle Space forwarding for a track's plugin: persist the override and
    /// apply it live to the open editor.
    fn set_plugin_forward_transport(&mut self, track_id: u64, forward: bool) {
        if let Some(unique_id) = self.track_plugin_unique_id(track_id) {
            self.settings
                .plugin_keys
                .set_forward_transport_for(&unique_id, forward);
            self.save_settings();
        }
        self.engine.set_plugin_editor_transport(track_id, forward);
    }

    /// Always-visible row of open plugin editors with a close button and a
    /// per-plugin "Space -> Motif" transport-forward toggle. Gives a reliable
    /// close under WMs (e.g. Hyprland) that draw no titlebar cross.
    fn show_open_editors_strip(&mut self, ui: &mut egui::Ui) {
        let editors = self.engine.open_plugin_editors();
        if editors.is_empty() {
            return;
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("Plugin editors:");
            for (track_id, title) in editors {
                ui.group(|ui| {
                    ui.label(&title);
                    let mut forward = self.plugin_forward_transport(track_id);
                    if ui
                        .checkbox(&mut forward, "Space -> Motif")
                        .on_hover_text(
                            "On: Space drives Motif play/pause while this editor is focused.\n\
                             Off: Space goes to the plugin. (Ctrl+W always closes the editor.)",
                        )
                        .changed()
                    {
                        self.set_plugin_forward_transport(track_id, forward);
                    }
                    if ui
                        .button("Close")
                        .on_hover_text("Close this plugin editor (or press Ctrl+W in it)")
                        .clicked()
                    {
                        self.engine.close_plugin_editor(track_id);
                    }
                });
            }
        });
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
        // Block project shortcuts while recovery / discard modals are up.
        if self.pending_recovery.is_some() || self.confirm_new_discard {
            if matches!(action, Action::BackToPlaylist) {
                // Escape does not dismiss recovery (must choose Restore/Discard).
            }
            return;
        }
        match action {
            Action::TogglePlayback => self.engine.toggle_playback(),
            Action::DeleteSelection => match self.center_view {
                CenterView::Playlist => self.delete_selected_clips(),
                CenterView::PianoRoll { .. } => self.delete_selected_notes(),
                CenterView::Settings => {}
            },
            Action::CopySelection => match self.center_view {
                CenterView::Playlist => self.copy_selected_clips(),
                CenterView::PianoRoll { .. } => self.copy_selected_notes(),
                CenterView::Settings => {}
            },
            Action::CutSelection => match self.center_view {
                CenterView::Playlist => self.cut_selected_clips(),
                CenterView::PianoRoll { .. } => self.cut_selected_notes(),
                CenterView::Settings => {}
            },
            Action::PasteSelection => match self.center_view {
                CenterView::Playlist => self.paste_clips_at_playhead(),
                CenterView::PianoRoll { clip_id } => self.paste_notes_at_playhead(clip_id),
                CenterView::Settings => {}
            },
            Action::DuplicateSelection => match self.center_view {
                CenterView::Playlist => self.duplicate_selected_clips(),
                CenterView::PianoRoll { .. } => self.duplicate_selected_notes(),
                CenterView::Settings => {}
            },
            Action::Undo => match self.center_view {
                CenterView::Settings => {}
                _ => self.undo_edit(),
            },
            Action::Redo => match self.center_view {
                CenterView::Settings => {}
                _ => self.redo_edit(),
            },
            Action::Save => self.save(),
            Action::Open => self.open_dialog(),
            Action::SaveProjectAs => self.save_as(),
            Action::NewProject => self.request_new_project(),
            Action::OpenProjectBrowser => self.show_project_browser = true,
            Action::BackToPlaylist => match self.center_view {
                CenterView::Settings => self.close_settings(),
                CenterView::PianoRoll { .. } => self.back_to_playlist(),
                CenterView::Playlist => {}
            },
        }
    }
}

impl eframe::App for DawApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let delta_seconds = ctx.input(|input| input.unstable_dt);
        let loop_end = self.project.loop_end_beats;
        self.sync_instruments();
        self.engine.advance(delta_seconds, loop_end);
        self.engine.schedule_project(&self.project);
        let editor_poll = self.engine.poll_plugin_editors();
        if editor_poll.any_open {
            ctx.request_repaint();
        }
        if editor_poll.toggle_playback {
            self.dispatch_action(Action::TogglePlayback);
        }

        if self.pending_recovery.is_none() && !self.confirm_new_discard {
            self.tick_autosave(delta_seconds);
        }

        let poll_filter = if self.pending_recovery.is_some() || self.confirm_new_discard {
            PollFilter::None
        } else if self.settings_ui.is_capturing() {
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
        self.update_window_title(ctx);

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

                self.show_file_menu(ui);

                if !matches!(self.center_view, CenterView::Settings)
                    && ui.button("Settings").clicked()
                {
                    self.open_settings();
                }

                let dirty_mark = if self.dirty() { " *" } else { "" };
                ui.label(format!("{}{dirty_mark}", self.project_name));

                if let CenterView::PianoRoll { clip_id } = self.center_view {
                    if let Some(clip) = self.project.clip(clip_id) {
                        ui.label(format!("Editing: {}", clip.name));
                    }
                }
                ui.label(&self.status_message);
            });

            self.show_open_editors_strip(ui);
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| match self.center_view {
                CenterView::Playlist => {
                    let (open_clip, editor_request) = {
                        let DawApp {
                            playlist,
                            project,
                            engine,
                            settings,
                            catalog,
                            history,
                            ..
                        } = self;
                        playlist.show(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            settings.themes.colors(),
                        );
                        (
                            playlist.take_open_clip_request(),
                            playlist.take_plugin_editor_request(),
                        )
                    };
                    if let Some(clip_id) = open_clip {
                        self.open_clip(clip_id);
                    }
                    if let Some(request) = editor_request {
                        self.handle_plugin_editor_request(ctx, frame, request);
                    }
                }
                CenterView::PianoRoll { clip_id } => {
                    if self.project.clip(clip_id).is_some() {
                        let DawApp {
                            piano_roll,
                            project,
                            engine,
                            settings,
                            history,
                            ..
                        } = self;
                        piano_roll.show(
                            ui,
                            clip_id,
                            project,
                            engine,
                            history,
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
                            &mut self.catalog,
                            &mut self.settings.plugin_extra_paths,
                            &mut self.settings.plugin_keys,
                            &mut self.settings.undo_limit,
                            &mut self.settings.autosave_enabled,
                            &mut self.settings.autosave_interval_secs,
                            &mut self.settings.recent_projects,
                        ) {
                            Some(SettingsAction::Back) => self.close_settings(),
                            Some(SettingsAction::ShortcutsChanged)
                            | Some(SettingsAction::ThemeChanged)
                            | Some(SettingsAction::PluginKeysChanged)
                            | Some(SettingsAction::ProjectChanged) => self.save_settings(),
                            Some(SettingsAction::EditingChanged) => {
                                self.history.set_limit(self.settings.undo_limit);
                                self.save_settings();
                            }
                            Some(SettingsAction::PluginsChanged) => {
                                self.save_settings();
                                self.save_plugin_cache();
                                self.engine.invalidate_instruments();
                                self.instrument_errors.clear();
                                self.sync_instruments();
                            }
                            None => {}
                        }
                    });
                }
            });

        if self.pending_recovery.is_some() {
            self.show_recovery_modal(ctx);
        } else {
            self.show_new_discard_modal(ctx);
            if let Some(action) = self.project_browser.show(
                ctx,
                &mut self.show_project_browser,
                &self.settings.recent_projects,
            ) {
                self.handle_project_browser_action(action);
            }
        }

        ctx.request_repaint();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty() && self.settings.autosave_enabled {
            self.write_recovery_backup();
        }
    }
}

#[cfg(target_os = "linux")]
fn host_x11_from_frame(frame: &eframe::Frame) -> Option<crate::engine::HostX11> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

    let display_handle = frame.display_handle().ok()?;
    let window_handle = frame.window_handle().ok()?;

    // Confirm we are under X11/XWayland and grab the screen index. The editor
    // parent opens its own connection, so we do not keep winit's Display pointer.
    let screen = match display_handle.as_raw() {
        RawDisplayHandle::Xlib(xlib) => xlib.screen,
        _ => return None,
    };

    let transient_for = match window_handle.as_raw() {
        RawWindowHandle::Xlib(xlib) => Some(xlib.window as u64),
        RawWindowHandle::Xcb(xcb) => Some(u64::from(xcb.window.get())),
        _ => None,
    };

    Some(crate::engine::HostX11 {
        screen,
        transient_for,
    })
}

#[cfg(not(target_os = "linux"))]
fn host_x11_from_frame(_frame: &eframe::Frame) -> Option<crate::engine::HostX11> {
    None
}
