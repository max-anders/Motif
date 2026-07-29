use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Align, Layout, Pos2, Rect, Response, Sense, Ui, UiBuilder, Vec2};

use crate::engine::{DawEngine, DecodedAudio, PluginCatalog, PluginRef};
use crate::model::{
    AudioClip, Clip, EditHistory, Project, Track, TrackInstrument, DEFAULT_CLIP_LENGTH_BEATS,
    SNAP_BEATS,
};
use crate::ui::automation::{
    automation_extra_height, AutomationUi, ADD_AUTOMATION_ROW_HEIGHT, AUTOMATION_LANE_BODY_HEIGHT,
};
use crate::ui::instrument_menu::{
    choice_to_instrument, show_instrument_picker, track_name_for_choice, InstrumentChoice,
    MENU_LIST_MAX_HEIGHT,
};
use crate::ui::clip_variations::{
    show_pattern_block_link_control, show_playlist_clip_link_control,
    show_playlist_clip_mute_control, show_playlist_clip_variation_menu,
};
use crate::ui::pattern_strip::pattern_block_rect;
use crate::ui::note_preview::{draw_note_preview, NotePreviewStyle};
use crate::ui::pattern_strip::PatternStripUi;
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_horizontal_wheel_controls, arrangement_beat_width_bounds, daw_editor_scroll_area,
    draw_loop_region, draw_playhead, draw_playback_anchor, draw_ruler, draw_timeline_grid_lines,
    handle_loop_region_pointer, handle_timeline_playhead_pointer, hit_test_loop_edge,
    is_timeline_pointer, timeline_x, with_solid_scrollbars, x_to_beat, LoopEdge, TimelineMetrics,
    DEFAULT_BEAT_WIDTH, RULER_HEIGHT, TIMELINE_GUTTER_WIDTH,
};
use crate::ui::track_rename::{PatternLaneRenameUi, TrackRenameUi};

pub(crate) const TRACK_HEADER_WIDTH: f32 = TIMELINE_GUTTER_WIDTH;
pub(crate) const LANE_HEIGHT: f32 = 72.0;
const ADD_TRACK_GAP: f32 = 14.0;
const ADD_TRACK_BUTTON_SIZE: f32 = 28.0;
const ADD_TRACK_ROW_HEIGHT: f32 = ADD_TRACK_GAP + ADD_TRACK_BUTTON_SIZE + 8.0;
const RESIZE_HANDLE_PX: f32 = 10.0;
const MS_BUTTON_SIZE: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipDragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Debug, Clone)]
pub(crate) struct ClipOriginal {
    clip_id: u64,
    start_beats: f32,
    length_beats: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct MarqueeDrag {
    start: Pos2,
    current: Pos2,
}

impl MarqueeDrag {
    pub(crate) fn new(start: Pos2, current: Pos2) -> Self {
        Self { start, current }
    }

    pub(crate) fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.current)
    }

    pub(crate) fn set_current(&mut self, pos: Pos2) {
        self.current = pos;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClipDrag {
    /// Primary clip under the pointer (resize target / open-on-double-click id).
    clip_id: u64,
    /// Track owning the primary clip; used to keep track selection in sync.
    track_id: u64,
    mode: ClipDragMode,
    pointer_start_beats: f32,
    originals: Vec<ClipOriginal>,
    /// Clips movers may overlap during this drag (Shift+drag duplicate sources).
    ignore_ids: Vec<u64>,
}

impl ClipDrag {
    pub(crate) fn moving_clip_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.originals.iter().map(|original| original.clip_id)
    }
}

