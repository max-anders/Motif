//! LFO / MSEG modulator chips shown under device tiles in the Devices view and strip.

use egui::{Pos2, RichText, Sense, Stroke, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    AutomationPoint, AutomationTarget, CurveKind, EditHistory, LfoModulator, LfoRate, LfoShape,
    Project, Track,
};
use crate::ui::theme::ThemeColors;

pub const CHIP_WIDTH: f32 = 146.0;
const CHIP_INNER_WIDTH: f32 = CHIP_WIDTH - 12.0;
const LFO_CONTROLS_WIDTH: f32 = CHIP_INNER_WIDTH;
/// Typical rendered height for a standard LFO chip (strip panel budgeting).
pub const CHIP_HEIGHT_STANDARD: f32 = 158.0;
/// Rendered height for an MSEG chip (header + canvas row).
pub const CHIP_HEIGHT_MSEG: f32 = 184.0;
const MSEG_GRID_STRIP_WIDTH: f32 = 36.0;
const MSEG_CANVAS_WIDTH: f32 = 168.0;
const MSEG_CANVAS_HEIGHT: f32 = 148.0;
const MSEG_CHIP_INNER_WIDTH: f32 =
    MSEG_GRID_STRIP_WIDTH + 6.0 + MSEG_CANVAS_WIDTH + 8.0 + LFO_CONTROLS_WIDTH;
const MSEG_POINT_RADIUS: f32 = 4.0;

/// Render modulators for one target (instrument or a specific insert FX), plus an add button.
#[allow(clippy::too_many_arguments)]
pub fn show_modulators_for_target(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    target_filter: TargetFilter,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    let modulator_ids: Vec<u64> = track
        .modulators
        .iter()
        .filter(|modulator| target_filter.matches(&modulator.target))
        .map(|modulator| modulator.id)
        .collect();

    for modulator_id in modulator_ids {
        show_modulator_chip(
            ui,
            project,
            track,
            track_id,
            modulator_id,
            engine,
            history,
            theme,
        );
        ui.add_space(4.0);
    }

    let add_label = match target_filter {
        TargetFilter::Instrument => "+ LFO",
        TargetFilter::Device { .. } => "+ LFO",
    };
    if ui
        .add(
            egui::Button::new(RichText::new(add_label).small().color(theme.text_muted))
                .fill(theme.widget_bg)
                .min_size(Vec2::new(CHIP_WIDTH, 22.0)),
        )
        .on_hover_text("Add LFO / MSEG modulator for this device")
        .clicked()
    {
        history.push_before(project.clone());
        let target = target_filter.to_target();
        project.add_modulator(track_id, target, "Parameter");
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

#[allow(clippy::too_many_arguments)]
fn show_modulator_chip(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    modulator_id: u64,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    let Some(snapshot) = project.modulator(track_id, modulator_id).cloned() else {
        return;
    };

    egui::Frame::new()
        .fill(theme.widget_bg)
        .stroke(Stroke::new(1.0_f32, theme.separator))
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            let is_mseg = snapshot.shape == LfoShape::Custom;
            ui.set_width(if is_mseg {
                MSEG_CHIP_INNER_WIDTH
            } else {
                CHIP_INNER_WIDTH
            });
            ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if is_mseg { "MSEG" } else { "LFO" })
                        .small()
                        .strong()
                        .color(theme.track_header_text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").on_hover_text("Remove modulator").clicked() {
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

            if is_mseg {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(MSEG_GRID_STRIP_WIDTH + 6.0 + MSEG_CANVAS_WIDTH);
                        show_mseg_editor(
                            ui,
                            project,
                            track_id,
                            modulator_id,
                            &snapshot,
                            history,
                            theme,
                        );
                    });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_width(LFO_CONTROLS_WIDTH);
                        paint_lfo_controls(
                            ui,
                            project,
                            track,
                            track_id,
                            modulator_id,
                            &snapshot,
                            engine,
                            history,
                            theme,
                        );
                    });
                });
            } else {
                paint_lfo_controls(
                    ui,
                    project,
                    track,
                    track_id,
                    modulator_id,
                    &snapshot,
                    engine,
                    history,
                    theme,
                );
            }

            let _ = track;
        });
}

