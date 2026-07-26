//! Modulator chips (preset LFO shapes + editable custom curve) under device tiles.

use egui::{Color32, Pos2, RichText, Sense, Stroke, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    AutomationPoint, AutomationTarget, CurveKind, EditHistory, LfoModulator, LfoRate, LfoShape,
    MacroMapping, MacroTarget, Project, Track,
};
use crate::ui::app_settings::AppSettings;
use crate::ui::favorites_panel::unique_id_for_target;
use crate::ui::macro_panel::show_map_to_macro_menu;
use crate::ui::param_pick::{show_param_pick_menu, ParamPickMode};
use crate::ui::theme::ThemeColors;

pub const CHIP_WIDTH: f32 = 146.0;
const MODULATOR_CONTROLS_WIDTH: f32 = CHIP_WIDTH - 12.0;
const MODULATOR_CANVAS_WIDTH: f32 = 300.0;
const MODULATOR_CANVAS_HEIGHT: f32 = 260.0;
const MODULATOR_CHIP_INNER_WIDTH: f32 = MODULATOR_CANVAS_WIDTH + 8.0 + MODULATOR_CONTROLS_WIDTH;
const DOCK_CANVAS_HEIGHT: f32 = 150.0;
const MSEG_POINT_RADIUS: f32 = 4.0;
const PRESET_WAVE_SAMPLES: usize = 128;
/// Coarse preset-to-MSEG conversion; editable points, not a high-res bake.
const MSEG_BAKE_STEPS: usize = 4;
const MODULATOR_ROW_GAP: f32 = 8.0;
const MODULATOR_STACK_GAP: f32 = 8.0;
const MOD_SECTION_GAP: f32 = 8.0;
const MOD_INNER_GAP: f32 = 5.0;
const CHIP_INNER_MARGIN: f32 = 8.0;

/// Wide (canvas + controls side-by-side) vs compact (stacked for the right dock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModulatorLayout {
    Wide,
    Compact,
}

/// Strip panel height budget for a wide modulator chip (canvas + controls).
pub const CHIP_HEIGHT_STANDARD: f32 = 320.0;
/// Alias kept for callers that referenced the MSEG-specific height.
pub const CHIP_HEIGHT_MSEG: f32 = CHIP_HEIGHT_STANDARD;

/// Device key for the instrument slot in modulator target selection.
pub const INSTRUMENT_MOD_TARGET_KEY: u64 = 0;

/// Build a target filter from a stored device key (`0` = instrument).
pub fn target_filter_from_device_key(device_key: u64) -> TargetFilter {
    if device_key == INSTRUMENT_MOD_TARGET_KEY {
        TargetFilter::Instrument
    } else {
        TargetFilter::Device { device_id: device_key }
    }
}

/// Clamp a stored device key to a target that still exists on the track.
pub fn normalize_modulator_target_key(track: &Track, device_key: u64) -> u64 {
    if device_key == INSTRUMENT_MOD_TARGET_KEY {
        return INSTRUMENT_MOD_TARGET_KEY;
    }
    if track.devices.iter().any(|device| device.id == device_key) {
        device_key
    } else {
        INSTRUMENT_MOD_TARGET_KEY
    }
}

/// Human-readable modulator target for the strip panel header.
pub fn modulator_target_label(track: &Track, target_filter: TargetFilter) -> String {
    match target_filter {
        TargetFilter::Instrument => format!(
            "Instrument ({})",
            truncate_label(track.instrument.display_name(), 18)
        ),
        TargetFilter::Device { device_id } => track
            .devices
            .iter()
            .find(|device| device.id == device_id)
            .map(|device| truncate_label(&device.name, 22))
            .unwrap_or_else(|| "FX".to_string()),
    }
}

pub fn modulator_count_for_target(track: &Track, target_filter: TargetFilter) -> usize {
    track
        .modulators
        .iter()
        .filter(|modulator| target_filter.matches(&modulator.target))
        .count()
}

/// Shared modulator editor for one target.
#[allow(clippy::too_many_arguments)]
pub fn show_modulator_panel(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    target_filter: TargetFilter,
    layout: ModulatorLayout,
    fixed_content_width: Option<f32>,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    theme: &ThemeColors,
) {
    ui.label(
        RichText::new(format!(
            "Mods: {}",
            modulator_target_label(track, target_filter)
        ))
        .small()
        .strong()
        .color(theme.track_header_text),
    );
    ui.add_space(4.0);
    let content_width = fixed_content_width.unwrap_or_else(|| ui.available_width());
    if layout == ModulatorLayout::Compact {
        ui.set_max_width(content_width);
    }
    show_modulators_for_target(
        ui,
        project,
        track,
        track_id,
        target_filter,
        layout,
        content_width,
        engine,
        history,
        settings,
        settings_dirty,
        theme,
    );
}

