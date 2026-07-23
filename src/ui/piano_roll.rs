use std::collections::HashSet;

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    Note, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
use crate::ui::timeline::{
    apply_piano_roll_wheel_controls, draw_ruler, ruler_rect, timeline_body_rect,
    TimelineMetrics, DEFAULT_BEAT_WIDTH, RULER_HEIGHT, TIMELINE_GUTTER_WIDTH,
};

const BLACK_KEY_WIDTH_RATIO: f32 = 0.62;
const RESIZE_HANDLE_PX: f32 = 12.0;

const DEFAULT_KEY_HEIGHT: f32 = 18.0;
const MIN_KEY_HEIGHT: f32 = 8.0;
const MAX_KEY_HEIGHT: f32 = 48.0;

/// Pitch shown near the top of the viewport on first open (C6).
const DEFAULT_TOP_PITCH: u8 = 84;

#[derive(Debug, Clone, Copy)]
struct ViewMetrics {
    beat_width: f32,
    key_height: f32,
}

impl ViewMetrics {
    fn timeline(&self) -> TimelineMetrics {
        TimelineMetrics {
            beat_width: self.beat_width,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Debug, Clone)]
struct ActiveDrag {
    note_id: u64,
    mode: DragMode,
    pointer_start_beats: f32,
    pointer_start_pitch: i32,
    originals: Vec<Note>,
}

#[derive(Debug, Clone)]
struct MarqueeDrag {
    start: Pos2,
    current: Pos2,
}

impl MarqueeDrag {
    fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.current)
    }
}

pub struct PianoRollUi {
    selected_note_ids: HashSet<u64>,
    active_drag: Option<ActiveDrag>,
    marquee: Option<MarqueeDrag>,
    dragging_playhead: bool,
    default_duration_beats: f32,
    beat_width: f32,
    key_height: f32,
    scroll_offset: Vec2,
}

impl PianoRollUi {
    pub fn selected_note_ids(&self) -> &HashSet<u64> {
        &self.selected_note_ids
    }

    pub fn clear_selection(&mut self) {
        self.selected_note_ids.clear();
    }
}

