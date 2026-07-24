use std::hash::Hash;

use egui::containers::scroll_area::ScrollBarVisibility;
use egui::style::ScrollStyle;
use egui::{Pos2, Rect, Response, ScrollArea, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{Project, MIN_LOOP_SPAN_BEATS, SNAP_BEATS};
use crate::ui::theme::ThemeColors;

/// Which end of the loop region is being dragged on the ruler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEdge {
    Start,
    End,
}

/// Hit-test half-width around a loop edge (screen px).
const LOOP_EDGE_HIT_PX: f32 = 8.0;

/// egui's default floating bars fade to invisible when idle. DAW editors need
/// always-opaque, space-taking bars so scroll position stays readable.
pub fn with_solid_scrollbars<R>(
    ui: &mut Ui,
    theme: &ThemeColors,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> R {
    let previous_scroll = ui.style().spacing.scroll;
    let previous_visuals = ui.visuals().clone();
    {
        let style = ui.style_mut();
        let mut scroll = ScrollStyle::solid();
        // Slightly wider than egui's 6px solid default for easier grabbing.
        scroll.bar_width = 10.0;
        scroll.handle_min_length = 24.0;
        style.spacing.scroll = scroll;

        style.visuals.extreme_bg_color = theme.scrollbar_track;
        style.visuals.widgets.inactive.bg_fill = theme.scrollbar_handle;
        style.visuals.widgets.inactive.weak_bg_fill = theme.scrollbar_handle;
        style.visuals.widgets.hovered.bg_fill = theme.scrollbar_handle_hovered;
        style.visuals.widgets.hovered.weak_bg_fill = theme.scrollbar_handle_hovered;
        style.visuals.widgets.active.bg_fill = theme.scrollbar_handle_active;
        style.visuals.widgets.active.weak_bg_fill = theme.scrollbar_handle_active;
    }
    let result = add_contents(ui);
    ui.style_mut().spacing.scroll = previous_scroll;
    ui.style_mut().visuals = previous_visuals;
    result
}

/// Shared ScrollArea config for playlist and piano-roll canvases.
pub fn daw_editor_scroll_area(id_salt: impl Hash) -> ScrollArea {
    ScrollArea::both()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
}

pub const TIMELINE_GUTTER_WIDTH: f32 = 72.0;
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
    Rect::from_min_max(Pos2::new(full.left(), full.top() + RULER_HEIGHT), full.max)
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

    let content_pos = pointer - viewport.min + *scroll_offset;

    if h_factor != 1.0 {
        let old = *beat_width;
        let new = (old * h_factor).clamp(MIN_BEAT_WIDTH, MAX_BEAT_WIDTH);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && content_pos.x > TIMELINE_GUTTER_WIDTH {
            let timeline_x_pos = content_pos.x - TIMELINE_GUTTER_WIDTH;
            scroll_offset.x += timeline_x_pos * (actual - 1.0);
        }
        *beat_width = new;
    }

    if v_factor != 1.0 {
        let old = *key_height;
        let new = (old * v_factor).clamp(min_key_height, max_key_height);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && content_pos.y > RULER_HEIGHT {
            let keys_y = content_pos.y - RULER_HEIGHT;
            scroll_offset.y += keys_y * (actual - 1.0);
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
    theme: &ThemeColors,
) {
    painter.rect_filled(ruler, 0.0, theme.ruler_bg);

    painter.rect_filled(
        Rect::from_min_max(
            ruler.min,
            Pos2::new(ruler.left() + TIMELINE_GUTTER_WIDTH, ruler.bottom()),
        ),
        0.0,
        theme.gutter_bg,
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
            theme.tick_major
        } else {
            theme.tick_minor
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
                theme.ruler_text,
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
            egui::Stroke::new(1.0_f32, theme.tick_sub),
        );
    }

    painter.line_segment(
        [
            Pos2::new(timeline.left() + TIMELINE_GUTTER_WIDTH, ruler.bottom()),
            Pos2::new(ruler.right(), ruler.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.separator),
    );
}

pub fn draw_playhead(
    painter: &egui::Painter,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    local_beat: f32,
    visible: bool,
    theme: &ThemeColors,
) {
    if !visible || local_beat < 0.0 {
        return;
    }
    let x = timeline_x(body, local_beat, metrics);
    painter.line_segment(
        [Pos2::new(x, ruler.top()), Pos2::new(x, body.bottom())],
        egui::Stroke::new(2.0_f32, theme.playhead),
    );
    painter.circle_filled(Pos2::new(x, ruler.center().y), 4.0, theme.playhead);
}

/// Highlight the active loop/cycle region: a translucent band across the body,
/// a marker strip along the bottom of the ruler, and vertical edges. Drawn only
/// when the caller has a valid enabled loop (`end > start`).
///
/// `highlighted_edge` thickens the active/hovered ruler handle so edges read as
/// adjustable.
pub fn draw_loop_region(
    painter: &egui::Painter,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    loop_start: f32,
    loop_end: f32,
    theme: &ThemeColors,
    highlighted_edge: Option<LoopEdge>,
) {
    if loop_end <= loop_start {
        return;
    }
    let x_start = timeline_x(body, loop_start, metrics);
    let x_end = timeline_x(body, loop_end, metrics);

    // Translucent band over the lanes.
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(x_start, body.top()),
            Pos2::new(x_end, body.bottom()),
        ),
        0.0,
        theme.loop_region_fill,
    );

    // Marker strip along the bottom of the ruler.
    let strip_top = ruler.bottom() - 4.0;
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(x_start, strip_top),
            Pos2::new(x_end, ruler.bottom()),
        ),
        0.0,
        theme.loop_region_edge,
    );

    // Vertical edges from the ruler strip down through the body.
    let edge = egui::Stroke::new(1.5_f32, theme.loop_region_edge);
    for x in [x_start, x_end] {
        painter.line_segment([Pos2::new(x, strip_top), Pos2::new(x, body.bottom())], edge);
    }

    // Grab handles on the ruler (thicker when hovered / dragged).
    for (edge_kind, x) in [(LoopEdge::Start, x_start), (LoopEdge::End, x_end)] {
        let hot = highlighted_edge == Some(edge_kind);
        let half_w = if hot { 3.5_f32 } else { 2.0_f32 };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x - half_w, ruler.top() + 2.0),
                Pos2::new(x + half_w, strip_top),
            ),
            1.0,
            theme.loop_region_edge,
        );
    }
}

