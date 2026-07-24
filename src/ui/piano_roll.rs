use std::collections::HashSet;

use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    EditHistory, Note, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_piano_roll_wheel_controls, daw_editor_scroll_area, draw_playhead, draw_ruler,
    handle_timeline_playhead_pointer, is_timeline_pointer, timeline_body_rect, timeline_x,
    with_solid_scrollbars, x_to_beat, TimelineMetrics, DEFAULT_BEAT_WIDTH, RULER_HEIGHT,
    TIMELINE_GUTTER_WIDTH,
};

const BLACK_KEY_WIDTH_RATIO: f32 = 0.62;
const RESIZE_HANDLE_PX: f32 = 12.0;

const DEFAULT_KEY_HEIGHT: f32 = 18.0;
const MIN_KEY_HEIGHT: f32 = 8.0;
const MAX_KEY_HEIGHT: f32 = 48.0;

/// Short preview when clicking to create a note (seconds).
const NOTE_CREATE_PREVIEW_SECS: f64 = 0.18;

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
    audition_pitch: Option<u8>,
    /// Track instrument used for keyboard / preview audition.
    audition_track_id: u64,
    /// When true, audition is held by keyboard or note drag until pointer up.
    audition_held: bool,
    /// Wall-clock deadline for a one-shot create preview (`None` if held).
    audition_until: Option<f64>,
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

    pub fn set_selection(&mut self, note_ids: impl IntoIterator<Item = u64>) {
        self.selected_note_ids.clear();
        self.selected_note_ids.extend(note_ids);
    }

    pub fn prune_selection(&mut self, clip_id: u64, project: &Project) {
        let Some(clip) = project.clip(clip_id) else {
            self.selected_note_ids.clear();
            return;
        };
        self.selected_note_ids
            .retain(|id| clip.note(*id).is_some());
    }

    pub fn release_audition(&mut self, engine: &mut dyn DawEngine) {
        if let Some(pitch) = self.audition_pitch.take() {
            engine.note_off(self.audition_track_id, pitch);
        }
        self.audition_held = false;
        self.audition_until = None;
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
            audition_pitch: None,
            audition_track_id: 0,
            audition_held: false,
            audition_until: None,
            default_duration_beats: DEFAULT_NOTE_DURATION_BEATS,
            beat_width: DEFAULT_BEAT_WIDTH,
            key_height: DEFAULT_KEY_HEIGHT,
            scroll_offset: Vec2::new(0.0, initial_scroll_y),
        }
    }
}

