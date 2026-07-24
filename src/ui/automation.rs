use egui::containers::scroll_area::ScrollBarVisibility;
use egui::{Align, Layout, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};

use crate::engine::{DawEngine, PluginParamInfo};
use crate::model::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveKind, EditHistory, Project,
};
use crate::ui::playlist::TRACK_HEADER_WIDTH;
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    draw_playhead, draw_timeline_grid_lines, timeline_x, with_solid_scrollbars, x_to_beat,
    TimelineMetrics, TIMELINE_GUTTER_WIDTH,
};

pub const AUTOMATION_LANE_BODY_HEIGHT: f32 = 56.0;
const LANE_HEADER_HEIGHT: f32 = 24.0;
const HANDLE_RADIUS: f32 = 5.0;
const HIT_RADIUS: f32 = 9.0;

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
    #[allow(clippy::too_many_arguments)]
    pub fn show_page_section(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        track_id: u64,
        track: &crate::model::Track,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        beat_width: f32,
        scroll_offset: Vec2,
        theme: &ThemeColors,
    ) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Automation")
                    .color(theme.track_header_text)
                    .strong(),
            );
            if ui.button("+ Add lane").clicked() {
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
        ui.add_space(4.0);

        if track.automation_lanes.is_empty() {
            ui.label(
                RichText::new("No automation lanes. Add one and pick a plugin parameter.")
                    .color(theme.text_muted)
                    .small(),
            );
            return;
        }

        let lane_ids: Vec<u64> = track.automation_lanes.iter().map(|lane| lane.id).collect();
        for lane_id in lane_ids {
            ui.add_space(4.0);
            self.show_lane_row(
                ui,
                project,
                track_id,
                lane_id,
                track,
                engine,
                history,
                beat_width,
                scroll_offset,
                theme,
                false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show_strip_section(
        &mut self,
        ui: &mut Ui,
        expanded: &mut bool,
        project: &mut Project,
        track_id: u64,
        track: &crate::model::Track,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        beat_width: f32,
        scroll_offset: Vec2,
        theme: &ThemeColors,
    ) {
        ui.horizontal(|ui| {
            let label = if *expanded {
                "Automation v"
            } else {
                "Automation >"
            };
            if ui.button(label).clicked() {
                *expanded = !*expanded;
            }
            if *expanded {
                if ui.small_button("+ lane").clicked() {
                    history.push_before(project.clone());
                    project.add_automation_lane(
                        track_id,
                        AutomationTarget::Instrument { param_id: 0 },
                        "Parameter",
                        0.0,
                        1.0,
                    );
                }
            }
        });

        if !*expanded {
            return;
        }

        if track.automation_lanes.is_empty() {
            ui.label(
                RichText::new("No lanes yet")
                    .color(theme.text_muted)
                    .small(),
            );
            return;
        }

        let lane_ids: Vec<u64> = track.automation_lanes.iter().map(|lane| lane.id).collect();
        for lane_id in lane_ids {
            self.show_lane_row(
                ui,
                project,
                track_id,
                lane_id,
                track,
                engine,
                history,
                beat_width,
                scroll_offset,
                theme,
                true,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_lane_row(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        track_id: u64,
        lane_id: u64,
        track: &crate::model::Track,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        beat_width: f32,
        scroll_offset: Vec2,
        theme: &ThemeColors,
        compact: bool,
    ) {
        let Some(lane_snapshot) = project.automation_lane(track_id, lane_id).cloned() else {
            return;
        };

        let header_height = if compact {
            LANE_HEADER_HEIGHT - 4.0
        } else {
            LANE_HEADER_HEIGHT
        };
        let row_height = header_height + AUTOMATION_LANE_BODY_HEIGHT;

        let (row_rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::hover());
        let mut row_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("automation_lane_row", track_id, lane_id))
                .max_rect(row_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        row_ui.set_clip_rect(row_rect);

        row_ui.horizontal(|ui| {
            self.lane_header(
                ui,
                project,
                track_id,
                lane_id,
                track,
                &lane_snapshot,
                engine,
                history,
                theme,
                compact,
            );
        });

        let metrics = TimelineMetrics { beat_width };
        let total_beats = project.arrangement_length_beats();
        let playhead = engine.current_beats();
        let canvas_rect = Rect::from_min_max(
            Pos2::new(row_rect.left(), row_rect.top() + header_height),
            row_rect.right_bottom(),
        );

        let mut canvas_ui = row_ui.new_child(
            UiBuilder::new()
                .id_salt(("automation_lane_canvas_host", track_id, lane_id))
                .max_rect(canvas_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        canvas_ui.set_clip_rect(canvas_rect);

        with_solid_scrollbars(&mut canvas_ui, theme, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt(("automation_lane_scroll", track_id, lane_id))
                .auto_shrink([false, false])
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .scroll_offset(scroll_offset)
                .enable_scrolling(false)
                .show(ui, |ui| {
                    let content_width = total_beats * metrics.beat_width;
                    let canvas_size = Vec2::new(
                        content_width.max(canvas_rect.width()),
                        AUTOMATION_LANE_BODY_HEIGHT,
                    );
                    let (response, painter) =
                        ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    let timeline = content.translate(Vec2::new(-TRACK_HEADER_WIDTH, 0.0));
                    let body = timeline;

                    painter.rect_filled(body, 0.0, theme.panel_bg);

                    let clip_painter = painter.with_clip_rect(content);
                    draw_timeline_grid_lines(
                        &clip_painter,
                        body,
                        metrics,
                        total_beats,
                        project.beats_per_bar,
                        theme,
                    );

                    if let Some(lane) = project.automation_lane(track_id, lane_id) {
                        draw_automation_curve(&clip_painter, body, lane, metrics, theme);
                    }

                    let ruler = Rect::from_min_max(body.min, Pos2::new(body.max.x, body.top()));
                    draw_playhead(
                        &clip_painter,
                        ruler,
                        body,
                        metrics,
                        playhead,
                        true,
                        theme,
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
                });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn lane_header(
        &self,
        ui: &mut Ui,
        project: &mut Project,
        track_id: u64,
        lane_id: u64,
        track: &crate::model::Track,
        lane: &AutomationLane,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        theme: &ThemeColors,
        compact: bool,
    ) {
        let device_id = target_device_id(&lane.target);
        let param_label = if lane.param_name.is_empty() {
            "(pick parameter)"
        } else {
            &lane.param_name
        };

        ui.label(
            RichText::new(truncate_label(param_label, if compact { 14 } else { 28 }))
                .color(theme.track_header_text)
                .small(),
        );

        ui.menu_button("Target", |ui| {
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
                    if let Some(lane) = project.automation_lane_mut(track_id, lane_id) {
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
        });

        let target_name = match &lane.target {
            AutomationTarget::Instrument { .. } => "Instrument",
            AutomationTarget::Device { device_id, .. } => track
                .devices
                .iter()
                .find(|device| device.id == *device_id)
                .map(|device| device.name.as_str())
                .unwrap_or("Device"),
        };
        ui.label(
            RichText::new(truncate_label(target_name, 16))
                .color(theme.text_muted)
                .small(),
        );

        ui.menu_button("Param", |ui| {
            let params = engine.plugin_parameters(track_id, device_id);
            if params.is_empty() {
                ui.label(
                    RichText::new("No parameters (load plugin first)")
                        .color(theme.text_muted)
                        .small(),
                );
            } else {
                for param in params {
                    if !param.automatable {
                        continue;
                    }
                    let label = truncate_label(&param.name, 32);
                    if ui.button(label).clicked() {
                        apply_param_selection(project, track_id, lane_id, &lane.target, &param, history);
                        ui.close_menu();
                    }
                }
            }
        });

        let mut enabled = lane.enabled;
        if ui.checkbox(&mut enabled, "On").changed() {
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
                painter.line_segment(
                    [prev_pos, Pos2::new(pos.x, prev_pos.y)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(pos.x, prev_pos.y), pos],
                    stroke,
                );
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
        if active_drag.is_some() {
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
            if hit_test_point(body, lane, pointer, metrics).is_some() {
                // Point drag starts below.
            } else {
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
}
