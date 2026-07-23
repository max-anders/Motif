use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{Note, Project, MAX_PITCH, MIN_PITCH, SNAP_BEATS};

const KEY_COLUMN_WIDTH: f32 = 56.0;
const RULER_HEIGHT: f32 = 26.0;
const KEY_HEIGHT: f32 = 18.0;
const BLACK_KEY_WIDTH_RATIO: f32 = 0.62;
const BEAT_WIDTH: f32 = 88.0;
const RESIZE_HANDLE_PX: f32 = 8.0;

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
    original: Note,
}

pub struct PianoRollUi {
    selected_note_id: Option<u64>,
    active_drag: Option<ActiveDrag>,
}

impl PianoRollUi {
    pub fn selected_note_id(&self) -> Option<u64> {
        self.selected_note_id
    }

    pub fn clear_selection(&mut self) {
        self.selected_note_id = None;
    }
}

impl Default for PianoRollUi {
    fn default() -> Self {
        Self {
            selected_note_id: None,
            active_drag: None,
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
        let total_beats = project.loop_end_beats.max(4.0);
        let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
        let content_size = Vec2::new(
            KEY_COLUMN_WIDTH + total_beats * BEAT_WIDTH,
            RULER_HEIGHT + pitch_span * KEY_HEIGHT,
        );
        let viewport = ui.available_size();
        let canvas_size = Vec2::new(
            content_size.x.max(viewport.x),
            content_size.y.max(viewport.y),
        );

        egui::ScrollArea::both()
            .id_salt("piano_roll_canvas")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_size(canvas_size);
                let (response, painter) =
                    ui.allocate_painter(canvas_size, Sense::click_and_drag());
                let rect = response.rect;
                let ruler_rect = ruler_rect(rect);
                let grid_rect = grid_rect(rect);
                let _scroll_offset = ui.min_rect().min.to_vec2() - rect.min.to_vec2();

                draw_ruler(&painter, ruler_rect, grid_rect, total_beats, project.beats_per_bar);
                draw_grid(&painter, grid_rect, total_beats, project.beats_per_bar);
                draw_keyboard(&painter, grid_rect, &project.notes, engine.current_beats());
                draw_notes(
                    &painter,
                    grid_rect,
                    &project.notes,
                    self.selected_note_id,
                    engine.current_beats(),
                );
                draw_playhead(&painter, ruler_rect, grid_rect, engine.current_beats());

                handle_pointer(
                    &response,
                    ruler_rect,
                    grid_rect,
                    project,
                    engine,
                    &mut self.selected_note_id,
                    &mut self.active_drag,
                );
            });
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

fn timeline_x(full: Rect, beat: f32) -> f32 {
    full.left() + KEY_COLUMN_WIDTH + beat * BEAT_WIDTH
}

fn x_to_beat(full: Rect, x: f32) -> f32 {
    (x - full.left() - KEY_COLUMN_WIDTH) / BEAT_WIDTH
}

fn seek_from_pointer(full: Rect, pointer: Pos2, engine: &mut dyn DawEngine) {
    if pointer.x <= full.left() + KEY_COLUMN_WIDTH {
        return;
    }
    let beat = Project::snap_beats(x_to_beat(full, pointer.x).max(0.0));
    engine.seek_beats(beat);
}

fn pitch_to_y(grid: Rect, pitch: u8) -> f32 {
    let row = (MAX_PITCH as i32 - pitch as i32) as f32;
    grid.top() + row * KEY_HEIGHT
}

fn y_to_pitch(grid: Rect, y: f32) -> u8 {
    let row = ((y - grid.top()) / KEY_HEIGHT).floor() as i32;
    let pitch = MAX_PITCH as i32 - row;
    Project::clamp_pitch(pitch)
}

fn note_rect(grid: Rect, note: &Note) -> Rect {
    let top = pitch_to_y(grid, note.pitch);
    Rect::from_min_max(
        Pos2::new(timeline_x(grid, note.start_beats), top + 1.0),
        Pos2::new(
            timeline_x(grid, note.end_beats()),
            top + KEY_HEIGHT - 1.0,
        ),
    )
}

fn hit_test_note<'a>(grid: Rect, notes: &'a [Note], pos: Pos2) -> Option<&'a Note> {
    notes
        .iter()
        .rev()
        .find(|note| note_rect(grid, note).contains(pos))
}

fn draw_ruler(
    painter: &egui::Painter,
    ruler: Rect,
    grid: Rect,
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
        let x = timeline_x(grid, beat as f32);
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
        let x = timeline_x(grid, beat);
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

fn draw_grid(painter: &egui::Painter, grid: Rect, total_beats: f32, beats_per_bar: f32) {
    let timeline_left = grid.left() + KEY_COLUMN_WIDTH;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(timeline_left, grid.top()), grid.max),
        0.0,
        Color32::from_rgb(18, 18, 22),
    );

    for pitch in MIN_PITCH..=MAX_PITCH {
        let y = pitch_to_y(grid, pitch);
        let row_color = if is_black_key(pitch) {
            Color32::from_rgb(26, 26, 32)
        } else {
            Color32::from_rgb(32, 32, 40)
        };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(timeline_left, y),
                Pos2::new(grid.right(), y + KEY_HEIGHT),
            ),
            0.0,
            row_color,
        );
    }

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = timeline_x(grid, beat as f32);
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
        let x = timeline_x(grid, beat);
        painter.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(34, 34, 44)),
        );
    }
}

