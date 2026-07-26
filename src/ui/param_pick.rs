//! Shared plugin-parameter pick menus with a Last tweaked MRU section.

use std::collections::HashSet;

use egui::{RichText, Ui};

use crate::engine::plugins::PluginParamInfo;
use crate::ui::app_settings::AppSettings;
use crate::ui::theme::ThemeColors;

/// How rows behave when chosen from a param pick menu.
#[derive(Debug, Clone, Copy)]
pub enum ParamPickMode {
    /// Click assigns/maps the parameter. Optional per-row `+fav`.
    Assign { show_fav_button: bool },
    /// Click stars the parameter; already-starred rows are disabled.
    AddFavorite,
}

/// Render Last tweaked (when present) + All parameters for automatable params.
///
/// Calls `on_select` when the user chooses a param in [`ParamPickMode::Assign`].
/// In [`ParamPickMode::AddFavorite`], starring is handled here (no `on_select`).
/// Always bumps last-tweaked on pick/star when `unique_id` is set.
#[allow(clippy::too_many_arguments)]
pub fn show_param_pick_menu(
    ui: &mut Ui,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    unique_id: Option<&str>,
    params: &[PluginParamInfo],
    theme: &ThemeColors,
    mode: ParamPickMode,
    empty_label: &str,
    name_max_chars: usize,
    mut on_select: impl FnMut(&PluginParamInfo),
) {
    let automatable: Vec<&PluginParamInfo> = params.iter().filter(|p| p.automatable).collect();
    if automatable.is_empty() {
        ui.label(
            RichText::new(empty_label)
                .small()
                .color(theme.text_muted),
        );
        return;
    }

    let by_id: std::collections::HashMap<u32, &PluginParamInfo> =
        automatable.iter().map(|p| (p.id, *p)).collect();

    let mut shown_ids = HashSet::new();
    let mru: Vec<&PluginParamInfo> = unique_id
        .map(|uid| {
            settings
                .last_tweaked_for(uid)
                .iter()
                .filter_map(|entry| by_id.get(&entry.param_id).copied())
                .collect()
        })
        .unwrap_or_default();

    let has_mru = !mru.is_empty();
    if has_mru {
        ui.label(
            RichText::new("Last tweaked")
                .small()
                .strong()
                .color(theme.text_muted),
        );
        for param in &mru {
            shown_ids.insert(param.id);
            show_param_row(
                ui,
                settings,
                settings_dirty,
                unique_id,
                param,
                mode,
                name_max_chars,
                &mut on_select,
            );
        }
        ui.separator();
        ui.label(
            RichText::new("All parameters")
                .small()
                .strong()
                .color(theme.text_muted),
        );
    }
    for param in &automatable {
        if shown_ids.contains(&param.id) {
            continue;
        }
        show_param_row(
            ui,
            settings,
            settings_dirty,
            unique_id,
            param,
            mode,
            name_max_chars,
            &mut on_select,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn show_param_row(
    ui: &mut Ui,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    unique_id: Option<&str>,
    param: &PluginParamInfo,
    mode: ParamPickMode,
    name_max_chars: usize,
    on_select: &mut impl FnMut(&PluginParamInfo),
) {
    match mode {
        ParamPickMode::Assign { show_fav_button } => {
            ui.horizontal(|ui| {
                if ui
                    .button(truncate_label(&param.name, name_max_chars))
                    .clicked()
                {
                    if let Some(uid) = unique_id {
                        if settings.touch_param(uid, param.id, param.name.clone()) {
                            *settings_dirty = true;
                        }
                    }
                    on_select(param);
                    ui.close_menu();
                }
                if show_fav_button {
                    if let Some(uid) = unique_id {
                        let starred = settings.has_favorite(uid, param.id);
                        if ui
                            .add_enabled(
                                !starred,
                                egui::Button::new(if starred { "fav" } else { "+fav" }).small(),
                            )
                            .on_hover_text(if starred {
                                "Already a favorite"
                            } else {
                                "Add to favorites"
                            })
                            .clicked()
                        {
                            let mut changed = settings.add_favorite(uid, param.id, param.name.clone());
                            changed |= settings.touch_param(uid, param.id, param.name.clone());
                            if changed {
                                *settings_dirty = true;
                            }
                        }
                    }
                }
            });
        }
        ParamPickMode::AddFavorite => {
            let Some(uid) = unique_id else {
                return;
            };
            let already = settings.has_favorite(uid, param.id);
            let row_label = if already {
                format!("* {}", truncate_label(&param.name, name_max_chars.saturating_sub(2)))
            } else {
                truncate_label(&param.name, name_max_chars)
            };
            if ui
                .add_enabled(!already, egui::Button::new(row_label))
                .clicked()
            {
                let mut changed = settings.add_favorite(uid, param.id, param.name.clone());
                changed |= settings.touch_param(uid, param.id, param.name.clone());
                if changed {
                    *settings_dirty = true;
                }
                ui.close_menu();
            }
        }
    }
}

fn truncate_label(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
