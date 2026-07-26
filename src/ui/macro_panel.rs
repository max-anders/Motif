//! Host macro knobs column for the devices dock.

use egui::{RichText, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    EditHistory, MacroMapping, MacroTarget, Project, Track,
};
use crate::ui::theme::ThemeColors;

pub const MACRO_COLUMN_WIDTH: f32 = 160.0;
const CHIP_INNER_MARGIN: f32 = 6.0;
const CHIP_GAP: f32 = 6.0;

/// Display label for a macro destination.
pub fn macro_target_label(track: &Track, target: &MacroTarget) -> String {
    match target {
        MacroTarget::Instrument { param_id } => {
            format!("Inst param #{param_id}")
        }
        MacroTarget::Device {
            device_id,
            param_id,
        } => {
            let device_name = track
                .devices
                .iter()
                .find(|device| device.id == *device_id)
                .map(|device| device.name.as_str())
                .unwrap_or("FX");
            format!("{device_name} #{param_id}")
        }
        MacroTarget::ModulatorRate { modulator_id } => {
            let name = modulator_display_name(track, *modulator_id);
            format!("{name} rate")
        }
        MacroTarget::ModulatorDepth { modulator_id } => {
            let name = modulator_display_name(track, *modulator_id);
            format!("{name} depth")
        }
    }
}

fn modulator_display_name(track: &Track, modulator_id: u64) -> String {
    track
        .modulators
        .iter()
        .find(|modulator| modulator.id == modulator_id)
        .map(|modulator| {
            if !modulator.name.is_empty() {
                modulator.name.clone()
            } else if !modulator.param_name.is_empty() {
                modulator.param_name.clone()
            } else {
                format!("Mod {}", modulator.id)
            }
        })
        .unwrap_or_else(|| format!("Mod {modulator_id}"))
}

/// Create a new macro and attach `mapping`, or append to an existing macro.
pub fn map_destination_to_macro(
    project: &mut Project,
    history: &mut EditHistory,
    track_id: u64,
    macro_id: Option<u64>,
    mapping: MacroMapping,
) -> Option<u64> {
    history.push_before(project.clone());
    let id = if let Some(macro_id) = macro_id {
        macro_id
    } else {
        let n = project
            .track(track_id)
            .map(|track| track.macros.len() + 1)
            .unwrap_or(1);
        project.add_macro(track_id, format!("Macro {n}"))?
    };
    project.add_macro_mapping(track_id, id, mapping);
    project.apply_macro_host_destinations(track_id);
    Some(id)
}

/// Show the macros column for one track.
pub fn show_macro_panel(
    ui: &mut Ui,
    project: &mut Project,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    track: &Track,
    theme: &ThemeColors,
    content_width: f32,
) {
    let track_id = track.id;
    ui.label(
        RichText::new("Macros")
            .small()
            .strong()
            .color(theme.track_header_text),
    );
    ui.add_space(4.0);

    let chip_width = content_width.max(120.0);
    egui::ScrollArea::vertical()
        .id_salt(("macros_dock", track_id))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(chip_width);
            ui.spacing_mut().item_spacing.y = CHIP_GAP;

            let macro_ids: Vec<u64> = project
                .track(track_id)
                .map(|t| t.macros.iter().map(|m| m.id).collect())
                .unwrap_or_default();

            for macro_id in macro_ids {
                show_macro_chip(
                    ui,
                    project,
                    engine,
                    history,
                    track_id,
                    macro_id,
                    theme,
                    chip_width,
                );
            }

            if ui
                .add(
                    egui::Button::new(RichText::new("+ Macro").small().color(theme.text_muted))
                        .fill(theme.widget_bg)
                        .min_size(Vec2::new(chip_width, 28.0)),
                )
                .on_hover_text("Add a host macro knob")
                .clicked()
            {
                history.push_before(project.clone());
                let n = project
                    .track(track_id)
                    .map(|t| t.macros.len() + 1)
                    .unwrap_or(1);
                project.add_macro(track_id, format!("Macro {n}"));
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn show_macro_chip(
    ui: &mut Ui,
    project: &mut Project,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    track_id: u64,
    macro_id: u64,
    theme: &ThemeColors,
    chip_width: f32,
) {
    let Some(snapshot) = project.macro_knob(track_id, macro_id).cloned() else {
        return;
    };
    let track_snapshot = project.track(track_id).cloned();

    egui::Frame::new()
        .fill(theme.widget_bg)
        .stroke(egui::Stroke::new(1.0_f32, theme.separator))
        .corner_radius(4.0)
        .inner_margin(CHIP_INNER_MARGIN)
        .show(ui, |ui| {
            ui.set_width((chip_width - CHIP_INNER_MARGIN * 2.0).max(100.0));

            ui.horizontal(|ui| {
                let mut name = snapshot.name.clone();
                let name_edit = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(ui.available_width() - 28.0)
                        .font(egui::TextStyle::Small),
                );
                if name_edit.changed() {
                    history.push_before(project.clone());
                    if let Some(macro_knob) = project.macro_knob_mut(track_id, macro_id) {
                        macro_knob.name = name;
                    }
                }
                if ui
                    .small_button("x")
                    .on_hover_text("Remove macro")
                    .clicked()
                {
                    history.push_before(project.clone());
                    project.remove_macro(track_id, macro_id);
                }
            });

            let mut value = snapshot.value;
            let slider = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).text("value"));
            if slider.changed() {
                history.push_before(project.clone());
                if let Some(macro_knob) = project.macro_knob_mut(track_id, macro_id) {
                    macro_knob.value = value.clamp(0.0, 1.0);
                }
                project.apply_macro_host_destinations(track_id);
            }

            if let Some(track) = &track_snapshot {
                for (index, mapping) in snapshot.mappings.iter().enumerate() {
                    let label = if !mapping.param_name.is_empty() {
                        mapping.param_name.clone()
                    } else {
                        macro_target_label(track, &mapping.target)
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(truncate_label(&label, 18))
                                .small()
                                .color(theme.text_muted),
                        );
                        if ui.small_button("-").on_hover_text("Remove mapping").clicked() {
                            history.push_before(project.clone());
                            if let Some(macro_knob) = project.macro_knob_mut(track_id, macro_id) {
                                if index < macro_knob.mappings.len() {
                                    macro_knob.mappings.remove(index);
                                }
                            }
                        }
                    });
                }
            }

            ui.menu_button(RichText::new("+ Map").small(), |ui| {
                show_add_mapping_menu(ui, project, engine, history, track_id, macro_id, theme);
            });
        });
}

