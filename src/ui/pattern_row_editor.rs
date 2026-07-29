//! Pattern row editor (Phase D2): slim piano-roll editor opened from the
//! pattern rack for melodic drafting. Step grids live inline on the rack.

use std::collections::HashSet;

use egui::{Align, Layout, Pos2, Rect, Response, RichText, Sense, Ui, UiBuilder, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    EditHistory, Note, PatternRowMode, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH,
    SNAP_BEATS,
};
use crate::ui::piano_roll::{
    beat_grid_rect, clear_audition, draw_grid, draw_keyboard, draw_marquee, draw_notes,
    handle_keyboard_audition, hit_test_note, hold_audition_pitch, note_rect, pitch_name,
    preview_pitch_briefly, resize_drag_mode, select_notes_in_rect, set_single_selection,
    tick_timed_audition, update_resize_hover_cursor, y_to_pitch, ActiveDrag, DragMode,
    MarqueeDrag, ViewMetrics, DEFAULT_KEY_HEIGHT, KEY_COLUMN_WIDTH, MAX_KEY_HEIGHT,
    MIN_KEY_HEIGHT,
};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_piano_roll_wheel_controls, daw_editor_scroll_area, draw_playhead, draw_ruler,
    is_timeline_pointer, with_solid_scrollbars, x_to_beat, DEFAULT_BEAT_WIDTH, MAX_BEAT_WIDTH,
    MIN_BEAT_WIDTH, RULER_HEIGHT,
};

/// Same zoom-out floor as the piano roll: the block fills at least this
/// fraction of the viewport, with symmetric empty margin beyond it.
const MIN_VIEW_FILL: f32 = 0.90;
const MAX_ZOOM_SPAN: f32 = 4.0;
const HARD_MAX_BEAT_WIDTH: f32 = 1600.0;
const EDGE_SNAP_PX: f32 = 24.0;

const STEP_GAP: f32 = 3.0;
const STEPS_PER_BEAT: usize = (1.0 / SNAP_BEATS) as usize;

pub enum PatternRowEditorAction {
    None,
    /// Back to the pattern rack (Escape or header button).
    Close,
}

#[derive(Debug, Clone)]
struct MelodyState {
    selected_note_ids: HashSet<u64>,
    active_drag: Option<ActiveDrag>,
    marquee: Option<MarqueeDrag>,
    drag_moved: bool,
    audition_pitch: Option<u8>,
    audition_held: bool,
    audition_until: Option<f64>,
    default_duration_beats: f32,
    beat_width: f32,
    key_height: f32,
    scroll_offset: Vec2,
    grid_view_w: f32,
    pending_fit: bool,
}

