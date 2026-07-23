use egui::{Color32, Pos2, Rect, Response, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{Project, SNAP_BEATS};

pub const TIMELINE_GUTTER_WIDTH: f32 = 56.0;
pub const RULER_HEIGHT: f32 = 26.0;

pub const DEFAULT_BEAT_WIDTH: f32 = 88.0;
pub const MIN_BEAT_WIDTH: f32 = 24.0;
pub const MAX_BEAT_WIDTH: f32 = 400.0;

const SCROLL_ZOOM_SPEED: f32 = 1.0 / 200.0;

#[derive(Debug, Clone, Copy)]
pub struct TimelineMetrics {
    pub beat_width: f32,
}

pub fn ruler_rect(full: Rect) -> Rect {
    Rect::from_min_max(full.min, Pos2::new(full.right(), full.top() + RULER_HEIGHT))
}

pub fn timeline_body_rect(full: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(full.left(), full.top() + RULER_HEIGHT),
        full.max,
    )
}

pub fn timeline_x(full: Rect, beat: f32, metrics: TimelineMetrics) -> f32 {
    full.left() + TIMELINE_GUTTER_WIDTH + beat * metrics.beat_width
}

pub fn x_to_beat(full: Rect, x: f32, metrics: TimelineMetrics) -> f32 {
    (x - full.left() - TIMELINE_GUTTER_WIDTH) / metrics.beat_width
}

pub fn seek_from_pointer(
    full: Rect,
    pointer: Pos2,
    metrics: TimelineMetrics,
    engine: &mut dyn DawEngine,
    beat_offset: f32,
) {
    if pointer.x <= full.left() + TIMELINE_GUTTER_WIDTH {
        return;
    }
    let local = Project::snap_beats(x_to_beat(full, pointer.x, metrics).max(0.0));
    engine.seek_beats((local + beat_offset).max(0.0));
}

pub fn is_timeline_pointer(timeline: Rect, pointer: Pos2) -> bool {
    timeline.contains(pointer) && pointer.x > timeline.left() + TIMELINE_GUTTER_WIDTH
}

pub fn is_ruler_timeline_pointer(ruler: Rect, pointer: Pos2) -> bool {
    ruler.contains(pointer) && pointer.x > ruler.left() + TIMELINE_GUTTER_WIDTH
}

/// Horizontal zoom via Ctrl/Cmd+Wheel and Shift+Wheel scroll when pointer is over viewport.
pub fn apply_horizontal_wheel_controls(
    ui: &Ui,
    viewport: Rect,
    beat_width: &mut f32,
    scroll_offset_x: &mut f32,
) {
    if !ui.rect_contains_pointer(viewport) {
        return;
    }

    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };

    let (modifiers, zoom_delta) = ui.input(|input| (input.modifiers, input.zoom_delta()));
    let zoom_horizontal = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;

    if zoom_horizontal && zoom_delta != 1.0 {
        let content_pos = pointer - viewport.min + Vec2::new(*scroll_offset_x, 0.0);
        let old = *beat_width;
        let new = (old * zoom_delta).clamp(MIN_BEAT_WIDTH, MAX_BEAT_WIDTH);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && content_pos.x > TIMELINE_GUTTER_WIDTH {
            let timeline_x_pos = content_pos.x - TIMELINE_GUTTER_WIDTH;
            *scroll_offset_x += timeline_x_pos * (actual - 1.0);
        }
        *beat_width = new;
    }
}

