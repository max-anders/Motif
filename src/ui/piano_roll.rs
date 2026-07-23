use std::collections::HashSet;

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    Note, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};

const KEY_COLUMN_WIDTH: f32 = 56.0;
const RULER_HEIGHT: f32 = 26.0;
const BLACK_KEY_WIDTH_RATIO: f32 = 0.62;
const RESIZE_HANDLE_PX: f32 = 12.0;

const DEFAULT_KEY_HEIGHT: f32 = 18.0;
const MIN_KEY_HEIGHT: f32 = 8.0;
const MAX_KEY_HEIGHT: f32 = 48.0;

const DEFAULT_BEAT_WIDTH: f32 = 88.0;
const MIN_BEAT_WIDTH: f32 = 24.0;
const MAX_BEAT_WIDTH: f32 = 400.0;

/// Matches egui's default `scroll_zoom_speed` so Alt-zoom feels like Ctrl-zoom.
const SCROLL_ZOOM_SPEED: f32 = 1.0 / 200.0;

#[derive(Debug, Clone, Copy)]
struct ViewMetrics {
    beat_width: f32,
    key_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Debug, Clone)]
struct ActiveDrag {
    /// Note under the pointer (used for snap / resize).
    note_id: u64,
    mode: DragMode,
    pointer_start_beats: f32,
    pointer_start_pitch: i32,
    /// Snapshot of every note being transformed (selection for Move, one note for resize).
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
    /// Length used for newly placed notes; updated when a note is resized.
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
        Self {
            selected_note_ids: HashSet::new(),
            active_drag: None,
            marquee: None,
            dragging_playhead: false,
            default_duration_beats: DEFAULT_NOTE_DURATION_BEATS,
            beat_width: DEFAULT_BEAT_WIDTH,
            key_height: DEFAULT_KEY_HEIGHT,
            scroll_offset: Vec2::ZERO,
        }
    }
}

impl PianoRollUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
    ) {
        let viewport_rect = ui.available_rect_before_wrap();
        apply_wheel_view_controls(
            ui,
            viewport_rect,
            &mut self.beat_width,
            &mut self.key_height,
            &mut self.scroll_offset,
        );

        let metrics = ViewMetrics {
            beat_width: self.beat_width,
            key_height: self.key_height,
        };
        let total_beats = project.loop_end_beats.max(4.0);
        let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
        let content_size = Vec2::new(
            KEY_COLUMN_WIDTH + total_beats * metrics.beat_width,
            RULER_HEIGHT + pitch_span * metrics.key_height,
        );
        let viewport = ui.available_size();
        let canvas_size = Vec2::new(
            content_size.x.max(viewport.x),
            content_size.y.max(viewport.y),
        );

        let output = egui::ScrollArea::both()
            .id_salt("piano_roll_canvas")
            .scroll_offset(self.scroll_offset)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_size(canvas_size);
                let (response, painter) =
                    ui.allocate_painter(canvas_size, Sense::click_and_drag());
                let rect = response.rect;
                let ruler_rect = ruler_rect(rect);
                let grid_rect = grid_rect(rect);

                handle_pointer(
                    &response,
                    ruler_rect,
                    grid_rect,
                    metrics,
                    project,
                    engine,
                    &mut self.selected_note_ids,
                    &mut self.active_drag,
                    &mut self.marquee,
                    &mut self.dragging_playhead,
                    &mut self.default_duration_beats,
                );

                draw_ruler(
                    &painter,
                    ruler_rect,
                    grid_rect,
                    metrics,
                    total_beats,
                    project.beats_per_bar,
                );
                draw_grid(
                    &painter,
                    grid_rect,
                    metrics,
                    total_beats,
                    project.beats_per_bar,
                );
                draw_keyboard(
                    &painter,
                    grid_rect,
                    metrics,
                    &project.notes,
                    engine.current_beats(),
                );
                draw_notes(
                    &painter,
                    grid_rect,
                    metrics,
                    &project.notes,
                    &self.selected_note_ids,
                    engine.current_beats(),
                );
                if let Some(marquee) = &self.marquee {
                    draw_marquee(&painter, marquee.rect());
                }
                draw_playhead(
                    &painter,
                    ruler_rect,
                    grid_rect,
                    metrics,
                    engine.current_beats(),
                );
            });

        self.scroll_offset = output.state.offset;
    }
}