impl Default for MelodyState {
    fn default() -> Self {
        Self {
            selected_note_ids: HashSet::new(),
            active_drag: None,
            marquee: None,
            drag_moved: false,
            audition_pitch: None,
            audition_held: false,
            audition_until: None,
            default_duration_beats: DEFAULT_NOTE_DURATION_BEATS,
            beat_width: DEFAULT_BEAT_WIDTH,
            key_height: DEFAULT_KEY_HEIGHT,
            scroll_offset: Vec2::ZERO,
            grid_view_w: 0.0,
            pending_fit: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct PatternRowEditorUi {
    /// (block_id, track_id) shown last frame; a change resets local view state.
    current: Option<(u64, u64)>,
    melody: MelodyState,
    dragging_playhead: bool,
}

impl PatternRowEditorUi {
    pub fn selected_note_ids(&self) -> &HashSet<u64> {
        &self.melody.selected_note_ids
    }

    pub fn set_selection(&mut self, note_ids: impl IntoIterator<Item = u64>) {
        self.melody.selected_note_ids.clear();
        self.melody.selected_note_ids.extend(note_ids);
    }

    pub fn clear_selection(&mut self) {
        self.melody.selected_note_ids.clear();
    }

    pub fn prune_selection(&mut self, block_id: u64, track_id: u64, project: &Project) {
        let notes = project.pattern_track_notes(block_id, track_id);
        self.melody
            .selected_note_ids
            .retain(|id| notes.iter().any(|note| note.id == *id));
    }

    pub fn select_all(&mut self, block_id: u64, track_id: u64, project: &Project) {
        self.melody.selected_note_ids.clear();
        self.melody.selected_note_ids.extend(
            project
                .pattern_track_notes(block_id, track_id)
                .iter()
                .map(|note| note.id),
        );
    }

    pub fn release_audition(&mut self, engine: &mut dyn DawEngine, track_id: u64) {
        if let Some(pitch) = self.melody.audition_pitch.take() {
            engine.note_off(track_id, pitch);
        }
        self.melody.audition_held = false;
        self.melody.audition_until = None;
    }

    fn reset_for(&mut self, block_id: u64, track_id: u64) {
        self.current = Some((block_id, track_id));
        self.melody = MelodyState::default();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        block_id: u64,
        track_id: u64,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        theme: &ThemeColors,
    ) -> PatternRowEditorAction {
        if self.current != Some((block_id, track_id)) {
            self.reset_for(block_id, track_id);
        }

        let Some(block) = project.pattern_block(block_id).cloned() else {
            return PatternRowEditorAction::Close;
        };
        let track_name = project
            .track(track_id)
            .map(|track| track.name.clone())
            .unwrap_or_else(|| String::from("Track"));

        let mut action = PatternRowEditorAction::None;

        ui.horizontal(|ui| {
            if ui.button("< Rack").clicked() {
                action = PatternRowEditorAction::Close;
            }
            ui.separator();
            ui.heading(format!("{track_name} - {}", block.name));
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "Melody - {:.1} beats",
                    block.length_beats
                ))
                .color(theme.text_muted)
                .small(),
            );
            ui.add_space(16.0);
            if ui.button("Steps").on_hover_text("Back to inline step grid on the rack").clicked() {
                history.push_before(project.clone());
                project.set_pattern_row_mode(block_id, track_id, Some(PatternRowMode::Step));
                action = PatternRowEditorAction::Close;
            }
        });
        ui.add_space(4.0);

        let body = ui.available_rect_before_wrap();
        self.show_melody(ui, body, block_id, track_id, &block, project, engine, history, theme);

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn show_melody(
        &mut self,
        ui: &mut Ui,
        full: Rect,
        block_id: u64,
        track_id: u64,
        block: &crate::model::PatternBlock,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        theme: &ThemeColors,
    ) {
        let state = &mut self.melody;
        let total_beats = block.length_beats.max(1.0);
        let beats_per_bar = project.beats_per_bar;

        let corner = Rect::from_min_max(
            full.min,
            Pos2::new(full.left() + KEY_COLUMN_WIDTH, full.top() + RULER_HEIGHT),
        );
        let ruler_area = Rect::from_min_max(
            Pos2::new(full.left() + KEY_COLUMN_WIDTH, full.top()),
            Pos2::new(full.right(), full.top() + RULER_HEIGHT),
        );
        let keys_area = Rect::from_min_max(
            Pos2::new(full.left(), full.top() + RULER_HEIGHT),
            Pos2::new(full.left() + KEY_COLUMN_WIDTH, full.bottom()),
        );
        let grid_area = Rect::from_min_max(
            Pos2::new(full.left() + KEY_COLUMN_WIDTH, full.top() + RULER_HEIGHT),
            full.max,
        );

        let view_w = if state.grid_view_w > 0.0 {
            state.grid_view_w
        } else {
            grid_area.width()
        };

        let fit_beat_width = (view_w / total_beats).max(0.0);
        let max_beat_width = (view_w * MAX_ZOOM_SPAN / total_beats)
            .max(MAX_BEAT_WIDTH)
            .min(HARD_MAX_BEAT_WIDTH);
        let min_beat_width = (fit_beat_width * MIN_VIEW_FILL)
            .max(MIN_BEAT_WIDTH)
            .min(max_beat_width);
        if state.pending_fit {
            state.beat_width = min_beat_width;
            state.scroll_offset.x = 0.0;
            state.pending_fit = false;
        } else {
            state.beat_width = state.beat_width.clamp(min_beat_width, max_beat_width);
        }

        let did_h_zoom = apply_piano_roll_wheel_controls(
            ui,
            grid_area,
            &mut state.beat_width,
            min_beat_width,
            max_beat_width,
            &mut state.key_height,
            &mut state.scroll_offset,
            MIN_KEY_HEIGHT,
            MAX_KEY_HEIGHT,
        );

        let metrics = ViewMetrics {
            beat_width: state.beat_width,
            key_height: state.key_height,
        };

        let view_beats = (view_w / metrics.beat_width).max(0.0);
        let lead_beats = ((view_beats - total_beats) / 2.0).max(0.0);
        let lead_px = lead_beats * metrics.beat_width;

        let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
        let content_size = Vec2::new(
            (total_beats + 2.0 * lead_beats) * metrics.beat_width,
            pitch_span * metrics.key_height,
        );
        let canvas_size = Vec2::new(
            content_size.x.max(view_w),
            content_size.y.max(grid_area.height()),
        );

        if did_h_zoom {
            let max_scroll_x = (canvas_size.x - view_w).max(0.0);
            if state.scroll_offset.x < EDGE_SNAP_PX {
                state.scroll_offset.x = 0.0;
            } else if state.scroll_offset.x > max_scroll_x - EDGE_SNAP_PX {
                state.scroll_offset.x = max_scroll_x;
            }
        }

        let global_playhead = engine.current_beats();
        let local_playhead = global_playhead - block.start_beats;
        let playhead_visible = local_playhead >= 0.0 && local_playhead <= total_beats;
        let playhead_draw = if playhead_visible { local_playhead } else { -1.0 };

        let notes: Vec<Note> = project.pattern_track_notes(block_id, track_id);

        let scroll = state.scroll_offset;
        let output = with_solid_scrollbars(ui, theme, |ui| {
            let mut grid_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt(("pattern_row_grid", block_id, track_id))
                    .max_rect(grid_area)
                    .layout(Layout::top_down(Align::LEFT)),
            );
            grid_ui.set_clip_rect(grid_area);
            daw_editor_scroll_area(("pattern_row_canvas", block_id, track_id))
                .scroll_offset(scroll)
                .show(&mut grid_ui, |ui| {
                    ui.set_min_size(canvas_size);
                    let (response, painter) =
                        ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    let beat_grid = beat_grid_rect(content, lead_px);
                    let playing = engine.is_playing();

                    draw_grid(
                        &painter,
                        content,
                        lead_px,
                        metrics,
                        total_beats,
                        beats_per_bar,
                        theme,
                    );
                    draw_notes(
                        &painter,
                        beat_grid,
                        metrics,
                        &notes,
                        &state.selected_note_ids,
                        playhead_draw,
                        playing,
                        theme,
                    );
                    if let Some(marquee) = &state.marquee {
                        draw_marquee(&painter, marquee.rect(), theme);
                    }
                    (response, content)
                })
        });

        let (response, content) = output.inner;
        state.scroll_offset = output.state.offset;
        state.grid_view_w = output.inner_rect.width();

        let beat_grid = beat_grid_rect(content, lead_px);
        let keys_grid = Rect::from_min_max(
            Pos2::new(keys_area.left(), content.top()),
            Pos2::new(keys_area.right(), content.bottom()),
        );
        let ruler_ref = Rect::from_min_max(
            Pos2::new(ruler_area.left() - KEY_COLUMN_WIDTH, ruler_area.top()),
            ruler_area.max,
        );

        tick_timed_audition(
            engine,
            track_id,
            &mut state.audition_pitch,
            &mut state.audition_held,
            &mut state.audition_until,
            ui.input(|input| input.time),
        );

        let keys_response = ui.interact(
            keys_area,
            ui.id().with(("pattern_row_keys", block_id, track_id)),
            Sense::click_and_drag(),
        );

        let gesture_active = state.active_drag.is_some() || state.marquee.is_some();
        let keyboard_handled = !gesture_active
            && handle_keyboard_audition(
                &keys_response,
                keys_area,
                keys_grid,
                metrics,
                engine,
                track_id,
                &mut state.audition_pitch,
                &mut state.audition_held,
                &mut state.audition_until,
            );

        if gesture_active || !keyboard_handled {
            handle_melody_pointer(
                &response,
                ruler_ref,
                beat_grid,
                metrics,
                block_id,
                track_id,
                block.length_beats,
                project,
                history,
                engine,
                &mut self.dragging_playhead,
                &mut state.selected_note_ids,
                &mut state.active_drag,
                &mut state.marquee,
                &mut state.drag_moved,
                &mut state.default_duration_beats,
                &mut state.audition_pitch,
                &mut state.audition_held,
                &mut state.audition_until,
            );
        }

        let playing = engine.is_playing();
        draw_keyboard(
            &ui.painter().with_clip_rect(keys_area),
            keys_grid,
            metrics,
            &notes,
            playhead_draw,
            playing,
            state.audition_pitch,
            theme,
        );
        draw_ruler(
            &ui.painter().with_clip_rect(ruler_area),
            ruler_ref,
            beat_grid,
            metrics.timeline(),
            total_beats,
            beats_per_bar,
            theme,
        );
        ui.painter().rect_filled(corner, 0.0, theme.gutter_bg);
        ui.painter().line_segment(
            [corner.right_top(), corner.right_bottom()],
            egui::Stroke::new(1.5_f32, theme.key_divider),
        );
        let playhead_clip = Rect::from_min_max(Pos2::new(grid_area.left(), full.top()), full.max);
        let clip_painter = ui.painter().with_clip_rect(playhead_clip);
        draw_playhead(
            &clip_painter,
            ruler_area,
            beat_grid,
            metrics.timeline(),
            local_playhead,
            playhead_visible,
            theme,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_melody_pointer(
    response: &Response,
    ruler: Rect,
    grid: Rect,
    metrics: ViewMetrics,
    block_id: u64,
    track_id: u64,
    block_length_beats: f32,
    project: &mut Project,
    history: &mut EditHistory,
    engine: &mut dyn DawEngine,
    dragging_playhead: &mut bool,
    selected_note_ids: &mut HashSet<u64>,
    active_drag: &mut Option<ActiveDrag>,
    marquee: &mut Option<MarqueeDrag>,
    drag_moved: &mut bool,
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

    let notes: Vec<Note> = project.pattern_track_notes(block_id, track_id);

    update_resize_hover_cursor(response, grid, &notes, metrics);

    let end_drag = response.drag_stopped()
        || (!primary_down && (active_drag.is_some() || marquee.is_some()));
    if end_drag {
        finish_melody_drag(
            active_drag,
            project,
            history,
            block_id,
            track_id,
            selected_note_ids,
            *drag_moved,
            default_duration_beats,
            engine,
            track_id,
            audition_pitch,
            audition_held,
            audition_until,
        );
        *marquee = None;
        *drag_moved = false;
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

    if response
        .ctx
        .input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        && is_timeline_pointer(grid, press_pos)
        && active_drag.is_none()
        && marquee.is_none()
    {
        if let Some(note) = hit_test_note(grid, &notes, press_pos, metrics) {
            if !selected_note_ids.contains(&note.id) {
                set_single_selection(selected_note_ids, note.id);
            }
            let note_bounds = note_rect(grid, note, metrics);
            if resize_drag_mode(note_bounds, press_pos.x).is_none() {
                hold_audition_pitch(
                    engine,
                    track_id,
                    audition_pitch,
                    audition_held,
                    audition_until,
                    note.pitch,
                );
            }
        }
    }

    if response
        .ctx
        .input(|input| input.pointer.button_released(egui::PointerButton::Primary))
        && active_drag.is_none()
        && *audition_held
    {
        clear_audition(engine, track_id, audition_pitch, audition_held, audition_until);
    }

    if let Some(drag) = active_drag.clone() {
        if primary_down && (response.dragged() || response.drag_started()) {
            *drag_moved = true;
            let snap_horizontal = !response.ctx.input(|input| input.modifiers.alt);
            let current_beats = x_to_beat(grid, pointer.x, timeline);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            match drag.mode {
                DragMode::Move => {
                    apply_melody_move_drag(
                        &drag,
                        project,
                        block_id,
                        track_id,
                        current_beats,
                        current_pitch,
                        snap_horizontal,
                    );
                    audition_melody_drag_pitch(
                        &drag,
                        project,
                        block_id,
                        track_id,
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
                    apply_melody_resize_drag(
                        &drag,
                        project,
                        block_id,
                        track_id,
                        current_beats,
                        snap_horizontal,
                    );
                }
            }
        }
        return;
    }

    if let Some(active_marquee) = marquee.as_mut() {
        if primary_down {
            active_marquee.current = pointer;
            *selected_note_ids = select_notes_in_rect(grid, &notes, active_marquee.rect(), metrics);
        }
        return;
    }

    if !full.contains(pointer) && !full.contains(press_pos) {
        return;
    }

    let shift_held = response.ctx.input(|input| input.modifiers.shift);

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if grid.contains(pointer) {
            if let Some(note) = hit_test_note(grid, &notes, pointer, metrics) {
                let note_id = note.id;
                let before = project.clone();
                project.remove_note_from_pattern_track(block_id, track_id, note_id);
                history.push_before(before);
                selected_note_ids.remove(&note_id);
            } else if is_timeline_pointer(grid, pointer) {
                *dragging_playhead = false;
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && !shift_held
        && is_timeline_pointer(grid, pointer)
    {
        if let Some(note) = hit_test_note(grid, &notes, pointer, metrics) {
            set_single_selection(selected_note_ids, note.id);
        } else {
            let pitch = y_to_pitch(grid, pointer.y, metrics);
            let start = Project::snap_beats(x_to_beat(grid, pointer.x, timeline).max(0.0));
            if start < block_length_beats {
                let before = project.clone();
                if let Some(note) = project.add_note_to_pattern_track(
                    block_id,
                    track_id,
                    pitch,
                    start,
                    *default_duration_beats,
                ) {
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
    }

    if response.drag_started_by(egui::PointerButton::Primary) && is_timeline_pointer(grid, press_pos)
    {
        if let Some(note) = hit_test_note(grid, &notes, press_pos, metrics).cloned() {
            *marquee = None;

            let note_bounds = note_rect(grid, &note, metrics);
            let mode = resize_drag_mode(note_bounds, press_pos.x).unwrap_or(DragMode::Move);

            let already_selected = selected_note_ids.contains(&note.id);
            if !already_selected {
                set_single_selection(selected_note_ids, note.id);
            }

            history.begin(project);

            let mut primary_id = note.id;
            let mut ignore_ids = Vec::new();
            if matches!(mode, DragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected_note_ids.iter().copied().collect();
                ignore_ids = source_ids.clone();
                let new_ids = project.duplicate_notes_in_pattern_track(
                    block_id, track_id, &source_ids, 0.0, 0, true,
                );
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

            let originals: Vec<Note> = project
                .pattern_track_notes(block_id, track_id)
                .into_iter()
                .filter(|n| selected_note_ids.contains(&n.id))
                .collect();

            *active_drag = Some(ActiveDrag {
                note_id: primary_id,
                mode,
                pointer_start_beats: x_to_beat(grid, press_pos.x, timeline),
                pointer_start_pitch: y_to_pitch(grid, press_pos.y, metrics) as i32,
                originals,
                ignore_ids,
            });
            *drag_moved = false;

            let current_beats = x_to_beat(grid, pointer.x, timeline);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            let snap_horizontal = !response.ctx.input(|input| input.modifiers.alt);
            if let Some(drag) = active_drag.clone() {
                match drag.mode {
                    DragMode::Move => {
                        apply_melody_move_drag(
                            &drag,
                            project,
                            block_id,
                            track_id,
                            current_beats,
                            current_pitch,
                            snap_horizontal,
                        );
                        audition_melody_drag_pitch(
                            &drag,
                            project,
                            block_id,
                            track_id,
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
                        apply_melody_resize_drag(
                            &drag,
                            project,
                            block_id,
                            track_id,
                            current_beats,
                            snap_horizontal,
                        );
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
                select_notes_in_rect(grid, &notes, Rect::from_two_pos(press_pos, pointer), metrics);
        }
    }
}

fn apply_melody_move_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    block_id: u64,
    track_id: u64,
    current_beats: f32,
    current_pitch: i32,
    snap_horizontal: bool,
) {
    let Some(primary) = drag.originals.iter().find(|note| note.id == drag.note_id) else {
        return;
    };

    let raw_delta_beats = current_beats - drag.pointer_start_beats;
    let desired_delta_beats = if snap_horizontal {
        Project::snap_beats(primary.start_beats + raw_delta_beats).max(0.0) - primary.start_beats
    } else {
        raw_delta_beats
    };
    let desired_delta_pitch = current_pitch - drag.pointer_start_pitch;

    let (delta_beats, delta_pitch) = project.clamp_pattern_note_move_deltas(
        block_id,
        &drag.originals,
        desired_delta_beats,
        desired_delta_pitch,
    );

    for original in &drag.originals {
        if let Some(note) = project.pattern_track_note_mut(block_id, track_id, original.id) {
            note.start_beats = (original.start_beats + delta_beats).max(0.0);
            note.pitch = Project::clamp_pitch(original.pitch as i32 + delta_pitch);
            note.duration_beats = original.duration_beats;
        }
    }
}

fn apply_melody_resize_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    block_id: u64,
    track_id: u64,
    current_beats: f32,
    snap_horizontal: bool,
) {
    let Some(primary) = drag.originals.iter().find(|note| note.id == drag.note_id) else {
        return;
    };

    let snapped_beats = |beats: f32| {
        if snap_horizontal {
            Project::snap_beats(beats)
        } else {
            beats
        }
    };

    let resizing_ids: Vec<u64> = drag.originals.iter().map(|note| note.id).collect();

    match drag.mode {
        DragMode::ResizeStart => {
            let desired_start = snapped_beats(current_beats.max(0.0));
            let raw_delta = desired_start - primary.start_beats;
            let delta = project.clamp_pattern_note_resize_start_delta(
                block_id,
                track_id,
                &drag.originals,
                raw_delta,
            );
            for original in &drag.originals {
                let end = original.end_beats();
                let bound = project.pattern_note_resize_start_bound(
                    block_id,
                    track_id,
                    original.id,
                    original.pitch,
                    original.start_beats,
                    &resizing_ids,
                );
                let new_start = (original.start_beats + delta)
                    .max(bound)
                    .max(0.0)
                    .min(end - SNAP_BEATS);
                if let Some(note) = project.pattern_track_note_mut(block_id, track_id, original.id) {
                    note.start_beats = new_start;
                    note.duration_beats = (end - new_start).max(SNAP_BEATS);
                    note.pitch = original.pitch;
                }
            }
        }
        DragMode::ResizeEnd => {
            let desired_end = snapped_beats(current_beats.max(0.0));
            let raw_delta = desired_end - primary.end_beats();
            let delta = project.clamp_pattern_note_resize_end_delta(
                block_id,
                track_id,
                &drag.originals,
                raw_delta,
            );
            for original in &drag.originals {
                let bound = project.pattern_note_resize_end_bound(
                    block_id,
                    track_id,
                    original.id,
                    original.pitch,
                    original.end_beats(),
                    &resizing_ids,
                );
                let new_end = (original.end_beats() + delta)
                    .min(bound)
                    .max(original.start_beats + SNAP_BEATS);
                if let Some(note) = project.pattern_track_note_mut(block_id, track_id, original.id) {
                    note.start_beats = original.start_beats;
                    note.duration_beats = (new_end - original.start_beats).max(SNAP_BEATS);
                    note.pitch = original.pitch;
                }
            }
        }
        DragMode::Move => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_melody_drag(
    active_drag: &mut Option<ActiveDrag>,
    project: &mut Project,
    history: &mut EditHistory,
    block_id: u64,
    track_id: u64,
    selected_note_ids: &mut HashSet<u64>,
    drag_moved: bool,
    default_duration_beats: &mut f32,
    engine: &mut dyn DawEngine,
    audition_track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    let Some(drag) = active_drag.take() else {
        return;
    };

    if !drag.ignore_ids.is_empty() && !drag_moved {
        history.abort(project);
        selected_note_ids.clear();
        selected_note_ids.extend(drag.ignore_ids.iter().copied());
        if *audition_held {
            clear_audition(engine, audition_track_id, audition_pitch, audition_held, audition_until);
        }
        return;
    }

    if drag_moved && matches!(drag.mode, DragMode::Move) {
        let moved_ids: Vec<u64> = drag.originals.iter().map(|note| note.id).collect();
        project.resolve_pattern_note_move_overlaps(block_id, track_id, &moved_ids);
    }

    history.commit(project);
    if matches!(drag.mode, DragMode::ResizeStart | DragMode::ResizeEnd) {
        if let Some(note) = project.pattern_track_note(block_id, track_id, drag.note_id) {
            *default_duration_beats = note.duration_beats;
        }
    }
    if matches!(drag.mode, DragMode::Move) && *audition_held {
        clear_audition(engine, audition_track_id, audition_pitch, audition_held, audition_until);
    }
}

#[allow(clippy::too_many_arguments)]
fn audition_melody_drag_pitch(
    drag: &ActiveDrag,
    project: &Project,
    block_id: u64,
    track_id: u64,
    engine: &mut dyn DawEngine,
    audition_track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    if !matches!(drag.mode, DragMode::Move) {
        return;
    }
    let Some(pitch) = project
        .pattern_track_note(block_id, track_id, drag.note_id)
        .map(|note| note.pitch)
    else {
        return;
    };
    hold_audition_pitch(
        engine,
        audition_track_id,
        audition_pitch,
        audition_held,
        audition_until,
        pitch,
    );
}

/// Compact FL-style step strip for the pattern rack: paint cells and handle
/// click/drag toggle. `step_paint` tracks in-progress drag-paint as
/// `(track_id, value)` for the row being painted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_inline_step_strip(
    ui: &mut Ui,
    rect: Rect,
    block_id: u64,
    track_id: u64,
    block: &crate::model::PatternBlock,
    project: &mut Project,
    engine: &dyn DawEngine,
    history: &mut EditHistory,
    step_paint: &mut Option<(u64, bool)>,
    theme: &ThemeColors,
) {
    let step_count = project.pattern_step_count(block_id).max(1);
    let beats_per_bar = project.beats_per_bar.max(1.0);
    let steps_per_bar = (beats_per_bar / SNAP_BEATS).round().max(1.0) as usize;

    let response = ui.interact(
        rect,
        ui.id().with((block_id, track_id, "rack_steps")),
        Sense::click_and_drag(),
    );

    let cell_w = ((rect.width() - STEP_GAP * (step_count as f32 - 1.0).max(0.0))
        / step_count as f32)
        .max(3.0);

    let local_beat = engine.current_beats() - block.start_beats;
    let playing = engine.is_playing();
    let active_step_index = if playing && local_beat >= 0.0 && local_beat < block.length_beats {
        Some((local_beat / SNAP_BEATS).floor() as usize)
    } else {
        None
    };

    let painter = ui.painter_at(rect);
    for i in 0..step_count {
        let x0 = rect.left() + i as f32 * (cell_w + STEP_GAP);
        let cell_rect = Rect::from_min_max(
            Pos2::new(x0, rect.top()),
            Pos2::new(x0 + cell_w, rect.bottom()),
        );
        let beat_group = i / STEPS_PER_BEAT;
        let bar_group = if steps_per_bar > 0 {
            beat_group / (steps_per_bar / STEPS_PER_BEAT).max(1)
        } else {
            0
        };
        let active = project.pattern_step_active(block_id, track_id, i);
        let bg = if active {
            theme.step_cell_active
        } else if bar_group % 2 == 1 {
            theme.step_cell_bg_accent
        } else {
            theme.step_cell_bg
        };
        painter.rect_filled(cell_rect, 2.0, bg);
        painter.rect_stroke(
            cell_rect,
            2.0,
            egui::Stroke::new(1.0_f32, theme.step_cell_border),
            egui::StrokeKind::Inside,
        );
        if active_step_index == Some(i) {
            painter.rect_filled(cell_rect, 2.0, theme.step_cell_playhead);
        }
    }

    let hovered_step = |pos: Pos2| -> Option<usize> {
        if pos.y < rect.top() || pos.y > rect.bottom() || pos.x < rect.left() {
            return None;
        }
        let idx = ((pos.x - rect.left()) / (cell_w + STEP_GAP)) as usize;
        if idx < step_count {
            Some(idx)
        } else {
            None
        }
    };

    let painting_this_row = step_paint.is_some_and(|(id, _)| id == track_id);

    if response.drag_started() {
        if let Some(idx) = response.interact_pointer_pos().and_then(hovered_step) {
            history.begin(project);
            let next = !project.pattern_step_active(block_id, track_id, idx);
            *step_paint = Some((track_id, next));
            project.set_pattern_step(block_id, track_id, idx, next);
        }
    } else if response.dragged() {
        if let (Some(pos), Some((paint_track, value))) = (response.interact_pointer_pos(), *step_paint)
        {
            if paint_track == track_id {
                if let Some(idx) = hovered_step(pos) {
                    project.set_pattern_step(block_id, track_id, idx, value);
                }
            }
        }
    } else if response.clicked() {
        if let Some(idx) = response.interact_pointer_pos().and_then(hovered_step) {
            history.push_before(project.clone());
            let next = !project.pattern_step_active(block_id, track_id, idx);
            project.set_pattern_step(block_id, track_id, idx, next);
        }
    }
    if response.drag_stopped() && painting_this_row {
        history.commit(project);
        *step_paint = None;
    }
}