impl Default for PianoRollUi {
    fn default() -> Self {
        let initial_scroll_y =
            (MAX_PITCH.saturating_sub(DEFAULT_TOP_PITCH)) as f32 * DEFAULT_KEY_HEIGHT;
        Self {
            selected_note_ids: HashSet::new(),
            active_drag: None,
            marquee: None,
            dragging_playhead: false,
            default_duration_beats: DEFAULT_NOTE_DURATION_BEATS,
            beat_width: DEFAULT_BEAT_WIDTH,
            key_height: DEFAULT_KEY_HEIGHT,
            scroll_offset: Vec2::new(0.0, initial_scroll_y),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PianoRollLayout {
    widget: Rect,
    corner: Rect,
    ruler: Rect,
    keys: Rect,
    grid: Rect,
}

impl PianoRollLayout {
    fn from_viewport(viewport: Rect) -> Self {
        let corner = Rect::from_min_max(
            viewport.min,
            Pos2::new(viewport.left() + TIMELINE_GUTTER_WIDTH, viewport.top() + RULER_HEIGHT),
        );
        Self {
            widget: viewport,
            corner,
            ruler: Rect::from_min_max(
                Pos2::new(corner.right(), viewport.top()),
                Pos2::new(viewport.right(), corner.bottom()),
            ),
            keys: Rect::from_min_max(
                Pos2::new(viewport.left(), corner.bottom()),
                Pos2::new(corner.right(), viewport.bottom()),
            ),
            grid: Rect::from_min_max(corner.max, viewport.max),
        }
    }

    fn virtual_full(&self, grid_canvas: Vec2) -> Rect {
        Rect::from_min_size(
            self.widget.min,
            Vec2::new(
                TIMELINE_GUTTER_WIDTH + grid_canvas.x,
                RULER_HEIGHT + grid_canvas.y,
            ),
        )
    }
}

fn content_grid_rect(grid_canvas: Vec2) -> Rect {
    Rect::from_min_size(Pos2::ZERO, grid_canvas)
}

fn draw_playhead_pinned(
    painter: &egui::Painter,
    ruler: Rect,
    grid: Rect,
    metrics: TimelineMetrics,
    local_beat: f32,
    visible: bool,
    scroll_x: f32,
) {
    if !visible || local_beat < 0.0 {
        return;
    }
    let x = grid.left() + local_beat * metrics.beat_width - scroll_x;
    painter.line_segment(
        [
            Pos2::new(x, ruler.top()),
            Pos2::new(x, grid.bottom()),
        ],
        egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 90, 90)),
    );
    painter.circle_filled(
        Pos2::new(x, ruler.center().y),
        4.0,
        Color32::from_rgb(255, 90, 90),
    );
}

fn handle_timeline_playhead_pointer(
    response: &Response,
    layout: PianoRollLayout,
    scroll: Vec2,
    ruler: Rect,
    grid: Rect,
    metrics: TimelineMetrics,
    engine: &mut dyn DawEngine,
    dragging_playhead: &mut bool,
    beat_offset: f32,
    seek_on_body_secondary: bool,
    from_content: bool,
) -> bool {
    if *dragging_playhead {
        if let Some(screen) = response.ctx.input(|input| input.pointer.latest_pos()) {
            let pointer = map_pointer_to_virtual(layout, scroll, screen);
            if response.ctx.input(|input| input.pointer.primary_down()) {
                crate::ui::timeline::seek_from_pointer(
                    grid,
                    pointer,
                    metrics,
                    engine,
                    beat_offset,
                );
                response
                    .ctx
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else {
                *dragging_playhead = false;
            }
        }
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return true;
    }

    let map_pos = |pos: Pos2| {
        if from_content {
            grid_pointer_to_virtual(layout, scroll, pos)
        } else {
            map_pointer_to_virtual(layout, scroll, pos)
        }
    };

    let Some(screen_pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return false;
    };
    let pointer = map_pos(screen_pointer);
    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .map(map_pos)
        .unwrap_or(pointer);

    if let Some(hover) = response.hover_pos() {
        let virtual_hover = map_pos(hover);
        if crate::ui::timeline::is_ruler_timeline_pointer(ruler, virtual_hover) {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    crate::ui::timeline::handle_timeline_playhead_pointer_with_positions(
        response,
        ruler,
        grid,
        metrics,
        engine,
        dragging_playhead,
        beat_offset,
        seek_on_body_secondary,
        Some(pointer),
        Some(press_pos),
    )
}

fn map_pointer_to_virtual(layout: PianoRollLayout, scroll: Vec2, pointer: Pos2) -> Pos2 {
    Pos2::new(
        pointer.x - layout.widget.left() + scroll.x,
        pointer.y - layout.widget.top() + scroll.y,
    )
}

fn grid_pointer_to_virtual(layout: PianoRollLayout, scroll: Vec2, pointer: Pos2) -> Pos2 {
    Pos2::new(
        pointer.x - layout.grid.left() + scroll.x + TIMELINE_GUTTER_WIDTH,
        pointer.y - layout.grid.top() + scroll.y + RULER_HEIGHT,
    )
}

fn draw_corner(painter: &egui::Painter, corner: Rect) {
    painter.rect_filled(corner, 0.0, Color32::from_rgb(22, 22, 28));
    painter.line_segment(
        [
            Pos2::new(corner.right(), corner.top()),
            Pos2::new(corner.right(), corner.bottom()),
        ],
        egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 55, 68)),
    );
    painter.line_segment(
        [
            Pos2::new(corner.left(), corner.bottom()),
            Pos2::new(corner.right(), corner.bottom()),
        ],
        egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 55, 68)),
    );
}

