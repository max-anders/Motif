use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use eframe::egui;

use crate::engine::{
    decode_audio_file, AudioEngine, DawEngine, DecodedAudio, EditorCloseBinding, LoopPlayback,
    ParamTouchEvent, PluginCatalog, PluginRef, PLUGIN_CACHE_FILE,
};
use crate::model::{
    clear_recovery, ensure_motif_extension, format_unix_time, legacy_project_path,
    load_project_from, load_recovery_meta, load_recovery_project, project_display_name,
    projects_dir, push_recent, save_project_to, write_recovery, EditClipboard, EditHistory,
    Project, TrackInstrument, PROJECT_EXTENSION, RecoveryMeta,
};
use crate::ui::{
    choice_to_instrument, show_inspector, track_name_for_choice, Action, AddBrowserAction,
    AddBrowserUi, AppSettings, AudioImportRequest, BrowserTab, Chord, DevicesUi, MixerUi,
    PerformanceUi, PianoRollUi, PlaylistUi, PluginEditorRequest, PollFilter,
    ProjectBrowserAction, ProjectBrowserUi, SettingsAction, SettingsUi, TransportUi,
    SETTINGS_FILE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CenterView {
    Playlist,
    PianoRoll { clip_id: u64 },
    Mixer,
    Devices,
    Performance,
    Settings,
}

struct AudioDecodeResult {
    path: PathBuf,
    result: Result<Arc<DecodedAudio>, String>,
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
    mixer: MixerUi,
    performance: PerformanceUi,
    devices: DevicesUi,
    settings_ui: SettingsUi,
    project_browser: ProjectBrowserUi,
    add_browser: AddBrowserUi,
    center_view: CenterView,
    /// View to restore when leaving Settings (playlist, mixer, or piano roll).
    settings_return: CenterView,
    settings: AppSettings,
    catalog: PluginCatalog,
    /// Per-track instrument load errors for playlist headers.
    instrument_errors: HashMap<u64, String>,
    /// Per-device (track_id, device_id) insert-FX load errors for the Devices view.
    device_errors: HashMap<(u64, u64), String>,
    /// Shared selection for Mixer + Inspector (playlist header / mixer strip).
    selected_track: Option<u64>,
    /// Toggleable properties panel (same Track facets as Mixer).
    show_inspector: bool,
    /// Bottom device strip (primary Devices UI over playlist / piano roll).
    show_devices_strip: bool,
    /// Session clipboard for notes/clips (Ctrl/Cmd+C/X/V).
    clipboard: EditClipboard,
    /// Snapshot undo/redo for clip and note edits.
    history: EditHistory,
    status_message: String,
    autosave_accum: f32,
    pending_recovery: Option<RecoveryMeta>,
    show_project_browser: bool,
    /// Unified add browser (Instruments / FX / Samples); `Some` while open.
    show_add_browser: Option<BrowserTab>,
    /// Confirm discard when New is requested while dirty.
    confirm_new_discard: bool,
    /// Force dirty (e.g. after restoring a recovery backup that has no clean disk match).
    dirty_forced: bool,
    decoded_audio: HashMap<PathBuf, Arc<DecodedAudio>>,
    pending_audio_decodes: HashSet<PathBuf>,
    audio_decode_errors: HashMap<PathBuf, String>,
    audio_decode_tx: Sender<AudioDecodeResult>,
    audio_decode_rx: Receiver<AudioDecodeResult>,
}

impl DawApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load_or_defaults(&Self::settings_path());
        let mut catalog = PluginCatalog::load_or_defaults(&Self::plugin_cache_path());
        catalog.extra_paths = settings.plugin_extra_paths.clone();

        let pending_recovery = load_recovery_meta();
        let (project, current_path, show_browser, status_message) =
            Self::startup_project(&settings, pending_recovery.is_some());

        let mut engine = AudioEngine::new(project.beats_per_second());
        engine.set_metronome_enabled(settings.metronome_enabled);
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
        let selected_track = project.tracks.first().map(|t| t.id);
        let (audio_decode_tx, audio_decode_rx) = mpsc::channel();

        let mut app = Self {
            project,
            saved_snapshot,
            current_path,
            project_name,
            engine,
            playlist: PlaylistUi::default(),
            piano_roll: PianoRollUi::default(),
            mixer: MixerUi::default(),
            performance: PerformanceUi::default(),
            devices: DevicesUi::default(),
            settings_ui: SettingsUi::default(),
            project_browser: ProjectBrowserUi::default(),
            add_browser: AddBrowserUi::default(),
            center_view: CenterView::Playlist,
            settings_return: CenterView::Playlist,
            settings,
            catalog,
            instrument_errors: HashMap::new(),
            device_errors: HashMap::new(),
            selected_track,
            show_inspector: false,
            show_devices_strip: false,
            clipboard: EditClipboard::Empty,
            history: EditHistory::new(undo_limit),
            status_message,
            autosave_accum: 0.0,
            pending_recovery,
            show_project_browser: show_browser,
            show_add_browser: None,
            confirm_new_discard: false,
            dirty_forced: false,
            decoded_audio: HashMap::new(),
            pending_audio_decodes: HashSet::new(),
            audio_decode_errors: HashMap::new(),
            audio_decode_tx,
            audio_decode_rx,
        };
        app.sync_instruments();
        app.queue_missing_audio_decodes();
        app.sync_plugin_editor_close_binding();
        app
    }

    fn plugin_editor_close_binding(&self) -> EditorCloseBinding {
        self.settings
            .shortcuts
            .primary_key_chord(Action::ClosePluginEditor)
            .and_then(|chord| chord_to_editor_close_binding(chord))
            .unwrap_or_default()
    }

    fn plugin_editor_close_display(&self) -> String {
        self.settings
            .shortcuts
            .primary_key_chord(Action::ClosePluginEditor)
            .map(|chord| chord.display())
            .unwrap_or_else(|| EditorCloseBinding::default().display())
    }

    fn devices_strip_shortcut_hint(&self) -> String {
        let chord = self
            .settings
            .shortcuts
            .primary_key_chord(Action::ToggleDevices)
            .map(|chord| chord.display())
            .unwrap_or_else(|| "D".to_string());
        format!(
            "Show or hide the device strip (instruments, FX, macros, modulators).\n\
             Shortcut: {chord} (remappable in Settings)."
        )
    }

    fn mixer_shortcut_hint(&self) -> String {
        let chord = self
            .settings
            .shortcuts
            .primary_key_chord(Action::ToggleMixer)
            .map(|chord| chord.display())
            .unwrap_or_else(|| "M".to_string());
        format!(
            "Open or close the mixer view.\nShortcut: {chord} (remappable in Settings)."
        )
    }

    fn sync_plugin_editor_close_binding(&mut self) {
        let binding = self.plugin_editor_close_binding();
        self.engine.set_plugin_editor_close_binding(binding);
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

    /// Syncs both per-track instrument voices and per-track insert-FX device
    /// chains every frame (kept in one call site so the two engine syncs
    /// never drift out of step).
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

        let device_updates = self.engine.sync_devices(&self.project, &self.catalog);
        for (track_id, device_id, error) in device_updates {
            if error.is_empty() {
                self.device_errors.remove(&(track_id, device_id));
            } else {
                self.device_errors.insert((track_id, device_id), error);
            }
        }
        self.device_errors.retain(|(track_id, device_id), _| {
            self.project
                .track(*track_id)
                .is_some_and(|track| track.devices.iter().any(|d| d.id == *device_id))
        });
    }

    fn queue_decode(&mut self, path: PathBuf) {
        if self.decoded_audio.contains_key(&path)
            || self.pending_audio_decodes.contains(&path)
            || self.audio_decode_errors.contains_key(&path)
        {
            return;
        }
        let tx = self.audio_decode_tx.clone();
        let decode_path = path.clone();
        let sample_rate = self.engine.sample_rate_hz();
        self.pending_audio_decodes.insert(path.clone());
        self.status_message = format!("Decoding sample: {}", path.display());
        std::thread::spawn(move || {
            let result = decode_audio_file(&decode_path, sample_rate).map(Arc::new);
            let _ = tx.send(AudioDecodeResult {
                path: decode_path,
                result,
            });
        });
    }

    fn queue_missing_audio_decodes(&mut self) {
        let mut pending = Vec::new();
        for track in &self.project.tracks {
            for clip in &track.clips {
                if let Some(audio) = clip.as_audio() {
                    pending.push(audio.source.clone());
                }
            }
        }
        for path in pending {
            self.queue_decode(path);
        }
    }

    fn sync_audio_clip_decode_state(&mut self) {
        for track in &mut self.project.tracks {
            for clip in &mut track.clips {
                if let Some(audio) = clip.as_audio_mut() {
                    audio.missing = self.audio_decode_errors.contains_key(&audio.source);
                }
            }
        }
    }

    fn apply_decoded_audio(&mut self, path: &Path, decoded: Arc<DecodedAudio>) {
        let duration_beats = decoded.duration_seconds() * self.project.beats_per_second();
        let length_beats = Project::snap_beats(duration_beats.max(crate::model::SNAP_BEATS));
        self.decoded_audio.insert(path.to_path_buf(), Arc::clone(&decoded));
        for track in &mut self.project.tracks {
            for clip in &mut track.clips {
                let Some(audio) = clip.as_audio_mut() else {
                    continue;
                };
                if audio.source == path {
                    if audio.length_beats <= crate::model::DEFAULT_CLIP_LENGTH_BEATS {
                        audio.length_beats = length_beats;
                    }
                    audio.missing = false;
                }
            }
        }
    }

    fn poll_audio_decodes(&mut self) {
        loop {
            match self.audio_decode_rx.try_recv() {
                Ok(result) => {
                    self.pending_audio_decodes.remove(&result.path);
                    match result.result {
                        Ok(decoded) => {
                            self.audio_decode_errors.remove(&result.path);
                            self.apply_decoded_audio(&result.path, decoded);
                            self.status_message =
                                format!("Decoded sample: {}", result.path.display());
                        }
                        Err(error) => {
                            self.audio_decode_errors.insert(result.path.clone(), error.clone());
                            self.status_message =
                                format!("Sample decode failed ({}): {error}", result.path.display());
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        self.sync_audio_clip_decode_state();
    }

    fn import_audio_clip(&mut self, request: AudioImportRequest) {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a", "aac"]);
        if let Some(dir) = projects_dir().ok() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else {
            self.status_message = String::from("Import sample cancelled");
            return;
        };
        self.add_audio_clip_from_path(request.track_id, request.start_beats, path);
    }

    fn sample_import_target(&self) -> AudioImportRequest {
        let track_id = self
            .selected_track
            .or_else(|| self.project.tracks.first().map(|track| track.id))
            .unwrap_or(0);
        AudioImportRequest {
            track_id,
            start_beats: Project::snap_beats(self.engine.current_beats().max(0.0)),
        }
    }

    fn add_audio_clip_from_path(&mut self, track_id: u64, start_beats: f32, path: PathBuf) {
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Sample")
            .to_string();
        let mut length_beats = crate::model::DEFAULT_CLIP_LENGTH_BEATS;
        if let Some(decoded) = self.decoded_audio.get(&path) {
            length_beats = Project::snap_beats(
                (decoded.duration_seconds() * self.project.beats_per_second())
                    .max(crate::model::SNAP_BEATS),
            );
        } else {
            self.queue_decode(path.clone());
        }
        if self.project.track(track_id).is_none() {
            self.status_message = String::from("Import failed: track not found");
            return;
        }
        if !self
            .project
            .clip_range_free(track_id, start_beats, length_beats, &[])
        {
            self.status_message = String::from("Import failed: overlaps existing clip");
            return;
        }
        self.history.push_before(self.project.clone());
        if let Some(clip_id) = self.project.add_audio_clip_to_track(
            track_id,
            path.clone(),
            name,
            start_beats,
            length_beats,
        ) {
            self.playlist.set_selection([clip_id]);
            self.selected_track = Some(track_id);
            self.settings.push_recent_sample(path.clone());
            self.save_settings();
            self.status_message = format!("Imported sample: {}", path.display());
        } else {
            self.status_message = String::from("Import failed: overlaps existing clip");
        }
    }

    fn open_add_browser(&mut self, tab: BrowserTab) {
        self.add_browser.prepare_open(tab);
        self.show_add_browser = Some(tab);
    }

    fn handle_add_browser_action(&mut self, action: AddBrowserAction) {
        match action {
            AddBrowserAction::CreateTrack(choice) => {
                let number = self.project.tracks.len() + 1;
                let name = track_name_for_choice(&choice, number);
                let instrument = choice_to_instrument(choice);
                self.history.push_before(self.project.clone());
                let track_id = self.project.add_track(&name, instrument);
                self.selected_track = Some(track_id);
                self.status_message = format!("Added track: {name}");
            }
            AddBrowserAction::AddEffect(entry) => {
                let Some(track_id) = self
                    .selected_track
                    .or_else(|| self.project.tracks.first().map(|track| track.id))
                else {
                    self.status_message = String::from("Add FX failed: no tracks");
                    return;
                };
                self.history.push_before(self.project.clone());
                if self
                    .project
                    .add_device(track_id, entry.format, &entry.unique_id, &entry.name)
                    .is_some()
                {
                    self.selected_track = Some(track_id);
                    self.status_message = format!("Added FX: {}", entry.name);
                } else {
                    self.status_message = String::from("Add FX failed: track not found");
                }
            }
            AddBrowserAction::AddSample(path) => {
                let request = self.sample_import_target();
                self.add_audio_clip_from_path(request.track_id, request.start_beats, path);
            }
            AddBrowserAction::BrowseSample => {
                let request = self.sample_import_target();
                self.import_audio_clip(request);
            }
            AddBrowserAction::Close => {}
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

    /// Apply plugin-GUI param touches from the audio thread into last-tweaked MRU.
    fn apply_param_touches(&mut self, touches: &[ParamTouchEvent]) {
        if touches.is_empty() {
            return;
        }
        let mut dirty = false;
        for touch in touches {
            let Some(track) = self.project.track(touch.track_id) else {
                continue;
            };
            let unique_id = match touch.device_id {
                None => match &track.instrument {
                    TrackInstrument::Plugin { unique_id, .. } if !unique_id.is_empty() => {
                        Some(unique_id.clone())
                    }
                    _ => None,
                },
                Some(device_id) => track
                    .devices
                    .iter()
                    .find(|device| device.id == device_id)
                    .map(|device| device.unique_id.clone())
                    .filter(|uid| !uid.is_empty()),
            };
            let Some(unique_id) = unique_id else {
                continue;
            };
            let name = self
                .engine
                .plugin_parameters(touch.track_id, touch.device_id)
                .into_iter()
                .find(|param| param.id == touch.param_id)
                .map(|param| param.name)
                .unwrap_or_default();
            if self
                .settings
                .touch_param(&unique_id, touch.param_id, name)
            {
                dirty = true;
            }
        }
        if dirty {
            self.save_settings();
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
        self.engine.reset_audio_state();
        self.engine.set_beats_per_second(project.beats_per_second());
        self.project = project;
        self.saved_snapshot = self.project.clone();
        self.dirty_forced = false;
        self.history.clear();
        self.center_view = CenterView::Playlist;
        self.settings_return = CenterView::Playlist;
        self.playlist.clear_selection();
        self.selected_track = self.project.tracks.first().map(|t| t.id);
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
        self.decoded_audio.clear();
        self.pending_audio_decodes.clear();
        self.audio_decode_errors.clear();
        self.sync_instruments();
        self.queue_missing_audio_decodes();
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
        self.engine.capture_device_states(&mut self.project);
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
        self.engine.capture_device_states(&mut self.project);
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
        self.engine.capture_device_states(&mut self.project);
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
        if let Some(track_id) = self.selected_track {
            if self.project.track(track_id).is_none() {
                self.selected_track = self.project.tracks.first().map(|t| t.id);
            }
        }
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
        self.engine.sync_samples(&self.project, &self.decoded_audio);
        self.engine.sync_channels(&self.project);
        self.engine.sync_automation(&self.project);
        self.engine.sync_modulators(&self.project);
        self.engine.sync_macros(&self.project);
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
        if let Some(clip) = self.project.midi_clip_mut(clip_id) {
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

    fn delete_track(&mut self, track_id: u64) {
        if !self.project.can_remove_track() || self.project.track(track_id).is_none() {
            return;
        }
        let clip_ids: Vec<u64> = self
            .project
            .track(track_id)
            .map(|track| track.clips.iter().map(|clip| clip.id()).collect())
            .unwrap_or_default();

        self.history.push_before(self.project.clone());

        for id in &clip_ids {
            if matches!(self.center_view, CenterView::PianoRoll { clip_id } if clip_id == *id) {
                self.center_view = CenterView::Playlist;
                self.piano_roll.clear_selection();
            }
            if matches!(self.settings_return, CenterView::PianoRoll { clip_id } if clip_id == *id)
            {
                self.settings_return = CenterView::Playlist;
            }
        }

        let removed = self.project.remove_track(track_id);
        debug_assert!(removed, "delete_track preconditions should guarantee removal");
        self.playlist.prune_selection(&self.project);
        if self.selected_track == Some(track_id) {
            self.selected_track = self.project.tracks.first().map(|t| t.id);
        }
        self.engine.close_plugin_editor(PluginRef::instrument(track_id));
        self.engine.all_notes_off();
        self.instrument_errors.remove(&track_id);
        self.device_errors.retain(|(tid, _), _| *tid != track_id);
        self.sync_instruments();
        self.engine.sync_samples(&self.project, &self.decoded_audio);
        self.engine.sync_channels(&self.project);
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
            let Some(clip) = self.project.midi_clip(clip_id) else {
                return;
            };
            Project::selection_span_beats(ids.iter().filter_map(|id| {
                clip.note(*id)
                    .map(|note| (note.start_beats, note.end_beats()))
            }))
        };
        self.history.push_before(self.project.clone());
        let new_ids = self.project.duplicate_notes_in_clip(clip_id, &ids, span, 0, false);
        if !new_ids.is_empty() {
            self.piano_roll.set_selection(new_ids);
        }
    }

    fn select_all_notes(&mut self) {
        let CenterView::PianoRoll { clip_id } = self.center_view else {
            return;
        };
        self.piano_roll.select_all_in_clip(clip_id, &self.project);
    }

    fn transpose_selected_notes(&mut self, delta_semitones: i32) {
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
        let before = self.project.clone();
        if self
            .project
            .transpose_notes_in_clip(clip_id, &ids, delta_semitones)
        {
            self.history.push_before(before);
            self.piano_roll.prune_selection(clip_id, &self.project);
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
                .map(|clip| (clip.start_beats(), clip.end_beats()))
        }));
        self.history.push_before(self.project.clone());
        let new_ids: Vec<u64> = self
            .project
            .duplicate_clips(&ids, span, false)
            .into_iter()
            .map(|(_, id)| id)
            .collect();
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
        let replace = self.piano_roll.all_notes_selected_in_clip(clip_id, &self.project);
        let clip_start = self
            .project
            .clip(clip_id)
            .map(|clip| clip.start_beats())
            .unwrap_or(0.0);
        let origin = if replace {
            0.0
        } else {
            (self.engine.current_beats() - clip_start).max(0.0)
        };
        let before = self.project.clone();
        if replace {
            let existing: Vec<u64> = self
                .project
                .midi_clip(clip_id)
                .map(|clip| clip.notes.iter().map(|note| note.id).collect())
                .unwrap_or_default();
            self.remove_notes_from_clip(clip_id, &existing);
        }
        let new_ids = self.project.paste_notes_into_clip(clip_id, &notes, origin);
        if new_ids.is_empty() {
            self.project = before;
            self.status_message = String::from("Paste failed (overlap)");
            return;
        }
        self.history.push_before(before);
        self.piano_roll.set_selection(new_ids);
        self.status_message = if replace {
            format!("Replaced clip notes with {} note(s)", notes.len())
        } else {
            format!("Pasted {} note(s)", notes.len())
        };
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
            self.status_message =
                String::from("Paste failed (overlap or missing track)");
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
        if self.project.midi_clip(clip_id).is_some() {
            self.piano_roll.clear_selection();
            self.piano_roll.request_fit_horizontal();
            self.center_view = CenterView::PianoRoll { clip_id };
        }
    }

    fn toggle_selected_track_plugin_editor(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
    ) {
        let Some(track_id) = self.selected_track else {
            self.status_message = String::from("No track selected");
            return;
        };
        let Some(track) = self.project.track(track_id) else {
            self.status_message = String::from("Selected track no longer exists");
            return;
        };
        let target = PluginRef::instrument(track_id);
        if !matches!(
            track.instrument,
            crate::model::TrackInstrument::Plugin { .. }
        ) {
            self.status_message = String::from("Selected track has no plugin instrument");
            return;
        }
        if self.engine.plugin_editor_is_open(target) {
            self.handle_plugin_editor_request(
                ctx,
                frame,
                PluginEditorRequest::Close {
                    track_id,
                    device_id: None,
                },
            );
        } else if !self.engine.plugin_slot_ready(target) {
            self.status_message = String::from("Plugin editor not ready (still loading)");
        } else {
            self.handle_plugin_editor_request(
                ctx,
                frame,
                PluginEditorRequest::Open {
                    track_id,
                    device_id: None,
                    title: track.name.clone(),
                },
            );
        }
    }

    fn close_selected_track_plugin_editor(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
    ) {
        let Some(track_id) = self.selected_track else {
            return;
        };
        let target = PluginRef::instrument(track_id);
        if self.engine.plugin_editor_is_open(target) {
            self.handle_plugin_editor_request(
                ctx,
                frame,
                PluginEditorRequest::Close {
                    track_id,
                    device_id: None,
                },
            );
        }
    }

    fn handle_plugin_editor_request(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        request: PluginEditorRequest,
    ) {
        match request {
            PluginEditorRequest::Open {
                track_id,
                device_id,
                title,
            } => {
                let target = PluginRef { track_id, device_id };
                let host_x11 = host_x11_from_frame(frame);
                let forward = self.plugin_forward_transport(target);
                match self.engine.open_plugin_editor(target, &title, host_x11, forward) {
                    Ok(()) => {
                        self.status_message = format!("Opened plugin editor: {title}");
                        ctx.request_repaint();
                    }
                    Err(error) => {
                        self.status_message = format!("Plugin editor: {error}");
                    }
                }
            }
            PluginEditorRequest::Close { track_id, device_id } => {
                self.engine
                    .close_plugin_editor(PluginRef { track_id, device_id });
                self.status_message = String::from("Closed plugin editor");
            }
        }
    }

    /// Plugin `unique_id` for a slot, if it hosts a plugin (instrument or device).
    fn plugin_unique_id_for(&self, target: PluginRef) -> Option<String> {
        let track = self
            .project
            .tracks
            .iter()
            .find(|track| track.id == target.track_id)?;
        match target.device_id {
            None => match &track.instrument {
                crate::model::TrackInstrument::Plugin { unique_id, .. } => Some(unique_id.clone()),
                crate::model::TrackInstrument::BuiltInPiano => None,
            },
            Some(device_id) => track
                .devices
                .iter()
                .find(|device| device.id == device_id)
                .map(|device| device.unique_id.clone()),
        }
    }

    /// Effective "forward Space to Motif" setting for a slot's plugin.
    fn plugin_forward_transport(&self, target: PluginRef) -> bool {
        match self.plugin_unique_id_for(target) {
            Some(unique_id) => self
                .settings
                .plugin_keys
                .forward_transport_for(&unique_id),
            None => self.settings.plugin_keys.forward_transport_default,
        }
    }

    /// Toggle Space forwarding for a slot's plugin: persist the override and
    /// apply it live to the open editor.
    fn set_plugin_forward_transport(&mut self, target: PluginRef, forward: bool) {
        if let Some(unique_id) = self.plugin_unique_id_for(target) {
            self.settings
                .plugin_keys
                .set_forward_transport_for(&unique_id, forward);
            self.save_settings();
        }
        self.engine.set_plugin_editor_transport(target, forward);
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
            for (target, title) in editors {
                ui.group(|ui| {
                    ui.label(&title);
                    let mut forward = self.plugin_forward_transport(target);
                    if ui
                        .checkbox(&mut forward, "Space -> Motif")
                        .on_hover_text(format!(
                            "On: Space drives Motif play/pause while this editor is focused.\n\
                             Off: Space goes to the plugin. ({} always closes the editor.)",
                            self.plugin_editor_close_display(),
                        ))
                        .changed()
                    {
                        self.set_plugin_forward_transport(target, forward);
                    }
                    if ui
                        .button("Close")
                        .on_hover_text(format!(
                            "Close this plugin editor (or press {} in it)",
                            self.plugin_editor_close_display(),
                        ))
                        .clicked()
                    {
                        self.engine.close_plugin_editor(target);
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

    fn open_mixer(&mut self) {
        if matches!(self.center_view, CenterView::Mixer) {
            return;
        }
        if matches!(self.center_view, CenterView::PianoRoll { .. }) {
            self.piano_roll.release_audition(&mut self.engine);
        }
        if self.selected_track.is_none() {
            self.selected_track = self.project.tracks.first().map(|t| t.id);
        }
        self.center_view = CenterView::Mixer;
    }

    fn open_performance(&mut self) {
        if matches!(self.center_view, CenterView::Performance) {
            return;
        }
        if matches!(self.center_view, CenterView::PianoRoll { .. }) {
            self.piano_roll.release_audition(&mut self.engine);
        }
        self.center_view = CenterView::Performance;
    }

    fn open_devices(&mut self) {
        if matches!(self.center_view, CenterView::Devices) {
            return;
        }
        if matches!(self.center_view, CenterView::PianoRoll { .. }) {
            self.piano_roll.release_audition(&mut self.engine);
        }
        if self.selected_track.is_none() {
            self.selected_track = self.project.tracks.first().map(|t| t.id);
        }
        self.center_view = CenterView::Devices;
    }

    fn toggle_devices_strip(&mut self) {
        self.show_devices_strip = !self.show_devices_strip;
        if self.show_devices_strip && self.selected_track.is_none() {
            self.selected_track = self.project.tracks.first().map(|t| t.id);
        }
    }

    fn devices_strip_visible(&self) -> bool {
        self.show_devices_strip
            && matches!(
                self.center_view,
                CenterView::Playlist | CenterView::PianoRoll { .. }
            )
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
            Action::PauseInPlace => {
                if self.engine.is_playing() {
                    self.engine.pause_in_place();
                } else {
                    self.engine.play();
                }
            }
            Action::ToggleLoop => {
                self.project.loop_enabled = !self.project.loop_enabled;
            }
            Action::DeleteSelection => match self.center_view {
                CenterView::Playlist => self.delete_selected_clips(),
                CenterView::PianoRoll { .. } => self.delete_selected_notes(),
                CenterView::Mixer
                | CenterView::Devices
                | CenterView::Performance
                | CenterView::Settings => {}
            },
            Action::CopySelection => match self.center_view {
                CenterView::Playlist => self.copy_selected_clips(),
                CenterView::PianoRoll { .. } => self.copy_selected_notes(),
                CenterView::Mixer
                | CenterView::Devices
                | CenterView::Performance
                | CenterView::Settings => {}
            },
            Action::CutSelection => match self.center_view {
                CenterView::Playlist => self.cut_selected_clips(),
                CenterView::PianoRoll { .. } => self.cut_selected_notes(),
                CenterView::Mixer
                | CenterView::Devices
                | CenterView::Performance
                | CenterView::Settings => {}
            },
            Action::PasteSelection => match self.center_view {
                CenterView::Playlist => self.paste_clips_at_playhead(),
                CenterView::PianoRoll { clip_id } => self.paste_notes_at_playhead(clip_id),
                CenterView::Mixer
                | CenterView::Devices
                | CenterView::Performance
                | CenterView::Settings => {}
            },
            Action::DuplicateSelection => match self.center_view {
                CenterView::Playlist => self.duplicate_selected_clips(),
                CenterView::PianoRoll { .. } => self.duplicate_selected_notes(),
                CenterView::Mixer
                | CenterView::Devices
                | CenterView::Performance
                | CenterView::Settings => {}
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
            Action::OpenInstrumentBrowser => self.open_add_browser(BrowserTab::Instruments),
            Action::OpenEffectBrowser => self.open_add_browser(BrowserTab::Fx),
            Action::OpenSampleBrowser => self.open_add_browser(BrowserTab::Samples),
            Action::BackToPlaylist => {
                if self.show_add_browser.is_some() {
                    self.show_add_browser = None;
                    return;
                }
                match self.center_view {
                    CenterView::Settings => self.close_settings(),
                    CenterView::PianoRoll { .. }
                    | CenterView::Mixer
                    | CenterView::Devices
                    | CenterView::Performance => {
                        self.back_to_playlist()
                    }
                    CenterView::Playlist => {}
                }
            }
            Action::ToggleMixer => match self.center_view {
                CenterView::Mixer => self.back_to_playlist(),
                CenterView::Settings => {}
                _ => self.open_mixer(),
            },
            Action::TogglePerformance => match self.center_view {
                CenterView::Performance => self.back_to_playlist(),
                CenterView::Settings => {}
                _ => self.open_performance(),
            },
            Action::ToggleDevices => match self.center_view {
                CenterView::Settings => {}
                _ => self.toggle_devices_strip(),
            },
            Action::DeleteTrack => {
                // Handled in `update` after track headers report pointer hover.
            }
            Action::TogglePluginEditor => {
                // Handled in `update` (needs egui Context + Frame for native editor).
            }
            Action::ClosePluginEditor => {
                // Handled in `update` when closing from the main Motif window.
            }
            Action::SelectAll => match self.center_view {
                CenterView::PianoRoll { .. } => self.select_all_notes(),
                _ => {}
            },
            Action::TransposeUpSemitone => match self.center_view {
                CenterView::PianoRoll { .. } => self.transpose_selected_notes(1),
                _ => {}
            },
            Action::TransposeDownSemitone => match self.center_view {
                CenterView::PianoRoll { .. } => self.transpose_selected_notes(-1),
                _ => {}
            },
            Action::TransposeUpOctave => match self.center_view {
                CenterView::PianoRoll { .. } => self.transpose_selected_notes(12),
                _ => {}
            },
            Action::TransposeDownOctave => match self.center_view {
                CenterView::PianoRoll { .. } => self.transpose_selected_notes(-12),
                _ => {}
            },
            Action::ExclusiveSolo => self.exclusive_solo_selected_track(),
            Action::ExclusiveMute => self.exclusive_mute_selected_track(),
        }
    }

    fn exclusive_solo_selected_track(&mut self) {
        let Some(track_id) = self.selected_track else {
            return;
        };
        if self.project.track(track_id).is_none() {
            return;
        }
        self.history.push_before(self.project.clone());
        self.project.exclusive_solo(track_id);
        self.engine.all_notes_off();
    }

    fn exclusive_mute_selected_track(&mut self) {
        let Some(track_id) = self.selected_track else {
            return;
        };
        if self.project.track(track_id).is_none() {
            return;
        }
        self.history.push_before(self.project.clone());
        self.project.exclusive_mute(track_id);
        self.engine.all_notes_off();
    }
}

impl eframe::App for DawApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Only guards the silent-degraded path (no audio thread), where the
        // engine still advances the playhead from this delta: a UI stall would
        // otherwise report the whole stall as one frame and jump the transport
        // across the arrangement. With audio available the engine derives the
        // playhead from the callback's sample counter and ignores this value.
        const MAX_TRANSPORT_DELTA_SECS: f32 = 0.1;
        let delta_seconds = ctx
            .input(|input| input.unstable_dt)
            .min(MAX_TRANSPORT_DELTA_SECS);
        let playback = LoopPlayback {
            enabled: self.project.loop_enabled,
            start_beats: self.project.loop_start_beats,
            end_beats: self.project.loop_end_beats,
            content_end_beats: self.project.content_end_beats(),
        };
        self.poll_audio_decodes();
        self.queue_missing_audio_decodes();
        self.sync_instruments();
        self.engine.sync_samples(&self.project, &self.decoded_audio);
        self.engine.sync_channels(&self.project);
        self.engine.sync_automation(&self.project);
        self.engine.sync_modulators(&self.project);
        self.engine.sync_macros(&self.project);
        self.engine.advance(delta_seconds, playback);
        self.engine.schedule_project(&self.project);
        let param_touches = self.engine.drain_param_touches();
        self.apply_param_touches(&param_touches);
        if !param_touches.is_empty() {
            ctx.request_repaint();
        }
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
        let mut delete_hovered_track = false;
        for action in self.settings.shortcuts.poll(ctx, poll_filter) {
            if action == Action::TogglePluginEditor {
                self.toggle_selected_track_plugin_editor(ctx, frame);
            } else if action == Action::ClosePluginEditor {
                self.close_selected_track_plugin_editor(ctx, frame);
            } else if action == Action::DeleteTrack {
                // Needs current-frame header hover from playlist/devices paint.
                delete_hovered_track = true;
            } else {
                self.dispatch_action(action);
            }
        }

        self.settings.themes.colors().apply_to_context(ctx);
        self.update_window_title(ctx);

        egui::TopBottomPanel::top("transport_panel").show(ctx, |ui| {
            ui.heading("Motif");
            ui.add_space(4.0);
            if TransportUi::show(ui, &mut self.project, &mut self.engine) {
                self.settings.metronome_enabled = self.engine.metronome_enabled();
                self.save_settings();
            }

            ui.separator();
            ui.horizontal(|ui| {
                match self.center_view {
                    CenterView::Settings => {
                        if ui.button("Back").clicked() {
                            self.close_settings();
                        }
                        ui.separator();
                    }
                    CenterView::PianoRoll { .. }
                    | CenterView::Mixer
                    | CenterView::Devices
                    | CenterView::Performance => {
                        if ui.button("Back to playlist").clicked() {
                            self.back_to_playlist();
                        }
                        ui.separator();
                    }
                    CenterView::Playlist => {}
                }

                self.show_file_menu(ui);

                if !matches!(self.center_view, CenterView::Settings | CenterView::Mixer)
                    && ui
                        .button("Mixer")
                        .on_hover_text(self.mixer_shortcut_hint())
                        .clicked()
                {
                    self.open_mixer();
                }

                if !matches!(
                    self.center_view,
                    CenterView::Settings | CenterView::Performance
                ) && ui.button("Perf").clicked()
                {
                    self.open_performance();
                }

                if !matches!(self.center_view, CenterView::Settings | CenterView::Devices)
                    && ui
                        .button(if self.show_devices_strip {
                            "Hide devices"
                        } else {
                            "Devices"
                        })
                        .on_hover_text(self.devices_strip_shortcut_hint())
                        .clicked()
                {
                    self.toggle_devices_strip();
                }

                if matches!(self.center_view, CenterView::Playlist | CenterView::Mixer)
                    && ui
                        .button(if self.show_inspector {
                            "Hide inspector"
                        } else {
                            "Inspector"
                        })
                        .clicked()
                {
                    self.show_inspector = !self.show_inspector;
                    if self.show_inspector && self.selected_track.is_none() {
                        self.selected_track = self.project.tracks.first().map(|t| t.id);
                    }
                }

                if !matches!(self.center_view, CenterView::Settings)
                    && ui.button("Settings").clicked()
                {
                    self.open_settings();
                }

                let dirty_mark = if self.dirty() { " *" } else { "" };
                ui.label(format!("{}{dirty_mark}", self.project_name));

                if let CenterView::PianoRoll { clip_id } = self.center_view {
                    if let Some(clip) = self.project.clip(clip_id) {
                        ui.label(format!("Editing: {}", clip.name()));
                    }
                }
                if matches!(self.center_view, CenterView::Mixer) {
                    ui.label("Mixer");
                }
                if matches!(self.center_view, CenterView::Devices) {
                    ui.label("Devices");
                }
                if matches!(self.center_view, CenterView::Performance) {
                    ui.label("Performance");
                }
                ui.label(&self.status_message);
            });

            self.show_open_editors_strip(ui);
        });

        if self.show_inspector
            && matches!(self.center_view, CenterView::Playlist | CenterView::Mixer)
        {
            egui::SidePanel::right("track_inspector")
                .default_width(260.0)
                .min_width(200.0)
                .show(ctx, |ui| {
                    let theme = self.settings.themes.colors().clone();
                    show_inspector(
                        ui,
                        &mut self.project,
                        &mut self.history,
                        self.selected_track,
                        &theme,
                    );
                });
        }

        if self.devices_strip_visible() {
            // Only the LFO column stretches: with it closed the dock is pinned to an
            // exact width so it always snaps back instead of keeping a dragged width.
            let plan = {
                let DawApp {
                    devices,
                    project,
                    selected_track,
                    settings,
                    ..
                } = self;
                let dock_track = selected_track.and_then(|id| project.track(id));
                devices.dock_panel_width(dock_track, settings)
            };
            let panel = egui::SidePanel::right("devices_dock");
            let panel = if plan.resizable {
                panel
                    .default_width(plan.default_width)
                    .width_range(plan.min_width..=plan.max_width)
                    .resizable(true)
            } else {
                panel.exact_width(plan.default_width).resizable(false)
            };
            let panel_response = panel.show(ctx, |ui| {
                    let theme = self.settings.themes.colors().clone();
                    let strip_output = {
                        let DawApp {
                            devices,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            selected_track,
                            settings,
                            ..
                        } = self;
                        devices.show_strip(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            selected_track,
                            settings,
                            &theme,
                        )
                    };
                    if strip_output.hide {
                        self.show_devices_strip = false;
                    }
                    if strip_output.expand {
                        self.open_devices();
                    }
                    if strip_output.settings_dirty {
                        self.save_settings();
                    }
                    if let Some(request) = self.devices.take_plugin_editor_request() {
                        self.handle_plugin_editor_request(ctx, frame, request);
                    }
                });
            self.devices
                .note_dock_panel_width(panel_response.response.rect.width());
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| match self.center_view {
                CenterView::Playlist => {
                    let (open_clip, editor_request, delete_track, import_audio, hovered_header, settings_dirty) = {
                        let DawApp {
                            playlist,
                            project,
                            engine,
                            settings,
                            catalog,
                            history,
                            selected_track,
                            decoded_audio,
                            ..
                        } = self;
                        let theme = settings.themes.colors().clone();
                        let settings_dirty = playlist.show(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            selected_track,
                            decoded_audio,
                            settings,
                            &theme,
                        );
                        (
                            playlist.take_open_clip_request(),
                            playlist.take_plugin_editor_request(),
                            playlist.take_delete_track_request(),
                            playlist.take_audio_import_request(),
                            playlist.hovered_track_header(),
                            settings_dirty,
                        )
                    };
                    if settings_dirty {
                        self.save_settings();
                    }
                    if let Some(clip_id) = open_clip {
                        self.open_clip(clip_id);
                    }
                    if let Some(request) = editor_request {
                        self.handle_plugin_editor_request(ctx, frame, request);
                    }
                    if let Some(track_id) = delete_track {
                        self.delete_track(track_id);
                    } else if delete_hovered_track {
                        if let Some(track_id) = hovered_header {
                            self.delete_track(track_id);
                        }
                    }
                    if let Some(request) = import_audio {
                        self.import_audio_clip(request);
                    }
                }
                CenterView::Mixer => {
                    let DawApp {
                        mixer,
                        project,
                        engine,
                        settings,
                        history,
                        selected_track,
                        ..
                    } = self;
                    mixer.show(
                        ui,
                        project,
                        engine,
                        history,
                        selected_track,
                        settings.themes.colors(),
                    );
                }
                CenterView::Devices => {
                    let (editor_request, open_clip, delete_track, hovered_header, settings_dirty) = {
                        let DawApp {
                            devices,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            settings,
                            selected_track,
                            decoded_audio,
                            ..
                        } = self;
                        let theme = settings.themes.colors().clone();
                        let settings_dirty = devices.show(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            selected_track,
                            decoded_audio,
                            settings,
                            &theme,
                        );
                        (
                            devices.take_plugin_editor_request(),
                            devices.take_open_clip_request(),
                            devices.take_delete_track_request(),
                            devices.hovered_track_header(),
                            settings_dirty,
                        )
                    };
                    if settings_dirty {
                        self.save_settings();
                    }
                    if let Some(request) = editor_request {
                        self.handle_plugin_editor_request(ctx, frame, request);
                    }
                    if let Some(clip_id) = open_clip {
                        self.open_clip(clip_id);
                    }
                    if let Some(track_id) = delete_track {
                        self.delete_track(track_id);
                    } else if delete_hovered_track {
                        if let Some(track_id) = hovered_header {
                            self.delete_track(track_id);
                        }
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
                CenterView::Performance => {
                    let DawApp {
                        performance,
                        project,
                        engine,
                        settings,
                        ..
                    } = self;
                    performance.show(ui, project, engine, settings.themes.colors());
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
                            Some(SettingsAction::ShortcutsChanged) => {
                                self.sync_plugin_editor_close_binding();
                                self.save_settings();
                            }
                            Some(SettingsAction::ThemeChanged)
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
            if let Some(action) = {
                let DawApp {
                    add_browser,
                    show_add_browser,
                    catalog,
                    settings,
                    ..
                } = self;
                add_browser.show(
                    ctx,
                    show_add_browser,
                    catalog,
                    &settings.recent_samples,
                )
            } {
                self.handle_add_browser_action(action);
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

fn chord_to_editor_close_binding(chord: Chord) -> Option<EditorCloseBinding> {
    let key_name = chord.storage_key_name()?.to_string();
    Some(EditorCloseBinding {
        key_name,
        ctrl_or_cmd: chord.ctrl_or_cmd,
        shift: chord.shift,
        alt: chord.alt,
    })
}
