//! Basic Settings screen (shortcuts section for now).

use egui::Ui;

use super::shortcuts::{
    CaptureOutcome, ShortcutRegistry,
};

#[derive(Debug, Default)]
pub struct SettingsUi {
    /// Binding row index waiting for a new chord.
    capturing: Option<usize>,
    message: String,
}

pub enum SettingsAction {
    Back,
    ShortcutsChanged,
}

impl SettingsUi {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }

    pub fn clear_capture(&mut self) {
        self.capturing = None;
    }

    pub fn show(&mut self, ui: &mut Ui, shortcuts: &mut ShortcutRegistry) -> Option<SettingsAction> {
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
        ui.label("Shortcuts — click Change, then press a key (Escape cancels).");
        ui.add_space(4.0);

        if let Some(index) = self.capturing {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                format!(
                    "Press a new key for \"{}\"… (Escape to cancel)",
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
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                                    "Listening…"
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

        if !self.message.is_empty() {
            ui.add_space(6.0);
            ui.label(&self.message);
        }

        result
    }
}
