use egui::{Align, Layout, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, Vec2};

use crate::engine::{DawEngine, PluginParamInfo};
use crate::model::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveKind, EditHistory, Project, Track,
};
use crate::ui::app_settings::AppSettings;
use crate::ui::favorites_panel::unique_id_for_target;
use crate::ui::modulator::INSTRUMENT_MOD_TARGET_KEY;
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    draw_timeline_grid_lines, timeline_x, x_to_beat, TimelineMetrics, TIMELINE_GUTTER_WIDTH,
};

pub const AUTOMATION_LANE_BODY_HEIGHT: f32 = 56.0;
pub const ADD_AUTOMATION_ROW_HEIGHT: f32 = 22.0;
const HANDLE_RADIUS: f32 = 5.0;
const HIT_RADIUS: f32 = 9.0;

/// Extra vertical space under a track's clip lane when automation is expanded.
pub fn automation_extra_height(lane_count: usize, expanded: bool) -> f32 {
    if !expanded {
        return 0.0;
    }
    lane_count as f32 * AUTOMATION_LANE_BODY_HEIGHT + ADD_AUTOMATION_ROW_HEIGHT
}

#[derive(Debug, Clone)]
struct PointDrag {
    track_id: u64,
    lane_id: u64,
    point_index: usize,
}

#[derive(Default)]
pub struct AutomationUi {
    active_drag: Option<PointDrag>,
}

