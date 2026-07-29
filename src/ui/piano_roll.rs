use std::collections::HashSet;

use egui::{Align, Layout, Pos2, Rect, Response, Sense, Ui, UiBuilder, Vec2};

use crate::engine::DawEngine;
use crate::model::{
    EditHistory, Note, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_piano_roll_wheel_controls, daw_editor_scroll_area, draw_playhead, draw_playback_anchor,
    draw_ruler, handle_timeline_playhead_pointer, is_timeline_pointer, timeline_x,
    with_solid_scrollbars,
    x_to_beat, TimelineMetrics, DEFAULT_BEAT_WIDTH, MAX_BEAT_WIDTH, MIN_BEAT_WIDTH, RULER_HEIGHT,
    TIMELINE_GUTTER_WIDTH,
};

/// Fixed width of the pinned piano-key column (left of the scrolling grid).
/// `pub(crate)`: shared with `pattern_row_editor` (slim melody-row piano roll).
pub(crate) const KEY_COLUMN_WIDTH: f32 = TIMELINE_GUTTER_WIDTH;

pub(crate) const BLACK_KEY_WIDTH_RATIO: f32 = 0.62;
pub(crate) const RESIZE_HANDLE_PX: f32 = 12.0;

/// Fraction of the viewport width the clip fills at maximum zoom-out. The rest
/// becomes symmetric empty "outside" margin on both sides of the clip.
const MIN_CLIP_VIEW_FILL: f32 = 0.90;
/// Max zoom-in for a clip expressed as how many viewport widths the whole clip may
/// span. Short clips get a proportionally larger px/beat ceiling so notes can be
/// enlarged; long clips fall back to the flat `MAX_BEAT_WIDTH`.
const MAX_CLIP_ZOOM_SPAN: f32 = 4.0;
/// Absolute upper bound on beat width so pathologically short clips / huge monitors
/// don't produce an absurd zoom-in ceiling.
const HARD_MAX_BEAT_WIDTH: f32 = 1600.0;
/// After a Ctrl+wheel zoom, if the resulting horizontal scroll lands within this
/// many pixels of either extreme, snap it flush to the edge. Cursor-anchored zoom
/// otherwise leaves a small residual offset when the pointer is near an edge,
/// which reads as the scrollbar "detaching" from the end.
const EDGE_SNAP_PX: f32 = 24.0;
pub(crate) const DEFAULT_KEY_HEIGHT: f32 = 18.0;
pub(crate) const MIN_KEY_HEIGHT: f32 = 8.0;
pub(crate) const MAX_KEY_HEIGHT: f32 = 48.0;

/// Short preview when clicking to create a note (seconds).
pub(crate) const NOTE_CREATE_PREVIEW_SECS: f64 = 0.18;

/// Pitch shown near the top of the viewport on first open (C6).
const DEFAULT_TOP_PITCH: u8 = 84;

/// `pub(crate)`: shared with `pattern_row_editor` (slim melody-row piano roll)
/// so both editors size notes/keys identically.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewMetrics {
    pub(crate) beat_width: f32,
    pub(crate) key_height: f32,
}