/// All modulators for one target (horizontal row or vertical stack).
#[allow(clippy::too_many_arguments)]
pub fn show_modulators_for_target(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    target_filter: TargetFilter,
    layout: ModulatorLayout,
    content_width: f32,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    theme: &ThemeColors,
) {
    let modulator_entries: Vec<(usize, u64)> = track
        .modulators
        .iter()
        .filter(|modulator| target_filter.matches(&modulator.target))
        .map(|modulator| modulator.id)
        .enumerate()
        .collect();

    let device_key = match target_filter {
        TargetFilter::Instrument => INSTRUMENT_MOD_TARGET_KEY,
        TargetFilter::Device { device_id } => device_id,
    };

    if modulator_entries.is_empty() {
        let button_width = match layout {
            ModulatorLayout::Compact => content_width.max(CHIP_WIDTH),
            ModulatorLayout::Wide => CHIP_WIDTH,
        };
        if ui
            .add(
                egui::Button::new(RichText::new("+ Mod").small().color(theme.text_muted))
                    .fill(theme.widget_bg)
                    .min_size(Vec2::new(button_width, 22.0)),
            )
            .on_hover_text("Add modulator for this device")
            .clicked()
        {
            history.push_before(project.clone());
            let target = target_filter.to_target();
            project.add_modulator(track_id, target, "");
        }
        return;
    }

    match layout {
        ModulatorLayout::Wide => {
            egui::ScrollArea::horizontal()
                .id_salt(("modulators_row", track_id, device_key))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = MODULATOR_ROW_GAP;
                        for (index, modulator_id) in &modulator_entries {
                            show_modulator_chip(
                                ui,
                                project,
                                track,
                                track_id,
                                *modulator_id,
                                *index,
                                layout,
                                MODULATOR_CHIP_INNER_WIDTH,
                                engine,
                                history,
                                settings,
                                settings_dirty,
                                theme,
                            );
                        }
                        show_add_mod_tile(
                            ui,
                            project,
                            track_id,
                            target_filter,
                            layout,
                            MODULATOR_CHIP_INNER_WIDTH,
                            history,
                            theme,
                        );
                    });
                });
        }
        ModulatorLayout::Compact => {
            let column_width = content_width.max(CHIP_WIDTH);
            egui::ScrollArea::vertical()
                .id_salt(("modulators_stack", track_id, device_key))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_min_width(column_width);
                    ui.set_max_width(column_width);
                    ui.spacing_mut().item_spacing.y = MODULATOR_STACK_GAP;
                    for (index, modulator_id) in &modulator_entries {
                        show_modulator_chip(
                            ui,
                            project,
                            track,
                            track_id,
                            *modulator_id,
                            *index,
                            layout,
                            column_width,
                            engine,
                            history,
                            settings,
                            settings_dirty,
                            theme,
                        );
                    }
                    show_add_mod_tile(
                        ui,
                        project,
                        track_id,
                        target_filter,
                        layout,
                        column_width,
                        history,
                        theme,
                    );
                });
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TargetFilter {
    Instrument,
    Device { device_id: u64 },
}

impl TargetFilter {
    fn matches(self, target: &AutomationTarget) -> bool {
        match (self, target) {
            (Self::Instrument, AutomationTarget::Instrument { .. }) => true,
            (
                Self::Device { device_id },
                AutomationTarget::Device {
                    device_id: tid, ..
                },
            ) => *tid == device_id,
            _ => false,
        }
    }

    fn to_target(self) -> AutomationTarget {
        match self {
            Self::Instrument => AutomationTarget::Instrument { param_id: 0 },
            Self::Device { device_id } => AutomationTarget::Device {
                device_id,
                param_id: 0,
            },
        }
    }
}

fn mod_chip_width(layout: ModulatorLayout, content_width: f32) -> f32 {
    match layout {
        ModulatorLayout::Wide => MODULATOR_CHIP_INNER_WIDTH,
        ModulatorLayout::Compact => content_width.max(CHIP_WIDTH),
    }
}

/// Width available *inside* a chip frame. Compact chips must fit their column,
/// so the frame's own margin is subtracted; wide chips size their column instead.
fn mod_chip_inner_width(layout: ModulatorLayout, chip_width: f32) -> f32 {
    match layout {
        ModulatorLayout::Wide => chip_width,
        ModulatorLayout::Compact => (chip_width - CHIP_INNER_MARGIN * 2.0).max(120.0),
    }
}