/// Wheel / modifier mapping (when pointer is over the piano roll):
/// - Wheel: vertical scroll
/// - Shift+Wheel: horizontal scroll
/// - Ctrl/Cmd+Wheel: horizontal zoom (beats)
/// - Alt+Wheel: vertical zoom (keys)
/// - Ctrl/Cmd+Alt+Wheel: zoom both axes
fn apply_wheel_view_controls(
    ui: &Ui,
    viewport: Rect,
    beat_width: &mut f32,
    key_height: &mut f32,
    scroll_offset: &mut Vec2,
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
        // Consume scroll so ScrollArea does not move while we zoom keys.
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

    // Content coordinate under the pointer (viewport-local + scroll).
    let content_pos = pointer - viewport.min + *scroll_offset;

    if h_factor != 1.0 {
        let old = *beat_width;
        let new = (old * h_factor).clamp(MIN_BEAT_WIDTH, MAX_BEAT_WIDTH);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && content_pos.x > KEY_COLUMN_WIDTH {
            let timeline_x = content_pos.x - KEY_COLUMN_WIDTH;
            scroll_offset.x += timeline_x * (actual - 1.0);
        }
        *beat_width = new;
    }

    if v_factor != 1.0 {
        let old = *key_height;
        let new = (old * v_factor).clamp(MIN_KEY_HEIGHT, MAX_KEY_HEIGHT);
        let actual = new / old;
        if (actual - 1.0).abs() > f32::EPSILON && content_pos.y > RULER_HEIGHT {
            let keys_y = content_pos.y - RULER_HEIGHT;
            scroll_offset.y += keys_y * (actual - 1.0);
        }
        *key_height = new;
    }
}

fn ruler_rect(full: Rect) -> Rect {
    Rect::from_min_max(full.min, Pos2::new(full.right(), full.top() + RULER_HEIGHT))
}

fn grid_rect(full: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(full.left(), full.top() + RULER_HEIGHT),
        full.max,
    )
}

fn timeline_x(full: Rect, beat: f32, metrics: ViewMetrics) -> f32 {
    full.left() + KEY_COLUMN_WIDTH + beat * metrics.beat_width
}

fn x_to_beat(full: Rect, x: f32, metrics: ViewMetrics) -> f32 {
    (x - full.left() - KEY_COLUMN_WIDTH) / metrics.beat_width
}

