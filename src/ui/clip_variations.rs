//! Clip variation list UI (piano-roll panel + playlist menu helpers).

use egui::{Align, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2};

use crate::model::{EditHistory, Project};
use crate::ui::theme::ThemeColors;

pub const VARIATIONS_PANEL_WIDTH: f32 = 168.0;

/// Draw the dismissable variations column into `panel_rect`.
/// Returns true if the active take changed (caller should clear note selection).
pub fn show_variations_panel(
    ui: &mut Ui,
    panel_rect: Rect,
    clip_id: u64,
    project: &mut Project,
    history: &mut EditHistory,
    theme: &ThemeColors,
    panel_open: &mut bool,
) -> bool {
    let mut panel_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("clip_variations_panel")
            .max_rect(panel_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    panel_ui.set_clip_rect(panel_rect);
    panel_ui.painter().rect_filled(panel_rect, 0.0, theme.panel_bg);
    panel_ui.painter().line_segment(
        [
            egui::pos2(panel_rect.left(), panel_rect.top()),
            egui::pos2(panel_rect.left(), panel_rect.bottom()),
        ],
        egui::Stroke::new(1.0, theme.separator),
    );

    let mut switched = false;
    panel_ui.horizontal(|ui| {
        ui.label(
            RichText::new("Variations")
                .strong()
                .color(theme.text_primary),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button(RichText::new("x").color(theme.text_muted))
                .on_hover_text("Hide variations panel")
                .clicked()
            {
                *panel_open = false;
            }
        });
    });
    panel_ui.separator();

    let Some(clip) = project.midi_clip(clip_id) else {
        return false;
    };
    let rows: Vec<(u64, String, bool, usize)> = clip
        .variations
        .iter()
        .map(|v| {
            (
                v.id,
                v.name.clone(),
                v.id == clip.active_variation_id,
                v.notes.len(),
            )
        })
        .collect();
    let can_delete = rows.len() > 1;
    let active_id = clip.active_variation_id;
    let mut rename_buf = clip
        .variation(active_id)
        .map(|v| v.name.clone())
        .unwrap_or_default();

    egui::ScrollArea::vertical()
        .id_salt("clip_variations_list")
        .max_height(panel_rect.height() * 0.45)
        .show(&mut panel_ui, |ui| {
            for (variation_id, name, is_active, note_count) in &rows {
                ui.horizontal(|ui| {
                    let label = if *is_active {
                        format!("> {name}")
                    } else {
                        name.clone()
                    };
                    let response = ui.selectable_label(*is_active, label);
                    if response.clicked() && !*is_active {
                        let before = project.clone();
                        if project.set_active_clip_variation(clip_id, *variation_id) {
                            history.push_before(before);
                            switched = true;
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{note_count}"))
                                .small()
                                .color(theme.text_muted),
                        );
                    });
                });
            }
        });

    panel_ui.add_space(6.0);
    panel_ui.horizontal(|ui| {
        if ui
            .button("New")
            .on_hover_text("Empty take; switch to it")
            .clicked()
        {
            let before = project.clone();
            if project.add_clip_variation_empty(clip_id).is_some() {
                history.push_before(before);
                switched = true;
            }
        }
        if ui
            .button("From current")
            .on_hover_text("Clone active melody into a new take and switch to it")
            .clicked()
        {
            let before = project.clone();
            if project.add_clip_variation_from_active(clip_id).is_some() {
                history.push_before(before);
                switched = true;
            }
        }
    });

    if can_delete {
        panel_ui.add_space(4.0);
        if panel_ui
            .button("Delete active")
            .on_hover_text("Remove the active take")
            .clicked()
        {
            let before = project.clone();
            if project.remove_clip_variation(clip_id, active_id) {
                history.push_before(before);
                switched = true;
            }
        }
    }

    panel_ui.add_space(8.0);
    panel_ui.label(
        RichText::new("Rename active")
            .small()
            .color(theme.text_muted),
    );
    let rename_response = panel_ui.add(
        egui::TextEdit::singleline(&mut rename_buf)
            .desired_width(VARIATIONS_PANEL_WIDTH - 24.0)
            .id_source(("var_name", clip_id, active_id)),
    );
    if rename_response.lost_focus()
        || (rename_response.has_focus() && panel_ui.input(|i| i.key_pressed(egui::Key::Enter)))
    {
        let trimmed = rename_buf.trim().to_string();
        let old_name = project
            .midi_clip(clip_id)
            .and_then(|c| c.variation(active_id))
            .map(|v| v.name.clone())
            .unwrap_or_default();
        if !trimmed.is_empty() && trimmed != old_name {
            let before = project.clone();
            if project.rename_clip_variation(clip_id, active_id, trimmed) {
                history.push_before(before);
            }
        }
    }

    switched
}