impl PianoRollUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        clip_id: u64,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        theme: &ThemeColors,
    ) {
        let (clip_start, total_beats, beats_per_bar) = {
            let Some(clip) = project.clip(clip_id) else {
                return;
            };
            (
                clip.start_beats,
                clip.length_beats.max(4.0),
                project.beats_per_bar,
            )
        };
        self.audition_track_id = project.track_id_for_clip(clip_id).unwrap_or(0);
        let track_id = self.audition_track_id;
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
        let content_size = Vec2::new(
            TIMELINE_GUTTER_WIDTH + total_beats * metrics.beat_width,
            RULER_HEIGHT + pitch_span * metrics.key_height,
        );
        let viewport = ui.available_size();
        let canvas_size = Vec2::new(
            content_size.x.max(viewport.x),
            content_size.y.max(viewport.y),
        );

        let global_playhead = engine.current_beats();
        let local_playhead = global_playhead - clip_start;
        let playhead_visible = local_playhead >= 0.0 && local_playhead <= total_beats;

        let clip_notes: Vec<Note> = project
            .clip(clip_id)
            .map(|clip| clip.notes.clone())
            .unwrap_or_default();

        // Offset used to place content this frame (wheel updates apply next frame).
        let scroll = self.scroll_offset;

        let output = with_solid_scrollbars(ui, theme, |ui| {
            daw_editor_scroll_area("piano_roll_canvas")
                .scroll_offset(scroll)
                .show(ui, |ui| {
                    ui.set_min_size(canvas_size);
                    let (response, painter) =
                        ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    let grid = timeline_body_rect(content);

                    // Visible viewport in screen space. Ruler stays pinned to the top
                    // (follows horizontal scroll); piano keys stay pinned to the left
                    // (follow vertical scroll).
                    let viewport = Rect::from_min_size(content.min + scroll, viewport_rect.size())
                        .intersect(ui.clip_rect());
                    let sticky_ruler = Rect::from_min_max(
                        viewport.min,
                        Pos2::new(viewport.right(), viewport.top() + RULER_HEIGHT),
                    );
                    // Timeline X origin tracks horizontal scroll (content space).
                    let ruler_timeline = Rect::from_min_max(
                        Pos2::new(content.left(), sticky_ruler.top()),
                        Pos2::new(content.right(), sticky_ruler.bottom()),
                    );
                    // Keyboard X is viewport-left; Y tracks vertical scroll (content space).
                    let sticky_keys = Rect::from_min_max(
                        Pos2::new(viewport.left(), sticky_ruler.bottom()),
                        Pos2::new(viewport.left() + TIMELINE_GUTTER_WIDTH, viewport.bottom()),
                    );
                    let keys_grid = Rect::from_min_max(
                        Pos2::new(viewport.left(), grid.top()),
                        Pos2::new(viewport.left() + grid.width(), grid.bottom()),
                    );

                    tick_timed_audition(
                        engine,
                        track_id,
                        &mut self.audition_pitch,
                        &mut self.audition_held,
                        &mut self.audition_until,
                        response.ctx.input(|input| input.time),
                    );

                    // In-flight note/marquee drags always get pointer updates (including
                    // release outside the grid). Keyboard / playhead only win when idle.
                    let gesture_active = self.active_drag.is_some() || self.marquee.is_some();
                    let keyboard_handled = !gesture_active
                        && handle_keyboard_audition(
                            &response,
                            sticky_keys,
                            keys_grid,
                            metrics,
                            engine,
                            track_id,
                            &mut self.audition_pitch,
                            &mut self.audition_held,
                            &mut self.audition_until,
                        );

                    let playhead_handled = !gesture_active
                        && handle_timeline_playhead_pointer(
                            &response,
                            sticky_ruler,
                            grid,
                            metrics.timeline(),
                            engine,
                            &mut self.dragging_playhead,
                            clip_start,
                            // Piano roll owns body right-click: delete note or seek.
                            false,
                        );

                    if gesture_active || (!keyboard_handled && !playhead_handled) {
                        handle_pointer(
                            &response,
                            sticky_ruler,
                            grid,
                            metrics,
                            clip_id,
                            project,
                            history,
                            engine,
                            clip_start,
                            track_id,
                            &mut self.selected_note_ids,
                            &mut self.active_drag,
                            &mut self.marquee,
                            &mut self.default_duration_beats,
                            &mut self.audition_pitch,
                            &mut self.audition_held,
                            &mut self.audition_until,
                        );
                    }

                    // Keep scrolled content out of the sticky piano / ruler strips so
                    // notes and grid never paint under (or through) the pinned chrome.
                    let timeline_clip = Rect::from_min_max(
                        Pos2::new(sticky_keys.right(), sticky_ruler.bottom()),
                        content.max,
                    );
                    let timeline_painter = painter.with_clip_rect(timeline_clip);

                    draw_grid(
                        &timeline_painter,
                        grid,
                        metrics,
                        total_beats,
                        beats_per_bar,
                        theme,
                    );
                    let playing = engine.is_playing();
                    draw_notes(
                        &timeline_painter,
                        grid,
                        metrics,
                        &clip_notes,
                        &self.selected_note_ids,
                        if playhead_visible {
                            local_playhead
                        } else {
                            -1.0
                        },
                        playing,
                        theme,
                    );
                    if let Some(marquee) = &self.marquee {
                        draw_marquee(&timeline_painter, marquee.rect(), theme);
                    }

                    // Sticky chrome on top of scrolled content.
                    draw_keyboard(
                        &painter.with_clip_rect(sticky_keys),
                        keys_grid,
                        metrics,
                        &clip_notes,
                        if playhead_visible {
                            local_playhead
                        } else {
                            -1.0
                        },
                        playing,
                        self.audition_pitch,
                        theme,
                    );
                    draw_ruler(
                        &painter.with_clip_rect(sticky_ruler),
                        sticky_ruler,
                        ruler_timeline,
                        metrics.timeline(),
                        total_beats,
                        beats_per_bar,
                        theme,
                    );
                    // Playhead last, clipped to the right of the piano keys so it stays
                    // above notes/ruler and never draws behind the pinned keyboard.
                    let playhead_clip = Rect::from_min_max(
                        Pos2::new(sticky_keys.right(), sticky_ruler.top()),
                        content.max,
                    );
                    draw_playhead(
                        &painter.with_clip_rect(playhead_clip),
                        sticky_ruler,
                        grid,
                        metrics.timeline(),
                        local_playhead,
                        playhead_visible,
                        theme,
                    );
                })
        });

        self.scroll_offset = output.state.offset;
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

fn note_rect(grid: Rect, note: &Note, metrics: ViewMetrics) -> Rect {
    let top = pitch_to_y(grid, note.pitch, metrics);
    Rect::from_min_max(
        Pos2::new(
            timeline_x(grid, note.start_beats, metrics.timeline()),
            top + 1.0,
        ),
        Pos2::new(
            timeline_x(grid, note.end_beats(), metrics.timeline()),
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
    theme: &ThemeColors,
) {
    let timeline_left = grid.left() + TIMELINE_GUTTER_WIDTH;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(timeline_left, grid.top()), grid.max),
        0.0,
        theme.panel_bg,
    );

    for pitch in MIN_PITCH..=MAX_PITCH {
        let y = pitch_to_y(grid, pitch, metrics);
        let row_color = if is_black_key(pitch) {
            theme.key_row_black
        } else {
            theme.key_row_white
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
        let x = timeline_x(grid, beat as f32, metrics.timeline());
        let is_bar = (beat as f32).rem_euclid(beats_per_bar) == 0.0;
        let color = if is_bar {
            theme.grid_bar
        } else {
            theme.grid_beat
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
        let x = timeline_x(grid, beat as f32, metrics.timeline());
        painter.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            egui::Stroke::new(1.0_f32, theme.grid_subbeat),
        );
    }
}

fn draw_keyboard(
    painter: &egui::Painter,
    grid: Rect,
    metrics: ViewMetrics,
    notes: &[Note],
    playhead_beats: f32,
    playing: bool,
    audition_pitch: Option<u8>,
    theme: &ThemeColors,
) {
    let keys = Rect::from_min_max(
        grid.min,
        Pos2::new(grid.left() + TIMELINE_GUTTER_WIDTH, grid.bottom()),
    );
    painter.rect_filled(keys, 0.0, theme.keys_bg);

    let is_pitch_active = |pitch: u8| {
        audition_pitch == Some(pitch)
            || (playing
                && playhead_beats >= 0.0
                && notes
                    .iter()
                    .any(|note| note.pitch == pitch && note.contains_beat(playhead_beats)))
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
            theme.white_key_active
        } else {
            theme.white_key
        };
        painter.rect_filled(key_rect, 0.0, fill);
        painter.line_segment(
            [
                Pos2::new(keys.left(), key_rect.bottom()),
                Pos2::new(keys.right(), key_rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, theme.white_key_border),
        );

        if pitch % 12 == 0 {
            painter.text(
                Pos2::new(keys.right() - 4.0, y + metrics.key_height * 0.5),
                egui::Align2::RIGHT_CENTER,
                pitch_name(pitch),
                egui::FontId::monospace(11.0),
                theme.white_key_label,
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
            theme.black_key_active
        } else {
            theme.black_key
        };
        painter.rect(
            key_rect,
            1.5,
            fill,
            egui::Stroke::new(1.0_f32, theme.black_key_border),
            egui::StrokeKind::Inside,
        );
    }

    painter.line_segment(
        [
            Pos2::new(keys.right(), keys.top()),
            Pos2::new(keys.right(), keys.bottom()),
        ],
        egui::Stroke::new(1.5_f32, theme.key_divider),
    );
}

fn set_audition_pitch(
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    pitch: u8,
) {
    match *audition_pitch {
        Some(current) if current == pitch => {}
        Some(current) => {
            engine.note_off(track_id, current);
            engine.note_on(track_id, pitch, 100);
            *audition_pitch = Some(pitch);
        }
        None => {
            engine.note_on(track_id, pitch, 100);
            *audition_pitch = Some(pitch);
        }
    }
}

fn clear_audition(
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    if let Some(pitch) = audition_pitch.take() {
        engine.note_off(track_id, pitch);
    }
    *audition_held = false;
    *audition_until = None;
}

fn tick_timed_audition(
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
    now: f64,
) {
    if *audition_held {
        return;
    }
    if let Some(until) = *audition_until {
        if now >= until {
            clear_audition(
                engine,
                track_id,
                audition_pitch,
                audition_held,
                audition_until,
            );
        }
    }
}

fn preview_pitch_briefly(
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
    pitch: u8,
    now: f64,
) {
    set_audition_pitch(engine, track_id, audition_pitch, pitch);
    *audition_held = false;
    *audition_until = Some(now + NOTE_CREATE_PREVIEW_SECS);
}

fn hold_audition_pitch(
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
    pitch: u8,
) {
    set_audition_pitch(engine, track_id, audition_pitch, pitch);
    *audition_held = true;
    *audition_until = None;
}

/// Hit-test and audition piano keys. Returns true when the interaction is owned by the keyboard.
fn handle_keyboard_audition(
    response: &Response,
    keys: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) -> bool {
    let primary_down = response.ctx.input(|input| input.pointer.primary_down());
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.ctx.input(|input| input.pointer.latest_pos()));

    if !primary_down {
        // Only release held keyboard/drag auditions; timed create previews keep ringing.
        if *audition_held {
            clear_audition(
                engine,
                track_id,
                audition_pitch,
                audition_held,
                audition_until,
            );
        }
        return pointer.is_some_and(|pos| keys.contains(pos));
    }

    let Some(pointer) = pointer else {
        if *audition_held {
            clear_audition(
                engine,
                track_id,
                audition_pitch,
                audition_held,
                audition_until,
            );
        }
        return false;
    };

    if !keys.contains(pointer) {
        // Leaving the keyboard while held from keys: release. Drag holds are owned by handle_pointer.
        return false;
    }

    let Some(pitch) = hit_test_key(keys, grid, pointer, metrics) else {
        return true;
    };

    hold_audition_pitch(
        engine,
        track_id,
        audition_pitch,
        audition_held,
        audition_until,
        pitch,
    );
    true
}

fn hit_test_key(keys: Rect, grid: Rect, pointer: Pos2, metrics: ViewMetrics) -> Option<u8> {
    let black_width = TIMELINE_GUTTER_WIDTH * BLACK_KEY_WIDTH_RATIO;

    // Black keys first so narrow keys win over the white key underneath.
    for pitch in MIN_PITCH..=MAX_PITCH {
        if !is_black_key(pitch) {
            continue;
        }
        let y = pitch_to_y(grid, pitch, metrics);
        let key_rect = Rect::from_min_max(
            Pos2::new(keys.left(), y + 1.0),
            Pos2::new(keys.left() + black_width, y + metrics.key_height - 1.0),
        );
        if key_rect.contains(pointer) {
            return Some(pitch);
        }
    }

    if pointer.x < keys.left() || pointer.x > keys.right() {
        return None;
    }

    Some(y_to_pitch(grid, pointer.y, metrics))
}

fn draw_notes(
    painter: &egui::Painter,
    rect: Rect,
    metrics: ViewMetrics,
    notes: &[Note],
    selected_ids: &HashSet<u64>,
    playhead_beats: f32,
    playing: bool,
    theme: &ThemeColors,
) {
    for note in notes {
        let note_rect = note_rect(rect, note, metrics);
        let is_selected = selected_ids.contains(&note.id);
        let is_active =
            playing && playhead_beats >= 0.0 && note.contains_beat(playhead_beats);

        let fill = if is_selected {
            theme.note_fill_selected
        } else {
            theme.note_fill
        };
        let (stroke_width, stroke_color) = if is_active {
            (2.0_f32, theme.note_stroke_active)
        } else if is_selected {
            (1.0_f32, theme.note_stroke_selected)
        } else {
            (1.0_f32, theme.note_stroke)
        };

        painter.rect(
            note_rect,
            3.0,
            fill,
            egui::Stroke::new(stroke_width, stroke_color),
            egui::StrokeKind::Inside,
        );

        let velocity_height = (note.velocity as f32 / 127.0) * (note_rect.height() - 4.0);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(
                    note_rect.left() + 2.0,
                    note_rect.bottom() - velocity_height - 1.0,
                ),
                Pos2::new(note_rect.right() - 2.0, note_rect.bottom() - 1.0),
            ),
            1.0,
            theme.note_velocity,
        );
    }
}