fn seek_from_pointer(
    full: Rect,
    pointer: Pos2,
    metrics: ViewMetrics,
    engine: &mut dyn DawEngine,
) {
    if pointer.x <= full.left() + KEY_COLUMN_WIDTH {
        return;
    }
    let beat = Project::snap_beats(x_to_beat(full, pointer.x, metrics).max(0.0));
    engine.seek_beats(beat);
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

fn note_rect(grid: Rect, note: &Note, metrics: ViewMetrics) -> Rect {
    let top = pitch_to_y(grid, note.pitch, metrics);
    Rect::from_min_max(
        Pos2::new(timeline_x(grid, note.start_beats, metrics), top + 1.0),
        Pos2::new(
            timeline_x(grid, note.end_beats(), metrics),
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

fn draw_ruler(
    painter: &egui::Painter,
    ruler: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    total_beats: f32,
    beats_per_bar: f32,
) {
    painter.rect_filled(ruler, 0.0, Color32::from_rgb(28, 28, 34));

    painter.rect_filled(
        Rect::from_min_max(
            ruler.min,
            Pos2::new(ruler.left() + KEY_COLUMN_WIDTH, ruler.bottom()),
        ),
        0.0,
        Color32::from_rgb(22, 22, 28),
    );

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(grid, beat as f32, metrics);
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
        let x = timeline_x(grid, beat, metrics);
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
            Pos2::new(grid.left() + KEY_COLUMN_WIDTH, ruler.bottom()),
            Pos2::new(ruler.right(), ruler.bottom()),
        ],
        egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 55, 68)),
    );
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
    let timeline_left = grid.left() + KEY_COLUMN_WIDTH;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(timeline_left, grid.top()), grid.max),
        0.0,
        Color32::from_rgb(18, 18, 22),
    );

    for pitch in MIN_PITCH..=MAX_PITCH {
        let y = pitch_to_y(grid, pitch, metrics);
        let row_color = if is_black_key(pitch) {
            Color32::from_rgb(26, 26, 32)
        } else {
            Color32::from_rgb(32, 32, 40)
        };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(timeline_left, y),
                Pos2::new(grid.right(), y + metrics.key_height),
            ),
            0.0,
            row_color,
        );
    }

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(grid, beat as f32, metrics);
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
        let x = timeline_x(grid, beat, metrics);
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
        Pos2::new(grid.left() + KEY_COLUMN_WIDTH, grid.bottom()),
    );
    painter.rect_filled(keys, 0.0, Color32::from_rgb(48, 48, 56));

    let is_pitch_active = |pitch: u8| {
        notes
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

        let is_c = pitch % 12 == 0;
        painter.text(
            Pos2::new(keys.right() - 4.0, y + metrics.key_height * 0.5),
            egui::Align2::RIGHT_CENTER,
            pitch_name(pitch),
            egui::FontId::monospace(if is_c { 11.0 } else { 9.0 }),
            if is_c {
                Color32::from_rgb(40, 40, 55)
            } else {
                Color32::from_rgb(90, 90, 105)
            },
        );
    }

    for pitch in MIN_PITCH..=MAX_PITCH {
        if !is_black_key(pitch) {
            continue;
        }

        let y = pitch_to_y(grid, pitch, metrics);
        let black_width = KEY_COLUMN_WIDTH * BLACK_KEY_WIDTH_RATIO;
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y + 1.0),
            Pos2::new(
                keys.left() + black_width,
                y + metrics.key_height - 1.0,
            ),
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

        painter.text(
            Pos2::new(key_rect.right() - 3.0, y + metrics.key_height * 0.5),
            egui::Align2::RIGHT_CENTER,
            pitch_name(pitch),
            egui::FontId::monospace(8.0),
            if is_active {
                Color32::from_rgb(40, 30, 20)
            } else {
                Color32::from_rgb(170, 170, 185)
            },
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
        let is_active = note.contains_beat(playhead_beats);

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

fn draw_playhead(
    painter: &egui::Painter,
    ruler: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    beat: f32,
) {
    let x = timeline_x(grid, beat, metrics);
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

fn is_timeline_pointer(grid: Rect, pointer: Pos2) -> bool {
    grid.contains(pointer) && pointer.x > grid.left() + KEY_COLUMN_WIDTH
}

fn is_ruler_timeline_pointer(ruler: Rect, pointer: Pos2) -> bool {
    ruler.contains(pointer) && pointer.x > ruler.left() + KEY_COLUMN_WIDTH
}

fn set_single_selection(selected_note_ids: &mut HashSet<u64>, note_id: u64) {
    selected_note_ids.clear();
    selected_note_ids.insert(note_id);
}

fn apply_move_drag(drag: &ActiveDrag, project: &mut Project, current_beats: f32, current_pitch: i32) {
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
        if let Some(note) = project.note_mut(original.id) {
            note.start_beats = (original.start_beats + snapped_delta_beats).max(0.0);
            note.pitch = Project::clamp_pitch(original.pitch as i32 + delta_pitch);
            note.duration_beats = original.duration_beats;
        }
    }
}

fn resize_drag_mode(note_bounds: Rect, pointer_x: f32) -> Option<DragMode> {
    let local_x = pointer_x - note_bounds.left();
    let width = note_bounds.width();
    // Keep a usable move zone in the middle when the note is wide enough.
    let handle = RESIZE_HANDLE_PX.min(width * 0.35);
    if local_x <= handle {
        Some(DragMode::ResizeStart)
    } else if local_x >= width - handle {
        Some(DragMode::ResizeEnd)
    } else {
        None
    }
}

fn update_resize_hover_cursor(
    response: &Response,
    grid: Rect,
    notes: &[Note],
    metrics: ViewMetrics,
) {
    let Some(hover) = response.hover_pos() else {
        return;
    };
    if !is_timeline_pointer(grid, hover) {
        return;
    }
    let Some(note) = hit_test_note(grid, notes, hover, metrics) else {
        return;
    };
    if resize_drag_mode(note_rect(grid, note, metrics), hover.x).is_some() {
        response
            .ctx
            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn apply_resize_drag(drag: &ActiveDrag, project: &mut Project, current_beats: f32) {
    let Some(original) = drag.originals.first() else {
        return;
    };
    let Some(note) = project.note_mut(drag.note_id) else {
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
    default_duration_beats: &mut f32,
) {
    if let Some(drag) = active_drag.take() {
        if matches!(drag.mode, DragMode::ResizeStart | DragMode::ResizeEnd) {
            if let Some(note) = project.note(drag.note_id) {
                *default_duration_beats = note.duration_beats;
            }
        }
    }
}

fn handle_pointer(
    response: &Response,
    ruler: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    project: &mut Project,
    engine: &mut dyn DawEngine,
    selected_note_ids: &mut HashSet<u64>,
    active_drag: &mut Option<ActiveDrag>,
    marquee: &mut Option<MarqueeDrag>,
    dragging_playhead: &mut bool,
    default_duration_beats: &mut f32,
) {
    let full = ruler.union(grid);

    update_resize_hover_cursor(response, grid, &project.notes, metrics);
    if let Some(hover) = response.hover_pos() {
        if is_ruler_timeline_pointer(ruler, hover) {
            response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            finish_active_drag(active_drag, project, default_duration_beats);
            *marquee = None;
            *dragging_playhead = false;
        }
        return;
    };

    // Press origin matters: by drag_started the pointer has already moved past the edge.
    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    // Keep scrubbing if the pointer leaves the ruler vertically while dragging.
    if *dragging_playhead {
        if response.dragged() {
            seek_from_pointer(grid, pointer, metrics, engine);
            response.ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.drag_stopped() {
            *dragging_playhead = false;
        }
        return;
    }

    // Continue an in-progress note drag even if the pointer leaves the canvas briefly.
    if let Some(drag) = active_drag.clone() {
        if response.dragged() {
            let current_beats = x_to_beat(grid, pointer.x, metrics);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            match drag.mode {
                DragMode::Move => {
                    apply_move_drag(&drag, project, current_beats, current_pitch);
                }
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    response
                        .ctx
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    apply_resize_drag(&drag, project, current_beats);
                }
            }
        }
        if response.drag_stopped() {
            finish_active_drag(active_drag, project, default_duration_beats);
            *marquee = None;
        }
        return;
    }

    if !full.contains(pointer) {
        return;
    }

    let shift_held = response.ctx.input(|input| input.modifiers.shift);

    // Ruler: left-click or drag moves the playhead.
    if is_ruler_timeline_pointer(ruler, press_pos)
        || is_ruler_timeline_pointer(ruler, pointer)
    {
        if response.drag_started_by(egui::PointerButton::Primary)
            && is_ruler_timeline_pointer(ruler, press_pos)
        {
            *active_drag = None;
            *marquee = None;
            *dragging_playhead = true;
            seek_from_pointer(grid, pointer, metrics, engine);
            return;
        }
        if response.clicked_by(egui::PointerButton::Primary) && !response.dragged() {
            seek_from_pointer(grid, pointer, metrics, engine);
            return;
        }
        if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
            seek_from_pointer(grid, pointer, metrics, engine);
            return;
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && shift_held
        && is_timeline_pointer(grid, pointer)
    {
        seek_from_pointer(grid, pointer, metrics, engine);
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if grid.contains(pointer) {
            if let Some(note) = hit_test_note(grid, &project.notes, pointer, metrics) {
                let note_id = note.id;
                project.remove_note(note_id);
                selected_note_ids.remove(&note_id);
            } else if is_timeline_pointer(grid, pointer) {
                seek_from_pointer(grid, pointer, metrics, engine);
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && !shift_held
        && is_timeline_pointer(grid, pointer)
    {
        if let Some(note) = hit_test_note(grid, &project.notes, pointer, metrics) {
            set_single_selection(selected_note_ids, note.id);
        } else {
            let pitch = y_to_pitch(grid, pointer.y, metrics);
            let start = Project::snap_beats(x_to_beat(grid, pointer.x, metrics).max(0.0));
            let note = project.add_note(pitch, start, *default_duration_beats);
            set_single_selection(selected_note_ids, note.id);
        }
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(grid, press_pos)
    {
        if let Some(note) = hit_test_note(grid, &project.notes, press_pos, metrics).cloned() {
            *marquee = None;

            let note_bounds = note_rect(grid, &note, metrics);
            let mode = resize_drag_mode(note_bounds, press_pos.x).unwrap_or(DragMode::Move);

            let already_selected = selected_note_ids.contains(&note.id);
            if !already_selected {
                set_single_selection(selected_note_ids, note.id);
            }

            let originals = match mode {
                DragMode::Move => project
                    .notes
                    .iter()
                    .filter(|n| selected_note_ids.contains(&n.id))
                    .cloned()
                    .collect(),
                DragMode::ResizeStart | DragMode::ResizeEnd => vec![note.clone()],
            };

            *active_drag = Some(ActiveDrag {
                note_id: note.id,
                mode,
                pointer_start_beats: x_to_beat(grid, press_pos.x, metrics),
                pointer_start_pitch: y_to_pitch(grid, press_pos.y, metrics) as i32,
                originals,
            });

            // Apply first drag frame immediately (pointer may already be past the edge).
            let current_beats = x_to_beat(grid, pointer.x, metrics);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            if let Some(drag) = active_drag.clone() {
                match drag.mode {
                    DragMode::Move => {
                        apply_move_drag(&drag, project, current_beats, current_pitch);
                    }
                    DragMode::ResizeStart | DragMode::ResizeEnd => {
                        response
                            .ctx
                            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        apply_resize_drag(&drag, project, current_beats);
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
                grid,
                &project.notes,
                active_marquee.rect(),
                metrics,
            );
        }
    }

    if response.drag_stopped() {
        finish_active_drag(active_drag, project, default_duration_beats);
        *marquee = None;
        *dragging_playhead = false;
    }
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}