impl PianoRollUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        clip_id: u64,
        project: &mut Project,
        engine: &mut dyn DawEngine,
    ) {
        let (clip_start, total_beats, beats_per_bar) = {
            let Some(clip) = project.clip(clip_id) else {
                return;
            };
            (clip.start_beats, clip.length_beats.max(4.0), project.beats_per_bar)
        };
        let viewport_rect = ui.available_rect_before_wrap();
        apply_piano_roll_wheel_controls(
            ui,
            viewport_rect,
            &mut self.beat_width,
            &mut self.key_height,
            &mut self.scroll_offset,
            MIN_KEY_HEIGHT,
            MAX_KEY_HEIGHT,
        );

        let metrics = ViewMetrics {
            beat_width: self.beat_width,
            key_height: self.key_height,
        };
        let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
        let viewport = ui.available_size();
        let grid_viewport = Vec2::new(
            (viewport.x - TIMELINE_GUTTER_WIDTH).max(0.0),
            (viewport.y - RULER_HEIGHT).max(0.0),
        );
        let grid_canvas = Vec2::new(
            total_beats * metrics.beat_width,
            pitch_span * metrics.key_height,
        )
        .max(grid_viewport);

        let global_playhead = engine.current_beats();
        let local_playhead = global_playhead - clip_start;
        let playhead_visible = local_playhead >= 0.0 && local_playhead <= total_beats;

        let clip_notes: Vec<Note> = project
            .clip(clip_id)
            .map(|clip| clip.notes.clone())
            .unwrap_or_default();

        ui.allocate_ui_at_rect(viewport_rect, |ui| {
            let layout = PianoRollLayout::from_viewport(ui.max_rect());
            let virtual_full = layout.virtual_full(grid_canvas);
            let ruler = ruler_rect(virtual_full);
            let grid = timeline_body_rect(virtual_full);

            let scroll_output = ui.allocate_ui_at_rect(layout.grid, |ui| {
                egui::ScrollArea::both()
                    .id_salt("piano_roll_grid")
                    .scroll_offset(self.scroll_offset)
                    .drag_to_scroll(false)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_size(grid_canvas);
                        let content = Rect::from_min_size(Pos2::ZERO, grid_canvas);
                        let (response, painter) =
                            ui.allocate_painter(grid_canvas, Sense::click_and_drag());
                        draw_grid(
                            &painter,
                            content,
                            metrics,
                            total_beats,
                            beats_per_bar,
                        );
                        draw_notes(
                            &painter,
                            content,
                            metrics,
                            &clip_notes,
                            &self.selected_note_ids,
                            if playhead_visible { local_playhead } else { -1.0 },
                        );
                        if let Some(marquee) = &self.marquee {
                            draw_marquee(&painter, marquee.rect());
                        }
                        response
                    })
            });
            self.scroll_offset = scroll_output.inner.state.offset;
            let grid_response = scroll_output.inner.inner;

            let ruler_response = ui.interact(
                layout.ruler,
                ui.id().with("piano_roll_ruler"),
                Sense::click_and_drag(),
            );
            let widget_painter = ui.painter();

            draw_corner(&widget_painter, layout.corner);

            {
                let ruler_painter = widget_painter.with_clip_rect(layout.ruler);
                let virtual_ruler = Rect::from_min_size(
                    Pos2::new(layout.widget.left() - self.scroll_offset.x, layout.widget.top()),
                    virtual_full.size(),
                );
                draw_ruler(
                    &ruler_painter,
                    ruler_rect(virtual_ruler),
                    timeline_body_rect(virtual_ruler),
                    metrics.timeline(),
                    total_beats,
                    beats_per_bar,
                );
            }

            {
                let keys_painter = widget_painter.with_clip_rect(layout.keys);
                let virtual_keys = Rect::from_min_size(
                    Pos2::new(
                        layout.widget.left(),
                        layout.widget.top() + RULER_HEIGHT - self.scroll_offset.y,
                    ),
                    virtual_full.size(),
                );
                draw_keyboard(
                    &keys_painter,
                    timeline_body_rect(virtual_keys),
                    metrics,
                    &clip_notes,
                    if playhead_visible { local_playhead } else { -1.0 },
                );
            }

            draw_playhead_pinned(
                &widget_painter,
                layout.ruler,
                layout.grid,
                metrics.timeline(),
                local_playhead,
                playhead_visible,
                self.scroll_offset.x,
            );

            let content = content_grid_rect(grid_canvas);
            let playhead_handled = handle_timeline_playhead_pointer(
                &ruler_response,
                layout,
                self.scroll_offset,
                ruler,
                grid,
                metrics.timeline(),
                engine,
                &mut self.dragging_playhead,
                clip_start,
                false,
                false,
            ) || handle_timeline_playhead_pointer(
                &grid_response,
                layout,
                self.scroll_offset,
                ruler,
                grid,
                metrics.timeline(),
                engine,
                &mut self.dragging_playhead,
                clip_start,
                false,
                true,
            );

            if !playhead_handled {
                handle_grid_pointer(
                    &grid_response,
                    content,
                    metrics,
                    clip_id,
                    project,
                    engine,
                    clip_start,
                    &mut self.selected_note_ids,
                    &mut self.active_drag,
                    &mut self.marquee,
                    &mut self.default_duration_beats,
                );
            }
        });
    }
}