/// Piano roll: Alt+Wheel vertical zoom uses scroll_y consumption; combined with horizontal above.
pub fn apply_piano_roll_wheel_controls(
    ui: &Ui,
    viewport: Rect,
    beat_width: &mut f32,
    key_height: &mut f32,
    scroll_offset: &mut Vec2,
    min_key_height: f32,
    max_key_height: f32,
) {
    if !ui.rect_contains_pointer(viewport) {
        return;
    }

    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };

    let (modifiers, zoom_delta) = ui.input(|input| (input.modifiers, input.zoom_delta()));
    let zoom_horizontal = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
    let zoom_vertical = modifiers.alt;

    if !zoom_horizontal && !zoom_vertical {
        return;
    }

    let mut h_factor = 1.0_f32;
    let mut v_factor = 1.0_f32;

    if zoom_horizontal && zoom_delta != 1.0 {
        h_factor = zoom_delta;
        if zoom_vertical {
            v_factor = zoom_delta;
        }
    }

    if zoom_vertical && !zoom_horizontal {
        let scroll_y = ui.ctx().input_mut(|input| {
            let dy = input.smooth_scroll_delta.y;
            if dy != 0.0 {
                input.smooth_scroll_delta = Vec2::ZERO;
            }
            dy
        });
        if scroll_y != 0.0 {
            v_factor = (SCROLL_ZOOM_SPEED * scroll_y).exp();
        }
    }

    if h_factor == 1.0 && v_factor == 1.0 {
        return;
    }

    let local = pointer - viewport.min;
    let content_pos = Vec2::new(
        if local.x > TIMELINE_GUTTER_WIDTH {
            local.x - TIMELINE_GUTTER_WIDTH + scroll_offset.x
        } else {
            scroll_offset.x
        },
        if local.y > RULER_HEIGHT {
            local.y - RULER_HEIGHT + scroll_offset.y
        } else {
            scroll_offset.y
        },
    );

    if h_factor != 1.0 {
        let old = *beat_width;
        let new = (old * h_factor).clamp(MIN_BEAT_WIDTH, MAX_BEAT_WIDTH);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && local.x > TIMELINE_GUTTER_WIDTH {
            scroll_offset.x += content_pos.x * (actual - 1.0);
        }
        *beat_width = new;
    }

    if v_factor != 1.0 {
        let old = *key_height;
        let new = (old * v_factor).clamp(min_key_height, max_key_height);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && local.y > RULER_HEIGHT {
            scroll_offset.y += content_pos.y * (actual - 1.0);
        }
        *key_height = new;
    }
}

pub fn draw_ruler(
    painter: &egui::Painter,
    ruler: Rect,
    timeline: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
) {
    painter.rect_filled(ruler, 0.0, Color32::from_rgb(28, 28, 34));

    painter.rect_filled(
        Rect::from_min_max(
            ruler.min,
            Pos2::new(ruler.left() + TIMELINE_GUTTER_WIDTH, ruler.bottom()),
        ),
        0.0,
        Color32::from_rgb(22, 22, 28),
    );

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(timeline, beat as f32, metrics);
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let tick_bottom = if is_bar {
            ruler.bottom() - 2.0
        } else {
            ruler.bottom() - 8.0
        };
        let color = if is_bar {
            Color32::from_rgb(130, 130, 150)
        } else {
            Color32::from_rgb(70, 70, 88)
        };
        painter.line_segment(
            [Pos2::new(x, ruler.bottom()), Pos2::new(x, tick_bottom)],
            egui::Stroke::new(if is_bar { 1.5_f32 } else { 1.0_f32 }, color),
        );

        if is_bar {
            let bar_number = beat / beats_per_bar as i32 + 1;
            painter.text(
                Pos2::new(x + 4.0, ruler.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("{bar_number}"),
                egui::FontId::monospace(11.0),
                Color32::from_rgb(190, 190, 205),
            );
        }
    }

    for subdivision in 0..=(beat_count * 4) {
        let beat = subdivision as f32 * SNAP_BEATS;
        if beat.fract() == 0.0 {
            continue;
        }
        let x = timeline_x(timeline, beat, metrics);
        painter.line_segment(
            [
                Pos2::new(x, ruler.bottom()),
                Pos2::new(x, ruler.bottom() - 5.0),
            ],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(52, 52, 64)),
        );
    }

    painter.line_segment(
        [
            Pos2::new(timeline.left() + TIMELINE_GUTTER_WIDTH, ruler.bottom()),
            Pos2::new(ruler.right(), ruler.bottom()),
        ],
        egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 55, 68)),
    );
}

pub fn draw_playhead(
    painter: &egui::Painter,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    local_beat: f32,
    visible: bool,
) {
    if !visible || local_beat < 0.0 {
        return;
    }
    let x = timeline_x(body, local_beat, metrics);
    painter.line_segment(
        [
            Pos2::new(x, ruler.top()),
            Pos2::new(x, body.bottom()),
        ],
        egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 90, 90)),
    );
    painter.circle_filled(
        Pos2::new(x, ruler.center().y),
        4.0,
        Color32::from_rgb(255, 90, 90),
    );
}