fn mod_canvas_size(layout: ModulatorLayout, chip_width: f32) -> Vec2 {
    match layout {
        ModulatorLayout::Wide => Vec2::new(MODULATOR_CANVAS_WIDTH, MODULATOR_CANVAS_HEIGHT),
        ModulatorLayout::Compact => {
            Vec2::new(mod_chip_inner_width(layout, chip_width), DOCK_CANVAS_HEIGHT)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn show_modulator_chip(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    modulator_id: u64,
    modulator_index: usize,
    layout: ModulatorLayout,
    content_width: f32,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    theme: &ThemeColors,
) {
    let Some(snapshot) = project.modulator(track_id, modulator_id).cloned() else {
        return;
    };

    let is_custom = snapshot.shape == LfoShape::Custom;
    let title = modulator_display_name(modulator_index, &snapshot);
    let chip_width = mod_chip_width(layout, content_width);
    let canvas_size = mod_canvas_size(layout, chip_width);

    egui::Frame::new()
        .fill(theme.widget_bg)
        .stroke(Stroke::new(1.0_f32, theme.separator))
        .corner_radius(4.0)
        .inner_margin(CHIP_INNER_MARGIN)
        .show(ui, |ui| {
            let inner_width = mod_chip_inner_width(layout, chip_width);
            ui.set_min_width(inner_width);
            ui.set_max_width(inner_width);
            ui.spacing_mut().item_spacing = Vec2::new(4.0, MOD_INNER_GAP);

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(title)
                        .small()
                        .strong()
                        .color(theme.track_header_text),
                );

                let mut name = snapshot.name.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(if layout == ModulatorLayout::Compact {
                                56.0
                            } else {
                                72.0
                            })
                            .hint_text("Label")
                            .font(egui::TextStyle::Body),
                    )
                    .changed()
                {
                    history.push_before(project.clone());
                    if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                        modulator.name = name;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("x")
                        .on_hover_text("Remove modulator")
                        .clicked()
                    {
                        history.push_before(project.clone());
                        project.remove_modulator(track_id, modulator_id);
                    }
                    let mut enabled = snapshot.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        history.push_before(project.clone());
                        if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                            modulator.enabled = enabled;
                        }
                    }
                });
            });

            ui.add_space(6.0);
            match layout {
                ModulatorLayout::Wide => {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(MODULATOR_CANVAS_WIDTH);
                            ui.set_max_width(MODULATOR_CANVAS_WIDTH);
                            if is_custom {
                                show_mseg_grid_bar(
                                    ui,
                                    project,
                                    track_id,
                                    modulator_id,
                                    snapshot.mseg_grid_divisions,
                                    history,
                                    theme,
                                );
                                ui.add_space(6.0);
                            }
                            show_modulator_canvas(
                                ui,
                                project,
                                track_id,
                                modulator_id,
                                &snapshot,
                                canvas_size,
                                engine,
                                history,
                                theme,
                            );
                        });
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.set_min_width(MODULATOR_CONTROLS_WIDTH);
                            ui.set_max_width(MODULATOR_CONTROLS_WIDTH);
                            paint_modulator_controls(
                                ui,
                                project,
                                track,
                                track_id,
                                modulator_id,
                                &snapshot,
                                engine,
                                history,
                                settings,
                                settings_dirty,
                                theme,
                            );
                        });
                    });
                }
                ModulatorLayout::Compact => {
                    if is_custom {
                        show_mseg_grid_bar(
                            ui,
                            project,
                            track_id,
                            modulator_id,
                            snapshot.mseg_grid_divisions,
                            history,
                            theme,
                        );
                        ui.add_space(6.0);
                    }
                    show_modulator_canvas(
                        ui,
                        project,
                        track_id,
                        modulator_id,
                        &snapshot,
                        canvas_size,
                        engine,
                        history,
                        theme,
                    );
                    ui.add_space(MOD_SECTION_GAP);
                    paint_modulator_controls(
                        ui,
                        project,
                        track,
                        track_id,
                        modulator_id,
                        &snapshot,
                        engine,
                        history,
                        settings,
                        settings_dirty,
                        theme,
                    );
                }
            }

            let _ = track;
        });
}