#[allow(clippy::too_many_arguments)]
fn paint_lfo_controls(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    track_id: u64,
    modulator_id: u64,
    snapshot: &LfoModulator,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
            let param_label = if snapshot.param_name.is_empty() {
                "(pick parameter)"
            } else {
                &snapshot.param_name
            };
            ui.label(
                RichText::new(truncate_label(param_label, 18))
                    .small()
                    .color(theme.text_muted),
            );

            ui.horizontal(|ui| {
                ui.menu_button("Param", |ui| {
                    let device_id = match &snapshot.target {
                        AutomationTarget::Instrument { .. } => None,
                        AutomationTarget::Device { device_id, .. } => Some(*device_id),
                    };
                    let params = engine.plugin_parameters(track_id, device_id);
                    if params.is_empty() {
                        ui.label(
                            RichText::new("No parameters")
                                .small()
                                .color(theme.text_muted),
                        );
                    } else {
                        for param in params {
                            if !param.automatable {
                                continue;
                            }
                            if ui.button(truncate_label(&param.name, 28)).clicked() {
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
                                ui.close_menu();
                            }
                        }
                    }
                });

                ui.menu_button(shape_label(snapshot.shape), |ui| {
                    for shape in [
                        LfoShape::Sine,
                        LfoShape::Triangle,
                        LfoShape::Saw,
                        LfoShape::Square,
                        LfoShape::Custom,
                    ] {
                        if ui
                            .selectable_label(snapshot.shape == shape, shape_label(shape))
                            .clicked()
                        {
                            history.push_before(project.clone());
                            if let Some(modulator) = project.modulator_mut(track_id, modulator_id)
                            {
                                modulator.shape = shape;
                                if shape == LfoShape::Custom && modulator.mseg_points.is_empty() {
                                    modulator.mseg_points = default_mseg_points();
                                }
                            }
                            ui.close_menu();
                        }
                    }
                });
            });

            ui.horizontal(|ui| {
                let is_sync = matches!(snapshot.rate, LfoRate::SyncBeats { .. });
                if ui.selectable_label(is_sync, "Sync").clicked() {
                    history.push_before(project.clone());
                    if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                        let beats = match modulator.rate {
                            LfoRate::SyncBeats { beats } => beats,
                            LfoRate::Hz { .. } => 1.0,
                        };
                        modulator.rate = LfoRate::SyncBeats { beats };
                    }
                }
                if ui.selectable_label(!is_sync, "Hz").clicked() {
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

            let rate_label = if snapshot.shape == LfoShape::Custom {
                "cycle"
            } else {
                "beats"
            };
            match snapshot.rate {
                LfoRate::SyncBeats { beats } => {
                    let mut beats = beats;
                    if ui
                        .add(
                            egui::Slider::new(&mut beats, 0.0625..=16.0)
                                .text(rate_label)
                                .logarithmic(true),
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
                    if ui
                        .add(egui::Slider::new(&mut hz, 0.01..=30.0).text("Hz").logarithmic(true))
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
            if ui
                .add(egui::Slider::new(&mut depth, 0.0..=1.0).text("depth"))
                .changed()
            {
                history.push_before(project.clone());
                if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                    modulator.depth = depth;
                }
            }

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

fn show_mseg_editor(
    ui: &mut Ui,
    project: &mut Project,
    track_id: u64,
    modulator_id: u64,
    snapshot: &LfoModulator,
    history: &mut EditHistory,
    theme: &ThemeColors,
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(MSEG_GRID_STRIP_WIDTH);
            ui.label(RichText::new("Grid").small().color(theme.text_muted));
            for (label, divisions) in [("Off", 0_u8), ("1/4", 4), ("1/8", 8), ("1/16", 16)] {
                let selected = snapshot.mseg_grid_divisions == divisions;
                if ui.selectable_label(selected, label).clicked() {
                    history.push_before(project.clone());
                    if let Some(modulator) = project.modulator_mut(track_id, modulator_id) {
                        modulator.mseg_grid_divisions = divisions;
                    }
                }
            }
        });

        ui.add_space(6.0);

        let (canvas, response) = ui.allocate_exact_size(
            Vec2::new(MSEG_CANVAS_WIDTH, MSEG_CANVAS_HEIGHT),
            Sense::click_and_drag(),
        );
        let painter = ui.painter().with_clip_rect(canvas);
        painter.rect_filled(canvas, 2.0, theme.panel_bg);
        painter.rect_stroke(
            canvas,
            2.0,
            Stroke::new(1.0_f32, theme.separator),
            egui::StrokeKind::Inside,
        );

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

        let points = &snapshot.mseg_points;
        if !points.is_empty() {
            let mut sorted: Vec<&AutomationPoint> = points.iter().collect();
            sorted.sort_by(|a, b| a.beat.total_cmp(&b.beat));
            let stroke = Stroke::new(1.5_f32, theme.accent);
            let mut prev: Option<Pos2> = None;
            for point in &sorted {
                let pos = mseg_point_to_pos(canvas, point.beat, point.value);
                if let Some(prev_pos) = prev {
                    painter.line_segment([prev_pos, pos], stroke);
                }
                painter.circle_filled(pos, MSEG_POINT_RADIUS, theme.accent);
                prev = Some(pos);
            }
        }

        if let Some(pointer) = response.interact_pointer_pos() {
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
    });
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
            mseg_point_to_pos(canvas, point.beat, point.value).distance(pos) <= MSEG_POINT_RADIUS * 2.5
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

fn shape_label(shape: LfoShape) -> &'static str {
    match shape {
        LfoShape::Sine => "Sine",
        LfoShape::Triangle => "Tri",
        LfoShape::Saw => "Saw",
        LfoShape::Square => "Sqr",
        LfoShape::Custom => "MSEG",
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