fn pitch_to_y(grid: Rect, pitch: u8, metrics: ViewMetrics) -> f32 {
    let row = (MAX_PITCH as i32 - pitch as i32) as f32;
    grid.top() + row * metrics.key_height
}

fn y_to_pitch(grid: Rect, y: f32, metrics: ViewMetrics) -> u8 {
    let row = ((y - grid.top()) / metrics.key_height).floor() as i32;
    let pitch = MAX_PITCH as i32 - row;
    Project::clamp_pitch(pitch)
}

fn content_timeline_x(grid: Rect, beat: f32, metrics: ViewMetrics) -> f32 {
    grid.left() + beat * metrics.beat_width
}

fn content_x_to_beat(grid: Rect, x: f32, metrics: ViewMetrics) -> f32 {
    (x - grid.left()) / metrics.beat_width
}

fn note_rect(grid: Rect, note: &Note, metrics: ViewMetrics) -> Rect {
    let top = pitch_to_y(grid, note.pitch, metrics);
    Rect::from_min_max(
        Pos2::new(content_timeline_x(grid, note.start_beats, metrics), top + 1.0),
        Pos2::new(
            content_timeline_x(grid, note.end_beats(), metrics),
            top + metrics.key_height - 1.0,
        ),
    )
}

fn hit_test_note<'a>(
    grid: Rect,
    notes: &'a [Note],
    pos: Pos2,
    metrics: ViewMetrics,
) -> Option<&'a Note> {
    notes
        .iter()
        .rev()
        .find(|note| note_rect(grid, note, metrics).contains(pos))
}

fn select_notes_in_rect(
    grid: Rect,
    notes: &[Note],
    selection: Rect,
    metrics: ViewMetrics,
) -> HashSet<u64> {
    notes
        .iter()
        .filter(|note| note_rect(grid, note, metrics).intersects(selection))
        .map(|note| note.id)
        .collect()
}

fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

fn draw_grid(
    painter: &egui::Painter,
    grid: Rect,
    metrics: ViewMetrics,
    total_beats: f32,
    beats_per_bar: f32,
) {
    painter.rect_filled(grid, 0.0, Color32::from_rgb(18, 18, 22));

    for pitch in MIN_PITCH..=MAX_PITCH {
        let y = pitch_to_y(grid, pitch, metrics);
        let row_color = if is_black_key(pitch) {
            Color32::from_rgb(26, 26, 32)
        } else {
            Color32::from_rgb(32, 32, 40)
        };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(grid.left(), y),
                Pos2::new(grid.right(), y + metrics.key_height),
            ),
            0.0,
            row_color,
        );
    }

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = content_timeline_x(grid, beat as f32, metrics);
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let color = if is_bar {
            Color32::from_rgb(90, 90, 110)
        } else {
            Color32::from_rgb(45, 45, 58)
        };
        painter.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            egui::Stroke::new(if is_bar { 1.5_f32 } else { 1.0_f32 }, color),
        );
    }

    for subdivision in 0..=(beat_count * 4) {
        let beat = subdivision as f32 * SNAP_BEATS;
        if (beat.rem_euclid(beats_per_bar)).fract() == 0.0 && beat.fract() == 0.0 {
            continue;
        }
        let x = content_timeline_x(grid, beat, metrics);
        painter.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(34, 34, 44)),
        );
    }
}