fn show_add_mod_tile(
    ui: &mut Ui,
    project: &mut Project,
    track_id: u64,
    target_filter: TargetFilter,
    layout: ModulatorLayout,
    content_width: f32,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    match layout {
        ModulatorLayout::Compact => {
            if ui
                .add(
                    egui::Button::new(RichText::new("+ Mod").small().color(theme.text_muted))
                        .fill(theme.widget_bg)
                        .min_size(Vec2::new(content_width.max(CHIP_WIDTH), 28.0)),
                )
                .on_hover_text("Add modulator for this device")
                .clicked()
            {
                history.push_before(project.clone());
                let target = target_filter.to_target();
                project.add_modulator(track_id, target, "");
            }
        }
        ModulatorLayout::Wide => {
            ui.allocate_ui_with_layout(
                Vec2::new(CHIP_WIDTH, MODULATOR_CANVAS_HEIGHT + 48.0),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.add_space(MODULATOR_CANVAS_HEIGHT * 0.35);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("+ Mod").small().color(theme.text_muted))
                                .fill(theme.widget_bg)
                                .min_size(Vec2::new(CHIP_WIDTH - 12.0, 28.0)),
                        )
                        .on_hover_text("Add modulator for this device")
                        .clicked()
                    {
                        history.push_before(project.clone());
                        let target = target_filter.to_target();
                        project.add_modulator(track_id, target, "");
                    }
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_modulator_controls(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    modulator_id: u64,
    snapshot: &LfoModulator,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    theme: &ThemeColors,
) {
    ui.spacing_mut().item_spacing.y = MOD_INNER_GAP;

    let param_label = modulator_param_display_name(&snapshot.param_name)
        .map(|name| truncate_label(name, 18))
        .unwrap_or_else(|| "(pick parameter)".to_string());
    ui.label(
        RichText::new(param_label)
            .small()
            .color(theme.text_muted),
    );

    ui.horizontal(|ui| {
        let param_menu_label = modulator_param_display_name(&snapshot.param_name)
            .map(|name| truncate_label(name, 14))
            .unwrap_or_else(|| "Pick parameter...".to_string());
        let param_assigned = modulator_param_assigned(&snapshot.param_name);
        let (fill, stroke, text_color) = chip_menu_colors(param_assigned, theme);
        let param_width = ui.available_width();
        egui::Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .corner_radius(3.0)
            .show(ui, |ui| {
                ui.set_min_width(param_width);
                ui.menu_button(
                    RichText::new(param_menu_label).small().color(text_color),
                    |ui| {
                        let device_id = match &snapshot.target {
                            AutomationTarget::Instrument { .. } => None,
                            AutomationTarget::Device { device_id, .. } => Some(*device_id),
                        };
                        let params = engine.plugin_parameters(track_id, device_id);
                        let plugin_uid = unique_id_for_target(
                            track,
                            match &snapshot.target {
                                AutomationTarget::Instrument { .. } => INSTRUMENT_MOD_TARGET_KEY,
                                AutomationTarget::Device { device_id, .. } => *device_id,
                            },
                        )
                        .map(str::to_string);
                        show_param_pick_menu(
                            ui,
                            settings,
                            settings_dirty,
                            plugin_uid.as_deref(),
                            &params,
                            theme,
                            ParamPickMode::Assign {
                                show_fav_button: true,
                            },
                            "No parameters",
                            24,
                            |param| {
                                history.push_before(project.clone());
                                if let Some(modulator) =
                                    project.modulator_mut(track_id, modulator_id)
                                {
                                    modulator.param_name = param.name.clone();
                                    modulator.target = match &modulator.target {
                                        AutomationTarget::Instrument { .. } => {
                                            AutomationTarget::Instrument {
                                                param_id: param.id,
                                            }
                                        }
                                        AutomationTarget::Device { device_id, .. } => {
                                            AutomationTarget::Device {
                                                device_id: *device_id,
                                                param_id: param.id,
                                            }
                                        }
                                    };
                                }
                            },
                        );
                        if param_assigned {
                            ui.separator();
                            let mapping = match &snapshot.target {
                                AutomationTarget::Instrument { param_id } => MacroMapping {
                                    target: MacroTarget::Instrument {
                                        param_id: *param_id,
                                    },
                                    param_name: snapshot.param_name.clone(),
                                    min: 0.0,
                                    max: 1.0,
                                },
                                AutomationTarget::Device {
                                    device_id,
                                    param_id,
                                } => MacroMapping {
                                    target: MacroTarget::Device {
                                        device_id: *device_id,
                                        param_id: *param_id,
                                    },
                                    param_name: snapshot.param_name.clone(),
                                    min: 0.0,
                                    max: 1.0,
                                },
                            };
                            ui.menu_button("Map to macro", |ui| {
                                show_map_to_macro_menu(
                                    ui, project, history, track_id, mapping, theme,
                                );
                            });
                            if let Some(uid) = unique_id_for_target(
                                track,
                                match &snapshot.target {
                                    AutomationTarget::Instrument { .. } => {
                                        INSTRUMENT_MOD_TARGET_KEY
                                    }
                                    AutomationTarget::Device { device_id, .. } => *device_id,
                                },
                            ) {
                                let param_id = match &snapshot.target {
                                    AutomationTarget::Instrument { param_id }
                                    | AutomationTarget::Device { param_id, .. } => *param_id,
                                };
                                if !settings.has_favorite(uid, param_id)
                                    && ui.button("Add to favorites").clicked()
                                {
                                    let mut changed = settings.add_favorite(
                                        uid,
                                        param_id,
                                        snapshot.param_name.clone(),
                                    );
                                    changed |= settings.touch_param(
                                        uid,
                                        param_id,
                                        snapshot.param_name.clone(),
                                    );
                                    if changed {
                                        *settings_dirty = true;
                                    }
                                    ui.close_menu();
                                }
                            }
                        }
                    },
                );
            });
    });

    ui.add_space(MOD_SECTION_GAP);
    show_shape_selector(
        ui,
        project,
        track_id,
        modulator_id,
        snapshot.shape,
        history,
        theme,
    );

    ui.add_space(MOD_SECTION_GAP);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let half = (ui.available_width() - 4.0) * 0.5;
        let toggle_size = Vec2::new(half.max(36.0), 22.0);
        let is_sync = matches!(snapshot.rate, LfoRate::SyncBeats { .. });
        if chip_toggle_button(ui, "Sync", is_sync, theme, toggle_size) {
            history.push_before(project.clone());
            if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                let beats = match modulator.rate {
                    LfoRate::SyncBeats { beats } => beats,
                    LfoRate::Hz { .. } => 1.0,
                };
                modulator.rate = LfoRate::SyncBeats { beats };
            }
        }
        if chip_toggle_button(ui, "Hz", !is_sync, theme, toggle_size) {
            history.push_before(project.clone());
            if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                let hz = match modulator.rate {
                    LfoRate::Hz { hz } => hz,
                    LfoRate::SyncBeats { .. } => 1.0,
                };
                modulator.rate = LfoRate::Hz { hz };
            }
        }
    });

    ui.add_space(4.0);
    let rate_label = if snapshot.shape == LfoShape::Custom {
        "cycle"
    } else {
        "beats"
    };
    match snapshot.rate {
        LfoRate::SyncBeats { beats } => {
            let mut beats = beats;
            if mod_slider(
                ui,
                egui::Slider::new(&mut beats, 0.0625..=16.0)
                    .text(rate_label)
                    .logarithmic(true),
                theme,
            )
            .changed()
            {
                history.push_before(project.clone());
                if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                    modulator.rate = LfoRate::SyncBeats { beats };
                }
            }
        }
        LfoRate::Hz { hz } => {
            let mut hz = hz;
            if mod_slider(
                ui,
                egui::Slider::new(&mut hz, 0.01..=30.0).text("Hz").logarithmic(true),
                theme,
            )
            .changed()
            {
                history.push_before(project.clone());
                if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                    modulator.rate = LfoRate::Hz { hz };
                }
            }
        }
    }

    let mut depth = snapshot.depth;
    if mod_slider(
        ui,
        egui::Slider::new(&mut depth, 0.0..=1.0).text("depth"),
        theme,
    )
    .changed()
    {
        history.push_before(project.clone());
        if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
            modulator.depth = depth;
        }
    }

    ui.horizontal(|ui| {
        ui.menu_button(RichText::new("Map rate").small(), |ui| {
            let mapping = MacroMapping::new(MacroTarget::ModulatorRate { modulator_id });
            show_map_to_macro_menu(ui, project, history, track_id, mapping, theme);
        });
        ui.menu_button(RichText::new("Map depth").small(), |ui| {
            let mapping = MacroMapping::new(MacroTarget::ModulatorDepth { modulator_id });
            show_map_to_macro_menu(ui, project, history, track_id, mapping, theme);
        });
    });

    ui.add_space(MOD_SECTION_GAP);
    ui.horizontal(|ui| {
        let mut bipolar = snapshot.bipolar;
        if ui.checkbox(&mut bipolar, "Bipolar").changed() {
            history.push_before(project.clone());
            if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                modulator.bipolar = bipolar;
            }
        }
    });

    let _ = track;
}