impl ViewMetrics {
    pub(crate) fn timeline(&self) -> TimelineMetrics {
        TimelineMetrics {
            beat_width: self.beat_width,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

/// `pub(crate)`: generic over any note list (no clip reference), shared with
/// `pattern_row_editor`'s melody mode.
#[derive(Debug, Clone)]
pub(crate) struct ActiveDrag {
    pub(crate) note_id: u64,
    pub(crate) mode: DragMode,
    pub(crate) pointer_start_beats: f32,
    pub(crate) pointer_start_pitch: i32,
    pub(crate) originals: Vec<Note>,
    /// Notes movers may overlap during this drag (Shift+drag duplicate sources).
    pub(crate) ignore_ids: Vec<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct MarqueeDrag {
    pub(crate) start: Pos2,
    pub(crate) current: Pos2,
}

impl MarqueeDrag {
    pub(crate) fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.current)
    }
}

pub struct PianoRollUi {
    selected_note_ids: HashSet<u64>,
    active_drag: Option<ActiveDrag>,
    marquee: Option<MarqueeDrag>,
    dragging_playhead: bool,
    /// True if pointer moved enough during an active note drag to count as a drag.
    drag_moved: bool,
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
    /// Actual horizontal viewport width of the grid scroll area from the previous
    /// frame (excludes the always-visible vertical scrollbar). Zoom-out fit and
    /// edge-snap math use this real width so the scrollbar reaches its extremes.
    grid_view_w: f32,
    /// When set, the next `show` applies the zoom-out floor (whole clip in view
    /// with breathing room) and resets horizontal scroll. Cleared after apply.
    pending_fit_horizontal: bool,
}

impl PianoRollUi {
    pub fn selected_note_ids(&self) -> &HashSet<u64> {
        &self.selected_note_ids
    }

    pub fn clear_selection(&mut self) {
        self.selected_note_ids.clear();
    }

    /// Fit the next paint to the clip zoom-out floor (used when opening from playlist).
    pub fn request_fit_horizontal(&mut self) {
        self.pending_fit_horizontal = true;
    }

    pub fn set_selection(&mut self, note_ids: impl IntoIterator<Item = u64>) {
        self.selected_note_ids.clear();
        self.selected_note_ids.extend(note_ids);
    }

    pub fn select_all_in_clip(&mut self, clip_id: u64, project: &Project) {
        self.selected_note_ids.clear();
        if let Some(clip) = project.midi_clip(clip_id) {
            self.selected_note_ids
                .extend(clip.notes.iter().map(|note| note.id));
        }
    }

    /// True when the clip has at least one note and every note in it is selected.
    pub fn all_notes_selected_in_clip(&self, clip_id: u64, project: &Project) -> bool {
        let Some(clip) = project.midi_clip(clip_id) else {
            return false;
        };
        !clip.notes.is_empty()
            && clip
                .notes
                .iter()
                .all(|note| self.selected_note_ids.contains(&note.id))
    }

    pub fn prune_selection(&mut self, clip_id: u64, project: &Project) {
        let Some(clip) = project.midi_clip(clip_id) else {
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
            drag_moved: false,
            audition_pitch: None,
            audition_track_id: 0,
            audition_held: false,
            audition_until: None,
            default_duration_beats: DEFAULT_NOTE_DURATION_BEATS,
            beat_width: DEFAULT_BEAT_WIDTH,
            key_height: DEFAULT_KEY_HEIGHT,
            scroll_offset: Vec2::new(0.0, initial_scroll_y),
            grid_view_w: 0.0,
            pending_fit_horizontal: false,
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
            let Some(clip) = project.midi_clip(clip_id) else {
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
        let full = ui.available_rect_before_wrap();
        // Side-by-side layout: a fixed key column + corner on the left, the ruler
        // across the top-right, and the scrolling note grid filling the rest. The
        // keyboard/ruler are separate widgets beside the grid, not overlays baked
        // into the scroll content, so beat 0 can never slide under the keyboard.
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

        // Horizontal viewport actually available to scroll content: the grid area
        // minus the always-visible vertical scrollbar (measured last frame). Using
        // the real inner width keeps the fit floor and edge-snap in sync with what
        // egui scrolls, so the scrollbar reaches both extremes exactly.
        let view_w = if self.grid_view_w > 0.0 {
            self.grid_view_w
        } else {
            grid_area.width()
        };

        // Zoom-out floor: allow the clip to shrink until it fills only
        // MIN_CLIP_VIEW_FILL of the grid width, leaving symmetric empty "outside"
        // margins around it (Bitwig-style breathing room). Below this the view
        // stops zooming out, so it never feels arbitrarily far away.
        let fit_beat_width = (view_w / total_beats).max(0.0);
        // Zoom-in ceiling scales with clip length: a short clip can be blown up
        // until it spans ~MAX_CLIP_ZOOM_SPAN viewport widths, so small clips aren't
        // stuck at the flat MAX_BEAT_WIDTH. Long clips keep the flat ceiling.
        let max_beat_width = (view_w * MAX_CLIP_ZOOM_SPAN / total_beats)
            .max(MAX_BEAT_WIDTH)
            .min(HARD_MAX_BEAT_WIDTH);
        let min_beat_width = (fit_beat_width * MIN_CLIP_VIEW_FILL)
            .max(MIN_BEAT_WIDTH)
            .min(max_beat_width);
        if self.pending_fit_horizontal {
            // Opening from playlist/devices: land at the zoom-out floor so the
            // whole clip is visible with Bitwig-style breathing room.
            self.beat_width = min_beat_width;
            self.scroll_offset.x = 0.0;
            self.pending_fit_horizontal = false;
        } else {
            // Re-fit on window resize / clip length change, not just on wheel.
            self.beat_width = self.beat_width.clamp(min_beat_width, max_beat_width);
        }

        let did_h_zoom = apply_piano_roll_wheel_controls(
            ui,
            grid_area,
            &mut self.beat_width,
            min_beat_width,
            max_beat_width,
            &mut self.key_height,
            &mut self.scroll_offset,
            MIN_KEY_HEIGHT,
            MAX_KEY_HEIGHT,
        );

        let metrics = ViewMetrics {
            beat_width: self.beat_width,
            key_height: self.key_height,
        };

        // Symmetric "outside" padding: when the clip is narrower than the grid
        // viewport, split the slack into equal lead-in / lead-out margins so the
        // clip sits centered with empty grid on both sides. When it is wider than
        // the viewport there is no slack (lead 0) and scrolling/zoom are normal.
        let view_beats = (view_w / metrics.beat_width).max(0.0);
        let lead_beats = ((view_beats - total_beats) / 2.0).max(0.0);
        let lead_px = lead_beats * metrics.beat_width;

        let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
        // Scroll content is the pure timeline: no key gutter, no ruler strip.
        let content_size = Vec2::new(
            (total_beats + 2.0 * lead_beats) * metrics.beat_width,
            pitch_span * metrics.key_height,
        );
        let canvas_size = Vec2::new(
            content_size.x.max(view_w),
            content_size.y.max(grid_area.height()),
        );

        // Sticky edges: pure cursor-anchored zoom leaves a few px of residual
        // horizontal scroll when the pointer sits near an edge, so the scrollbar
        // never quite reaches the end and beat 0 (or the clip tail) stays just
        // off-screen. Snap to the extreme when we land within EDGE_SNAP_PX of it,
        // but only right after a zoom so manual scrolling is left untouched.
        if did_h_zoom {
            let max_scroll_x = (canvas_size.x - view_w).max(0.0);
            if self.scroll_offset.x < EDGE_SNAP_PX {
                self.scroll_offset.x = 0.0;
            } else if self.scroll_offset.x > max_scroll_x - EDGE_SNAP_PX {
                self.scroll_offset.x = max_scroll_x;
            }
        }

        let global_playhead = engine.current_beats();
        let local_playhead = global_playhead - clip_start;
        let playhead_visible = local_playhead >= 0.0 && local_playhead <= total_beats;
        let playhead_draw = if playhead_visible { local_playhead } else { -1.0 };
        let local_anchor = engine.playback_anchor_beats() - clip_start;
        let anchor_visible = local_anchor >= 0.0 && local_anchor <= total_beats;

        let clip_notes: Vec<Note> = project
            .midi_clip(clip_id)
            .map(|clip| clip.notes.clone())
            .unwrap_or_default();

        let scroll = self.scroll_offset;

        // ---- Scrolling note grid (keyboard column + ruler are drawn beside it) ----
        let output = with_solid_scrollbars(ui, theme, |ui| {
            let mut grid_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("piano_roll_grid")
                    .max_rect(grid_area)
                    .layout(Layout::top_down(Align::LEFT)),
            );
            grid_ui.set_clip_rect(grid_area);
            daw_editor_scroll_area("piano_roll_canvas")
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
                        &clip_notes,
                        &self.selected_note_ids,
                        playhead_draw,
                        playing,
                        theme,
                    );
                    if let Some(marquee) = &self.marquee {
                        draw_marquee(&painter, marquee.rect(), theme);
                    }
                    (response, content)
                })
        });

        let (response, content) = output.inner;
        self.scroll_offset = output.state.offset;
        // Cache the true inner viewport width (excludes the vertical scrollbar) for
        // next frame's fit-floor and edge-snap math.
        self.grid_view_w = output.inner_rect.width();

        let beat_grid = beat_grid_rect(content, lead_px);
        // Keyboard rows share the grid's vertical origin/scroll.
        let keys_grid = Rect::from_min_max(
            Pos2::new(keys_area.left(), content.top()),
            Pos2::new(keys_area.right(), content.bottom()),
        );
        // Shift the ruler reference left by the key column so the shared
        // ruler/playhead helpers (which add a gutter internally) resolve beat 0
        // to the grid's left edge in this split layout.
        let ruler_ref = Rect::from_min_max(
            Pos2::new(ruler_area.left() - KEY_COLUMN_WIDTH, ruler_area.top()),
            ruler_area.max,
        );

        tick_timed_audition(
            engine,
            track_id,
            &mut self.audition_pitch,
            &mut self.audition_held,
            &mut self.audition_until,
            ui.input(|input| input.time),
        );

        let keys_response = ui.interact(
            keys_area,
            ui.id().with("piano_roll_keys"),
            Sense::click_and_drag(),
        );
        let ruler_response = ui.interact(
            ruler_area,
            ui.id().with("piano_roll_ruler"),
            Sense::click_and_drag(),
        );

        // In-flight note/marquee drags keep pointer ownership; keyboard and ruler
        // only win when idle.
        let gesture_active = self.active_drag.is_some() || self.marquee.is_some();
        let keyboard_handled = !gesture_active
            && handle_keyboard_audition(
                &keys_response,
                keys_area,
                keys_grid,
                metrics,
                engine,
                track_id,
                &mut self.audition_pitch,
                &mut self.audition_held,
                &mut self.audition_until,
            );

        // Ruler and grid are separate interact regions (side-by-side layout), so
        // playhead scrubbing must consult both. Playlist uses one shared response.
        // Plain secondary clicks on the grid delete a note in handle_pointer;
        // Shift+secondary click/drag seeks via the shared helper.
        let playhead_handled = !gesture_active && !keyboard_handled && {
            if self.dragging_playhead {
                // Continue on whichever region still owns the pointer.
                let active = if response.interact_pointer_pos().is_some() {
                    &response
                } else {
                    &ruler_response
                };
                handle_timeline_playhead_pointer(
                    active,
                    ruler_ref,
                    beat_grid,
                    metrics.timeline(),
                    engine,
                    &mut self.dragging_playhead,
                    clip_start,
                )
            } else {
                handle_timeline_playhead_pointer(
                    &ruler_response,
                    ruler_ref,
                    beat_grid,
                    metrics.timeline(),
                    engine,
                    &mut self.dragging_playhead,
                    clip_start,
                ) || handle_timeline_playhead_pointer(
                    &response,
                    ruler_ref,
                    beat_grid,
                    metrics.timeline(),
                    engine,
                    &mut self.dragging_playhead,
                    clip_start,
                )
            }
        };

        if gesture_active || (!keyboard_handled && !playhead_handled) {
            handle_pointer(
                &response,
                ruler_ref,
                beat_grid,
                metrics,
                clip_id,
                project,
                history,
                engine,
                track_id,
                &mut self.selected_note_ids,
                &mut self.active_drag,
                &mut self.marquee,
                &mut self.drag_moved,
                &mut self.default_duration_beats,
                &mut self.audition_pitch,
                &mut self.audition_held,
                &mut self.audition_until,
            );
        }

        // ---- Chrome beside the grid: keyboard, ruler, corner, playhead ----
        let playing = engine.is_playing();
        draw_keyboard(
            &ui.painter().with_clip_rect(keys_area),
            keys_grid,
            metrics,
            &clip_notes,
            playhead_draw,
            playing,
            self.audition_pitch,
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
        // Corner box above the key column, with a divider matching the keyboard.
        ui.painter().rect_filled(corner, 0.0, theme.gutter_bg);
        ui.painter().line_segment(
            [corner.right_top(), corner.right_bottom()],
            egui::Stroke::new(1.5_f32, theme.key_divider),
        );
        // Playhead spans ruler + grid, clipped to the right of the key column so
        // it never draws over the keyboard or corner.
        let playhead_clip = Rect::from_min_max(Pos2::new(grid_area.left(), full.top()), full.max);
        let clip_painter = ui.painter().with_clip_rect(playhead_clip);
        draw_playback_anchor(
            &clip_painter,
            ruler_area,
            beat_grid,
            metrics.timeline(),
            local_anchor,
            local_playhead,
            anchor_visible,
            theme,
        );
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

/// Beat-mapping rect for the grid scroll content. The shared `timeline_x` /
/// `x_to_beat` helpers add `TIMELINE_GUTTER_WIDTH` to `rect.left()`, so shifting
/// left by the key column makes beat 0 resolve to `content.left() + lead_px`
/// (the lead-in margin) while keeping the vertical origin at `content.top()`.
pub(crate) fn beat_grid_rect(content: Rect, lead_px: f32) -> Rect {
    Rect::from_min_max(
        Pos2::new(content.left() + lead_px - KEY_COLUMN_WIDTH, content.top()),
        content.max,
    )
}

pub(crate) fn pitch_to_y(grid: Rect, pitch: u8, metrics: ViewMetrics) -> f32 {
    let row = (MAX_PITCH as i32 - pitch as i32) as f32;
    grid.top() + row * metrics.key_height
}

pub(crate) fn y_to_pitch(grid: Rect, y: f32, metrics: ViewMetrics) -> u8 {
    let row = ((y - grid.top()) / metrics.key_height).floor() as i32;
    let pitch = MAX_PITCH as i32 - row;
    Project::clamp_pitch(pitch)
}

pub(crate) fn note_rect(grid: Rect, note: &Note, metrics: ViewMetrics) -> Rect {
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

pub(crate) fn hit_test_note<'a>(
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

pub(crate) fn select_notes_in_rect(
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

pub(crate) fn is_black_key(pitch: u8) -> bool {
    matches!(pitch % 12, 1 | 3 | 6 | 8 | 10)
}

pub(crate) fn draw_grid(
    painter: &egui::Painter,
    grid: Rect,
    lead_px: f32,
    metrics: ViewMetrics,
    total_beats: f32,
    beats_per_bar: f32,
    theme: &ThemeColors,
) {
    // The scroll content is the pure timeline (no key column), so the grid fills
    // from its own left edge.
    let timeline_left = grid.left();
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

    // Beat 0 sits after the lead-in margin; grid lines share that origin.
    let beat_origin = timeline_left + lead_px;
    let beat_x = |beat: f32| beat_origin + beat * metrics.beat_width;

    let beat_count = total_beats.ceil() as i32;
    for beat in 0..=beat_count {
        let x = beat_x(beat as f32);
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
        let x = beat_x(beat);
        painter.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            egui::Stroke::new(1.0_f32, theme.grid_subbeat),
        );
    }

    // Dim the empty region on either side of the clip so the playable range
    // reads as distinct from the outside margins.
    if lead_px > 0.5 {
        let shade = egui::Color32::from_black_alpha(64);
        let clip_end_x = beat_x(total_beats);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(timeline_left, grid.top()),
                Pos2::new(beat_origin, grid.bottom()),
            ),
            0.0,
            shade,
        );
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(clip_end_x, grid.top()),
                Pos2::new(grid.right(), grid.bottom()),
            ),
            0.0,
            shade,
        );
    }
}

