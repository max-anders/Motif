use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{Note, Project, MAX_PITCH, MIN_PITCH, SNAP_BEATS};

const KEY_COLUMN_WIDTH: f32 = 44.0;
const KEY_HEIGHT: f32 = 18.0;
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
        let grid_size = Vec2::new(
            KEY_COLUMN_WIDTH + total_beats * BEAT_WIDTH,
            pitch_span * KEY_HEIGHT,
        );

        ui.horizontal(|ui| {
            ui.label(format!("{} notes", project.notes.len()));
            ui.separator();
            ui.label("Click empty grid: add note");
            ui.label("Drag body: move");
            ui.label("Drag edges: resize");
            ui.label("Delete: remove selected");
        });

        ui.add_space(4.0);

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (response, painter) =
                    ui.allocate_painter(grid_size, Sense::click_and_drag());
                let rect = response.rect;
                let _scroll_offset = ui.min_rect().min.to_vec2() - rect.min.to_vec2();

                draw_grid(&painter, rect, total_beats, project.beats_per_bar);
                draw_notes(
                    &painter,
                    rect,
                    &project.notes,
                    self.selected_note_id,
                    engine.current_beats(),
                );
                draw_playhead(&painter, rect, engine.current_beats());

                handle_pointer(
                    &response,
                    rect,
                    project,
                    engine,
                    &mut self.selected_note_id,
                    &mut self.active_drag,
                );

                if ui.ui_contains_pointer() && ui.input(|i| i.pointer.secondary_clicked()) {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        if let Some(note) = hit_test_note(rect, &project.notes, pointer) {
                            let note_id = note.id;
                            project.remove_note(note_id);
                            if self.selected_note_id == Some(note_id) {
                                self.selected_note_id = None;
                            }
                        }
                    }
                }
            });
    }
}

fn pitch_to_y(rect: Rect, pitch: u8) -> f32 {
    let row = (MAX_PITCH as i32 - pitch as i32) as f32;
    rect.top() + row * KEY_HEIGHT
}

fn y_to_pitch(rect: Rect, y: f32) -> u8 {
    let row = ((y - rect.top()) / KEY_HEIGHT).floor() as i32;
    let pitch = MAX_PITCH as i32 - row;
    Project::clamp_pitch(pitch)
}

fn beat_to_x(rect: Rect, beat: f32) -> f32 {
    rect.left() + KEY_COLUMN_WIDTH + beat * BEAT_WIDTH
}

fn x_to_beat(rect: Rect, x: f32) -> f32 {
    (x - rect.left() - KEY_COLUMN_WIDTH) / BEAT_WIDTH
}

fn note_rect(rect: Rect, note: &Note) -> Rect {
    let top = pitch_to_y(rect, note.pitch);
    Rect::from_min_max(
        Pos2::new(beat_to_x(rect, note.start_beats), top + 1.0),
        Pos2::new(
            beat_to_x(rect, note.end_beats()),
            top + KEY_HEIGHT - 1.0,
        ),
    )
}

fn hit_test_note<'a>(rect: Rect, notes: &'a [Note], pos: Pos2) -> Option<&'a Note> {
    notes
        .iter()
        .rev()
        .find(|note| note_rect(rect, note).contains(pos))
}

fn draw_grid(painter: &egui::Painter, rect: Rect, total_beats: f32, beats_per_bar: f32) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(18, 18, 22));

    for pitch in MIN_PITCH..=MAX_PITCH {
        let y = pitch_to_y(rect, pitch);
        let is_black_key = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
        let row_color = if is_black_key {
            Color32::from_rgb(26, 26, 32)
        } else {
            Color32::from_rgb(32, 32, 40)
        };
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left(), y),
                Pos2::new(rect.right(), y + KEY_HEIGHT),
            ),
            0.0,
            row_color,
        );

        if pitch % 12 == 0 {
            painter.text(
                Pos2::new(rect.left() + 4.0, y + 3.0),
                egui::Align2::LEFT_TOP,
                pitch_name(pitch),
                egui::FontId::monospace(10.0),
                Color32::from_rgb(150, 150, 165),
            );
        }
    }

    painter.rect_filled(
        Rect::from_min_max(
            rect.min,
            Pos2::new(rect.left() + KEY_COLUMN_WIDTH, rect.bottom()),
        ),
        0.0,
        Color32::from_rgb(24, 24, 30),
    );

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = beat_to_x(rect, beat as f32);
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let color = if is_bar {
            Color32::from_rgb(90, 90, 110)
        } else {
            Color32::from_rgb(45, 45, 58)
        };
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            egui::Stroke::new(if is_bar { 1.5_f32 } else { 1.0_f32 }, color),
        );

        if is_bar {
            painter.text(
                Pos2::new(x + 4.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{}", beat / beats_per_bar as i32 + 1),
                egui::FontId::monospace(11.0),
                Color32::from_rgb(170, 170, 185),
            );
        }
    }

    for subdivision in 0..=(beat_count * 4) {
        let beat = subdivision as f32 * SNAP_BEATS;
        if (beat.rem_euclid(beats_per_bar)).fract() == 0.0 && beat.fract() == 0.0 {
            continue;
        }
        let x = beat_to_x(rect, beat);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            egui::Stroke::new(1.0_f32, Color32::from_rgb(34, 34, 44)),
        );
    }
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

fn draw_playhead(painter: &egui::Painter, rect: Rect, beat: f32) {
    let x = beat_to_x(rect, beat);
    painter.line_segment(
        [
            Pos2::new(x, rect.top()),
            Pos2::new(x, rect.bottom()),
        ],
        egui::Stroke::new(2.0_f32, Color32::from_rgb(255, 90, 90)),
    );
}

fn handle_pointer(
    response: &Response,
    rect: Rect,
    project: &mut Project,
    engine: &mut dyn DawEngine,
    selected_note_id: &mut Option<u64>,
    active_drag: &mut Option<ActiveDrag>,
) {
    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            *active_drag = None;
        }
        return;
    };

    if !rect.contains(pointer) {
        return;
    }

    if response.drag_started() && response.clicked_by(egui::PointerButton::Primary) {
        if let Some(note) = hit_test_note(rect, &project.notes, pointer).cloned() {
            *selected_note_id = Some(note.id);

            let note_rect = note_rect(rect, &note);
            let local_x = pointer.x - note_rect.left();
            let mode = if local_x <= RESIZE_HANDLE_PX {
                DragMode::ResizeStart
            } else if local_x >= note_rect.width() - RESIZE_HANDLE_PX {
                DragMode::ResizeEnd
            } else {
                DragMode::Move
            };

            *active_drag = Some(ActiveDrag {
                note_id: note.id,
                mode,
                pointer_start_beats: x_to_beat(rect, pointer.x),
                pointer_start_pitch: y_to_pitch(rect, pointer.y) as i32,
                original: note,
            });
        } else if pointer.x > rect.left() + KEY_COLUMN_WIDTH {
            let pitch = y_to_pitch(rect, pointer.y);
            let start = Project::snap_beats(x_to_beat(rect, pointer.x).max(0.0));
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

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && pointer.x > rect.left() + KEY_COLUMN_WIDTH
    {
        let beat = Project::snap_beats(x_to_beat(rect, pointer.x).max(0.0));
        engine.seek_beats(beat);
    }

    if let Some(drag) = active_drag.clone() {
        if !response.dragged() {
            return;
        }

        let current_beats = x_to_beat(rect, pointer.x);
        let current_pitch = y_to_pitch(rect, pointer.y) as i32;

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
