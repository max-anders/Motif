//! Settings center view: sidebar categories for theme, plugins, shortcuts, and editing.

use std::path::{Path, PathBuf};

use egui::Ui;

use crate::engine::PluginCatalog;
use crate::model::{MAX_UNDO_LIMIT, MIN_UNDO_LIMIT};

use super::shortcuts::{
    Action, ApplyChordOutcome, CaptureOutcome, Chord, ShortcutRegistry,
};
use super::theme::{ThemeCatalog, DEFAULT_THEME_NAME};

/// Where a newly captured chord should be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureTarget {
    Replace(usize),
    Add(Action),
}

/// Chord that conflicts with another action; waiting for Yes/No.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingConflict {
    target: CaptureTarget,
    chord: Chord,
    with: Action,
}

/// Settings categories shown in the left nav.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsSection {
    #[default]
    Theme,
    Plugins,
    Shortcuts,
    Editing,
}

impl SettingsSection {
    const ALL: [Self; 4] = [
        Self::Theme,
        Self::Plugins,
        Self::Shortcuts,
        Self::Editing,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Plugins => "Plugins",
            Self::Shortcuts => "Shortcuts",
            Self::Editing => "Editing",
        }
    }
}

#[derive(Debug, Default)]
pub struct SettingsUi {
    section: SettingsSection,
    /// Replace or add target waiting for a new chord.
    capturing: Option<CaptureTarget>,
    /// Conflict confirmation after capture (chord already used elsewhere).
    pending_conflict: Option<PendingConflict>,
    message: String,
    save_name: String,
    extra_path_draft: String,
}

pub enum SettingsAction {
    Back,
    ShortcutsChanged,
    ThemeChanged,
    PluginsChanged,
    EditingChanged,
}