/// Hit-test a loop edge on the ruler brace (not the body lanes — avoids fighting
/// clip resize handles).
pub fn hit_test_loop_edge(
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    loop_start: f32,
    loop_end: f32,
    pointer: Pos2,
) -> Option<LoopEdge> {
    if loop_end <= loop_start {
        return None;
    }
    if !is_ruler_timeline_pointer(ruler, pointer) {
        return None;
    }
    let x_start = timeline_x(body, loop_start, metrics);
    let x_end = timeline_x(body, loop_end, metrics);
    let dist_start = (pointer.x - x_start).abs();
    let dist_end = (pointer.x - x_end).abs();
    if dist_start > LOOP_EDGE_HIT_PX && dist_end > LOOP_EDGE_HIT_PX {
        return None;
    }
    if dist_start <= dist_end {
        Some(LoopEdge::Start)
    } else {
        Some(LoopEdge::End)
    }
}

/// Drag loop start/end on the ruler. Returns true while the gesture owns the
/// pointer (active drag), so callers can skip playhead scrub / clip picks.
pub fn handle_loop_region_pointer(
    response: &Response,
    ruler: Rect,
    body: Rect,
    metrics: TimelineMetrics,
    project: &mut Project,
    dragging: &mut Option<LoopEdge>,
) -> bool {
    let Some((loop_start, loop_end)) = project.loop_span() else {
        *dragging = None;
        return false;
    };

    if dragging.is_none() {
        if let Some(hover) = response.hover_pos() {
            if hit_test_loop_edge(ruler, body, metrics, loop_start, loop_end, hover).is_some() {
                response
                    .ctx
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        }
    }

    if let Some(edge) = *dragging {
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                apply_loop_edge_drag(project, edge, body, pointer, metrics);
                response
                    .ctx
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        }
        if response.drag_stopped()
            || !response
                .ctx
                .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
        {
            *dragging = None;
        }
        return true;
    }

    let Some(pointer) = response.interact_pointer_pos() else {
        return false;
    };
    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(edge) =
            hit_test_loop_edge(ruler, body, metrics, loop_start, loop_end, press_pos)
        {
            *dragging = Some(edge);
            apply_loop_edge_drag(project, edge, body, pointer, metrics);
            response
                .ctx
                .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            return true;
        }
    }

    false
}

fn apply_loop_edge_drag(
    project: &mut Project,
    edge: LoopEdge,
    body: Rect,
    pointer: Pos2,
    metrics: TimelineMetrics,
) {
    let beat = Project::snap_beats(x_to_beat(body, pointer.x, metrics).max(0.0));
    match edge {
        LoopEdge::Start => {
            project.loop_start_beats =
                beat.clamp(0.0, project.loop_end_beats - MIN_LOOP_SPAN_BEATS);
        }
        LoopEdge::End => {
            project.loop_end_beats = beat.max(project.loop_start_beats + MIN_LOOP_SPAN_BEATS);
        }
    }
}

pub fn draw_timeline_grid_lines(
    painter: &egui::Painter,
    body: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
    theme: &ThemeColors,
) {
    let timeline_left = body.left() + TIMELINE_GUTTER_WIDTH;
    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(body, beat as f32, metrics);
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let color = if is_bar {
            theme.grid_bar
        } else {
            theme.grid_beat
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
            egui::Stroke::new(1.0_f32, theme.grid_subbeat),
        );
    }

    let _ = timeline_left;
}

/// Shared playhead scrubbing: ruler click/drag (primary or secondary), body
/// right-click drag, shift+click, optional body right-click seek.
///
/// Body secondary *drag* always scrubs (playlist + piano roll). When
/// `seek_on_body_secondary` is false, body secondary *clicks* are left for the
/// caller (e.g. piano roll: delete note under cursor, else seek).
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
    let full = ruler.union(body);

    if let Some(hover) = response.hover_pos() {
        if is_ruler_timeline_pointer(ruler, hover) {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return false;
    };

    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    if *dragging_playhead {
        if response.dragged() {
            seek_from_pointer(body, pointer, metrics, engine, beat_offset);
            response
                .ctx
                .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
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
    let primary_or_secondary_drag = response.drag_started_by(egui::PointerButton::Primary)
        || response.drag_started_by(egui::PointerButton::Secondary);
    let ruler_drag_started =
        primary_or_secondary_drag && is_ruler_timeline_pointer(ruler, press_pos);
    // Right-click drag on the arrangement / note grid scrubs like the ruler.
    let body_secondary_drag_started = response.drag_started_by(egui::PointerButton::Secondary)
        && is_timeline_pointer(body, press_pos);

    if is_ruler_timeline_pointer(ruler, press_pos) || is_ruler_timeline_pointer(ruler, pointer) {
        if ruler_drag_started {
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

    if body_secondary_drag_started {
        *dragging_playhead = true;
        seek_from_pointer(body, pointer, metrics, engine, beat_offset);
        return true;
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