/// `device_id: None` means the track's instrument; `Some(id)` means one of
/// its insert-FX devices (see `crate::engine::PluginRef`, which this maps to
/// 1:1 — kept as plain fields here so the UI layer stays engine-agnostic).
#[derive(Debug, Clone)]
pub enum PluginEditorRequest {
    Open {
        track_id: u64,
        device_id: Option<u64>,
        title: String,
    },
    Close {
        track_id: u64,
        device_id: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct AudioImportRequest {
    pub track_id: u64,
    pub start_beats: f32,
}

pub struct PlaylistUi {
    selected_clip_ids: HashSet<u64>,
    active_drag: Option<ClipDrag>,
    marquee: Option<MarqueeDrag>,
    dragging_playhead: bool,
    dragging_loop_edge: Option<LoopEdge>,
    beat_width: f32,
    scroll_offset: Vec2,
    /// Timeline viewport width from the previous frame (excludes track headers + scrollbar).
    timeline_view_w: f32,
    /// Set when user clicks a clip without dragging (consumed by app).
    open_clip_request: Option<u64>,
    /// Open/close native plugin editor (consumed by app).
    plugin_editor_request: Option<PluginEditorRequest>,
    /// Delete track (consumed by app for piano-roll / engine cleanup).
    delete_track_request: Option<u64>,
    /// Duplicate track (consumed by app).
    duplicate_track_request: Option<u64>,
    /// Delete pattern lane (consumed by app).
    delete_pattern_lane_request: Option<u64>,
    /// Duplicate pattern lane (consumed by app).
    duplicate_pattern_lane_request: Option<u64>,
    /// Track header currently under the pointer (for Delete track shortcut).
    hovered_track_header: Option<u64>,
    /// Pattern lane header currently under the pointer.
    hovered_pattern_lane_header: Option<u64>,
    /// Import audio clip request (consumed by app).
    audio_import_request: Option<AudioImportRequest>,
    /// True if pointer moved enough during drag to count as a drag, not a click.
    drag_moved: bool,
    add_track_search: String,
    change_instrument_search: String,
    /// Last instrument load errors for display on lanes.
    instrument_errors: HashMap<u64, String>,
    /// Tracks whose automation fold-out is expanded under the clip lane.
    automation_expanded: HashSet<u64>,
    automation: AutomationUi,
    pattern_strips: HashMap<u64, PatternStripUi>,
}

impl Default for PlaylistUi {
    fn default() -> Self {
        Self {
            selected_clip_ids: HashSet::new(),
            active_drag: None,
            marquee: None,
            dragging_playhead: false,
            dragging_loop_edge: None,
            beat_width: DEFAULT_BEAT_WIDTH,
            scroll_offset: Vec2::ZERO,
            timeline_view_w: 0.0,
            open_clip_request: None,
            plugin_editor_request: None,
            delete_track_request: None,
            duplicate_track_request: None,
            delete_pattern_lane_request: None,
            duplicate_pattern_lane_request: None,
            hovered_track_header: None,
            hovered_pattern_lane_header: None,
            audio_import_request: None,
            drag_moved: false,
            add_track_search: String::new(),
            change_instrument_search: String::new(),
            instrument_errors: HashMap::new(),
            automation_expanded: HashSet::new(),
            automation: AutomationUi::default(),
            pattern_strips: HashMap::new(),
        }
    }
}

/// Per-track vertical layout for variable-height playlist rows.
#[derive(Debug, Clone)]
struct TrackLayout {
    /// Y offset of each track block relative to `body.top()`.
    tops: Vec<f32>,
    /// Total block height (clip lane + optional automation fold-out).
    heights: Vec<f32>,
}

impl TrackLayout {
    fn from_project(project: &Project, automation_expanded: &HashSet<u64>) -> Self {
        let mut tops = Vec::with_capacity(project.tracks.len());
        let mut heights = Vec::with_capacity(project.tracks.len());
        let mut y = 0.0_f32;
        for track in &project.tracks {
            let expanded = automation_expanded.contains(&track.id);
            let height =
                LANE_HEIGHT + automation_extra_height(track.automation_lanes.len(), expanded);
            tops.push(y);
            heights.push(height);
            y += height;
        }
        Self { tops, heights }
    }

    fn total_height(&self) -> f32 {
        self.tops
            .last()
            .zip(self.heights.last())
            .map(|(top, height)| top + height)
            .unwrap_or(0.0)
            .max(LANE_HEIGHT)
    }

    fn clip_lane_rect(&self, body: Rect, track_index: usize) -> Option<Rect> {
        let top = *self.tops.get(track_index)?;
        let lane_top = body.top() + top;
        Some(Rect::from_min_max(
            Pos2::new(body.left(), lane_top),
            Pos2::new(body.right(), lane_top + LANE_HEIGHT),
        ))
    }

    fn track_at_y(&self, body: Rect, y: f32) -> Option<(usize, bool)> {
        let rel = y - body.top();
        for (index, (top, height)) in self.tops.iter().zip(self.heights.iter()).enumerate() {
            if rel >= *top && rel < top + height {
                let in_clip_lane = rel < top + LANE_HEIGHT;
                return Some((index, in_clip_lane));
            }
        }
        None
    }
}

impl PlaylistUi {
    pub fn selected_clip_ids(&self) -> &HashSet<u64> {
        &self.selected_clip_ids
    }

    pub fn take_open_clip_request(&mut self) -> Option<u64> {
        self.open_clip_request.take()
    }

    pub fn take_plugin_editor_request(&mut self) -> Option<PluginEditorRequest> {
        self.plugin_editor_request.take()
    }

    pub fn take_delete_track_request(&mut self) -> Option<u64> {
        self.delete_track_request.take()
    }

    pub fn take_duplicate_track_request(&mut self) -> Option<u64> {
        self.duplicate_track_request.take()
    }

    pub fn take_delete_pattern_lane_request(&mut self) -> Option<u64> {
        self.delete_pattern_lane_request.take()
    }

    pub fn take_duplicate_pattern_lane_request(&mut self) -> Option<u64> {
        self.duplicate_pattern_lane_request.take()
    }

    pub fn hovered_track_header(&self) -> Option<u64> {
        self.hovered_track_header
    }

    pub fn take_audio_import_request(&mut self) -> Option<AudioImportRequest> {
        self.audio_import_request.take()
    }

    pub fn clear_selection(&mut self) {
        self.selected_clip_ids.clear();
    }

    pub fn set_selection(&mut self, clip_ids: impl IntoIterator<Item = u64>) {
        self.selected_clip_ids.clear();
        self.selected_clip_ids.extend(clip_ids);
    }

    fn sync_pattern_strips(&mut self, project: &Project) {
        self.pattern_strips
            .retain(|lane_id, _| project.pattern_lane(*lane_id).is_some());
        for lane in &project.pattern_lanes {
            self.pattern_strips.entry(lane.id).or_default();
        }
        for strip in self.pattern_strips.values_mut() {
            strip.prune_selection(project);
        }
    }

    fn any_pattern_strip_gesture_active(&self) -> bool {
        self.pattern_strips.values().any(|strip| strip.gesture_active())
    }

    pub fn prune_selection(&mut self, project: &Project) {
        self.selected_clip_ids
            .retain(|id| project.clip(*id).is_some());
        self.sync_pattern_strips(project);
    }

    pub fn selected_pattern_block_ids(&self) -> Vec<u64> {
        self.pattern_strips
            .values()
            .flat_map(|strip| strip.selected_block_ids().iter().copied())
            .collect()
    }

    pub fn clear_pattern_selection(&mut self) {
        for strip in self.pattern_strips.values_mut() {
            strip.clear_selection();
        }
    }

    pub fn set_pattern_selection(
        &mut self,
        project: &Project,
        block_ids: impl IntoIterator<Item = u64>,
    ) {
        self.clear_pattern_selection();
        let mut by_lane: HashMap<u64, Vec<u64>> = HashMap::new();
        for id in block_ids {
            if let Some(lane_id) = project.pattern_lane_id_for_block(id) {
                by_lane.entry(lane_id).or_default().push(id);
            }
        }
        for (lane_id, ids) in by_lane {
            if let Some(strip) = self.pattern_strips.get_mut(&lane_id) {
                strip.set_selection(ids);
            }
        }
    }

    pub fn take_open_pattern_block_request(&mut self) -> Option<u64> {
        for strip in self.pattern_strips.values_mut() {
            if let Some(id) = strip.take_open_block_request() {
                return Some(id);
            }
        }
        None
    }

    pub fn set_instrument_errors(&mut self, errors: HashMap<u64, String>) {
        self.instrument_errors = errors;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        selected_track: &mut Option<u64>,
        selected_pattern_lane: &mut Option<u64>,
        decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
        settings: &mut crate::ui::app_settings::AppSettings,
        theme: &ThemeColors,
        track_rename: &mut TrackRenameUi,
        pattern_lane_rename: &mut PatternLaneRenameUi,
    ) -> bool {
        // CentralPanel uses Frame::NONE; paint the full panel so nothing shows through.
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);
        self.hovered_track_header = None;
        self.hovered_pattern_lane_header = None;
        let mut settings_dirty = false;

        ui.horizontal(|ui| {
            egui::menu::menu_button(ui, "Add track", |ui| {
                if add_track_from_picker(
                    ui,
                    project,
                    catalog,
                    history,
                    selected_track,
                    &mut self.add_track_search,
                    "add_track",
                ) {
                    ui.close_menu();
                }
            });
            if ui.button("Import sample...").clicked() {
                let track_id = selected_track
                    .or_else(|| project.tracks.first().map(|track| track.id))
                    .unwrap_or(0);
                if track_id != 0 {
                    self.audio_import_request = Some(AudioImportRequest {
                        track_id,
                        start_beats: Project::snap_beats(engine.current_beats().max(0.0)),
                    });
                    *selected_track = Some(track_id);
                }
            }
        });
        ui.add_space(4.0);

        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, theme.panel_bg);

        // Side-by-side layout (same model as the piano roll): fixed header column +
        // corner on the left, ruler across the top-right, scrolling timeline in the
        // rest. Headers/ruler are separate widgets beside the scroll content, not
        // sticky overlays floating over beats.
        let corner = Rect::from_min_max(
            full.min,
            Pos2::new(full.left() + TRACK_HEADER_WIDTH, full.top() + RULER_HEIGHT),
        );
        let ruler_area = Rect::from_min_max(
            Pos2::new(full.left() + TRACK_HEADER_WIDTH, full.top()),
            Pos2::new(full.right(), full.top() + RULER_HEIGHT),
        );
        let headers_area = Rect::from_min_max(
            Pos2::new(full.left(), full.top() + RULER_HEIGHT),
            Pos2::new(full.left() + TRACK_HEADER_WIDTH, full.bottom()),
        );
        let timeline_area = Rect::from_min_max(
            Pos2::new(full.left() + TRACK_HEADER_WIDTH, full.top() + RULER_HEIGHT),
            full.max,
        );

        let total_beats = project.arrangement_length_beats();
        let timeline_view_w = if self.timeline_view_w > 0.0 {
            self.timeline_view_w
        } else {
            timeline_area.width().max(1.0)
        };
        let (min_beat_width, max_beat_width) =
            arrangement_beat_width_bounds(timeline_view_w, total_beats);
        self.beat_width = self.beat_width.clamp(min_beat_width, max_beat_width);
        // Zoom over ruler + timeline (header column is outside beat space).
        let zoom_viewport = Rect::from_min_max(
            Pos2::new(timeline_area.left(), full.top()),
            timeline_area.max,
        );
        apply_horizontal_wheel_controls(
            ui,
            zoom_viewport,
            &mut self.beat_width,
            &mut self.scroll_offset.x,
            min_beat_width,
            max_beat_width,
            0.0,
        );

        let metrics = TimelineMetrics {
            beat_width: self.beat_width,
        };
        let layout = TrackLayout::from_project(project, &self.automation_expanded);
        project.ensure_pattern_lane();
        self.sync_pattern_strips(project);
        if selected_pattern_lane
            .filter(|id| project.pattern_lane(*id).is_some())
            .is_none()
        {
            *selected_pattern_lane = project.pattern_lanes.first().map(|lane| lane.id);
        }
        let track_area_height = layout.total_height();
        let pattern_lanes_height =
            PatternStripUi::pattern_lanes_area_height(project.pattern_lanes.len());
        // Pure timeline scroll content: no header gutter, no ruler strip.
        let content_height = track_area_height + pattern_lanes_height + ADD_TRACK_ROW_HEIGHT;
        let content_width = total_beats * metrics.beat_width;
        let canvas_size = Vec2::new(
            content_width.max(timeline_view_w),
            content_height.max(timeline_area.height()),
        );

        let scroll = self.scroll_offset;
        let output = with_solid_scrollbars(ui, theme, |ui| {
            let mut timeline_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("playlist_timeline")
                    .max_rect(timeline_area)
                    .layout(Layout::top_down(Align::LEFT)),
            );
            timeline_ui.set_clip_rect(timeline_area);
            daw_editor_scroll_area("playlist_canvas")
                .scroll_offset(scroll)
                .show(&mut timeline_ui, |ui| {
                    ui.set_min_size(canvas_size);
                    let (response, painter) =
                        ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    painter.rect_filled(content, 0.0, theme.panel_bg);
                    // Shared timeline helpers add TIMELINE_GUTTER_WIDTH internally;
                    // shift left so beat 0 lands on content.left().
                    let body = playlist_beat_body(content);
                    let layout = TrackLayout::from_project(project, &self.automation_expanded);
                    let track_area_height = layout.total_height();
                    let pattern_lanes_height =
                        PatternStripUi::pattern_lanes_area_height(project.pattern_lanes.len());

                    let pattern_lane_hit = response.interact_pointer_pos().and_then(|pos| {
                        project.pattern_lanes.iter().enumerate().find_map(|(idx, lane)| {
                            PatternStripUi::contains_y(body, track_area_height, idx, pos.y)
                                .then_some((idx, lane.id))
                        })
                    });
                    let on_pattern_strip = pattern_lane_hit.is_some();
                    let gesture_active = self.active_drag.is_some()
                        || self.marquee.is_some()
                        || self.any_pattern_strip_gesture_active();

                    // Empty rect: header column is a separate widget, so timeline
                    // hit-tests never land on headers. Kept so clip helpers stay shared.
                    let no_header_overlay = Rect::NOTHING;

                    if on_pattern_strip || self.any_pattern_strip_gesture_active() {
                        let pattern_lane_work: Vec<(usize, u64)> = project
                            .pattern_lanes
                            .iter()
                            .enumerate()
                            .map(|(idx, lane)| (idx, lane.id))
                            .collect();
                        for (lane_index, lane_id) in pattern_lane_work {
                            let on_this_strip = pattern_lane_hit
                                .map(|(idx, _)| idx == lane_index)
                                .unwrap_or(false);
                            let strip_active = self
                                .pattern_strips
                                .get(&lane_id)
                                .is_some_and(|strip| strip.gesture_active());
                            if !on_this_strip && !strip_active {
                                continue;
                            }
                            let Some(strip) = self.pattern_strips.get_mut(&lane_id) else {
                                continue;
                            };
                            let strip_rect =
                                PatternStripUi::strip_rect(body, track_area_height, lane_index);
                            strip.handle_pointer(
                                &response,
                                body,
                                strip_rect,
                                lane_id,
                                metrics,
                                project,
                                history,
                                &mut self.selected_clip_ids,
                            );
                        }
                    } else if !gesture_active
                        && handle_timeline_playhead_pointer(
                            &response,
                            // Approximate ruler ref for body seeks; real ruler interact
                            // runs after scroll (side-by-side dual Response).
                            ruler_area.translate(Vec2::new(-TRACK_HEADER_WIDTH, 0.0)),
                            body,
                            metrics,
                            engine,
                            &mut self.dragging_playhead,
                            0.0,
                        )
                    {
                        // Playhead owns the pointer; skip clip picks this frame.
                    } else if !gesture_active
                        || self.active_drag.is_some()
                        || self.marquee.is_some()
                    {
                        handle_clip_pointer(
                            &response,
                            body,
                            no_header_overlay,
                            &layout,
                            metrics,
                            project,
                            history,
                            &mut self.selected_clip_ids,
                            &mut self.active_drag,
                            &mut self.marquee,
                            &mut self.open_clip_request,
                            &mut self.drag_moved,
                            selected_track,
                            &mut self.pattern_strips,
                        );
                    }

                    let raise_clip_ids: HashSet<u64> = self
                        .active_drag
                        .as_ref()
                        .map(|drag| drag.moving_clip_ids().collect())
                        .unwrap_or_default();
                    let mut variation_menu_targets: Vec<(u64, Rect)> = Vec::new();
                    for (index, track) in project.tracks.iter().enumerate() {
                        let Some(lane_rect) = layout.clip_lane_rect(body, index) else {
                            continue;
                        };
                        let audible = project.track_audible(track);
                        let override_windows =
                            project.pattern_override_windows_for_track(track.id);
                        draw_lane_timeline(
                            &painter,
                            lane_rect,
                            body,
                            metrics,
                            total_beats,
                            project.beats_per_bar,
                            &track.clips,
                            &self.selected_clip_ids,
                            &raise_clip_ids,
                            audible,
                            project.bpm,
                            decoded_audio,
                            &override_windows,
                            theme,
                        );
                        if self.active_drag.is_none() {
                            for clip in &track.clips {
                                let clip_rect = clip_block_rect(body, lane_rect, clip, metrics);
                                variation_menu_targets.push((clip.id(), clip_rect));
                            }
                        }
                    }
                    for (clip_id, clip_rect) in variation_menu_targets {
                        if project.clip(clip_id).and_then(|c| c.as_midi()).is_some() {
                            show_playlist_clip_link_control(
                                ui,
                                clip_rect,
                                clip_id,
                                project,
                                history,
                                theme,
                            );
                            let _ = show_playlist_clip_variation_menu(
                                ui,
                                clip_rect,
                                clip_id,
                                project,
                                history,
                                theme,
                            );
                        }
                        show_playlist_clip_mute_control(
                            ui,
                            clip_rect,
                            clip_id,
                            project,
                            history,
                            engine,
                            theme,
                        );
                    }

                    let priority_row =
                        PatternStripUi::priority_row_rect(body, track_area_height);
                    PatternStripUi::paint_priority_timeline(
                        &painter,
                        priority_row,
                        project.pattern_overrides_playlist,
                        theme,
                    );

                    let mut pattern_link_targets: Vec<(u64, Rect)> = Vec::new();
                    let pattern_gesture = self.any_pattern_strip_gesture_active();
                    for (lane_index, lane) in project.pattern_lanes.iter().enumerate() {
                        let strip_rect =
                            PatternStripUi::strip_rect(body, track_area_height, lane_index);
                        if let Some(strip) = self.pattern_strips.get(&lane.id) {
                            strip.paint(
                                &painter,
                                strip_rect,
                                body,
                                metrics,
                                total_beats,
                                project.beats_per_bar,
                                project,
                                lane.id,
                                theme,
                            );
                        }
                        if !pattern_gesture {
                            for block in &lane.blocks {
                                let block_rect =
                                    pattern_block_rect(body, strip_rect, block, metrics);
                                pattern_link_targets.push((block.id, block_rect));
                            }
                        }
                    }
                    for (block_id, block_rect) in pattern_link_targets {
                        show_pattern_block_link_control(
                            ui,
                            block_rect,
                            block_id,
                            project,
                            history,
                            theme,
                        );
                    }

                    let add_lane_row = PatternStripUi::add_lane_row_rect(
                        body,
                        track_area_height,
                        project.pattern_lanes.len(),
                    );
                    let add_lane_timeline = Rect::from_min_max(
                        Pos2::new(content.left(), add_lane_row.top()),
                        Pos2::new(content.right(), add_lane_row.bottom()),
                    );
                    painter.rect_filled(
                        add_lane_timeline,
                        0.0,
                        theme.lane_bg.gamma_multiply(0.92),
                    );

                    if let Some(marquee) = &self.marquee {
                        draw_marquee(&painter, marquee.rect(), theme);
                    }

                    // Automation fold-out timeline curves (headers live in the fixed column).
                    let track_ids: Vec<u64> = project.tracks.iter().map(|t| t.id).collect();
                    for (index, track_id) in track_ids.iter().copied().enumerate() {
                        if !self.automation_expanded.contains(&track_id) {
                            continue;
                        }
                        let Some(clip_lane) = layout.clip_lane_rect(body, index) else {
                            continue;
                        };
                        let lane_count = project
                            .track(track_id)
                            .map(|t| t.automation_lanes.len())
                            .unwrap_or(0);
                        for (lane_i, lane_id) in project
                            .track(track_id)
                            .map(|t| {
                                t.automation_lanes
                                    .iter()
                                    .map(|lane| lane.id)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                            .into_iter()
                            .enumerate()
                        {
                            let sub_top =
                                clip_lane.bottom() + lane_i as f32 * AUTOMATION_LANE_BODY_HEIGHT;
                            let auto_body = Rect::from_min_max(
                                Pos2::new(body.left(), sub_top),
                                Pos2::new(body.right(), sub_top + AUTOMATION_LANE_BODY_HEIGHT),
                            );
                            self.automation.show_lane_timeline(
                                ui,
                                auto_body,
                                metrics,
                                project,
                                track_id,
                                lane_id,
                                history,
                                theme,
                                total_beats,
                                project.beats_per_bar,
                            );
                        }
                        let add_top =
                            clip_lane.bottom() + lane_count as f32 * AUTOMATION_LANE_BODY_HEIGHT;
                        let add_body = Rect::from_min_max(
                            Pos2::new(content.left(), add_top),
                            Pos2::new(content.right(), add_top + ADD_AUTOMATION_ROW_HEIGHT),
                        );
                        painter.rect_filled(add_body, 0.0, theme.panel_bg.gamma_multiply(0.9));
                    }

                    // Compact "+" centered in the visible timeline, under the last lane.
                    let add_center = Pos2::new(
                        timeline_area.center().x,
                        content.top()
                            + track_area_height
                            + pattern_lanes_height
                            + ADD_TRACK_GAP
                            + ADD_TRACK_BUTTON_SIZE * 0.5,
                    );
                    let add_button =
                        Rect::from_center_size(add_center, Vec2::splat(ADD_TRACK_BUTTON_SIZE));
                    ui.allocate_new_ui(UiBuilder::new().max_rect(add_button), |ui| {
                        egui::menu::menu_button(
                            ui,
                            egui::RichText::new("+")
                                .size(18.0)
                                .color(theme.text_muted),
                            |ui| {
                                if add_track_from_picker(
                                    ui,
                                    project,
                                    catalog,
                                    history,
                                    selected_track,
                                    &mut self.add_track_search,
                                    "add_track_lane",
                                ) {
                                    ui.close_menu();
                                }
                            },
                        )
                        .response
                        .on_hover_text("Add track");
                    });

                    (response, content, body, layout, track_area_height)
                })
        });

        let (response, _content, body, layout, track_area_height) = output.inner;
        self.scroll_offset = output.state.offset;
        self.timeline_view_w = output.inner_rect.width().max(1.0);

        // Shift ruler reference so shared helpers (gutter-aware) line up with beat 0.
        let ruler_ref = Rect::from_min_max(
            Pos2::new(ruler_area.left() - TRACK_HEADER_WIDTH, ruler_area.top()),
            ruler_area.max,
        );

        let ruler_response = ui.interact(
            ruler_area,
            ui.id().with("playlist_ruler"),
            Sense::click_and_drag(),
        );

        let gesture_active = self.active_drag.is_some()
            || self.marquee.is_some()
            || self.any_pattern_strip_gesture_active();
        let loop_handled = !gesture_active
            && handle_loop_region_pointer(
                &ruler_response,
                ruler_ref,
                body,
                metrics,
                project,
                &mut self.dragging_loop_edge,
            );

        // Ruler + timeline are separate interact regions (side-by-side), so playhead
        // scrubbing must consult the ruler here as well. Body seeks already ran inside
        // the scroll callback (before clip picks); continue an in-flight scrub on
        // whichever region still owns the pointer.
        if !gesture_active && !loop_handled {
            if self.dragging_playhead {
                let active = if response.interact_pointer_pos().is_some() {
                    &response
                } else {
                    &ruler_response
                };
                handle_timeline_playhead_pointer(
                    active,
                    ruler_ref,
                    body,
                    metrics,
                    engine,
                    &mut self.dragging_playhead,
                    0.0,
                );
            } else {
                handle_timeline_playhead_pointer(
                    &ruler_response,
                    ruler_ref,
                    body,
                    metrics,
                    engine,
                    &mut self.dragging_playhead,
                    0.0,
                );
            }
        }

        // ---- Fixed header column (vertical scroll synced via content.top()) ----
        ui.painter()
            .with_clip_rect(headers_area)
            .rect_filled(headers_area, 0.0, theme.track_header_bg);
        ui.painter().rect_filled(corner, 0.0, theme.gutter_bg);
        ui.painter().line_segment(
            [corner.right_top(), corner.right_bottom()],
            egui::Stroke::new(1.5_f32, theme.key_divider),
        );
        ui.painter().line_segment(
            [headers_area.right_top(), headers_area.right_bottom()],
            egui::Stroke::new(1.5_f32, theme.key_divider),
        );

        let headers_painter = ui.painter().with_clip_rect(headers_area);
        let track_ids: Vec<u64> = project.tracks.iter().map(|t| t.id).collect();
        let mut next_automation_expanded = self.automation_expanded.clone();

        for (index, track) in project.tracks.iter().enumerate() {
            let Some(lane_rect) = layout.clip_lane_rect(body, index) else {
                continue;
            };
            let header = Rect::from_min_max(
                Pos2::new(headers_area.left(), lane_rect.top()),
                Pos2::new(headers_area.right(), lane_rect.bottom()),
            );
            draw_track_header(
                &headers_painter,
                header,
                track.name.as_str(),
                track.instrument.display_name(),
                track.instrument.format_badge(),
                self.instrument_errors.get(&track.id).map(String::as_str),
                theme,
            );
        }

        let priority_row = PatternStripUi::priority_row_rect(body, track_area_height);
        let priority_header = Rect::from_min_max(
            Pos2::new(headers_area.left(), priority_row.top()),
            Pos2::new(headers_area.right(), priority_row.bottom()),
        );
        PatternStripUi::show_priority_header(
            ui,
            priority_header,
            headers_area,
            project,
            theme,
        );

        for (lane_index, lane) in project.pattern_lanes.iter().enumerate() {
            let strip_rect = PatternStripUi::strip_rect(body, track_area_height, lane_index);
            let header = Rect::from_min_max(
                Pos2::new(headers_area.left(), strip_rect.top()),
                Pos2::new(headers_area.right(), strip_rect.bottom()),
            );
            let lane_snapshot = lane.clone();
            let mut row_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt(("playlist_pattern_lane_header", lane.id))
                    .max_rect(header.intersect(headers_area))
                    .layout(Layout::top_down(Align::Min)),
            );
            row_ui.set_clip_rect(headers_area);
            pattern_lane_header_row(
                &mut row_ui,
                header,
                headers_area,
                &lane_snapshot,
                lane.id,
                project,
                history,
                theme,
                *selected_pattern_lane == Some(lane.id),
                selected_pattern_lane,
                &mut self.delete_pattern_lane_request,
                &mut self.duplicate_pattern_lane_request,
                &mut self.hovered_pattern_lane_header,
                pattern_lane_rename,
            );
        }

        let add_lane_row = PatternStripUi::add_lane_row_rect(
            body,
            track_area_height,
            project.pattern_lanes.len(),
        );
        let add_lane_header = Rect::from_min_max(
            Pos2::new(headers_area.left(), add_lane_row.top()),
            Pos2::new(headers_area.right(), add_lane_row.bottom()),
        );
        headers_painter.rect_filled(add_lane_header, 0.0, theme.track_header_bg);
        let add_lane_button = Rect::from_center_size(
            add_lane_header.center(),
            Vec2::new(ADD_TRACK_BUTTON_SIZE, ADD_TRACK_BUTTON_SIZE),
        );
        let add_lane_response = ui.interact(
            add_lane_button,
            ui.id().with("add_pattern_lane"),
            Sense::click(),
        );
        headers_painter.rect_stroke(
            add_lane_button,
            4.0,
            egui::Stroke::new(1.0_f32, theme.separator),
            egui::StrokeKind::Inside,
        );
        headers_painter.text(
            add_lane_button.center(),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(16.0),
            theme.text_muted,
        );
        if add_lane_response.clicked() {
            history.push_before(project.clone());
            let new_id = project.add_pattern_lane();
            *selected_pattern_lane = Some(new_id);
        }
        add_lane_response.on_hover_text("Add pattern lane");

        for (index, track_id) in track_ids.iter().copied().enumerate() {
            let Some(lane_rect) = layout.clip_lane_rect(body, index) else {
                continue;
            };
            let header = Rect::from_min_max(
                Pos2::new(headers_area.left(), lane_rect.top()),
                Pos2::new(headers_area.right(), lane_rect.bottom()),
            );
            let track_snapshot = project
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .cloned();
            let Some(track_snapshot) = track_snapshot else {
                continue;
            };
            let mut auto_expanded = next_automation_expanded.contains(&track_id);
            // Child UI pinned to the header so M/S allocate_ui_at_rect cannot
            // rewind the parent cursor (same fix as devices headers).
            let mut row_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt(("playlist_header_row", track_id))
                    .max_rect(header.intersect(headers_area))
                    .layout(Layout::top_down(Align::Min)),
            );
            row_ui.set_clip_rect(headers_area);
            track_header_row(
                &mut row_ui,
                header,
                headers_area,
                &track_snapshot,
                track_id,
                project,
                engine,
                catalog,
                history,
                &mut self.change_instrument_search,
                self.instrument_errors.get(&track_id).map(String::as_str),
                theme,
                *selected_track == Some(track_id),
                selected_track,
                &mut self.plugin_editor_request,
                &mut self.delete_track_request,
                &mut self.duplicate_track_request,
                &mut self.hovered_track_header,
                Some(&mut auto_expanded),
                "playlist",
                track_rename,
            );
            if auto_expanded {
                next_automation_expanded.insert(track_id);
            } else {
                next_automation_expanded.remove(&track_id);
            }
        }