impl SettingsUi {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some() || self.pending_conflict.is_some()
    }

    pub fn clear_capture(&mut self) {
        self.capturing = None;
        self.pending_conflict = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        shortcuts: &mut ShortcutRegistry,
        themes: &mut ThemeCatalog,
        catalog: &mut PluginCatalog,
        plugin_extra_paths: &mut Vec<PathBuf>,
        undo_limit: &mut usize,
    ) -> Option<SettingsAction> {
        let mut result = None;

        ui.horizontal(|ui| {
            ui.heading("Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Back").clicked() {
                    self.clear_capture();
                    result = Some(SettingsAction::Back);
                }
            });
        });
        ui.add_space(8.0);

        let available = ui.available_size();
        ui.allocate_ui_with_layout(
            available,
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                // Left category nav
                ui.allocate_ui_with_layout(
                    egui::vec2(160.0, available.y),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(160.0);
                        ui.set_max_width(160.0);
                        ui.strong("Categories");
                        ui.add_space(4.0);
                        for section in SettingsSection::ALL {
                            let selected = self.section == section;
                            if ui
                                .selectable_label(selected, section.label())
                                .clicked()
                                && !selected
                            {
                                if self.section == SettingsSection::Shortcuts {
                                    self.clear_capture();
                                }
                                self.section = section;
                                self.message.clear();
                            }
                        }
                    },
                );

                ui.separator();

                // Active category content
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("settings_section_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.section {
                                SettingsSection::Theme => {
                                    if let Some(action) = self.show_theme_section(ui, themes) {
                                        result = Some(action);
                                    }
                                }
                                SettingsSection::Plugins => {
                                    if let Some(action) =
                                        self.show_plugins_section(ui, catalog, plugin_extra_paths)
                                    {
                                        result = Some(action);
                                    }
                                }
                                SettingsSection::Shortcuts => {
                                    if let Some(action) =
                                        self.show_shortcuts_section(ui, shortcuts)
                                    {
                                        result = Some(action);
                                    }
                                }
                                SettingsSection::Editing => {
                                    if let Some(action) =
                                        self.show_editing_section(ui, undo_limit)
                                    {
                                        result = Some(action);
                                    }
                                }
                            });
                    },
                );
            },
        );

        if !self.message.is_empty() {
            ui.add_space(6.0);
            ui.label(&self.message);
        }

        result
    }

    fn show_editing_section(
        &mut self,
        ui: &mut Ui,
        undo_limit: &mut usize,
    ) -> Option<SettingsAction> {
        let mut result = None;

        ui.heading("Editing");
        ui.label("Undo keeps project snapshots for clip and note edits. Older steps drop when the limit is reached.");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label("Undo steps");
            let mut value = *undo_limit as u32;
            let response = ui.add(
                egui::DragValue::new(&mut value)
                    .range(MIN_UNDO_LIMIT as u32..=MAX_UNDO_LIMIT as u32)
                    .speed(1.0),
            );
            if response.changed() {
                *undo_limit = value as usize;
                result = Some(SettingsAction::EditingChanged);
            }
        });
        ui.label(format!(
            "Range {MIN_UNDO_LIMIT}-{MAX_UNDO_LIMIT}. Default is 50."
        ));

        result
    }

    fn show_plugins_section(
        &mut self,
        ui: &mut Ui,
        catalog: &mut PluginCatalog,
        plugin_extra_paths: &mut Vec<PathBuf>,
    ) -> Option<SettingsAction> {
        let mut result = None;

        ui.heading("Plugin Manager");
        ui.label(
            "Scan native Linux CLAP/VST3 instruments. Plugin editors open via track header menu (need X11 or XWayland). Do not add yabridge paths — scanning them aborts Motif.",
        );
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(format!(
                "Instruments cached: {}",
                catalog.instrument_count()
            ));
            if let Some(ts) = catalog.scanned_at_unix {
                ui.label(format!("Last scan (unix): {ts}"));
            } else {
                ui.label("Never scanned");
            }
            if ui.button("Rescan").clicked() {
                catalog.extra_paths = plugin_extra_paths.clone();
                catalog.rescan();
                self.message = format!(
                    "Scan complete: {} instrument(s)",
                    catalog.instrument_count()
                );
                result = Some(SettingsAction::PluginsChanged);
            }
        });

        if let Some(error) = &catalog.last_error {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }

        ui.add_space(8.0);
        ui.strong("Extra scan paths");
        ui.label("Optional native plugin directories (not yabridge). Saved with settings.");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.extra_path_draft)
                    .desired_width(320.0)
                    .hint_text(format!(
                        "{}/.clap",
                        std::env::var("HOME").unwrap_or_else(|_| "~".into())
                    )),
            );
            if ui.button("Add path").clicked() {
                let draft = self.extra_path_draft.trim().to_string();
                if draft.is_empty() {
                    self.message = "Enter a directory path before clicking Add path".into();
                } else {
                    let path = PathBuf::from(&draft);
                    if path_contains_yabridge(&path) {
                        self.message = format!(
                            "Refused: {draft} is a yabridge path (would crash Motif on Rescan)"
                        );
                    } else if plugin_extra_paths.iter().any(|p| p == &path) {
                        self.message = format!("Already listed: {draft}");
                    } else {
                        plugin_extra_paths.push(path);
                        self.extra_path_draft.clear();
                        catalog.extra_paths = plugin_extra_paths.clone();
                        self.message = format!("Added: {draft}");
                        result = Some(SettingsAction::PluginsChanged);
                    }
                }
            }
            if ui
                .add_enabled(
                    !plugin_extra_paths.is_empty(),
                    egui::Button::new("Clear paths"),
                )
                .clicked()
            {
                plugin_extra_paths.clear();
                catalog.extra_paths.clear();
                self.message = "Cleared extra scan paths".into();
                result = Some(SettingsAction::PluginsChanged);
            }
        });

        if plugin_extra_paths.is_empty() {
            ui.weak("No extra paths yet.");
        } else {
            ui.add_space(4.0);
            let mut remove_index = None;
            for (index, path) in plugin_extra_paths.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.monospace(format!("{}. {}", index + 1, path.display()));
                    if ui.small_button("Remove").clicked() {
                        remove_index = Some(index);
                    }
                });
            }
            if let Some(index) = remove_index {
                let removed = plugin_extra_paths.remove(index);
                catalog.extra_paths = plugin_extra_paths.clone();
                self.message = format!("Removed: {}", removed.display());
                result = Some(SettingsAction::PluginsChanged);
            }
        }

        if self.message.starts_with("Added:")
            || self.message.starts_with("Refused:")
            || self.message.starts_with("Already listed:")
            || self.message.starts_with("Enter a directory")
            || self.message.starts_with("Cleared ")
            || self.message.starts_with("Removed:")
        {
            ui.add_space(4.0);
            let color = if self.message.starts_with("Refused:")
                || self.message.starts_with("Enter a directory")
            {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().strong_text_color()
            };
            ui.colored_label(color, &self.message);
        }

        if catalog.instrument_count() > 0 {
            ui.add_space(8.0);
            ui.strong("Cached instruments");
            egui::ScrollArea::vertical()
                .id_salt("settings_plugin_list")
                .max_height(180.0)
                .show(ui, |ui| {
                    for entry in &catalog.entries {
                        ui.label(format!(
                            "{} [{}] — {}",
                            entry.name,
                            entry.format_badge(),
                            entry.vendor
                        ));
                    }
                });
        }

        result
    }

    fn show_theme_section(
        &mut self,
        ui: &mut Ui,
        themes: &mut ThemeCatalog,
    ) -> Option<SettingsAction> {
        let mut result = None;

        ui.heading("Theme");
        ui.label("Colors used across Motif. Edit slots, then Save as to keep a named theme.");
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Active theme");
            let mut selected = themes.active_name().to_string();
            let names = themes.theme_names();
            egui::ComboBox::from_id_salt("active_theme_combo")
                .selected_text(&selected)
                .show_ui(ui, |ui| {
                    for name in &names {
                        if ui.selectable_label(selected == *name, name).clicked() {
                            selected = name.clone();
                        }
                    }
                });
            if selected != themes.active_name() && themes.set_active(&selected) {
                self.message = format!("Theme \"{selected}\" active");
                result = Some(SettingsAction::ThemeChanged);
            }

            if ui
                .add_enabled(
                    themes.active_name() != DEFAULT_THEME_NAME,
                    egui::Button::new("Delete theme"),
                )
                .clicked()
            {
                let name = themes.active_name().to_string();
                match themes.delete(&name) {
                    Ok(()) => {
                        self.message = format!("Deleted theme \"{name}\"");
                        result = Some(SettingsAction::ThemeChanged);
                    }
                    Err(error) => self.message = error,
                }
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Save as");
            ui.add(
                egui::TextEdit::singleline(&mut self.save_name)
                    .desired_width(180.0)
                    .hint_text("Theme name"),
            );
            if ui.button("Save theme").clicked() {
                match themes.save_as(&self.save_name) {
                    Ok(()) => {
                        self.message = format!("Saved theme \"{}\"", themes.active_name());
                        self.save_name.clear();
                        result = Some(SettingsAction::ThemeChanged);
                    }
                    Err(error) => self.message = error,
                }
            }
            if ui.button("Reset colors to factory").clicked() {
                themes.reset_active_colors_to_factory();
                self.message = "Active theme colors reset to factory Default Dark".into();
                result = Some(SettingsAction::ThemeChanged);
            }
        });

        ui.add_space(8.0);

        let mut changed = false;
        let mut last_group = "";
        for (group, label, color) in themes.colors_mut().editable_slots_mut() {
            if group != last_group {
                if !last_group.is_empty() {
                    ui.add_space(6.0);
                }
                ui.strong(group);
                last_group = group;
            }
            ui.horizontal(|ui| {
                if ui.color_edit_button_srgba(color).changed() {
                    changed = true;
                }
                ui.label(label);
            });
        }

        if changed {
            result = Some(SettingsAction::ThemeChanged);
        }

        result
    }

    fn show_shortcuts_section(
        &mut self,
        ui: &mut Ui,
        shortcuts: &mut ShortcutRegistry,
    ) -> Option<SettingsAction> {
        let mut result = None;
        let busy = self.is_capturing();

        ui.heading("Shortcuts");
        ui.label("Actions can have several keys. Change / Add, then press a chord (Escape cancels).");
        ui.label("If a chord is already used, confirm Override to move it to this action.");
        ui.add_space(4.0);

        if let Some(pending) = self.pending_conflict {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} is already used by \"{}\". Override?",
                    pending.chord.display(),
                    pending.with.label()
                ),
            );
            ui.horizontal(|ui| {
                if ui.button("Override").clicked() {
                    match Self::apply_captured(shortcuts, pending.target, pending.chord, true) {
                        Ok(ApplyChordOutcome::Applied) => {
                            self.message = format!(
                                "Bound to {} (was {})",
                                pending.chord.display(),
                                pending.with.label()
                            );
                            result = Some(SettingsAction::ShortcutsChanged);
                        }
                        Ok(ApplyChordOutcome::Unchanged) => {
                            self.message = "Already bound".into();
                        }
                        Ok(ApplyChordOutcome::Conflict { with }) => {
                            self.message = format!("Still conflicts with {}", with.label());
                        }
                        Err(error) => {
                            self.message = error;
                        }
                    }
                    self.clear_capture();
                }
                if ui.button("Cancel").clicked() {
                    self.pending_conflict = None;
                    self.message = "Override cancelled".into();
                }
            });
        } else if let Some(target) = self.capturing {
            let action_label = match target {
                CaptureTarget::Replace(index) => shortcuts
                    .get(index)
                    .map(|(action, _)| action.label())
                    .unwrap_or("?"),
                CaptureTarget::Add(action) => action.label(),
            };
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Press a new key for \"{action_label}\"... (Escape to cancel)"),
            );
            match ShortcutRegistry::capture_from_context(ui.ctx()) {
                CaptureOutcome::Cancel => {
                    self.capturing = None;
                    self.message = "Rebind cancelled".into();
                }
                CaptureOutcome::Chord(chord) => {
                    match Self::apply_captured(shortcuts, target, chord, false) {
                        Ok(ApplyChordOutcome::Applied) => {
                            self.capturing = None;
                            self.message = format!("Bound to {}", chord.display());
                            result = Some(SettingsAction::ShortcutsChanged);
                        }
                        Ok(ApplyChordOutcome::Unchanged) => {
                            self.capturing = None;
                            self.message = format!("{} already bound", chord.display());
                        }
                        Ok(ApplyChordOutcome::Conflict { with }) => {
                            self.capturing = None;
                            self.pending_conflict = Some(PendingConflict {
                                target,
                                chord,
                                with,
                            });
                            self.message.clear();
                        }
                        Err(error) => {
                            self.message = error;
                        }
                    }
                }
                CaptureOutcome::Pending => {}
            }
        }

        ui.add_space(8.0);
        let actions = shortcuts.actions_in_order();
        for action in actions {
            ui.strong(action.label());
            let indices = shortcuts.indices_for_action(action);
            egui::Grid::new(format!("shortcut_action_{action:?}"))
                .num_columns(3)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for index in indices {
                        let Some((_, binding)) = shortcuts.get(index) else {
                            continue;
                        };
                        ui.label(binding.display());
                        if binding.is_rebindable() {
                            let listening = self.capturing == Some(CaptureTarget::Replace(index));
                            let label = if listening { "Listening..." } else { "Change" };
                            if ui
                                .add_enabled(!busy, egui::Button::new(label))
                                .clicked()
                            {
                                self.capturing = Some(CaptureTarget::Replace(index));
                                self.pending_conflict = None;
                                self.message.clear();
                            }
                            if ui
                                .add_enabled(
                                    !busy && shortcuts.can_remove_binding(index),
                                    egui::Button::new("Remove"),
                                )
                                .clicked()
                            {
                                match shortcuts.remove_binding(index) {
                                    Ok(()) => {
                                        self.message = "Binding removed".into();
                                        result = Some(SettingsAction::ShortcutsChanged);
                                    }
                                    Err(error) => self.message = error,
                                }
                            }
                        } else {
                            ui.label("(system)");
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
            ui.horizontal(|ui| {
                let add_listening = self.capturing == Some(CaptureTarget::Add(action));
                let label = if add_listening {
                    "Listening..."
                } else {
                    "+ Add shortcut"
                };
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    self.capturing = Some(CaptureTarget::Add(action));
                    self.pending_conflict = None;
                    self.message.clear();
                }
            });
            ui.add_space(8.0);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Reset to defaults"))
                .clicked()
            {
                shortcuts.reset_defaults();
                self.clear_capture();
                self.message = "Shortcuts reset to defaults".into();
                result = Some(SettingsAction::ShortcutsChanged);
            }
        });

        result
    }

    fn apply_captured(
        shortcuts: &mut ShortcutRegistry,
        target: CaptureTarget,
        chord: Chord,
        override_conflict: bool,
    ) -> Result<ApplyChordOutcome, String> {
        match target {
            CaptureTarget::Replace(index) => {
                shortcuts.try_set_key_binding(index, chord, override_conflict)
            }
            CaptureTarget::Add(action) => {
                shortcuts.try_add_key_binding(action, chord, override_conflict)
            }
        }
    }
}

fn path_contains_yabridge(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("yabridge"))
    })
}
