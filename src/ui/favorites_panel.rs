//! Selection-gated favorite plugin params column for the devices dock.

use egui::{RichText, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    AutomationTarget, EditHistory, MacroMapping, MacroTarget, Project, Track, TrackInstrument,
};
use crate::ui::app_settings::AppSettings;
use crate::ui::macro_panel::show_map_to_macro_menu;
use crate::ui::modulator::{TargetFilter, INSTRUMENT_MOD_TARGET_KEY};
use crate::ui::theme::ThemeColors;

pub const FAVORITES_COLUMN_WIDTH: f32 = 160.0;
const CHIP_INNER_MARGIN: f32 = 6.0;
const CHIP_GAP: f32 = 6.0;

/// Plugin identity for the selected device/instrument slot, if hostable.
pub fn unique_id_for_target(track: &Track, target_key: u64) -> Option<&str> {
    if target_key == INSTRUMENT_MOD_TARGET_KEY {
        match &track.instrument {
            TrackInstrument::Plugin { unique_id, .. } if !unique_id.is_empty() => {
                Some(unique_id.as_str())
            }
            _ => None,
        }
    } else {
        track
            .devices
            .iter()
            .find(|device| device.id == target_key)
            .and_then(|device| {
                if device.unique_id.is_empty() {
                    None
                } else {
                    Some(device.unique_id.as_str())
                }
            })
    }
}

/// True when the dock should show the Favorites column (hosted plugin selected).
/// Empty lists still show the column so `+ Favorite` can add the first knob.
pub fn favorites_column_visible(
    track: &Track,
    target_key: u64,
    _settings: &AppSettings,
) -> bool {
    unique_id_for_target(track, target_key).is_some()
}

fn device_id_for_target(target_key: u64) -> Option<u64> {
    if target_key == INSTRUMENT_MOD_TARGET_KEY {
        None
    } else {
        Some(target_key)
    }
}

fn automation_target_for(target_key: u64, param_id: u32) -> AutomationTarget {
    match device_id_for_target(target_key) {
        None => AutomationTarget::Instrument { param_id },
        Some(device_id) => AutomationTarget::Device {
            device_id,
            param_id,
        },
    }
}

fn macro_target_for(target_key: u64, param_id: u32) -> MacroTarget {
    match device_id_for_target(target_key) {
        None => MacroTarget::Instrument { param_id },
        Some(device_id) => MacroTarget::Device {
            device_id,
            param_id,
        },
    }
}

fn target_label(track: &Track, target_key: u64) -> String {
    if target_key == INSTRUMENT_MOD_TARGET_KEY {
        truncate_label(track.instrument.display_name(), 18)
    } else {
        track
            .devices
            .iter()
            .find(|device| device.id == target_key)
            .map(|device| truncate_label(&device.name, 18))
            .unwrap_or_else(|| "FX".to_string())
    }
}