pub fn draw_timeline_grid_lines(
    painter: &egui::Painter,
    body: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
) {
    let timeline_left = body.left() + TIMELINE_GUTTER_WIDTH;
    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(body, beat as f32, metrics);
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let color = if is_bar {
            Color32::from_rgb(90, 90, 110)
        } else {
            Color32::from_rgb(45, 45, 58)
        };
        painter.line_segment(
            [Pos2::new(x, body.top()), Pos2::new(x, body.bottom())],
            egui::Stroke::new(if is_bar { 1.5_f32 } else { 1.0_f32 }, color),
        );
    }

    for subdivision in 0..=(beat_count * 4) {
        let beat = subdivision as f32 * SNAP_BEATS;
        if (beat.rem_euclid(beats_per_bar)).fract() == 0.0 && beat.fract() == 0.0 {
            continue;
        }
        let x = timeline_x(body, beat, metrics);
        painter.line_segment(
            [Pos2::new(x, body.top()), Pos2::new(x, body.bottom())],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(34, 34, 44)),
        );
    }

    let _ = timeline_left;
}

/// Shared playhead scrubbing: ruler click/drag, shift+click, optional body right-click seek.
///
/// When `seek_on_body_secondary` is false, body right-clicks are left for the caller
/// (e.g. piano roll: delete note under cursor, else seek).
pub fn handle_timeline_playhead_pointer(
    response: &Response,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    engine: &mut dyn DawEngine,
    dragging_playhead: &mut bool,
    beat_offset: f32,
    seek_on_body_secondary: bool,
) -> bool {
    handle_timeline_playhead_pointer_with_positions(
        response,
        ruler,
        body,
        metrics,
        engine,
        dragging_playhead,
        beat_offset,
        seek_on_body_secondary,
        None,
        None,
    )
}

pub fn handle_timeline_playhead_pointer_with_positions(
    response: &Response,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    engine: &mut dyn DawEngine,
    dragging_playhead: &mut bool,
    beat_offset: f32,
    seek_on_body_secondary: bool,
    pointer_override: Option<Pos2>,
    press_override: Option<Pos2>,
) -> bool {
    let full = ruler.union(body);

    if let Some(hover) = pointer_override.or_else(|| response.hover_pos()) {
        if is_ruler_timeline_pointer(ruler, hover) {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    let Some(pointer) = pointer_override.or_else(|| response.interact_pointer_pos()) else {
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return false;
    };

    let press_pos = press_override.unwrap_or_else(|| {
        response
            .ctx
            .input(|input| input.pointer.press_origin())
            .unwrap_or(pointer)
    });

    if *dragging_playhead {
        if response.dragged() {
            seek_from_pointer(body, pointer, metrics, engine, beat_offset);
            response.ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return true;
    }

    if !full.contains(pointer) {
        return false;
    }

    let shift_held = response.ctx.input(|input| input.modifiers.shift);

    if is_ruler_timeline_pointer(ruler, press_pos) || is_ruler_timeline_pointer(ruler, pointer)
    {
        if response.drag_started_by(egui::PointerButton::Primary)
            && is_ruler_timeline_pointer(ruler, press_pos)
        {
            *dragging_playhead = true;
            seek_from_pointer(body, pointer, metrics, engine, beat_offset);
            return true;
        }
        if response.clicked_by(egui::PointerButton::Primary) && !response.dragged() {
            seek_from_pointer(body, pointer, metrics, engine, beat_offset);
            return true;
        }
        if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
            seek_from_pointer(body, pointer, metrics, engine, beat_offset);
            return true;
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && shift_held
        && is_timeline_pointer(body, pointer)
    {
        seek_from_pointer(body, pointer, metrics, engine, beat_offset);
        return true;
    }

    if seek_on_body_secondary
        && response.clicked_by(egui::PointerButton::Secondary)
        && !response.dragged()
        && is_timeline_pointer(body, pointer)
    {
        seek_from_pointer(body, pointer, metrics, engine, beat_offset);
        return true;
    }

    false
}