        for (index, track_id) in track_ids.iter().copied().enumerate() {
            if !self.automation_expanded.contains(&track_id) {
                continue;
            }
            let Some(clip_lane) = layout.clip_lane_rect(body, index) else {
                continue;
            };
            let track_snapshot = project.track(track_id).cloned();
            let Some(track_snapshot) = track_snapshot else {
                continue;
            };
            let lane_ids: Vec<u64> = track_snapshot
                .automation_lanes
                .iter()
                .map(|lane| lane.id)
                .collect();
            for (lane_i, lane_id) in lane_ids.iter().copied().enumerate() {
                let sub_top = clip_lane.bottom() + lane_i as f32 * AUTOMATION_LANE_BODY_HEIGHT;
                let sub_header = Rect::from_min_max(
                    Pos2::new(headers_area.left(), sub_top),
                    Pos2::new(
                        headers_area.right(),
                        sub_top + AUTOMATION_LANE_BODY_HEIGHT,
                    ),
                );
                self.automation.show_lane_header(
                    ui,
                    sub_header,
                    project,
                    track_id,
                    lane_id,
                    &track_snapshot,
                    engine,
                    history,
                    settings,
                    &mut settings_dirty,
                    theme,
                );
            }
            let add_top =
                clip_lane.bottom() + lane_ids.len() as f32 * AUTOMATION_LANE_BODY_HEIGHT;
            let add_row = Rect::from_min_max(
                Pos2::new(headers_area.left(), add_top),
                Pos2::new(
                    headers_area.right(),
                    add_top + ADD_AUTOMATION_ROW_HEIGHT,
                ),
            );
            self.automation.show_add_lane_row(
                ui,
                add_row,
                project,
                track_id,
                history,
                theme,
            );
        }