fn show_mseg_grid_bar(
    ui: &mut Ui,
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    grid_divisions: u8,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Grid").small().color(theme.text_muted));
        ui.add_space(4.0);
        for (label, divisions) in [("Off", 0_u8), ("1/4", 4), ("1/8", 8), ("1/16", 16)] {
            let selected = grid_divisions == divisions;
            if chip_toggle_button(ui, label, selected, theme, Vec2::new(30.0, 20.0)) {
                history.push_before(project.clone());
                if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                    modulator.mseg_grid_divisions = divisions;
                }
            }
        }
    });
}

fn show_modulator_canvas(
    ui: &mut Ui,
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    snapshot: &LfoModulator,
    canvas_size: Vec2,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    let is_custom = snapshot.shape == LfoShape::Custom;
    let sense = if is_custom {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (canvas, response) = ui.allocate_exact_size(canvas_size, sense);
    let painter = ui.painter().with_clip_rect(canvas);
    painter.rect_filled(canvas, 2.0, theme.panel_bg);
    painter.rect_stroke(
        canvas,
        2.0,
        Stroke::new(1.0_f32, theme.separator),
        egui::StrokeKind::Inside,
    );

    if snapshot.bipolar {
        let mid_y = canvas.center().y;
        painter.line_segment(
            [
                Pos2::new(canvas.left(), mid_y),
                Pos2::new(canvas.right(), mid_y),
            ],
            Stroke::new(1.0_f32, theme.separator.gamma_multiply(0.45)),
        );
    }

    let cycle_phase = if engine.is_playing() {
        Some(modulator_cycle_phase01(engine, track_id, modulator_id, snapshot))
    } else {
        None
    };
    if let Some(phase) = cycle_phase {
        paint_cycle_playhead_background(canvas, &painter, phase, theme);
    }

    if is_custom {
        let grid_divisions = snapshot.mseg_grid_divisions;
        if grid_divisions > 0 {
            let grid_stroke = Stroke::new(1.0_f32, theme.separator.gamma_multiply(0.35));
            for step in 1..grid_divisions {
                let t = step as f32 / grid_divisions as f32;
                let x = canvas.left() + t * canvas.width();
                painter.line_segment(
                    [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
                    grid_stroke,
                );
                let y = canvas.bottom() - t * canvas.height();
                painter.line_segment(
                    [Pos2::new(canvas.left(), y), Pos2::new(canvas.right(), y)],
                    grid_stroke,
                );
            }
        }
        paint_custom_curve(canvas, &painter, &snapshot.mseg_points, theme.accent, true);
        handle_custom_curve_input(
            canvas,
            &response,
            project,
            track_id,
            modulator_id,
            snapshot,
            history,
        );
    } else {
        paint_preset_curve(canvas, &painter, snapshot.shape, snapshot.bipolar, theme.accent);
    }

    if let Some(phase) = cycle_phase {
        paint_cycle_playhead_line(canvas, &painter, phase, theme);
    }

    if !is_custom {
        response.on_hover_text("Preset waveform (click MSEG to draw your own)");
    }
}

/// Cycle position `0..1` matching the audio thread's modulator phase.
fn modulator_cycle_phase01(
    engine: &dyn DawEngine,
    track_id: u64,
    modulator_id: u64,
    modulator: &LfoModulator,
) -> f32 {
    let phase_offset = modulator.phase.rem_euclid(1.0);
    match modulator.rate {
        LfoRate::SyncBeats { beats } => {
            let period = beats.max(0.0625);
            (engine.current_beats() / period + phase_offset).rem_euclid(1.0)
        }
        LfoRate::Hz { .. } => {
            let free = engine
                .free_lfo_phase(track_id, modulator_id)
                .unwrap_or(0.0);
            (free + phase_offset).rem_euclid(1.0)
        }
    }
}

fn paint_cycle_playhead_background(
    canvas: egui::Rect,
    painter: &egui::Painter,
    phase01: f32,
    theme: &ThemeColors,
) {
    let phase = phase01.clamp(0.0, 1.0);
    if phase <= 0.0 {
        return;
    }
    let x = canvas.left() + phase * canvas.width();
    let fill = egui::Rect::from_min_max(
        Pos2::new(canvas.left(), canvas.top()),
        Pos2::new(x, canvas.bottom()),
    );
    painter.rect_filled(fill, 0.0, theme.playhead.gamma_multiply(0.12));
}

fn paint_cycle_playhead_line(
    canvas: egui::Rect,
    painter: &egui::Painter,
    phase01: f32,
    theme: &ThemeColors,
) {
    let phase = phase01.clamp(0.0, 1.0);
    let x = canvas.left() + phase * canvas.width();
    painter.line_segment(
        [Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())],
        Stroke::new(1.5_f32, theme.playhead),
    );
}

fn show_shape_selector(
    ui: &mut Ui,
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    current_shape: LfoShape,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let preset_width = ((ui.available_width() - 4.0 * 3.0) / 4.0).max(24.0);
        let preset_size = Vec2::new(preset_width, 22.0);
        for shape in [
            LfoShape::Sine,
            LfoShape::Triangle,
            LfoShape::Saw,
            LfoShape::Square,
        ] {
            if chip_toggle_button(
                ui,
                shape_label(shape),
                current_shape == shape,
                theme,
                preset_size,
            ) {
                set_modulator_shape(project, track_id, modulator_id, shape, history);
            }
        }
    });
    ui.add_space(4.0);
    let is_custom = current_shape == LfoShape::Custom;
    let mseg_width = ui.available_width();
    let mseg_label = if is_custom {
        RichText::new("MSEG (editing)").small().strong()
    } else {
        RichText::new("Draw custom (MSEG)").small().strong()
    };
    if ui
        .add(chip_action_button(
            mseg_label,
            is_custom,
            theme,
            Vec2::new(mseg_width, 26.0),
        ))
        .on_hover_text("Draw a custom multi-segment envelope")
        .clicked()
    {
        set_modulator_shape(
            project,
            track_id,
            modulator_id,
            LfoShape::Custom,
            history,
        );
    }
}

