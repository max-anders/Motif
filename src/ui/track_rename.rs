//! Rename dialog for playlist tracks (header context menu, F2, mixer, inspector).

use egui::{Context, Key};

use crate::model::{EditHistory, Project};

#[derive(Debug, Default)]
pub struct TrackRenameUi {
    track_id: Option<u64>,
    draft: String,
    original: String,
}

impl TrackRenameUi {
    pub fn begin(&mut self, track_id: u64, current_name: &str) {
        self.track_id = Some(track_id);
        self.draft = current_name.to_string();
        self.original = current_name.to_string();
    }

    pub fn cancel(&mut self) {
        self.track_id = None;
        self.draft.clear();
        self.original.clear();
    }

    pub fn is_active(&self) -> bool {
        self.track_id.is_some()
    }

    pub fn active_track_id(&self) -> Option<u64> {
        self.track_id
    }

    pub fn show_window(
        &mut self,
        ctx: &Context,
        project: &mut Project,
        history: &mut EditHistory,
    ) {
        let Some(track_id) = self.track_id else {
            return;
        };
        if project.track(track_id).is_none() {
            self.cancel();
            return;
        }

        let mut open = true;
        let mut commit = false;
        let mut cancel_request = false;

        egui::Window::new("Rename track")
            .collapsible(false)
            .resizable(false)
            .auto_sized()
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Track name");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.draft)
                        .desired_width(220.0)
                        .hint_text("Name"),
                );
                response.request_focus();

                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                let escape = ui.input(|i| i.key_pressed(Key::Escape));

                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_request = true;
                    }
                });

                if enter {
                    commit = true;
                }
                if escape {
                    cancel_request = true;
                }
            });

        if commit {
            self.commit(project, history);
        } else if cancel_request || !open {
            self.cancel();
        }
    }

    fn commit(&mut self, project: &mut Project, history: &mut EditHistory) {
        let Some(track_id) = self.track_id.take() else {
            return;
        };
        let original = std::mem::take(&mut self.original);
        let new_name = normalize_track_name(&self.draft, &original);
        self.draft.clear();
        if new_name != original {
            history.push_before(project.clone());
            if let Some(track) = project.track_mut(track_id) {
                track.name = new_name;
            }
        }
    }
}

pub fn normalize_track_name(draft: &str, fallback: &str) -> String {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn apply_track_name_if_changed(
    history: &mut EditHistory,
    project: &mut Project,
    track_id: u64,
    original: &str,
    draft: &str,
) {
    let new_name = normalize_track_name(draft, original);
    if new_name != original {
        history.push_before(project.clone());
        if let Some(track) = project.track_mut(track_id) {
            track.name = new_name;
        }
    }
}