/// Show starred params for the selected plugin slot.
#[allow(clippy::too_many_arguments)]
pub fn show_favorites_panel(
    ui: &mut Ui,
    project: &mut Project,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    track: &Track,
    target_filter: TargetFilter,
    theme: &ThemeColors,
    content_width: f32,
) -> bool {
    let track_id = track.id;
    let target_key = match target_filter {
        TargetFilter::Instrument => INSTRUMENT_MOD_TARGET_KEY,
        TargetFilter::Device { device_id } => device_id,
    };
    let Some(unique_id) = unique_id_for_target(track, target_key).map(str::to_string) else {
        return false;
    };

    let mut settings_dirty = false;
    let chip_width = content_width.max(120.0);

    ui.label(
        RichText::new("Favorites")
            .small()
            .strong()
            .color(theme.track_header_text),
    );
    ui.label(
        RichText::new(target_label(track, target_key))
            .small()
            .color(theme.text_muted),
    );
    ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .id_salt(("favorites_dock", track_id, target_key))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(chip_width);
            ui.spacing_mut().item_spacing.y = CHIP_GAP;

            let favorites: Vec<_> = settings.favorites_for(&unique_id).to_vec();

            for fav in &favorites {
                egui::Frame::new()
                    .fill(theme.widget_bg)
                    .stroke(egui::Stroke::new(1.0_f32, theme.separator))
                    .corner_radius(4.0)
                    .inner_margin(CHIP_INNER_MARGIN)
                    .show(ui, |ui| {
                        ui.set_width((chip_width - CHIP_INNER_MARGIN * 2.0).max(100.0));
                        let label = if fav.name.is_empty() {
                            format!("Param {}", fav.param_id)
                        } else {
                            fav.name.clone()
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(truncate_label(&label, 16))
                                    .small()
                                    .strong()
                                    .color(theme.track_header_text),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .small_button("x")
                                    .on_hover_text("Remove favorite")
                                    .clicked()
                                {
                                    if settings.remove_favorite(&unique_id, fav.param_id) {
                                        settings_dirty = true;
                                    }
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Auto").small().color(theme.button_text),
                                    )
                                    .fill(theme.widget_bg_hovered)
                                    .min_size(Vec2::new(40.0, 18.0)),
                                )
                                .on_hover_text("Add automation lane for this parameter")
                                .clicked()
                            {
                                let device_id = device_id_for_target(target_key);
                                let params = engine.plugin_parameters(track_id, device_id);
                                let (param_min, param_max, param_name) = params
                                    .iter()
                                    .find(|p| p.id == fav.param_id)
                                    .map(|p| (p.min, p.max, p.name.clone()))
                                    .unwrap_or((0.0, 1.0, fav.name.clone()));
                                history.push_before(project.clone());
                                project.add_automation_lane(
                                    track_id,
                                    automation_target_for(target_key, fav.param_id),
                                    if param_name.is_empty() {
                                        label.clone()
                                    } else {
                                        param_name
                                    },
                                    param_min,
                                    param_max,
                                );
                            }
                            ui.menu_button(RichText::new("Macro").small(), |ui| {
                                let mapping = MacroMapping {
                                    target: macro_target_for(target_key, fav.param_id),
                                    param_name: fav.name.clone(),
                                    min: 0.0,
                                    max: 1.0,
                                };
                                show_map_to_macro_menu(
                                    ui, project, history, track_id, mapping, theme,
                                );
                            });
                        });
                    });
            }

            ui.menu_button(
                RichText::new("+ Favorite").small().color(theme.text_muted),
                |ui| {
                    let device_id = device_id_for_target(target_key);
                    let params = engine.plugin_parameters(track_id, device_id);
                    if params.is_empty() {
                        ui.label(
                            RichText::new("No parameters")
                                .small()
                                .color(theme.text_muted),
                        );
                    }
                    for param in params {
                        if !param.automatable {
                            continue;
                        }
                        let already = settings.has_favorite(&unique_id, param.id);
                        let row_label = if already {
                            format!("* {}", truncate_label(&param.name, 26))
                        } else {
                            truncate_label(&param.name, 28)
                        };
                        if ui
                            .add_enabled(!already, egui::Button::new(row_label))
                            .clicked()
                        {
                            if settings.add_favorite(&unique_id, param.id, param.name.clone()) {
                                settings_dirty = true;
                            }
                            ui.close_menu();
                        }
                    }
                },
            );
        });

    settings_dirty
}

/// Menu helper: star a param for the plugin identified by `unique_id`.
pub fn add_favorite_menu_button(
    ui: &mut Ui,
    settings: &mut AppSettings,
    unique_id: &str,
    param_id: u32,
    param_name: &str,
) -> bool {
    if unique_id.is_empty() {
        return false;
    }
    if settings.has_favorite(unique_id, param_id) {
        ui.label(
            RichText::new("Already a favorite")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        return false;
    }
    if ui.button("Add to favorites").clicked() {
        let changed = settings.add_favorite(unique_id, param_id, param_name);
        ui.close_menu();
        return changed;
    }
    false
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