fn draw_keyboard(
    painter: &egui::Painter,
    grid: Rect,
    metrics: ViewMetrics,
    notes: &[Note],
    playhead_beats: f32,
) {
    let keys = Rect::from_min_max(
        grid.min,
        Pos2::new(grid.left() + TIMELINE_GUTTER_WIDTH, grid.bottom()),
    );
    painter.rect_filled(keys, 0.0, Color32::from_rgb(48, 48, 56));

    let is_pitch_active = |pitch: u8| {
        playhead_beats >= 0.0
            && notes
                .iter()
                .any(|note| note.pitch == pitch && note.contains_beat(playhead_beats))
    };

    for pitch in MIN_PITCH..=MAX_PITCH {
        if is_black_key(pitch) {
            continue;
        }

        let y = pitch_to_y(grid, pitch, metrics);
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y),
            Pos2::new(keys.right(), y + metrics.key_height),
        );
        let is_active = is_pitch_active(pitch);
        let fill = if is_active {
            Color32::from_rgb(255, 200, 120)
        } else {
            Color32::from_rgb(232, 232, 238)
        };
        painter.rect_filled(key_rect, 0.0, fill);
        painter.line_segment(
            [
                Pos2::new(keys.left(), key_rect.bottom()),
                Pos2::new(keys.right(), key_rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(150, 150, 160)),
        );

        if pitch % 12 == 0 {
            painter.text(
                Pos2::new(keys.right() - 4.0, y + metrics.key_height * 0.5),
                egui::Align2::RIGHT_CENTER,
                pitch_name(pitch),
                egui::FontId::monospace(11.0),
                Color32::from_rgb(40, 40, 55),
            );
        }
    }

    for pitch in MIN_PITCH..=MAX_PITCH {
        if !is_black_key(pitch) {
            continue;
        }

        let y = pitch_to_y(grid, pitch, metrics);
        let black_width = TIMELINE_GUTTER_WIDTH * BLACK_KEY_WIDTH_RATIO;
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y + 1.0),
            Pos2::new(keys.left() + black_width, y + metrics.key_height - 1.0),
        );
        let is_active = is_pitch_active(pitch);
        let fill = if is_active {
            Color32::from_rgb(255, 160, 70)
        } else {
            Color32::from_rgb(28, 28, 34)
        };
        painter.rect(
            key_rect,
            1.5,
            fill,
            egui::Stroke::new(1.0_f32, Color32::from_rgb(12, 12, 16)),
            egui::StrokeKind::Inside,
        );
    }

    painter.line_segment(
        [
            Pos2::new(keys.right(), keys.top()),
            Pos2::new(keys.right(), keys.bottom()),
        ],
        egui::Stroke::new(1.5_f32, Color32::from_rgb(70, 70, 85)),
    );
}

fn draw_notes(
    painter: &egui::Painter,
    rect: Rect,
    metrics: ViewMetrics,
    notes: &[Note],
    selected_ids: &HashSet<u64>,
    playhead_beats: f32,
) {
    for note in notes {
        let note_rect = note_rect(rect, note, metrics);
        let is_selected = selected_ids.contains(&note.id);
        let is_active = playhead_beats >= 0.0 && note.contains_beat(playhead_beats);

        let fill = if is_active {
            Color32::from_rgb(255, 180, 70)
        } else if is_selected {
            Color32::from_rgb(120, 190, 255)
        } else {
            Color32::from_rgb(70, 130, 220)
        };

        painter.rect(
            note_rect,
            3.0,
            fill,
            egui::Stroke::new(
                1.0_f32,
                if is_selected {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(180, 210, 255)
                },
            ),
            egui::StrokeKind::Inside,
        );

        let velocity_height = (note.velocity as f32 / 127.0) * (note_rect.height() - 4.0);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(note_rect.left() + 2.0, note_rect.bottom() - velocity_height - 1.0),
                Pos2::new(note_rect.right() - 2.0, note_rect.bottom() - 1.0),
            ),
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 70),
        );
    }
}

fn draw_marquee(painter: &egui::Painter, selection: Rect) {
    painter.rect(
        selection,
        0.0,
        Color32::from_rgba_unmultiplied(120, 180, 255, 40),
        egui::Stroke::new(1.0_f32, Color32::from_rgb(160, 210, 255)),
        egui::StrokeKind::Inside,
    );
}

fn set_single_selection(selected_note_ids: &mut HashSet<u64>, note_id: u64) {
    selected_note_ids.clear();
    selected_note_ids.insert(note_id);
}