fn draw_marquee(painter: &egui::Painter, selection: Rect, theme: &ThemeColors) {
    painter.rect(
        selection,
        0.0,
        theme.marquee_fill,
        egui::Stroke::new(1.0_f32, theme.marquee_stroke),
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
    let Some(primary) = drag.originals.iter().find(|note| note.id == drag.note_id) else {
        return;
    };

    let raw_delta_beats = current_beats - drag.pointer_start_beats;
    let mut snapped_delta_beats =
        Project::snap_beats(primary.start_beats + raw_delta_beats).max(0.0) - primary.start_beats;

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
        if let Some(note) = project
            .clip_mut(clip_id)
            .and_then(|clip| clip.note_mut(original.id))
        {
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

fn apply_resize_drag(drag: &ActiveDrag, project: &mut Project, clip_id: u64, current_beats: f32) {
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
    history: &mut EditHistory,
    clip_id: u64,
    default_duration_beats: &mut f32,
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    if let Some(drag) = active_drag.take() {
        history.commit(project);
        if matches!(drag.mode, DragMode::ResizeStart | DragMode::ResizeEnd) {
            if let Some(note) = project
                .clip(clip_id)
                .and_then(|clip| clip.note(drag.note_id))
            {
                *default_duration_beats = note.duration_beats;
            }
        }
        if matches!(drag.mode, DragMode::Move) && *audition_held {
            clear_audition(
                engine,
                track_id,
                audition_pitch,
                audition_held,
                audition_until,
            );
        }
    }
}

fn audition_primary_drag_pitch(
    drag: &ActiveDrag,
    project: &Project,
    clip_id: u64,
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    if !matches!(drag.mode, DragMode::Move) {
        return;
    }
    let Some(pitch) = project
        .clip(clip_id)
        .and_then(|clip| clip.note(drag.note_id))
        .map(|note| note.pitch)
    else {
        return;
    };
    hold_audition_pitch(
        engine,
        track_id,
        audition_pitch,
        audition_held,
        audition_until,
        pitch,
    );
}

fn handle_pointer(
    response: &Response,
    ruler: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    clip_id: u64,
    project: &mut Project,
    history: &mut EditHistory,
    engine: &mut dyn DawEngine,
    clip_start_beats: f32,
    track_id: u64,
    selected_note_ids: &mut HashSet<u64>,
    active_drag: &mut Option<ActiveDrag>,
    marquee: &mut Option<MarqueeDrag>,
    default_duration_beats: &mut f32,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    let full = ruler.union(grid);
    let timeline = metrics.timeline();
    let now = response.ctx.input(|input| input.time);
    let primary_down = response
        .ctx
        .input(|input| input.pointer.button_down(egui::PointerButton::Primary));

    let clip_notes: Vec<Note> = project
        .clip(clip_id)
        .map(|clip| clip.notes.clone())
        .unwrap_or_default();

    update_resize_hover_cursor(response, grid, &clip_notes, metrics);

    // End note/marquee drags even when the pointer left the grid or the sense
    // area (otherwise the marquee rect stays painted forever).
    let end_drag = response.drag_stopped()
        || (!primary_down && (active_drag.is_some() || marquee.is_some()));
    if end_drag {
        finish_active_drag(
            active_drag,
            project,
            history,
            clip_id,
            default_duration_beats,
            engine,
            track_id,
            audition_pitch,
            audition_held,
            audition_until,
        );
        *marquee = None;
    }

    let Some(pointer) = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos())
        .or_else(|| response.ctx.pointer_interact_pos())
    else {
        return;
    };

    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    if let Some(drag) = active_drag.clone() {
        if primary_down && (response.dragged() || response.drag_started()) {
            let current_beats = x_to_beat(grid, pointer.x, timeline);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            match drag.mode {
                DragMode::Move => {
                    apply_move_drag(&drag, project, clip_id, current_beats, current_pitch);
                    audition_primary_drag_pitch(
                        &drag,
                        project,
                        clip_id,
                        engine,
                        track_id,
                        audition_pitch,
                        audition_held,
                        audition_until,
                    );
                }
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    response
                        .ctx
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    apply_resize_drag(&drag, project, clip_id, current_beats);
                }
            }
        }
        return;
    }

    // Keep marquee alive / updating even outside the grid bounds.
    if let Some(active_marquee) = marquee.as_mut() {
        if primary_down {
            active_marquee.current = pointer;
            *selected_note_ids =
                select_notes_in_rect(grid, &clip_notes, active_marquee.rect(), metrics);
        }
        return;
    }

    if !full.contains(pointer) && !full.contains(press_pos) {
        return;
    }

    let shift_held = response.ctx.input(|input| input.modifiers.shift);

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if grid.contains(pointer) {
            if let Some(note) = hit_test_note(grid, &clip_notes, pointer, metrics) {
                let note_id = note.id;
                let before = project.clone();
                if let Some(clip) = project.clip_mut(clip_id) {
                    clip.remove_note(note_id);
                    history.push_before(before);
                }
                selected_note_ids.remove(&note_id);
            } else if is_timeline_pointer(grid, pointer) {
                crate::ui::timeline::seek_from_pointer(
                    grid,
                    pointer,
                    timeline,
                    engine,
                    clip_start_beats,
                );
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && !shift_held
        && is_timeline_pointer(grid, pointer)
    {
        if let Some(note) = hit_test_note(grid, &clip_notes, pointer, metrics) {
            set_single_selection(selected_note_ids, note.id);
        } else {
            let pitch = y_to_pitch(grid, pointer.y, metrics);
            let start = Project::snap_beats(x_to_beat(grid, pointer.x, timeline).max(0.0));
            let before = project.clone();
            if let Some(note) =
                project.add_note_to_clip(clip_id, pitch, start, *default_duration_beats)
            {
                history.push_before(before);
                set_single_selection(selected_note_ids, note.id);
                preview_pitch_briefly(
                    engine,
                    track_id,
                    audition_pitch,
                    audition_held,
                    audition_until,
                    note.pitch,
                    now,
                );
            }
        }
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(grid, press_pos)
    {
        if let Some(note) = hit_test_note(grid, &clip_notes, press_pos, metrics).cloned() {
            *marquee = None;

            let note_bounds = note_rect(grid, &note, metrics);
            let mode = resize_drag_mode(note_bounds, press_pos.x).unwrap_or(DragMode::Move);

            let already_selected = selected_note_ids.contains(&note.id);
            if !already_selected {
                set_single_selection(selected_note_ids, note.id);
            }

            // Snapshot before Shift-duplicate so one undo covers dup+move.
            history.begin(project);

            // Shift+Move: leave originals, drag duplicates (same as playlist clips).
            let mut primary_id = note.id;
            if matches!(mode, DragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected_note_ids.iter().copied().collect();
                let new_ids = project.duplicate_notes_in_clip(clip_id, &source_ids, 0.0, 0);
                if let Some(mapped_primary) = source_ids
                    .iter()
                    .position(|id| *id == note.id)
                    .and_then(|index| new_ids.get(index).copied())
                {
                    primary_id = mapped_primary;
                } else if let Some(first) = new_ids.first().copied() {
                    primary_id = first;
                }
                selected_note_ids.clear();
                selected_note_ids.extend(new_ids);
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
                DragMode::ResizeStart | DragMode::ResizeEnd => project
                    .clip(clip_id)
                    .and_then(|clip| clip.note(primary_id).copied())
                    .into_iter()
                    .collect(),
            };

            *active_drag = Some(ActiveDrag {
                note_id: primary_id,
                mode,
                pointer_start_beats: x_to_beat(grid, press_pos.x, timeline),
                pointer_start_pitch: y_to_pitch(grid, press_pos.y, metrics) as i32,
                originals,
            });

            let current_beats = x_to_beat(grid, pointer.x, timeline);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            if let Some(drag) = active_drag.clone() {
                match drag.mode {
                    DragMode::Move => {
                        apply_move_drag(&drag, project, clip_id, current_beats, current_pitch);
                        audition_primary_drag_pitch(
                            &drag,
                            project,
                            clip_id,
                            engine,
                            track_id,
                            audition_pitch,
                            audition_held,
                            audition_until,
                        );
                    }
                    DragMode::ResizeStart | DragMode::ResizeEnd => {
                        response
                            .ctx
                            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        apply_resize_drag(&drag, project, clip_id, current_beats);
                    }
                }
            }
        } else if is_timeline_pointer(grid, press_pos) {
            *active_drag = None;
            selected_note_ids.clear();
            *marquee = Some(MarqueeDrag {
                start: press_pos,
                current: pointer,
            });
            *selected_note_ids =
                select_notes_in_rect(grid, &clip_notes, Rect::from_two_pos(press_pos, pointer), metrics);
        }
    }
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}