        self.automation_expanded = next_automation_expanded;

        // ---- Ruler + playhead / loop (beside the timeline, not over headers) ----
        draw_ruler(
            &ui.painter().with_clip_rect(ruler_area),
            ruler_ref,
            body,
            metrics,
            total_beats,
            project.beats_per_bar,
            theme,
        );
        let playhead_clip =
            Rect::from_min_max(Pos2::new(timeline_area.left(), full.top()), full.max);
        let clip_painter = ui.painter().with_clip_rect(playhead_clip);
        if let Some((loop_start, loop_end)) = project.loop_span() {
            let hover_edge = ruler_response.hover_pos().and_then(|pos| {
                hit_test_loop_edge(ruler_ref, body, metrics, loop_start, loop_end, pos)
            });
            let highlighted = self.dragging_loop_edge.or(hover_edge);
            draw_loop_region(
                &clip_painter,
                ruler_ref,
                body,
                metrics,
                loop_start,
                loop_end,
                theme,
                highlighted,
            );
        }
        let playhead = engine.current_beats();
        let anchor = engine.playback_anchor_beats();
        draw_playback_anchor(
            &clip_painter,
            ruler_ref,
            body,
            metrics,
            anchor,
            playhead,
            true,
            theme,
        );
        draw_playhead(
            &clip_painter,
            ruler_ref,
            body,
            metrics,
            playhead,
            true,
            theme,
        );

        if self.active_drag.is_none() {
            self.drag_moved = false;
        }
        settings_dirty
    }
}

/// Beat-mapping rect for playlist scroll content. Shared `timeline_x` / `x_to_beat`
/// helpers add `TIMELINE_GUTTER_WIDTH` to `rect.left()`, so shifting left by the
/// header column makes beat 0 resolve to `content.left()`.
fn playlist_beat_body(content: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(content.left() - TRACK_HEADER_WIDTH, content.top()),
        content.max,
    )
}

/// Instrument picker that creates a track; returns true when a choice was made.
fn add_track_from_picker(
    ui: &mut Ui,
    project: &mut Project,
    catalog: &PluginCatalog,
    history: &mut EditHistory,
    selected_track: &mut Option<u64>,
    search: &mut String,
    id_salt: &str,
) -> bool {
    let Some(choice) = show_instrument_picker(
        ui,
        catalog,
        search,
        id_salt,
        false,
        MENU_LIST_MAX_HEIGHT,
    ) else {
        return false;
    };
    let number = project.tracks.len() + 1;
    let name = track_name_for_choice(&choice, number);
    let instrument = choice_to_instrument(choice);
    history.push_before(project.clone());
    let track_id = project.add_track(&name, instrument);
    *selected_track = Some(track_id);
    search.clear();
    true
}