fn apply_move_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    clip_id: u64,
    current_beats: f32,
    current_pitch: i32,
) {
    let Some(primary) = drag
        .originals
        .iter()
        .find(|note| note.id == drag.note_id)
    else {
        return;
    };

    let raw_delta_beats = current_beats - drag.pointer_start_beats;
    let mut snapped_delta_beats = Project::snap_beats(primary.start_beats + raw_delta_beats)
        .max(0.0)
        - primary.start_beats;

    let min_start = drag
        .originals
        .iter()
        .map(|note| note.start_beats)
        .fold(f32::INFINITY, f32::min);
    if min_start + snapped_delta_beats < 0.0 {
        snapped_delta_beats = -min_start;
    }

    let raw_delta_pitch = current_pitch - drag.pointer_start_pitch;
    let min_pitch = drag
        .originals
        .iter()
        .map(|note| note.pitch as i32)
        .min()
        .unwrap_or(MIN_PITCH as i32);
    let max_pitch = drag
        .originals
        .iter()
        .map(|note| note.pitch as i32)
        .max()
        .unwrap_or(MAX_PITCH as i32);
    let delta_pitch = raw_delta_pitch
        .max(MIN_PITCH as i32 - min_pitch)
        .min(MAX_PITCH as i32 - max_pitch);

    for original in &drag.originals {
        if let Some(note) = project.clip_mut(clip_id).and_then(|clip| clip.note_mut(original.id)) {
            note.start_beats = (original.start_beats + snapped_delta_beats).max(0.0);
            note.pitch = Project::clamp_pitch(original.pitch as i32 + delta_pitch);
            note.duration_beats = original.duration_beats;
        }
    }
}

fn resize_drag_mode(note_bounds: Rect, pointer_x: f32) -> Option<DragMode> {
    let local_x = pointer_x - note_bounds.left();
    let width = note_bounds.width();
    let handle = RESIZE_HANDLE_PX.min(width * 0.35);
    if local_x <= handle {
        Some(DragMode::ResizeStart)
    } else if local_x >= width - handle {
        Some(DragMode::ResizeEnd)
    } else {
        None
    }
}

