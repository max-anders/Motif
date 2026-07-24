//! In-app Recent Projects loader modal.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Align2, Color32, Context, Ui, Vec2, Window};

use crate::model::project_display_name;

#[derive(Debug, Clone)]
pub enum ProjectBrowserAction {
    New,
    OpenPath(PathBuf),
    OpenDialog,
    RemoveRecent(PathBuf),
    Close,
}

#[derive(Debug, Default)]
pub struct ProjectBrowserUi;

impl ProjectBrowserUi {
    /// Show the projects modal. Returns an action when the user chooses something.
    pub fn show(
        &mut self,
        ctx: &Context,
        open: &mut bool,
        recent: &[PathBuf],
    ) -> Option<ProjectBrowserAction> {
        if !*open {
            return None;
        }

        let mut action = None;
        let mut still_open = *open;

        Window::new("Projects")
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(520.0, 420.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut still_open)
            .show(ctx, |ui| {
                action = self.show_contents(ui, recent);
            });

        if !still_open && *open {
            *open = false;
            if action.is_none() {
                action = Some(ProjectBrowserAction::Close);
            }
        } else {
            *open = still_open;
        }

        // Closing actions also dismiss the modal.
        if matches!(
            action,
            Some(
                ProjectBrowserAction::New
                    | ProjectBrowserAction::OpenPath(_)
                    | ProjectBrowserAction::OpenDialog
                    | ProjectBrowserAction::Close
            )
        ) {
            *open = false;
        }

        action
    }

    fn show_contents(
        &mut self,
        ui: &mut Ui,
        recent: &[PathBuf],
    ) -> Option<ProjectBrowserAction> {
        let mut action = None;

        ui.label("Open a recent project, start a new one, or browse for a .motif file.");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui.button("New project").clicked() {
                action = Some(ProjectBrowserAction::New);
            }
            if ui.button("Open...").clicked() {
                action = Some(ProjectBrowserAction::OpenDialog);
            }
        });

        ui.add_space(10.0);
        ui.strong("Recent projects");
        ui.add_space(4.0);

        if recent.is_empty() {
            ui.weak("No recent projects yet.");
        } else {
            egui::ScrollArea::vertical()
                .id_salt("project_browser_recent")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut remove: Option<PathBuf> = None;
                    let mut open_path: Option<PathBuf> = None;

                    for path in recent {
                        let exists = path.exists();
                        let name = project_display_name(path);
                        let modified = file_modified_label(path);
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    if exists {
                                        ui.strong(&name);
                                    } else {
                                        ui.colored_label(
                                            Color32::from_rgb(220, 140, 80),
                                            format!("{name} (missing)"),
                                        );
                                    }
                                    ui.weak(path.display().to_string());
                                    ui.weak(modified);
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("Remove").clicked() {
                                            remove = Some(path.clone());
                                        }
                                        if ui
                                            .add_enabled(exists, egui::Button::new("Open"))
                                            .clicked()
                                        {
                                            open_path = Some(path.clone());
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }

                    if let Some(path) = open_path {
                        action = Some(ProjectBrowserAction::OpenPath(path));
                    } else if let Some(path) = remove {
                        action = Some(ProjectBrowserAction::RemoveRecent(path));
                    }
                });
        }

        action
    }
}

fn file_modified_label(path: &Path) -> String {
    match path.metadata().and_then(|m| m.modified()) {
        Ok(modified) => match modified.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(dur) => format!(
                "Modified: {}",
                crate::model::format_unix_time(dur.as_secs())
            ),
            Err(_) => "Modified: unknown".into(),
        },
        Err(_) => "Modified: unknown".into(),
    }
}