fn chip_button_colors(selected: bool, theme: &ThemeColors) -> (Color32, Color32, Color32) {
    if selected {
        (
            theme.accent,
            theme.accent,
            theme.panel_bg,
        )
    } else {
        (
            theme.panel_bg,
            theme.separator,
            theme.button_text,
        )
    }
}

fn chip_menu_colors(assigned: bool, theme: &ThemeColors) -> (Color32, Color32, Color32) {
    (
        theme.panel_bg,
        theme.separator,
        if assigned {
            theme.text_primary
        } else {
            theme.text_muted
        },
    )
}

fn chip_button(
    text: RichText,
    selected: bool,
    theme: &ThemeColors,
    min_size: Vec2,
) -> egui::Button<'static> {
    let (fill, stroke, text_color) = chip_button_colors(selected, theme);
    egui::Button::new(text.color(text_color))
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(3.0)
        .min_size(min_size)
}

fn chip_toggle_button(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    theme: &ThemeColors,
    min_size: Vec2,
) -> bool {
    ui.add(chip_button(
        RichText::new(label).small().strong(),
        selected,
        theme,
        min_size,
    ))
    .clicked()
}

fn chip_action_button(
    text: RichText,
    active: bool,
    theme: &ThemeColors,
    min_size: Vec2,
) -> egui::Button<'static> {
    if active {
        chip_button(text, true, theme, min_size)
    } else {
        egui::Button::new(text.color(theme.accent))
            .fill(theme.panel_bg)
            .stroke(Stroke::new(1.5_f32, theme.accent.gamma_multiply(0.75)))
            .corner_radius(3.0)
            .min_size(min_size)
    }
}