impl AutomationUi {
    /// Sticky-left controls for one automation lane (param/target menus, on/off, remove).
    #[allow(clippy::too_many_arguments)]
    pub fn show_lane_header(
        &self,
        ui: &mut Ui,
        header: Rect,
        project: &mut Project,
        track_id: u64,
        lane_id: u64,
        track: &Track,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        settings: &mut AppSettings,
        settings_dirty: &mut bool,
        theme: &ThemeColors,
    ) {
        let Some(lane) = project.automation_lane(track_id, lane_id).cloned() else {
            return;
        };

        ui.painter().rect_filled(header, 0.0, theme.track_header_bg);
        ui.painter().line_segment(
            [
                Pos2::new(header.left(), header.bottom()),
                Pos2::new(header.right(), header.bottom()),
            ],
            Stroke::new(1.0_f32, theme.separator),
        );

        ui.allocate_ui_at_rect(header.shrink2(Vec2::new(2.0, 1.0)), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(2.0, 1.0);
            ui.vertical(|ui| {
                let param_label = if lane.param_name.is_empty() {
                    "(pick parameter)"
                } else {
                    &lane.param_name
                };
                ui.label(
                    RichText::new(truncate_label(param_label, 14))
                        .color(theme.track_header_text)
                        .small(),
                );
                ui.horizontal(|ui| {
                    ui.menu_button("T", |ui| {
                        if ui.button("Instrument").clicked() {
                            history.push_before(project.clone());
                            if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
                                lane.target = AutomationTarget::Instrument { param_id: 0 };
                                lane.param_name.clear();
                                lane.param_min = 0.0;
                                lane.param_max = 1.0;
                                lane.points.clear();
                            }
                            ui.close_menu();
                        }
                        for device in &track.devices {
                            let label = truncate_label(&device.name, 24);
                            if ui.button(label).clicked() {
                                history.push_before(project.clone());
                                if let Some(lane) = project.automation_lane_mut(track_id, lane_id)
                                {
                                    lane.target = AutomationTarget::Device {
                                        device_id: device.id,
                                        param_id: 0,
                                    };
                                    lane.param_name.clear();
                                    lane.param_min = 0.0;
                                    lane.param_max = 1.0;
                                    lane.points.clear();
                                }
                                ui.close_menu();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Target");

                    ui.menu_button("P", |ui| {
                        let device_id = target_device_id(&lane.target);
                        let params = engine.plugin_parameters(track_id, device_id);
                        if params.is_empty() {
                            ui.label(
                                RichText::new("No parameters (load plugin first)")
                                    .color(theme.text_muted)
                                    .small(),
                            );
                        } else {
                            let plugin_uid = unique_id_for_target(
                                track,
                                match &lane.target {
                                    AutomationTarget::Instrument { .. } => {
                                        INSTRUMENT_MOD_TARGET_KEY
                                    }
                                    AutomationTarget::Device { device_id, .. } => *device_id,
                                },
                            )
                            .map(str::to_string);
                            for param in params {
                                if !param.automatable {
                                    continue;
                                }
                                ui.horizontal(|ui| {
                                    let label = truncate_label(&param.name, 28);
                                    if ui.button(label).clicked() {
                                        apply_param_selection(
                                            project,
                                            track_id,
                                            lane_id,
                                            &lane.target,
                                            &param,
                                            history,
                                        );
                                        ui.close_menu();
                                    }
                                    if let Some(uid) = &plugin_uid {
                                        let starred = settings.has_favorite(uid, param.id);
                                        if ui
                                            .add_enabled(
                                                !starred,
                                                egui::Button::new(if starred {
                                                    "fav"
                                                } else {
                                                    "+fav"
                                                })
                                                .small(),
                                            )
                                            .on_hover_text(if starred {
                                                "Already a favorite"
                                            } else {
                                                "Add to favorites"
                                            })
                                            .clicked()
                                        {
                                            if settings.add_favorite(
                                                uid,
                                                param.id,
                                                param.name.clone(),
                                            ) {
                                                *settings_dirty = true;
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    })
                    .response
                    .on_hover_text("Parameter");

                    let mut enabled = lane.enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        history.push_before(project.clone());
                        if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
                            lane.enabled = enabled;
                        }
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("x").on_hover_text("Remove lane").clicked() {
                            history.push_before(project.clone());
                            project.remove_automation_lane(track_id, lane_id);
                        }
                    });
                });
            });
        });
    }

    /// Draw + edit one automation lane timeline body (shared playlist scroll).
    #[allow(clippy::too_many_arguments)]
    pub fn show_lane_timeline(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        metrics: TimelineMetrics,
        project: &mut Project,
        track_id: u64,
        lane_id: u64,
        history: &mut EditHistory,
        theme: &ThemeColors,
        total_beats: f32,
        beats_per_bar: f32,
    ) {
        let id = ui.id().with(("automation_lane_timeline", track_id, lane_id));
        let response = ui.interact(body, id, Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(body);

        painter.rect_filled(body, 0.0, theme.panel_bg.gamma_multiply(0.85));
        draw_timeline_grid_lines(
            &painter,
            body,
            metrics,
            total_beats,
            beats_per_bar,
            theme,
        );

        if let Some(lane) = project.automation_lane(track_id, lane_id) {
            draw_automation_curve(&painter, body, lane, metrics, theme);
            if !lane.enabled {
                painter.rect_filled(
                    body,
                    0.0,
                    egui::Color32::from_black_alpha(80),
                );
            }
        }

        painter.line_segment(
            [
                Pos2::new(body.left(), body.bottom()),
                Pos2::new(body.right(), body.bottom()),
            ],
            Stroke::new(1.0_f32, theme.separator),
        );

        handle_lane_pointer(
            &response,
            body,
            metrics,
            project,
            track_id,
            lane_id,
            history,
            &mut self.active_drag,
        );
    }

    /// Compact "+ Add lane" control for the sticky header column.
    pub fn show_add_lane_row(
        &self,
        ui: &mut Ui,
        row: Rect,
        project: &mut Project,
        track_id: u64,
        history: &mut EditHistory,
        theme: &ThemeColors,
    ) {
        ui.painter().rect_filled(row, 0.0, theme.track_header_bg);
        ui.allocate_ui_at_rect(row.shrink2(Vec2::new(4.0, 2.0)), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("+ Auto")
                            .small()
                            .color(theme.text_muted),
                    )
                    .fill(theme.widget_bg)
                    .min_size(Vec2::new(row.width() - 8.0, row.height() - 4.0)),
                )
                .on_hover_text("Add automation lane")
                .clicked()
            {
                history.push_before(project.clone());
                project.add_automation_lane(
                    track_id,
                    AutomationTarget::Instrument { param_id: 0 },
                    "Parameter",
                    0.0,
                    1.0,
                );
            }
        });
    }
}

fn apply_param_selection(
    project: &mut Project,
    track_id: u64,
    lane_id: u64,
    target: &AutomationTarget,
    param: &PluginParamInfo,
    history: &mut EditHistory,
) {
    history.push_before(project.clone());
    let Some(lane) = project.automation_lane_mut(track_id, lane_id) else {
        return;
    };
    lane.param_name = param.name.clone();
    lane.param_min = param.min;
    lane.param_max = param.max;
    lane.target = match target {
        AutomationTarget::Instrument { .. } => AutomationTarget::Instrument {
            param_id: param.id,
        },
        AutomationTarget::Device { device_id, .. } => AutomationTarget::Device {
            device_id: *device_id,
            param_id: param.id,
        },
    };
}

fn target_device_id(target: &AutomationTarget) -> Option<u64> {
    match target {
        AutomationTarget::Instrument { .. } => None,
        AutomationTarget::Device { device_id, .. } => Some(*device_id),
    }
}

fn value_to_y(body: Rect, value: f32) -> f32 {
    let t = value.clamp(0.0, 1.0);
    body.bottom() - t * body.height()
}

fn y_to_value(body: Rect, y: f32) -> f32 {
    ((body.bottom() - y) / body.height()).clamp(0.0, 1.0)
}

fn draw_automation_curve(
    painter: &egui::Painter,
    body: Rect,
    lane: &AutomationLane,
    metrics: TimelineMetrics,
    theme: &ThemeColors,
) {
    if lane.points.is_empty() {
        return;
    }

    let mut sorted: Vec<&AutomationPoint> = lane.points.iter().collect();
    sorted.sort_by(|a, b| a.beat.total_cmp(&b.beat));

    let stroke = Stroke::new(2.0_f32, theme.accent);
    let mut prev: Option<Pos2> = None;

    for (index, point) in sorted.iter().enumerate() {
        let pos = Pos2::new(
            timeline_x(body, point.beat, metrics),
            value_to_y(body, point.value),
        );

        if let Some(prev_pos) = prev {
            if matches!(sorted[index - 1].curve, CurveKind::Hold) {
                painter.line_segment([prev_pos, Pos2::new(pos.x, prev_pos.y)], stroke);
                painter.line_segment([Pos2::new(pos.x, prev_pos.y), pos], stroke);
            } else {
                painter.line_segment([prev_pos, pos], stroke);
            }
        }
        prev = Some(pos);
    }

    for point in sorted {
        let center = Pos2::new(
            timeline_x(body, point.beat, metrics),
            value_to_y(body, point.value),
        );
        painter.circle_filled(center, HANDLE_RADIUS, theme.accent);
        painter.circle_stroke(
            center,
            HANDLE_RADIUS,
            Stroke::new(1.0_f32, theme.track_header_text),
        );
    }
}

fn hit_test_point(body: Rect, lane: &AutomationLane, pos: Pos2, metrics: TimelineMetrics) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (index, point) in lane.points.iter().enumerate() {
        let center = Pos2::new(
            timeline_x(body, point.beat, metrics),
            value_to_y(body, point.value),
        );
        let dist = center.distance(pos);
        if dist <= HIT_RADIUS {
            match best {
                Some((_, best_dist)) if dist >= best_dist => {}
                _ => best = Some((index, dist)),
            }
        }
    }
    best.map(|(index, _)| index)
}

fn is_lane_timeline_pointer(body: Rect, pointer: Pos2) -> bool {
    body.contains(pointer) && pointer.x > body.left() + TIMELINE_GUTTER_WIDTH
}

fn beat_from_pointer(body: Rect, pointer: Pos2, metrics: TimelineMetrics, snap: bool) -> f32 {
    let mut beat = x_to_beat(body, pointer.x, metrics).max(0.0);
    if snap {
        beat = Project::snap_beats(beat);
    }
    beat
}

fn sort_lane_points(project: &mut Project, track_id: u64, lane_id: u64) {
    if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
        lane.points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
}

fn handle_lane_pointer(
    response: &Response,
    body: Rect,
    metrics: TimelineMetrics,
    project: &mut Project,
    track_id: u64,
    lane_id: u64,
    history: &mut EditHistory,
    active_drag: &mut Option<PointDrag>,
) {
    let primary_down = response
        .ctx
        .input(|input| input.pointer.button_down(egui::PointerButton::Primary));

    if response.drag_stopped() || (!primary_down && active_drag.is_some()) {
        if active_drag
            .as_ref()
            .is_some_and(|drag| drag.track_id == track_id && drag.lane_id == lane_id)
        {
            sort_lane_points(project, track_id, lane_id);
            history.commit(project);
            *active_drag = None;
        }
    }

    let Some(pointer) = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos())
    else {
        return;
    };

    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    let snap = !response.ctx.input(|input| input.modifiers.alt);

    if let Some(drag) = active_drag.clone() {
        if drag.track_id == track_id
            && drag.lane_id == lane_id
            && primary_down
            && (response.dragged() || response.drag_started())
        {
            let beat = beat_from_pointer(body, pointer, metrics, snap);
            let value = y_to_value(body, pointer.y);
            if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
                if let Some(point) = lane.points.get_mut(drag.point_index) {
                    point.beat = beat;
                    point.value = value;
                }
            }
        }
        return;
    }

    if !body.contains(pointer) && !body.contains(press_pos) {
        return;
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if let Some(lane) = project.automation_lane(track_id, lane_id) {
            if let Some(index) = hit_test_point(body, lane, pointer, metrics) {
                history.push_before(project.clone());
                if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
                    lane.points.remove(index);
                }
                return;
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && is_lane_timeline_pointer(body, pointer)
    {
        if let Some(lane) = project.automation_lane(track_id, lane_id) {
            if hit_test_point(body, lane, pointer, metrics).is_none() {
                let beat = beat_from_pointer(body, pointer, metrics, snap);
                let value = y_to_value(body, pointer.y);
                history.push_before(project.clone());
                if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
                    lane.points.push(AutomationPoint {
                        beat,
                        value,
                        curve: CurveKind::Linear,
                    });
                    sort_lane_points(project, track_id, lane_id);
                }
            }
        }
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_lane_timeline_pointer(body, press_pos)
    {
        if let Some(lane) = project.automation_lane(track_id, lane_id) {
            if let Some(index) = hit_test_point(body, lane, press_pos, metrics) {
                history.begin(project);
                *active_drag = Some(PointDrag {
                    track_id,
                    lane_id,
                    point_index: index,
                });
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_y_round_trip() {
        let body = Rect::from_min_max(Pos2::new(0.0, 10.0), Pos2::new(100.0, 50.0));
        assert!((y_to_value(body, value_to_y(body, 0.0)) - 0.0).abs() < 0.001);
        assert!((y_to_value(body, value_to_y(body, 1.0)) - 1.0).abs() < 0.001);
        assert!((y_to_value(body, value_to_y(body, 0.42)) - 0.42).abs() < 0.001);
    }

    #[test]
    fn automation_extra_height_collapsed_is_zero() {
        assert_eq!(automation_extra_height(3, false), 0.0);
    }

    #[test]
    fn automation_extra_height_expanded() {
        assert_eq!(
            automation_extra_height(2, true),
            2.0 * AUTOMATION_LANE_BODY_HEIGHT + ADD_AUTOMATION_ROW_HEIGHT
        );
    }
}