fn show_add_mapping_menu(
    ui: &mut Ui,
    project: &mut Project,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    track_id: u64,
    macro_id: u64,
    theme: &ThemeColors,
) {
    let Some(track) = project.track(track_id).cloned() else {
        return;
    };

    ui.label(RichText::new("Plugin").small().strong());
    ui.menu_button("Instrument", |ui| {
        let params = engine.plugin_parameters(track_id, None);
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
            if ui.button(truncate_label(&param.name, 28)).clicked() {
                let mapping = MacroMapping {
                    target: MacroTarget::Instrument {
                        param_id: param.id,
                    },
                    param_name: param.name.clone(),
                    min: 0.0,
                    max: 1.0,
                };
                map_destination_to_macro(project, history, track_id, Some(macro_id), mapping);
                ui.close_menu();
            }
        }
    });
    for device in &track.devices {
        let device_id = device.id;
        let device_name = truncate_label(&device.name, 22);
        ui.menu_button(device_name, |ui| {
            let params = engine.plugin_parameters(track_id, Some(device_id));
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
                if ui.button(truncate_label(&param.name, 28)).clicked() {
                    let mapping = MacroMapping {
                        target: MacroTarget::Device {
                            device_id,
                            param_id: param.id,
                        },
                        param_name: param.name.clone(),
                        min: 0.0,
                        max: 1.0,
                    };
                    map_destination_to_macro(project, history, track_id, Some(macro_id), mapping);
                    ui.close_menu();
                }
            }
        });
    }

    ui.separator();
    ui.label(RichText::new("Modulators").small().strong());
    if track.modulators.is_empty() {
        ui.label(
            RichText::new("No modulators")
                .small()
                .color(theme.text_muted),
        );
    }
    for modulator in &track.modulators {
        let label = modulator_display_name(&track, modulator.id);
        ui.menu_button(truncate_label(&label, 20), |ui| {
            if ui.button("Rate").clicked() {
                let mapping = MacroMapping::new(MacroTarget::ModulatorRate {
                    modulator_id: modulator.id,
                });
                map_destination_to_macro(project, history, track_id, Some(macro_id), mapping);
                ui.close_menu();
            }
            if ui.button("Depth").clicked() {
                let mapping = MacroMapping::new(MacroTarget::ModulatorDepth {
                    modulator_id: modulator.id,
                });
                map_destination_to_macro(project, history, track_id, Some(macro_id), mapping);
                ui.close_menu();
            }
        });
    }
}

/// Menu to map a destination onto a new or existing macro (from LFO chip / param).
pub fn show_map_to_macro_menu(
    ui: &mut Ui,
    project: &mut Project,
    history: &mut EditHistory,
    track_id: u64,
    mapping: MacroMapping,
    theme: &ThemeColors,
) {
    if ui.button("New macro").clicked() {
        map_destination_to_macro(project, history, track_id, None, mapping.clone());
        ui.close_menu();
    }
    let macros: Vec<(u64, String)> = project
        .track(track_id)
        .map(|track| {
            track
                .macros
                .iter()
                .map(|m| (m.id, m.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    if macros.is_empty() {
        ui.label(
            RichText::new("No macros yet")
                .small()
                .color(theme.text_muted),
        );
        return;
    }
    ui.separator();
    for (macro_id, name) in macros {
        if ui.button(truncate_label(&name, 22)).clicked() {
            map_destination_to_macro(project, history, track_id, Some(macro_id), mapping.clone());
            ui.close_menu();
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