pub(crate) fn draw_keyboard(
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

pub(crate) fn set_audition_pitch(
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

pub(crate) fn clear_audition(
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

pub(crate) fn tick_timed_audition(
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

pub(crate) fn preview_pitch_briefly(
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

pub(crate) fn hold_audition_pitch(
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
pub(crate) fn handle_keyboard_audition(
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

pub(crate) fn hit_test_key(keys: Rect, grid: Rect, pointer: Pos2, metrics: ViewMetrics) -> Option<u8> {
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

pub(crate) fn draw_notes(
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

pub(crate) fn draw_marquee(painter: &egui::Painter, selection: Rect, theme: &ThemeColors) {
    painter.rect(
        selection,
        0.0,
        theme.marquee_fill,
        egui::Stroke::new(1.0_f32, theme.marquee_stroke),
        egui::StrokeKind::Inside,
    );
}

pub(crate) fn set_single_selection(selected_note_ids: &mut HashSet<u64>, note_id: u64) {
    selected_note_ids.clear();
    selected_note_ids.insert(note_id);
}

fn apply_move_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    clip_id: u64,
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

    let (delta_beats, delta_pitch) = project.clamp_note_move_deltas(
        clip_id,
        &drag.originals,
        desired_delta_beats,
        desired_delta_pitch,
        &drag.ignore_ids,
    );

    for original in &drag.originals {
        if let Some(note) = project
            .midi_clip_mut(clip_id)
            .and_then(|clip| clip.note_mut(original.id))
        {
            note.start_beats = (original.start_beats + delta_beats).max(0.0);
            note.pitch = Project::clamp_pitch(original.pitch as i32 + delta_pitch);
            note.duration_beats = original.duration_beats;
        }
    }
}

pub(crate) fn resize_drag_mode(note_bounds: Rect, pointer_x: f32) -> Option<DragMode> {
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

pub(crate) fn update_resize_hover_cursor(
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

fn apply_resize_drag(
    drag: &ActiveDrag,
    project: &mut Project,
    clip_id: u64,
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
            let delta = project.clamp_note_resize_start_delta(clip_id, &drag.originals, raw_delta);
            for original in &drag.originals {
                let end = original.end_beats();
                let bound = project.note_resize_start_bound(
                    clip_id,
                    original.id,
                    original.pitch,
                    original.start_beats,
                    &resizing_ids,
                );
                let new_start = (original.start_beats + delta)
                    .max(bound)
                    .max(0.0)
                    .min(end - SNAP_BEATS);
                if let Some(note) = project
                    .midi_clip_mut(clip_id)
                    .and_then(|clip| clip.note_mut(original.id))
                {
                    note.start_beats = new_start;
                    note.duration_beats = (end - new_start).max(SNAP_BEATS);
                    note.pitch = original.pitch;
                }
            }
        }
        DragMode::ResizeEnd => {
            let desired_end = snapped_beats(current_beats.max(0.0));
            let raw_delta = desired_end - primary.end_beats();
            let delta = project.clamp_note_resize_end_delta(clip_id, &drag.originals, raw_delta);
            for original in &drag.originals {
                let bound = project.note_resize_end_bound(
                    clip_id,
                    original.id,
                    original.pitch,
                    original.end_beats(),
                    &resizing_ids,
                );
                let new_end = (original.end_beats() + delta)
                    .min(bound)
                    .max(original.start_beats + SNAP_BEATS);
                if let Some(note) = project
                    .midi_clip_mut(clip_id)
                    .and_then(|clip| clip.note_mut(original.id))
                {
                    note.start_beats = original.start_beats;
                    note.duration_beats = (new_end - original.start_beats).max(SNAP_BEATS);
                    note.pitch = original.pitch;
                }
            }
        }
        DragMode::Move => {}
    }
}

fn finish_active_drag(
    active_drag: &mut Option<ActiveDrag>,
    project: &mut Project,
    history: &mut EditHistory,
    clip_id: u64,
    selected_note_ids: &mut HashSet<u64>,
    drag_moved: bool,
    default_duration_beats: &mut f32,
    engine: &mut dyn DawEngine,
    track_id: u64,
    audition_pitch: &mut Option<u8>,
    audition_held: &mut bool,
    audition_until: &mut Option<f64>,
) {
    let Some(drag) = active_drag.take() else {
        return;
    };

    // Shift+click without move: discard stacked copies.
    if !drag.ignore_ids.is_empty() && !drag_moved {
        history.abort(project);
        selected_note_ids.clear();
        selected_note_ids.extend(drag.ignore_ids.iter().copied());
        if *audition_held {
            clear_audition(
                engine,
                track_id,
                audition_pitch,
                audition_held,
                audition_until,
            );
        }
        return;
    }

    if drag_moved && matches!(drag.mode, DragMode::Move) {
        let moved_ids: Vec<u64> = drag.originals.iter().map(|note| note.id).collect();
        project.resolve_note_move_overlaps(clip_id, &moved_ids);
    }

    history.commit(project);
    if matches!(drag.mode, DragMode::ResizeStart | DragMode::ResizeEnd) {
        if let Some(note) = project
            .midi_clip(clip_id)
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
        .midi_clip(clip_id)
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
    track_id: u64,
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

    let clip_notes: Vec<Note> = project
        .midi_clip(clip_id)
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

    // Audition the note's pitch immediately on press (before egui drag threshold).
    if response.ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary))
        && is_timeline_pointer(grid, press_pos)
        && active_drag.is_none()
        && marquee.is_none()
    {
        if let Some(note) = hit_test_note(grid, &clip_notes, press_pos, metrics) {
            if !selected_note_ids.contains(&note.id) {
                set_single_selection(selected_note_ids, note.id);
            }
            let note_bounds = note_rect(grid, &note, metrics);
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

    if response.ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
        && active_drag.is_none()
        && *audition_held
    {
        clear_audition(
            engine,
            track_id,
            audition_pitch,
            audition_held,
            audition_until,
        );
    }

    if let Some(drag) = active_drag.clone() {
        if primary_down && (response.dragged() || response.drag_started()) {
            *drag_moved = true;
            let snap_horizontal = !response.ctx.input(|input| input.modifiers.alt);
            let current_beats = x_to_beat(grid, pointer.x, timeline);
            let current_pitch = y_to_pitch(grid, pointer.y, metrics) as i32;
            match drag.mode {
                DragMode::Move => {
                    apply_move_drag(
                        &drag,
                        project,
                        clip_id,
                        current_beats,
                        current_pitch,
                        snap_horizontal,
                    );
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
                    apply_resize_drag(&drag, project, clip_id, current_beats, snap_horizontal);
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
                if let Some(clip) = project.midi_clip_mut(clip_id) {
                    clip.remove_note(note_id);
                    history.push_before(before);
                }
                selected_note_ids.remove(&note_id);
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
            let mut ignore_ids = Vec::new();
            if matches!(mode, DragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected_note_ids.iter().copied().collect();
                ignore_ids = source_ids.clone();
                let new_ids =
                    project.duplicate_notes_in_clip(clip_id, &source_ids, 0.0, 0, true);
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

            let originals = project
                .midi_clip(clip_id)
                .map(|clip| {
                    clip.notes
                        .iter()
                        .filter(|n| selected_note_ids.contains(&n.id))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

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
                        apply_move_drag(
                            &drag,
                            project,
                            clip_id,
                            current_beats,
                            current_pitch,
                            snap_horizontal,
                        );
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
                        apply_resize_drag(&drag, project, clip_id, current_beats, snap_horizontal);
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

pub(crate) fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}