/// Compact toggle when the panel is closed (top-right of piano roll).
pub fn show_variations_panel_toggle(
    ui: &mut Ui,
    rect: Rect,
    panel_open: &mut bool,
    theme: &ThemeColors,
) {
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt("clip_variations_toggle")
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    child.set_clip_rect(rect);
    if child
        .button(RichText::new("Variations").small().color(theme.text_primary))
        .on_hover_text("Show clip variations")
        .clicked()
    {
        *panel_open = true;
    }
}

/// Playlist chrome: small menu on a MIDI clip to switch / create takes.
/// Returns true if the active variation changed.
pub fn show_playlist_clip_variation_menu(
    ui: &mut Ui,
    clip_rect: Rect,
    clip_id: u64,
    project: &mut Project,
    history: &mut EditHistory,
    theme: &ThemeColors,
) -> bool {
    let Some(clip) = project.midi_clip(clip_id) else {
        return false;
    };
    let variation_count = clip.variations.len();
    let active_name = clip
        .active_variation()
        .map(|v| v.name.clone())
        .unwrap_or_else(|| "?".into());
    let rows: Vec<(u64, String, bool)> = clip
        .variations
        .iter()
        .map(|v| (v.id, v.name.clone(), v.id == clip.active_variation_id))
        .collect();

    let btn_w = 28.0_f32.min(clip_rect.width() * 0.35);
    let btn_h = 16.0_f32.min(clip_rect.height() - 4.0).max(12.0);
    if btn_w < 16.0 || clip_rect.height() < 18.0 {
        return false;
    }
    let btn_rect = Rect::from_min_size(
        egui::pos2(clip_rect.right() - btn_w - 3.0, clip_rect.top() + 2.0),
        Vec2::new(btn_w, btn_h),
    );

    let mut switched = false;
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("clip_var_menu", clip_id))
            .max_rect(btn_rect)
            .layout(Layout::centered_and_justified(egui::Direction::TopDown)),
    );
    child.set_clip_rect(clip_rect);
    let label = if variation_count > 1 {
        active_name
    } else {
        String::from("...")
    };
    let response = child.add(
        egui::Button::new(RichText::new(label).small().color(theme.clip_label))
            .fill(theme.clip_fill.gamma_multiply(0.55))
            .sense(Sense::click()),
    );

    let popup_id = response.id.with("var_popup");
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }

    egui::popup::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::popup::PopupCloseBehavior::CloseOnClick,
        |ui| {
            ui.set_min_width(120.0);
            for (id, name, is_active) in &rows {
                if ui.selectable_label(*is_active, name).clicked() && !*is_active {
                    let before = project.clone();
                    if project.set_active_clip_variation(clip_id, *id) {
                        history.push_before(before);
                        switched = true;
                    }
                }
            }
            ui.separator();
            if ui.button("New empty").clicked() {
                let before = project.clone();
                if project.add_clip_variation_empty(clip_id).is_some() {
                    history.push_before(before);
                    switched = true;
                }
            }
            if ui.button("New from current").clicked() {
                let before = project.clone();
                if project.add_clip_variation_from_active(clip_id).is_some() {
                    history.push_before(before);
                    switched = true;
                }
            }
        },
    );

    switched
}