fn mod_slider(
    ui: &mut Ui,
    slider: egui::Slider<'_>,
    theme: &ThemeColors,
) -> egui::Response {
    egui::Frame::new()
        .fill(theme.panel_bg)
        .stroke(Stroke::new(
            1.0_f32,
            theme.separator.gamma_multiply(0.55),
        ))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add(slider.trailing_fill(true))
        })
        .response
}

fn set_modulator_shape(
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    shape: LfoShape,
    history: &mut EditHistory,
) {
    history.push_before(project.clone());
    let Some(modulator) = project.modulator_mut(track_id, modulator_id) else {
        return;
    };
    let prev_shape = modulator.shape;
    modulator.shape = shape;
    if shape == LfoShape::Custom {
        if prev_shape != LfoShape::Custom {
            modulator.mseg_points = bake_shape_to_mseg_points(prev_shape, modulator.bipolar);
        } else if modulator.mseg_points.is_empty() {
            modulator.mseg_points = default_mseg_points();
        }
    }
}

fn paint_preset_curve(
    canvas: egui::Rect,
    painter: &egui::Painter,
    shape: LfoShape,
    bipolar: bool,
    color: Color32,
) {
    let stroke = Stroke::new(1.5_f32, color);
    let mut prev: Option<Pos2> = None;
    for step in 0..=PRESET_WAVE_SAMPLES {
        let cycle = step as f32 / PRESET_WAVE_SAMPLES as f32;
        let wave = preview_lfo_wave(shape, f64::from(cycle)) as f32;
        let pos = signal_to_canvas_pos(canvas, cycle, wave, bipolar);
        if let Some(prev_pos) = prev {
            painter.line_segment([prev_pos, pos], stroke);
        }
        prev = Some(pos);
    }
}

fn paint_custom_curve(
    canvas: egui::Rect,
    painter: &egui::Painter,
    points: &[AutomationPoint],
    color: Color32,
    show_handles: bool,
) {
    if points.is_empty() {
        return;
    }
    let mut sorted: Vec<&AutomationPoint> = points.iter().collect();
    sorted.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let stroke = Stroke::new(1.5_f32, color);
    let mut prev: Option<Pos2> = None;
    for point in &sorted {
        let pos = mseg_point_to_pos(canvas, point.beat, point.value);
        if let Some(prev_pos) = prev {
            painter.line_segment([prev_pos, pos], stroke);
        }
        if show_handles {
            painter.circle_filled(pos, MSEG_POINT_RADIUS, color);
        }
        prev = Some(pos);
    }
}

fn handle_custom_curve_input(
    canvas: egui::Rect,
    response: &egui::Response,
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    snapshot: &LfoModulator,
    history: &mut EditHistory,
) {
    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    let points = &snapshot.mseg_points;
    let near = nearest_mseg_point_index(canvas, points, pointer);
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(index) = near {
            if response.drag_started() {
                history.push_before(project.clone());
            }
            if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                let (cycle, value) =
                    mseg_pos_to_cycle_value(canvas, pointer, modulator.mseg_grid_divisions);
                modulator.mseg_points[index].beat = cycle;
                modulator.mseg_points[index].value = value;
                modulator
                    .mseg_points
                    .sort_by(|a, b| a.beat.total_cmp(&b.beat));
            }
        }
    } else if response.clicked_by(egui::PointerButton::Primary) && near.is_none() {
        let (cycle, value) =
            mseg_pos_to_cycle_value(canvas, pointer, snapshot.mseg_grid_divisions);
        history.push_before(project.clone());
        if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
            modulator.mseg_points.push(AutomationPoint {
                beat: cycle,
                value,
                curve: CurveKind::Linear,
            });
            modulator
                .mseg_points
                .sort_by(|a, b| a.beat.total_cmp(&b.beat));
        }
    } else if response.clicked_by(egui::PointerButton::Secondary) {
        history.push_before(project.clone());
        if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
            if modulator.mseg_points.len() > 2 {
                if let Some(index) = near {
                    modulator.mseg_points.remove(index);
                }
            }
        }
    }
}