pub(crate) fn ms_toggle_button(ui: &mut Ui, label: &str, active: bool, theme: &ThemeColors) -> bool {
    let fill = if active {
        theme.accent
    } else {
        theme.widget_bg
    };
    let stroke = if active {
        theme.accent
    } else {
        theme.separator
    };
    let text = if active {
        theme.panel_bg
    } else {
        theme.button_text
    };
    let button = egui::Button::new(
        egui::RichText::new(label)
            .size(10.0)
            .color(text)
            .monospace(),
    )
    .fill(fill)
    .stroke(egui::Stroke::new(1.0_f32, stroke))
    .min_size(Vec2::new(MS_BUTTON_SIZE, MS_BUTTON_SIZE));
    ui.add(button).clicked()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn track_header_row(
    ui: &mut Ui,
    header: Rect,
    paint_clip: Rect,
    track_snapshot: &Track,
    track_id: u64,
    project: &mut Project,
    engine: &mut dyn DawEngine,
    catalog: &PluginCatalog,
    history: &mut EditHistory,
    change_instrument_search: &mut String,
    instrument_error: Option<&str>,
    theme: &ThemeColors,
    is_selected: bool,
    select_track_request: &mut Option<u64>,
    plugin_editor_request: &mut Option<PluginEditorRequest>,
    delete_track_request: &mut Option<u64>,
    duplicate_track_request: &mut Option<u64>,
    hovered_track_header: &mut Option<u64>,
    automation_expanded: Option<&mut bool>,
    id_scope: &'static str,
    track_rename: &mut TrackRenameUi,
) {
    draw_track_header(
        &ui.painter().with_clip_rect(paint_clip),
        header,
        track_snapshot.name.as_str(),
        track_snapshot.instrument.display_name(),
        track_snapshot.instrument.format_badge(),
        instrument_error,
        theme,
    );
    if is_selected {
        ui.painter().rect_stroke(
            header.shrink(1.0),
            0.0,
            egui::Stroke::new(2.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }

    let id = ui.id().with((id_scope, "track_header", track_id));
    let header_response = ui.interact(header, id, Sense::click());
    if ui.rect_contains_pointer(header) {
        *hovered_track_header = Some(track_id);
    }
    if header_response.clicked() {
        *select_track_request = Some(track_id);
    }

    let name_rect = track_header_name_rect(header);
    let name_id = ui.id().with((id_scope, "track_name", track_id));
    let name_response = ui.interact(name_rect, name_id, Sense::click());
    if name_response.double_clicked() {
        track_rename.begin(track_id, &track_snapshot.name);
    }

    if let Some(expanded) = automation_expanded {
        let disclose = Rect::from_min_size(
            Pos2::new(header.left() + 2.0, header.bottom() - 20.0),
            Vec2::new(18.0, 16.0),
        );
        ui.allocate_ui_at_rect(disclose, |ui| {
            let label = if *expanded { "v" } else { ">" };
            let lane_count = track_snapshot.automation_lanes.len();
            let tip = if *expanded {
                "Collapse automation lanes"
            } else if lane_count == 0 {
                "Show automation lanes"
            } else {
                "Show automation lanes"
            };
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(label)
                            .size(11.0)
                            .color(theme.text_muted)
                            .monospace(),
                    )
                    .fill(theme.widget_bg)
                    .min_size(Vec2::new(16.0, 14.0)),
                )
                .on_hover_text(tip)
                .clicked()
            {
                *expanded = !*expanded;
            }
        });
    }

    let controls = Rect::from_min_max(
        Pos2::new(header.right() - MS_BUTTON_SIZE * 2.0 - 8.0, header.top() + 8.0),
        Pos2::new(header.right() - 4.0, header.bottom() - 8.0),
    );
    ui.allocate_ui_at_rect(controls, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let muted = track_snapshot.muted;
            let solo = track_snapshot.solo;
            if ms_toggle_button(ui, "M", muted, theme) {
                history.push_before(project.clone());
                let exclusive = ui.input(|i| i.modifiers.shift);
                if exclusive {
                    project.exclusive_mute(track_id);
                } else if let Some(track) = project.track_mut(track_id) {
                    track.muted = !track.muted;
                }
                engine.all_notes_off();
            }
            if ms_toggle_button(ui, "S", solo, theme) {
                history.push_before(project.clone());
                let exclusive = ui.input(|i| i.modifiers.shift);
                if exclusive {
                    project.exclusive_solo(track_id);
                } else if let Some(track) = project.track_mut(track_id) {
                    track.solo = !track.solo;
                }
                engine.all_notes_off();
            }
        });
    });

    let track_name = track_snapshot.name.clone();
    let instrument = track_snapshot.instrument.clone();
    let editor_open = engine.plugin_editor_is_open(PluginRef::instrument(track_id));
    let slot_ready = engine.plugin_slot_ready(PluginRef::instrument(track_id));
    let is_plugin = matches!(instrument, TrackInstrument::Plugin { .. });

    header_response.context_menu(|ui| {
        if is_plugin {
            if editor_open {
                if ui.button("Close plugin editor").clicked() {
                    *plugin_editor_request = Some(PluginEditorRequest::Close {
                        track_id,
                        device_id: None,
                    });
                    ui.close_menu();
                }
            } else {
                let label = if slot_ready {
                    "Open plugin editor"
                } else {
                    "Open plugin editor (loading...)"
                };
                if ui
                    .add_enabled(slot_ready, egui::Button::new(label))
                    .clicked()
                {
                    *plugin_editor_request = Some(PluginEditorRequest::Open {
                        track_id,
                        device_id: None,
                        title: track_name.clone(),
                    });
                    ui.close_menu();
                }
            }
            ui.separator();
        }
        if ui.button("Rename track...").clicked() {
            track_rename.begin(track_id, &track_snapshot.name);
            ui.close_menu();
        }
        ui.separator();
        ui.label("Change instrument");
        ui.separator();
        if let Some(choice) = show_instrument_picker(
            ui,
            catalog,
            change_instrument_search,
            &format!("chg_{track_id}"),
            false,
            MENU_LIST_MAX_HEIGHT,
        ) {
            let rename = match &choice {
                InstrumentChoice::Plugin(entry) => Some(entry.name.clone()),
                InstrumentChoice::BuiltInPiano => None,
            };
            let instrument = choice_to_instrument(choice);
            if let Some(track) = project.track_mut(track_id) {
                if let Some(name) = rename {
                    track.name = name;
                }
                // New instrument identity -- drop prior plugin blob.
                track.plugin_state = None;
                track.instrument = instrument;
            }
            change_instrument_search.clear();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Mute").clicked() {
            history.push_before(project.clone());
            if let Some(track) = project.track_mut(track_id) {
                track.muted = !track.muted;
            }
            engine.all_notes_off();
            ui.close_menu();
        }
        if ui.button("Mute exclusive").clicked() {
            history.push_before(project.clone());
            project.exclusive_mute(track_id);
            engine.all_notes_off();
            ui.close_menu();
        }
        if ui.button("Solo").clicked() {
            history.push_before(project.clone());
            if let Some(track) = project.track_mut(track_id) {
                track.solo = !track.solo;
            }
            engine.all_notes_off();
            ui.close_menu();
        }
        if ui.button("Solo exclusive").clicked() {
            history.push_before(project.clone());
            project.exclusive_solo(track_id);
            engine.all_notes_off();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Duplicate track").clicked() {
            *duplicate_track_request = Some(track_id);
            ui.close_menu();
        }
        let can_delete = project.can_remove_track();
        if ui
            .add_enabled(can_delete, egui::Button::new("Delete track"))
            .on_disabled_hover_text("Cannot delete the last track")
            .clicked()
        {
            *delete_track_request = Some(track_id);
            ui.close_menu();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn pattern_lane_header_row(
    ui: &mut Ui,
    header: Rect,
    paint_clip: Rect,
    lane_snapshot: &crate::model::PatternLane,
    lane_id: u64,
    project: &Project,
    _history: &mut EditHistory,
    theme: &ThemeColors,
    is_selected: bool,
    select_pattern_lane_request: &mut Option<u64>,
    delete_pattern_lane_request: &mut Option<u64>,
    duplicate_pattern_lane_request: &mut Option<u64>,
    hovered_pattern_lane_header: &mut Option<u64>,
    pattern_lane_rename: &mut PatternLaneRenameUi,
) {
    PatternStripUi::paint_lane_header(
        &ui.painter().with_clip_rect(paint_clip),
        header,
        lane_snapshot.name.as_str(),
        theme,
    );
    if is_selected {
        ui.painter().rect_stroke(
            header.shrink(1.0),
            0.0,
            egui::Stroke::new(2.0_f32, theme.accent),
            egui::StrokeKind::Inside,
        );
    }

    let id = ui.id().with(("playlist", "pattern_lane_header", lane_id));
    let header_response = ui.interact(header, id, Sense::click());
    if ui.rect_contains_pointer(header) {
        *hovered_pattern_lane_header = Some(lane_id);
    }
    if header_response.clicked() {
        *select_pattern_lane_request = Some(lane_id);
    }

    let name_rect = pattern_lane_header_name_rect(header);
    let name_id = ui.id().with(("playlist", "pattern_lane_name", lane_id));
    let name_response = ui.interact(name_rect, name_id, Sense::click());
    if name_response.double_clicked() {
        pattern_lane_rename.begin(lane_id, &lane_snapshot.name);
    }

    header_response.context_menu(|ui| {
        if ui.button("Rename pattern lane...").clicked() {
            pattern_lane_rename.begin(lane_id, &lane_snapshot.name);
            ui.close_menu();
        }
        if ui.button("Duplicate pattern lane").clicked() {
            *duplicate_pattern_lane_request = Some(lane_id);
            ui.close_menu();
        }
        ui.separator();
        let can_delete = project.can_remove_pattern_lane();
        if ui
            .add_enabled(can_delete, egui::Button::new("Delete pattern lane"))
            .on_disabled_hover_text("Cannot delete the last pattern lane")
            .clicked()
        {
            *delete_pattern_lane_request = Some(lane_id);
            ui.close_menu();
        }
    });
}

fn pattern_lane_header_name_rect(header: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(header.left() + 4.0, header.top() + 4.0),
        Pos2::new(header.right() - 4.0, header.bottom() - 4.0),
    )
}

fn track_header_name_rect(header: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(header.left() + 4.0, header.top() + 4.0),
        Pos2::new(header.right() - MS_BUTTON_SIZE * 2.0 - 12.0, header.top() + 36.0),
    )
}

fn draw_track_header(
    painter: &egui::Painter,
    header: Rect,
    track_name: &str,
    instrument_name: &str,
    format_badge: Option<&str>,
    load_error: Option<&str>,
    theme: &ThemeColors,
) {
    painter.rect_filled(header, 0.0, theme.track_header_bg);
    let badge = format_badge.unwrap_or("Piano");
    let sub = format!("{badge} · {instrument_name}");
    painter.text(
        Pos2::new(header.left() + 6.0, header.top() + 14.0),
        egui::Align2::LEFT_CENTER,
        truncate_label(track_name, 12),
        egui::FontId::proportional(12.0),
        theme.track_header_text,
    );
    painter.text(
        Pos2::new(header.left() + 6.0, header.top() + 30.0),
        egui::Align2::LEFT_CENTER,
        truncate_label(&sub, 16),
        egui::FontId::proportional(10.0),
        theme.text_muted,
    );
    if let Some(error) = load_error {
        painter.text(
            Pos2::new(header.left() + 6.0, header.bottom() - 12.0),
            egui::Align2::LEFT_CENTER,
            truncate_label(error, 24),
            egui::FontId::proportional(9.0),
            theme.accent_warning,
        );
    }
    painter.line_segment(
        [
            Pos2::new(header.left(), header.bottom()),
            Pos2::new(header.right(), header.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.separator),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_lane_timeline(
    painter: &egui::Painter,
    lane: Rect,
    timeline: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
    clips: &[Clip],
    selected: &HashSet<u64>,
    raise_clip_ids: &HashSet<u64>,
    audible: bool,
    bpm: f32,
    decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
    override_windows: &[(f32, f32)],
    theme: &ThemeColors,
) {
    let timeline_lane = Rect::from_min_max(
        Pos2::new(lane.left() + TRACK_HEADER_WIDTH, lane.top()),
        lane.max,
    );
    let lane_fill = if audible {
        theme.lane_bg
    } else {
        theme.lane_bg.gamma_multiply(0.55)
    };
    painter.rect_filled(timeline_lane, 0.0, lane_fill);
    // Use `lane` (same left as body/ruler), not `timeline_lane`: timeline_x already
    // offsets by TIMELINE_GUTTER_WIDTH / TRACK_HEADER_WIDTH.
    draw_timeline_grid_lines(painter, lane, metrics, total_beats, beats_per_bar, theme);

    let mut draw_order: Vec<&Clip> = clips.iter().collect();
    if !raise_clip_ids.is_empty() {
        // Dragged clips paint last so they stay visually on top while overlapping.
        draw_order.sort_by_key(|clip| raise_clip_ids.contains(&clip.id()));
    }

    for clip in draw_order {
        let clip_rect = clip_block_rect(timeline, lane, clip, metrics);
        let is_selected = selected.contains(&clip.id());
        let is_audio = clip.as_audio().is_some();
        let clip_muted = clip.muted();
        let ghosted = audible
            && !clip_muted
            && !is_audio
            && override_windows.iter().any(|(win_start, win_end)| {
                Project::beat_ranges_overlap(
                    clip.start_beats(),
                    clip.end_beats(),
                    *win_start,
                    *win_end,
                )
            });
        let is_linked = clip
            .as_midi()
            .map(|m| m.link_group_id.is_some())
            .unwrap_or(false);
        let fill = if clip_muted {
            if is_selected {
                theme.clip_fill_selected.gamma_multiply(0.35)
            } else if is_audio {
                theme.clip_fill.gamma_multiply(0.4)
            } else {
                theme.lane_bg.gamma_multiply(0.75)
            }
        } else if ghosted {
            theme.clip_ghosted.gamma_multiply(0.42)
        } else if is_linked && !is_selected {
            theme.clip_linked_fill.gamma_multiply(0.9)
        } else if is_selected {
            if is_audio {
                theme.clip_fill_selected.gamma_multiply(0.85)
            } else {
                theme.clip_fill_selected
            }
        } else {
            if is_audio {
                theme.clip_fill.gamma_multiply(0.75)
            } else {
                theme.clip_fill
            }
        };
        let stroke_color = if is_selected {
            theme.clip_stroke_selected
        } else if is_linked {
            theme.clip_linked_stroke
        } else {
            theme.clip_stroke
        };
        painter.rect(
            clip_rect,
            4.0,
            fill,
            egui::Stroke::new(1.5_f32, stroke_color),
            egui::StrokeKind::Inside,
        );

        let label = if let Some(audio) = clip.as_audio() {
            if audio.missing {
                format!("[A] {} (missing)", clip.name())
            } else {
                format!("[A] {}", clip.name())
            }
        } else if let Some(midi) = clip.as_midi() {
            if midi.variations.len() > 1 {
                let take = midi
                    .active_variation()
                    .map(|v| v.name.as_str())
                    .unwrap_or("?");
                format!("[M] {} · {}", clip.name(), take)
            } else {
                format!("[M] {}", clip.name())
            }
        } else {
            format!("[M] {}", clip.name())
        };
        // MIDI clips reserve the top-left for the link control.
        let label_x = if is_audio {
            clip_rect.left() + 6.0
        } else {
            clip_rect.left() + 24.0
        };
        let label_max_w = (clip_rect.right() - label_x - 8.0).max(0.0);
        painter.with_clip_rect(clip_rect).text(
            Pos2::new(label_x, clip_rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            truncate_label_for_width(&label, label_max_w),
            egui::FontId::proportional(11.0),
            theme.clip_label,
        );

        if let Some(audio) = clip.as_audio() {
            draw_clip_waveform(painter, clip_rect, audio, bpm, decoded_audio, theme);
        } else {
            draw_clip_note_preview(painter, clip_rect, clip, theme);
        }
    }

    painter.line_segment(
        [
            Pos2::new(lane.left() + TRACK_HEADER_WIDTH, lane.bottom()),
            Pos2::new(lane.right(), lane.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.separator),
    );
}

/// Approximate character budget for the 11px proportional clip label font.
/// Combined with the `with_clip_rect` call at the label draw site, this is a
/// belt-and-suspenders guard: even if the estimate is slightly off, the hard
/// clip rect stops text from bleeding into a neighboring clip.
const CLIP_LABEL_AVG_CHAR_WIDTH: f32 = 6.0;

/// Top-left link control and top-right take menu own secondary clicks.
fn clip_chrome_blocks_secondary(clip_rect: Rect, pointer: Pos2) -> bool {
    let zone_h = 20.0_f32.min(clip_rect.height());
    let top = Rect::from_min_max(
        Pos2::new(clip_rect.left(), clip_rect.top()),
        Pos2::new(clip_rect.right(), clip_rect.top() + zone_h),
    );
    if !top.contains(pointer) {
        return false;
    }
    pointer.x < clip_rect.left() + 24.0 || pointer.x > clip_rect.right() - 34.0
}

fn truncate_label_for_width(text: &str, available_width: f32) -> String {
    if available_width <= 0.0 {
        return String::new();
    }
    let max_chars = (available_width / CLIP_LABEL_AVG_CHAR_WIDTH).floor().max(1.0) as usize;
    truncate_label(text, max_chars)
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

pub(crate) fn clip_block_rect(
    timeline: Rect,
    lane: Rect,
    clip: &Clip,
    metrics: TimelineMetrics,
) -> Rect {
    let left = timeline_x(timeline, clip.start_beats(), metrics);
    let right = timeline_x(timeline, clip.end_beats(), metrics);
    Rect::from_min_max(
        Pos2::new(left + 1.0, lane.top() + 4.0),
        Pos2::new(right - 1.0, lane.bottom() - 4.0),
    )
}

/// Cap on peak samples scanned per pixel column, and on columns drawn per
/// clip, so a very long/wide audio clip can't blow up per-frame paint cost
/// (this runs every repaint since egui is immediate-mode).
const WAVEFORM_MAX_SAMPLES_PER_COLUMN: usize = 256;
const WAVEFORM_MAX_COLUMNS: usize = 3000;

/// Draws a min/max peak waveform for an audio clip, using whatever PCM is
/// already decoded and cached for playback (`decoded_audio`, keyed by
/// source path - see `App::decoded_audio` / `engine::sample::decode_audio_file`).
/// If the source hasn't been decoded yet (still loading, or missing), this
/// draws nothing - same as a clip with no preview data.
fn draw_clip_waveform(
    painter: &egui::Painter,
    clip_rect: Rect,
    audio: &AudioClip,
    bpm: f32,
    decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
    theme: &ThemeColors,
) {
    let Some(decoded) = decoded_audio.get(&audio.source) else {
        return;
    };
    if decoded.frames == 0 || audio.length_beats <= 0.0 {
        return;
    }
    let bps = (bpm / 60.0).max(0.0001);
    // Frames the clip's visible length would span if the source were long
    // enough to fill it; matches the time mapping used for playback in
    // `AudioEngine` (start/length_beats converted via bpm, not resampled).
    let frames_for_length = (audio.length_beats / bps) * decoded.device_sample_rate as f32;
    if frames_for_length < 1.0 {
        return;
    }
    let visible_frames = frames_for_length.min(decoded.frames as f32);
    // Fraction of the clip block that actually has audio under it - the rest
    // (when length_beats outlasts the source) stays blank, matching playback
    // (silence once the buffer runs out).
    let width_fraction = (visible_frames / frames_for_length).clamp(0.0, 1.0);

    let preview_top = clip_rect.top() + 20.0;
    let preview_height = (clip_rect.height() - 24.0).max(8.0);
    let mid_y = preview_top + preview_height / 2.0;
    let half_height = preview_height / 2.0;

    let content_width = (clip_rect.width() - 8.0).max(1.0);
    let waveform_width = content_width * width_fraction;
    let columns = (waveform_width.round() as usize)
        .clamp(1, WAVEFORM_MAX_COLUMNS);
    let x0 = clip_rect.left() + 4.0;
    let clipped = painter.with_clip_rect(clip_rect);

    for col in 0..columns {
        let start = ((col as f32 / columns as f32) * visible_frames) as usize;
        let end = (((col + 1) as f32 / columns as f32) * visible_frames).ceil() as usize;
        let end = end.max(start + 1).min(decoded.frames);
        let start = start.min(end.saturating_sub(1));
        let step = ((end - start) / WAVEFORM_MAX_SAMPLES_PER_COLUMN).max(1);

        let mut min_v = 0.0_f32;
        let mut max_v = 0.0_f32;
        let mut i = start;
        while i < end {
            let l = decoded.left.get(i).copied().unwrap_or(0.0);
            let r = decoded.right.get(i).copied().unwrap_or(0.0);
            let sample = (l + r) * 0.5;
            min_v = min_v.min(sample);
            max_v = max_v.max(sample);
            i += step;
        }

        let x = x0 + col as f32 * (waveform_width / columns as f32);
        let y_top = mid_y - max_v.clamp(-1.0, 1.0) * half_height;
        let y_bottom = mid_y - min_v.clamp(-1.0, 1.0) * half_height;
        clipped.line_segment(
            [Pos2::new(x, y_top), Pos2::new(x, y_bottom.max(y_top + 1.0))],
            egui::Stroke::new(1.0_f32, theme.clip_note_preview),
        );
    }
}

fn draw_clip_note_preview(
    painter: &egui::Painter,
    clip_rect: Rect,
    clip: &Clip,
    theme: &ThemeColors,
) {
    let Some(clip) = clip.as_midi() else {
        return;
    };
    draw_note_preview(
        painter,
        clip_rect,
        clip.active_notes(),
        clip.length_beats,
        theme,
        &NotePreviewStyle::clip_thumbnail(),
    );
}

pub(crate) fn hit_test_clip<'a>(
    timeline: Rect,
    lane: Rect,
    clips: &'a [Clip],
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<&'a Clip> {
    clips
        .iter()
        .rev()
        .find(|clip| clip_block_rect(timeline, lane, clip, metrics).contains(pos))
}

pub(crate) fn clip_resize_mode(bounds: Rect, pointer_x: f32) -> Option<ClipDragMode> {
    let local_x = pointer_x - bounds.left();
    let width = bounds.width();
    let handle = RESIZE_HANDLE_PX.min(width * 0.35);
    if local_x <= handle {
        Some(ClipDragMode::ResizeStart)
    } else if local_x >= width - handle {
        Some(ClipDragMode::ResizeEnd)
    } else {
        None
    }
}

fn update_clip_resize_hover_cursor(
    response: &Response,
    body: Rect,
    sticky_headers: Rect,
    layout: &TrackLayout,
    project: &Project,
    metrics: TimelineMetrics,
) {
    let Some(hover) = response.hover_pos() else {
        return;
    };
    if !body.contains(hover) || sticky_headers.contains(hover) {
        return;
    }
    let Some((track_index, in_clip_lane)) = layout.track_at_y(body, hover.y) else {
        return;
    };
    if !in_clip_lane {
        return;
    }
    let Some(lane) = layout.clip_lane_rect(body, track_index) else {
        return;
    };
    let Some(clip) = hit_test_clip(
        body,
        lane,
        &project.tracks[track_index].clips,
        hover,
        metrics,
    ) else {
        return;
    };
    let bounds = clip_block_rect(body, lane, clip, metrics);
    if clip_resize_mode(bounds, hover.x).is_some() {
        response
            .ctx
            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

/// Fixed-height clip lane for single-track views (devices mini-playlist).
#[allow(dead_code)]
pub(crate) fn lane_rect_for_track(body: Rect, track_index: usize) -> Rect {
    let lane_top = body.top() + track_index as f32 * LANE_HEIGHT;
    Rect::from_min_max(
        Pos2::new(body.left(), lane_top),
        Pos2::new(body.right(), lane_top + LANE_HEIGHT),
    )
}

fn select_clips_in_rect(
    body: Rect,
    layout: &TrackLayout,
    project: &Project,
    selection: Rect,
    metrics: TimelineMetrics,
) -> HashSet<u64> {
    project
        .tracks
        .iter()
        .enumerate()
        .flat_map(|(index, track)| {
            let lane = layout.clip_lane_rect(body, index)?;
            Some(
                track
                    .clips
                    .iter()
                    .filter_map(move |clip| {
                        let clip_rect = clip_block_rect(body, lane, clip, metrics);
                        clip_rect.intersects(selection).then_some(clip.id())
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

fn select_clips_in_rect_single_track(
    body: Rect,
    lane: Rect,
    clips: &[Clip],
    selection: Rect,
    metrics: TimelineMetrics,
) -> HashSet<u64> {
    clips
        .iter()
        .filter(|clip| {
            clip_block_rect(body, lane, clip, metrics).intersects(selection)
        })
        .map(|clip| clip.id())
        .collect()
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

fn sync_selected_track_from_clips(
    project: &Project,
    selected: &HashSet<u64>,
    selected_track: &mut Option<u64>,
) {
    if let Some(&first_id) = selected.iter().next() {
        if let Some(track_id) = project.track_id_for_clip(first_id) {
            *selected_track = Some(track_id);
        }
    }
}

fn clear_all_pattern_strips(strips: &mut HashMap<u64, PatternStripUi>) {
    for strip in strips.values_mut() {
        strip.clear_selection();
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_clip_pointer(
    response: &Response,
    body: Rect,
    sticky_headers: Rect,
    layout: &TrackLayout,
    metrics: TimelineMetrics,
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    active_drag: &mut Option<ClipDrag>,
    marquee: &mut Option<MarqueeDrag>,
    open_clip_request: &mut Option<u64>,
    drag_moved: &mut bool,
    selected_track: &mut Option<u64>,
    pattern_strips: &mut HashMap<u64, PatternStripUi>,
) {
    update_clip_resize_hover_cursor(response, body, sticky_headers, layout, project, metrics);

    let primary_down = response
        .ctx
        .input(|input| input.pointer.button_down(egui::PointerButton::Primary));

    // End clip/marquee drags even when the pointer left the sense area.
    let end_drag = response.drag_stopped()
        || (!primary_down && (active_drag.is_some() || marquee.is_some()));
    if end_drag {
        if let Some(drag) = active_drag.take() {
            finish_clip_drag(project, history, selected, &drag, *drag_moved);
            *selected_track = Some(drag.track_id);
        }
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
    let (shift_held, ctrl_or_cmd) = response.ctx.input(|input| {
        (
            input.modifiers.shift,
            input.modifiers.ctrl || input.modifiers.command || input.modifiers.mac_cmd,
        )
    });

    if let Some(drag) = active_drag.clone() {
        if primary_down && (response.dragged() || response.drag_started()) {
            *drag_moved = true;
            let current_beats = x_to_beat(body, pointer.x, metrics);
            if matches!(
                drag.mode,
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd
            ) {
                response
                    .ctx
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            apply_clip_drag(project, &drag, current_beats);
        }
        return;
    }

    // Keep marquee alive / updating even outside body bounds.
    if let Some(active_marquee) = marquee.as_mut() {
        if primary_down {
            active_marquee.current = pointer;
            *selected =
                select_clips_in_rect(body, layout, project, active_marquee.rect(), metrics);
            sync_selected_track_from_clips(project, selected, selected_track);
            clear_all_pattern_strips(pattern_strips);
        }
        return;
    }

    // Sticky track headers own this column; do not treat it as empty-lane / clip hits.
    if sticky_headers.contains(press_pos) || sticky_headers.contains(pointer) {
        return;
    }

    if !body.contains(pointer) && !body.contains(press_pos) {
        return;
    }

    // Find which track clip-lane was hit (ignore automation fold-out rows).
    let Some((track_index, in_clip_lane)) = layout.track_at_y(body, press_pos.y) else {
        return;
    };
    if !in_clip_lane {
        return;
    }
    let track_id = project.tracks[track_index].id;
    let Some(lane) = layout.clip_lane_rect(body, track_index) else {
        return;
    };

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(lane, press_pos)
    {
        if let Some(clip) = hit_test_clip(
            body,
            lane,
            &project.tracks[track_index].clips,
            press_pos,
            metrics,
        )
        .cloned()
        {
            *marquee = None;

            let bounds = clip_block_rect(body, lane, &clip, metrics);
            let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);

            let already_selected = selected.contains(&clip.id());
            if !already_selected {
                selected.clear();
                selected.insert(clip.id());
            }
            clear_all_pattern_strips(pattern_strips);
            *selected_track = Some(track_id);

            // Snapshot before Shift-duplicate so one undo covers dup+move.
            history.begin(project);

            let mut primary_id = clip.id();
            let mut ignore_ids = Vec::new();
            // Shift+Move: leave originals, drag duplicates (same as piano-roll notes).
            if matches!(mode, ClipDragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected.iter().copied().collect();
                ignore_ids = source_ids.clone();
                let created = project.duplicate_clips(&source_ids, 0.0, true);
                if let Some((_, mapped_primary)) =
                    created.iter().find(|(src, _)| *src == clip.id())
                {
                    primary_id = *mapped_primary;
                } else if let Some((_, first)) = created.first() {
                    primary_id = *first;
                }
                selected.clear();
                selected.extend(created.into_iter().map(|(_, id)| id));
            }

            let originals = match mode {
                ClipDragMode::Move => selected
                    .iter()
                    .filter_map(|id| {
                        project.clip(*id).map(|c| ClipOriginal {
                            clip_id: c.id(),
                            start_beats: c.start_beats(),
                            length_beats: c.length_beats(),
                        })
                    })
                    .collect(),
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd => project
                    .clip(primary_id)
                    .map(|c| {
                        vec![ClipOriginal {
                            clip_id: c.id(),
                            start_beats: c.start_beats(),
                            length_beats: c.length_beats(),
                        }]
                    })
                    .unwrap_or_default(),
            };

            *active_drag = Some(ClipDrag {
                clip_id: primary_id,
                track_id,
                mode,
                pointer_start_beats: x_to_beat(body, press_pos.x, metrics),
                originals,
                ignore_ids,
            });
            return;
        }

        // Empty lane: Ctrl+drag marquee multi-select
        if is_timeline_pointer(lane, press_pos) && ctrl_or_cmd {
            *active_drag = None;
            selected.clear();
            *marquee = Some(MarqueeDrag {
                start: press_pos,
                current: pointer,
            });
            *selected = select_clips_in_rect(
                body,
                layout,
                project,
                Rect::from_two_pos(press_pos, pointer),
                metrics,
            );
            sync_selected_track_from_clips(project, selected, selected_track);
        }
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if let Some(clip) = hit_test_clip(
            body,
            lane,
            &project.tracks[track_index].clips,
            pointer,
            metrics,
        ) {
            let clip_rect = clip_block_rect(body, lane, clip, metrics);
            // Link / take chrome own secondary clicks (join menu / no-op).
            if clip.as_midi().is_some() && clip_chrome_blocks_secondary(clip_rect, pointer) {
                return;
            }
            let clip_id = clip.id();
            let before = project.clone();
            project.remove_clip(clip_id);
            history.push_before(before);
            selected.remove(&clip_id);
            clear_all_pattern_strips(pattern_strips);
            return;
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && is_timeline_pointer(lane, pointer)
    {
        if let Some(clip) = hit_test_clip(
            body,
            lane,
            &project.tracks[track_index].clips,
            pointer,
            metrics,
        ) {
            if ctrl_or_cmd {
                // Toggle multi-select without opening (parity with selection editing).
                if !selected.remove(&clip.id()) {
                    selected.insert(clip.id());
                }
            } else {
                selected.clear();
                selected.insert(clip.id());
            }
            clear_all_pattern_strips(pattern_strips);
            *selected_track = Some(track_id);
        } else {
            let start = Project::snap_beats(x_to_beat(body, pointer.x, metrics).max(0.0));
            let before = project.clone();
            if let Some(clip_id) =
                project.add_clip_to_track(track_id, start, DEFAULT_CLIP_LENGTH_BEATS)
            {
                history.push_before(before);
                selected.clear();
                selected.insert(clip_id);
                clear_all_pattern_strips(pattern_strips);
                *selected_track = Some(track_id);
            }
        }
    }

    if response.double_clicked_by(egui::PointerButton::Primary) && body.contains(pointer) {
        if let Some(clip_id) = hit_test_clip_id(body, layout, project, pointer, metrics) {
            selected.clear();
            selected.insert(clip_id);
            clear_all_pattern_strips(pattern_strips);
            *selected_track = Some(track_id);
            *open_clip_request = Some(clip_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_single_track_clip_pointer(
    response: &Response,
    body: Rect,
    lane: Rect,
    track_id: u64,
    clips: &[Clip],
    metrics: TimelineMetrics,
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    active_drag: &mut Option<ClipDrag>,
    marquee: &mut Option<MarqueeDrag>,
    open_clip_request: &mut Option<u64>,
    drag_moved: &mut bool,
) {
    if let Some(hover) = response.hover_pos() {
        if body.contains(hover) {
            if let Some(clip) = hit_test_clip(body, lane, clips, hover, metrics) {
                let bounds = clip_block_rect(body, lane, clip, metrics);
                if clip_resize_mode(bounds, hover.x).is_some() {
                    response
                        .ctx
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
    }

    let primary_down = response
        .ctx
        .input(|input| input.pointer.button_down(egui::PointerButton::Primary));

    let end_drag = response.drag_stopped()
        || (!primary_down && (active_drag.is_some() || marquee.is_some()));
    if end_drag {
        if let Some(drag) = active_drag.take() {
            finish_clip_drag(project, history, selected, &drag, *drag_moved);
        }
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
    let (shift_held, ctrl_or_cmd) = response.ctx.input(|input| {
        (
            input.modifiers.shift,
            input.modifiers.ctrl || input.modifiers.command || input.modifiers.mac_cmd,
        )
    });

    if let Some(drag) = active_drag.clone() {
        if primary_down && (response.dragged() || response.drag_started()) {
            *drag_moved = true;
            let current_beats = x_to_beat(body, pointer.x, metrics);
            if matches!(
                drag.mode,
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd
            ) {
                response
                    .ctx
                    .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            apply_clip_drag(project, &drag, current_beats);
        }
        return;
    }

    if let Some(active_marquee) = marquee.as_mut() {
        if primary_down {
            active_marquee.current = pointer;
            *selected = select_clips_in_rect_single_track(
                body,
                lane,
                clips,
                active_marquee.rect(),
                metrics,
            );
        }
        return;
    }

    if !body.contains(pointer) && !body.contains(press_pos) {
        return;
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(lane, press_pos)
    {
        if let Some(clip) = hit_test_clip(body, lane, clips, press_pos, metrics).cloned() {
            *marquee = None;

            let bounds = clip_block_rect(body, lane, &clip, metrics);
            let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);

            let already_selected = selected.contains(&clip.id());
            if !already_selected {
                selected.clear();
                selected.insert(clip.id());
            }

            history.begin(project);

            let mut primary_id = clip.id();
            let mut ignore_ids = Vec::new();
            if matches!(mode, ClipDragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected.iter().copied().collect();
                ignore_ids = source_ids.clone();
                let created = project.duplicate_clips(&source_ids, 0.0, true);
                if let Some((_, mapped_primary)) =
                    created.iter().find(|(src, _)| *src == clip.id())
                {
                    primary_id = *mapped_primary;
                } else if let Some((_, first)) = created.first() {
                    primary_id = *first;
                }
                selected.clear();
                selected.extend(created.into_iter().map(|(_, id)| id));
            }

            let originals = match mode {
                ClipDragMode::Move => selected
                    .iter()
                    .filter_map(|id| {
                        project.clip(*id).map(|c| ClipOriginal {
                            clip_id: c.id(),
                            start_beats: c.start_beats(),
                            length_beats: c.length_beats(),
                        })
                    })
                    .collect(),
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd => project
                    .clip(primary_id)
                    .map(|c| {
                        vec![ClipOriginal {
                            clip_id: c.id(),
                            start_beats: c.start_beats(),
                            length_beats: c.length_beats(),
                        }]
                    })
                    .unwrap_or_default(),
            };

            *active_drag = Some(ClipDrag {
                clip_id: primary_id,
                track_id,
                mode,
                pointer_start_beats: x_to_beat(body, press_pos.x, metrics),
                originals,
                ignore_ids,
            });
            return;
        }

        if is_timeline_pointer(lane, press_pos) && ctrl_or_cmd {
            *active_drag = None;
            selected.clear();
            *marquee = Some(MarqueeDrag {
                start: press_pos,
                current: pointer,
            });
            *selected = select_clips_in_rect_single_track(
                body,
                lane,
                clips,
                Rect::from_two_pos(press_pos, pointer),
                metrics,
            );
        }
    }

    if response.clicked_by(egui::PointerButton::Secondary) && !response.dragged() {
        if let Some(clip) = hit_test_clip(body, lane, clips, pointer, metrics) {
            let clip_rect = clip_block_rect(body, lane, clip, metrics);
            if clip.as_midi().is_some() && clip_chrome_blocks_secondary(clip_rect, pointer) {
                return;
            }
            let clip_id = clip.id();
            let before = project.clone();
            project.remove_clip(clip_id);
            history.push_before(before);
            selected.remove(&clip_id);
            return;
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && is_timeline_pointer(lane, pointer)
    {
        if let Some(clip) = hit_test_clip(body, lane, clips, pointer, metrics) {
            if ctrl_or_cmd {
                if !selected.remove(&clip.id()) {
                    selected.insert(clip.id());
                }
            } else {
                selected.clear();
                selected.insert(clip.id());
            }
        } else {
            let start = Project::snap_beats(x_to_beat(body, pointer.x, metrics).max(0.0));
            let before = project.clone();
            if let Some(clip_id) =
                project.add_clip_to_track(track_id, start, DEFAULT_CLIP_LENGTH_BEATS)
            {
                history.push_before(before);
                selected.clear();
                selected.insert(clip_id);
            }
        }
    }

    if response.double_clicked_by(egui::PointerButton::Primary) && body.contains(pointer) {
        if let Some(clip) = hit_test_clip(body, lane, clips, pointer, metrics) {
            selected.clear();
            selected.insert(clip.id());
            *open_clip_request = clip.as_midi().map(|_| clip.id());
        }
    }
}

fn hit_test_clip_id(
    body: Rect,
    layout: &TrackLayout,
    project: &Project,
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<u64> {
    let (track_index, in_clip_lane) = layout.track_at_y(body, pos.y)?;
    if !in_clip_lane {
        return None;
    }
    let lane = layout.clip_lane_rect(body, track_index)?;
    hit_test_clip(body, lane, &project.tracks[track_index].clips, pos, metrics).and_then(|clip| {
        if clip.as_midi().is_some() {
            Some(clip.id())
        } else {
            None
        }
    })
}

fn finish_clip_drag(
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    drag: &ClipDrag,
    drag_moved: bool,
) {
    // Shift+click without move: discard stacked copies.
    if !drag.ignore_ids.is_empty() && !drag_moved {
        history.abort(project);
        selected.clear();
        selected.extend(drag.ignore_ids.iter().copied());
        return;
    }
    if drag_moved && matches!(drag.mode, ClipDragMode::Move) {
        let moved_ids: Vec<u64> = drag.originals.iter().map(|original| original.clip_id).collect();
        project.resolve_clip_move_overlaps(&moved_ids);
    }
    history.commit(project);
    if !drag_moved {
        selected.clear();
        selected.insert(drag.clip_id);
    }
}

pub(crate) fn apply_clip_drag(project: &mut Project, drag: &ClipDrag, current_beats: f32) {
    match drag.mode {
        ClipDragMode::Move => {
            let Some(primary) = drag
                .originals
                .iter()
                .find(|original| original.clip_id == drag.clip_id)
            else {
                return;
            };
            let raw_delta = current_beats - drag.pointer_start_beats;
            let desired_delta =
                Project::snap_beats(primary.start_beats + raw_delta).max(0.0) - primary.start_beats;
            let originals: Vec<(u64, f32, f32)> = drag
                .originals
                .iter()
                .map(|original| {
                    (
                        original.clip_id,
                        original.start_beats,
                        original.length_beats,
                    )
                })
                .collect();
            let snapped_delta =
                project.clamp_clip_move_delta(&originals, desired_delta, &drag.ignore_ids);
            for original in &drag.originals {
                if let Some(clip) = project.clip_mut(original.clip_id) {
                    clip.set_start_beats((original.start_beats + snapped_delta).max(0.0));
                }
            }
        }
        ClipDragMode::ResizeStart => {
            let Some(original) = drag.originals.first() else {
                return;
            };
            let end = original.start_beats + original.length_beats;
            let left_bound = project.clip_resize_start_bound(drag.clip_id, original.start_beats);
            let new_start = Project::snap_beats(current_beats.max(0.0));
            let clamped_start = new_start
                .max(left_bound)
                .min(end - SNAP_BEATS)
                .max(0.0);
            let Some(clip) = project.clip_mut(drag.clip_id) else {
                return;
            };
            clip.set_start_beats(clamped_start);
            clip.set_length_beats(end - clamped_start);
        }
        ClipDragMode::ResizeEnd => {
            let Some(original) = drag.originals.first() else {
                return;
            };
            let right_bound = project.clip_resize_end_bound(
                drag.clip_id,
                original.start_beats + original.length_beats,
            );
            let new_end = Project::snap_beats(current_beats.max(0.0));
            let clamped_end = new_end
                .min(right_bound)
                .max(original.start_beats + SNAP_BEATS);
            let Some(clip) = project.clip_mut(drag.clip_id) else {
                return;
            };
            clip.set_start_beats(original.start_beats);
            clip.set_length_beats(clamped_end - original.start_beats);
        }
    }
}