fn draw_keyboard(
    painter: &egui::Painter,
    grid: Rect,
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

        let y = pitch_to_y(grid, pitch);
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y),
            Pos2::new(keys.right(), y + KEY_HEIGHT),
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
            Pos2::new(keys.right() - 4.0, y + KEY_HEIGHT * 0.5),
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

        let y = pitch_to_y(grid, pitch);
        let black_width = KEY_COLUMN_WIDTH * BLACK_KEY_WIDTH_RATIO;
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y + 1.0),
            Pos2::new(keys.left() + black_width, y + KEY_HEIGHT - 1.0),
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
            Pos2::new(key_rect.right() - 3.0, y + KEY_HEIGHT * 0.5),
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
    notes: &[Note],
    selected_id: Option<u64>,
    playhead_beats: f32,
) {
    for note in notes {
        let note_rect = note_rect(rect, note);
        let is_selected = selected_id == Some(note.id);
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

fn draw_playhead(painter: &egui::Painter, ruler: Rect, grid: Rect, beat: f32) {
    let x = timeline_x(grid, beat);
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

fn handle_pointer(
    response: &Response,
    ruler: Rect,
    grid: Rect,
    project: &mut Project,
    engine: &mut dyn DawEngine,
    selected_note_id: &mut Option<u64>,
    active_drag: &mut Option<ActiveDrag>,
) {
    let full = ruler.union(grid);
    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            *active_drag = None;
        }
        return;
    };

    if !full.contains(pointer) {
        return;
    }

    if response.drag_started() && response.clicked_by(egui::PointerButton::Primary) {
        if grid.contains(pointer) {
            if let Some(note) = hit_test_note(grid, &project.notes, pointer).cloned() {
                *selected_note_id = Some(note.id);

                let note_bounds = note_rect(grid, &note);
                let local_x = pointer.x - note_bounds.left();
                let mode = if local_x <= RESIZE_HANDLE_PX {
                    DragMode::ResizeStart
                } else if local_x >= note_bounds.width() - RESIZE_HANDLE_PX {
                    DragMode::ResizeEnd
                } else {
                    DragMode::Move
                };

                *active_drag = Some(ActiveDrag {
                    note_id: note.id,
                    mode,
                    pointer_start_beats: x_to_beat(grid, pointer.x),
                    pointer_start_pitch: y_to_pitch(grid, pointer.y) as i32,
                    original: note,
                });
            } else if pointer.x > grid.left() + KEY_COLUMN_WIDTH {
                let pitch = y_to_pitch(grid, pointer.y);
                let start = Project::snap_beats(x_to_beat(grid, pointer.x).max(0.0));
                let note = project.add_note(pitch, start, 1.0);
                *selected_note_id = Some(note.id);
                *active_drag = Some(ActiveDrag {
                    note_id: note.id,
                    mode: DragMode::ResizeEnd,
                    pointer_start_beats: start,
                    pointer_start_pitch: pitch as i32,
                    original: note,
                });
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary) && !response.dragged() {
        if ruler.contains(pointer) || grid.contains(pointer) {
            seek_from_pointer(grid, pointer, engine);
        }
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if ruler.contains(pointer) {
            seek_from_pointer(grid, pointer, engine);
        } else if grid.contains(pointer) {
            if let Some(note) = hit_test_note(grid, &project.notes, pointer) {
                let note_id = note.id;
                project.remove_note(note_id);
                if *selected_note_id == Some(note_id) {
                    *selected_note_id = None;
                }
            } else {
                seek_from_pointer(grid, pointer, engine);
            }
        }
    }

    if let Some(drag) = active_drag.clone() {
        if !response.dragged() {
            return;
        }

        let current_beats = x_to_beat(grid, pointer.x);
        let current_pitch = y_to_pitch(grid, pointer.y) as i32;

        if let Some(note) = project.note_mut(drag.note_id) {
            match drag.mode {
                DragMode::Move => {
                    let delta_beats = current_beats - drag.pointer_start_beats;
                    let delta_pitch = current_pitch - drag.pointer_start_pitch;
                    note.start_beats =
                        Project::snap_beats((drag.original.start_beats + delta_beats).max(0.0));
                    note.pitch =
                        Project::clamp_pitch(drag.original.pitch as i32 + delta_pitch);
                    note.duration_beats = drag.original.duration_beats;
                }
                DragMode::ResizeStart => {
                    let new_start = Project::snap_beats(current_beats.max(0.0));
                    let end = drag.original.end_beats();
                    note.start_beats = new_start.min(end - SNAP_BEATS);
                    note.duration_beats = (end - note.start_beats).max(SNAP_BEATS);
                    note.pitch = drag.original.pitch;
                }
                DragMode::ResizeEnd => {
                    let new_end = Project::snap_beats(current_beats.max(0.0));
                    note.start_beats = drag.original.start_beats;
                    note.duration_beats =
                        (new_end - drag.original.start_beats).max(SNAP_BEATS);
                    note.pitch = drag.original.pitch;
                }
            }
        }
    }

    if response.drag_stopped() {
        *active_drag = None;
    }
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}