/// Matches [`crate::engine::audio`] preset evaluation for canvas preview.
fn preview_lfo_wave(shape: LfoShape, phase01: f64) -> f64 {
    let p = phase01.rem_euclid(1.0);
    match shape {
        LfoShape::Sine => (p * std::f64::consts::TAU).sin(),
        LfoShape::Triangle => {
            if p < 0.25 {
                p * 4.0
            } else if p < 0.75 {
                2.0 - p * 4.0
            } else {
                p * 4.0 - 4.0
            }
        }
        LfoShape::Saw => 2.0 * p - 1.0,
        LfoShape::Square => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::Custom => 0.0,
    }
}

fn signal_to_canvas_pos(canvas: egui::Rect, cycle: f32, bipolar_wave: f32, bipolar: bool) -> Pos2 {
    let x = canvas.left() + cycle.clamp(0.0, 1.0) * canvas.width();
    let y = if bipolar {
        canvas.center().y - bipolar_wave.clamp(-1.0, 1.0) * canvas.height() * 0.5
    } else {
        let unipolar = ((bipolar_wave + 1.0) * 0.5).clamp(0.0, 1.0);
        canvas.bottom() - unipolar * canvas.height()
    };
    Pos2::new(x, y)
}

fn bake_shape_to_mseg_points(shape: LfoShape, _bipolar: bool) -> Vec<AutomationPoint> {
    if shape == LfoShape::Custom {
        return default_mseg_points();
    }
    let steps = match shape {
        LfoShape::Sine => 6,
        LfoShape::Triangle | LfoShape::Saw => 2,
        LfoShape::Square => 4,
        LfoShape::Custom => 0,
    };
    (0..=steps)
        .map(|step| {
            let cycle = step as f32 / steps as f32;
            let wave = preview_lfo_wave(shape, f64::from(cycle)) as f32;
            let value = ((wave + 1.0) * 0.5).clamp(0.0, 1.0);
            AutomationPoint {
                beat: cycle,
                value,
                curve: CurveKind::Linear,
            }
        })
        .collect()
}

fn mseg_point_to_pos(canvas: egui::Rect, cycle: f32, value: f32) -> Pos2 {
    Pos2::new(
        canvas.left() + cycle.clamp(0.0, 1.0) * canvas.width(),
        canvas.bottom() - value.clamp(0.0, 1.0) * canvas.height(),
    )
}

fn mseg_pos_to_cycle_value(
    canvas: egui::Rect,
    pos: Pos2,
    grid_divisions: u8,
) -> (f32, f32) {
    let mut cycle = ((pos.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0);
    let mut value = ((canvas.bottom() - pos.y) / canvas.height()).clamp(0.0, 1.0);
    if grid_divisions > 0 {
        cycle = snap_mseg_axis(cycle, grid_divisions);
        value = snap_mseg_axis(value, grid_divisions);
    }
    (cycle, value)
}

fn snap_mseg_axis(value: f32, divisions: u8) -> f32 {
    let step = 1.0 / divisions as f32;
    (value / step).round() * step
}

fn nearest_mseg_point_index(
    canvas: egui::Rect,
    points: &[AutomationPoint],
    pos: Pos2,
) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let pa = mseg_point_to_pos(canvas, a.beat, a.value);
            let pb = mseg_point_to_pos(canvas, b.beat, b.value);
            pa.distance(pos)
                .partial_cmp(&pb.distance(pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|(_, point)| {
            mseg_point_to_pos(canvas, point.beat, point.value).distance(pos)
                <= MSEG_POINT_RADIUS * 2.5
        })
        .map(|(index, _)| index)
}

fn default_mseg_points() -> Vec<AutomationPoint> {
    vec![
        AutomationPoint {
            beat: 0.0,
            value: 0.0,
            curve: CurveKind::Linear,
        },
        AutomationPoint {
            beat: 0.5,
            value: 1.0,
            curve: CurveKind::Linear,
        },
        AutomationPoint {
            beat: 1.0,
            value: 0.0,
            curve: CurveKind::Linear,
        },
    ]
}

fn modulator_display_name(index: usize, modulator: &LfoModulator) -> String {
    let label = if modulator.name.trim().is_empty() {
        format!("Mod {}", index + 1)
    } else {
        modulator.name.trim().to_string()
    };
    if let Some(param_name) = modulator_param_display_name(&modulator.param_name) {
        format!("{} ({})", label, truncate_label(param_name, 14))
    } else {
        label
    }
}

fn modulator_param_assigned(param_name: &str) -> bool {
    !param_name.is_empty() && param_name != "Parameter"
}

fn modulator_param_display_name(param_name: &str) -> Option<&str> {
    if modulator_param_assigned(param_name) {
        Some(param_name)
    } else {
        None
    }
}

fn shape_label(shape: LfoShape) -> &'static str {
    match shape {
        LfoShape::Sine => "Sine",
        LfoShape::Triangle => "Tri",
        LfoShape::Saw => "Saw",
        LfoShape::Square => "Sqr",
        LfoShape::Custom => "Custom",
    }
}

fn truncate_label(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let trimmed: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{trimmed}...")
    }
}
