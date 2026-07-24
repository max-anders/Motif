//! Settings center view: shortcut remapping + theme colors.

use egui::Ui;

use super::shortcuts::{CaptureOutcome, ShortcutRegistry};
use super::theme::{ThemeCatalog, DEFAULT_THEME_NAME};

#[derive(Debug, Default)]
pub struct SettingsUi {
    /// Binding row index waiting for a new chord.
    capturing: Option<usize>,
    message: String,
    save_name: String,
}

pub enum SettingsAction {
    Back,
    ShortcutsChanged,
    ThemeChanged,
}

impl SettingsUi {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }

    pub fn clear_capture(&mut self) {
        self.capturing = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        shortcuts: &mut ShortcutRegistry,
        themes: &mut ThemeCatalog,
    ) -> Option<SettingsAction> {
        let mut result = None;

        ui.horizontal(|ui| {
            ui.heading("Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Back").clicked() {
                    self.capturing = None;
                    result = Some(SettingsAction::Back);
                }
            });
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(action) = self.show_theme_section(ui, themes) {
                    result = Some(action);
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                if let Some(action) = self.show_shortcuts_section(ui, shortcuts) {
                    result = Some(action);
                }
            });

        if !self.message.is_empty() {
            ui.add_space(6.0);
            ui.label(&self.message);
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

        ui.heading("Shortcuts");
        ui.label("Click Change, then press a key (Escape cancels).");
        ui.add_space(4.0);

        if let Some(index) = self.capturing {
            ui.colored_label(
                // Fallback chrome color; theme accent_warning applied via visuals warn too.
                ui.visuals().warn_fg_color,
                format!(
                    "Press a new key for \"{}\"... (Escape to cancel)",
                    shortcuts
                        .get(index)
                        .map(|(action, _)| action.label())
                        .unwrap_or("?")
                ),
            );
            match ShortcutRegistry::capture_from_context(ui.ctx()) {
                CaptureOutcome::Cancel => {
                    self.capturing = None;
                    self.message = "Rebind cancelled".into();
                }
                CaptureOutcome::Chord(chord) => {
                    match shortcuts.try_set_key_binding(index, chord) {
                        Ok(()) => {
                            self.capturing = None;
                            self.message = format!("Bound to {}", chord.display());
                            result = Some(SettingsAction::ShortcutsChanged);
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
        egui::Grid::new("shortcut_bindings_grid")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Action");
                ui.strong("Binding");
                ui.strong("");
                ui.end_row();

                let rows: Vec<(usize, _, _)> = shortcuts.iter().collect();
                for (index, action, binding) in rows {
                    ui.label(action.label());
                    ui.label(binding.display());
                    if binding.is_rebindable() {
                        let label = if self.capturing == Some(index) {
                            "Listening..."
                        } else {
                            "Change"
                        };
                        if ui
                            .add_enabled(self.capturing.is_none(), egui::Button::new(label))
                            .clicked()
                        {
                            self.capturing = Some(index);
                            self.message.clear();
                        }
                    } else {
                        ui.label("(system)");
                    }
                    ui.end_row();
                }
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.capturing.is_none(), egui::Button::new("Reset to defaults"))
                .clicked()
            {
                shortcuts.reset_defaults();
                self.message = "Shortcuts reset to defaults".into();
                result = Some(SettingsAction::ShortcutsChanged);
            }
        });

        result
    }
}