fn update_grid_hover_cursor(
    response: &Response,
    content: Rect,
    notes: &[Note],
    metrics: ViewMetrics,
) {
    let Some(hover) = response.hover_pos() else {
        return;
    };
    if !content.contains(hover) {
        return;
    }
    let Some(note) = hit_test_note(content, notes, hover, metrics) else {
        return;
    };
    if resize_drag_mode(note_rect(content, note, metrics), hover.x).is_some() {
        response
            .ctx
            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn apply_resize_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    clip_id: u64,
    current_beats: f32,
) {
    let Some(original) = drag.originals.first() else {
        return;
    };
    let Some(note) = project
        .clip_mut(clip_id)
        .and_then(|clip| clip.note_mut(drag.note_id))
    else {
        return;
    };

    match drag.mode {
        DragMode::ResizeStart => {
            let new_start = Project::snap_beats(current_beats.max(0.0));
            let end = original.end_beats();
            note.start_beats = new_start.min(end - SNAP_BEATS);
            note.duration_beats = (end - note.start_beats).max(SNAP_BEATS);
            note.pitch = original.pitch;
        }
        DragMode::ResizeEnd => {
            let new_end = Project::snap_beats(current_beats.max(0.0));
            note.start_beats = original.start_beats;
            note.duration_beats = (new_end - original.start_beats).max(SNAP_BEATS);
            note.pitch = original.pitch;
        }
        DragMode::Move => {}
    }
}

fn finish_active_drag(
    active_drag: &mut Option<ActiveDrag>,
    project: &Project,
    clip_id: u64,
    default_duration_beats: &mut f32,
) {
    if let Some(drag) = active_drag.take() {
        if matches!(drag.mode, DragMode::ResizeStart | DragMode::ResizeEnd) {
            if let Some(note) = project.clip(clip_id).and_then(|clip| clip.note(drag.note_id)) {
                *default_duration_beats = note.duration_beats;
            }
        }
    }
}

fn handle_grid_pointer(
    response: &Response,
    content: Rect,
    metrics: ViewMetrics,
    clip_id: u64,
    project: &mut Project,
    engine: &mut dyn DawEngine,
    clip_start_beats: f32,
    selected_note_ids: &mut HashSet<u64>,
    active_drag: &mut Option<ActiveDrag>,
    marquee: &mut Option<MarqueeDrag>,
    default_duration_beats: &mut f32,
) {
    let clip_notes: Vec<Note> = project
        .clip(clip_id)
        .map(|clip| clip.notes.clone())
        .unwrap_or_default();

    update_grid_hover_cursor(response, content, &clip_notes, metrics);

    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            finish_active_drag(active_drag, project, clip_id, default_duration_beats);
            *marquee = None;
        }
        return;
    };

    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    if let Some(drag) = active_drag.clone() {
        if response.dragged() {
            let current_beats = content_x_to_beat(content, pointer.x, metrics);
            let current_pitch = y_to_pitch(content, pointer.y, metrics) as i32;
            match drag.mode {
                DragMode::Move => {
                    apply_move_drag(&drag, project, clip_id, current_beats, current_pitch);
                }
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    response
                        .ctx
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    apply_resize_drag(&drag, project, clip_id, current_beats);
                }
            }
        }
        if response.drag_stopped() {
            finish_active_drag(active_drag, project, clip_id, default_duration_beats);
            *marquee = None;
        }
        return;
    }

    if !content.contains(pointer) {
        return;
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if let Some(note) = hit_test_note(content, &clip_notes, pointer, metrics) {
            let note_id = note.id;
            if let Some(clip) = project.clip_mut(clip_id) {
                clip.remove_note(note_id);
            }
            selected_note_ids.remove(&note_id);
        } else {
            let beat = Project::snap_beats(content_x_to_beat(content, pointer.x, metrics).max(0.0));
            engine.seek_beats((beat + clip_start_beats).max(0.0));
        }
    }

    if response.clicked_by(egui::PointerButton::Primary) && !response.dragged() {
        if let Some(note) = hit_test_note(content, &clip_notes, pointer, metrics) {
            set_single_selection(selected_note_ids, note.id);
        } else {
            let pitch = y_to_pitch(content, pointer.y, metrics);
            let start =
                Project::snap_beats(content_x_to_beat(content, pointer.x, metrics).max(0.0));
            if let Some(note) =
                project.add_note_to_clip(clip_id, pitch, start, *default_duration_beats)
            {
                set_single_selection(selected_note_ids, note.id);
            }
        }
    }

    if response.drag_started_by(egui::PointerButton::Primary) && content.contains(press_pos) {
        if let Some(note) = hit_test_note(content, &clip_notes, press_pos, metrics).cloned() {
            *marquee = None;

            let note_bounds = note_rect(content, &note, metrics);
            let mode = resize_drag_mode(note_bounds, press_pos.x).unwrap_or(DragMode::Move);

            let already_selected = selected_note_ids.contains(&note.id);
            if !already_selected {
                set_single_selection(selected_note_ids, note.id);
            }

            let originals = match mode {
                DragMode::Move => project
                    .clip(clip_id)
                    .map(|clip| {
                        clip.notes
                            .iter()
                            .filter(|n| selected_note_ids.contains(&n.id))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                DragMode::ResizeStart | DragMode::ResizeEnd => vec![note.clone()],
            };

            *active_drag = Some(ActiveDrag {
                note_id: note.id,
                mode,
                pointer_start_beats: content_x_to_beat(content, press_pos.x, metrics),
                pointer_start_pitch: y_to_pitch(content, press_pos.y, metrics) as i32,
                originals,
            });

            let current_beats = content_x_to_beat(content, pointer.x, metrics);
            let current_pitch = y_to_pitch(content, pointer.y, metrics) as i32;
            if let Some(drag) = active_drag.clone() {
                match drag.mode {
                    DragMode::Move => {
                        apply_move_drag(&drag, project, clip_id, current_beats, current_pitch);
                    }
                    DragMode::ResizeStart | DragMode::ResizeEnd => {
                        response
                            .ctx
                            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        apply_resize_drag(&drag, project, clip_id, current_beats);
                    }
                }
            }
        } else {
            *active_drag = None;
            selected_note_ids.clear();
            *marquee = Some(MarqueeDrag {
                start: press_pos,
                current: pointer,
            });
        }
    }

    if let Some(active_marquee) = marquee.as_mut() {
        if response.dragged() {
            active_marquee.current = pointer;
            *selected_note_ids = select_notes_in_rect(
                content,
                &clip_notes,
                active_marquee.rect(),
                metrics,
            );
        }
    }

    if response.drag_stopped() {
        finish_active_drag(active_drag, project, clip_id, default_duration_beats);
        *marquee = None;
    }
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}
