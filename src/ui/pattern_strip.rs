//! Pattern lane strip under the playlist (section MIDI overrides).

use std::collections::HashSet;

use egui::{Pos2, Rect, Response, Vec2};

use crate::model::{
    EditHistory, PatternBlock, Project, DEFAULT_CLIP_LENGTH_BEATS, SNAP_BEATS,
};
use crate::ui::playlist::{clip_resize_mode, ClipDragMode, MarqueeDrag, TRACK_HEADER_WIDTH};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    draw_timeline_grid_lines, is_timeline_pointer, timeline_x, x_to_beat, TimelineMetrics,
};

pub const PATTERN_STRIP_HEIGHT: f32 = 40.0;
pub const PATTERN_STRIP_GAP: f32 = 4.0;
pub const PATTERN_STRIP_LANE_GAP: f32 = 2.0;
pub const PATTERN_PRIORITY_ROW_HEIGHT: f32 = 26.0;
pub const ADD_PATTERN_LANE_ROW_HEIGHT: f32 = 28.0;
const SOLO_BUTTON_WIDTH: f32 = 18.0;

#[derive(Debug, Clone)]
struct PatternBlockOriginal {
    block_id: u64,
    start_beats: f32,
    length_beats: f32,
}

#[derive(Debug, Clone)]
struct PatternBlockDrag {
    lane_id: u64,
    block_id: u64,
    mode: ClipDragMode,
    pointer_start_beats: f32,
    originals: Vec<PatternBlockOriginal>,
    ignore_ids: Vec<u64>,
}

impl PatternBlockDrag {
    fn moving_block_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.originals.iter().map(|original| original.block_id)
    }
}

pub struct PatternStripUi {
    selected_block_ids: HashSet<u64>,
    active_drag: Option<PatternBlockDrag>,
    marquee: Option<MarqueeDrag>,
    drag_moved: bool,
    /// Stub for Phase D rack editor.
    open_block_request: Option<u64>,
}

impl Default for PatternStripUi {
    fn default() -> Self {
        Self {
            selected_block_ids: HashSet::new(),
            active_drag: None,
            marquee: None,
            drag_moved: false,
            open_block_request: None,
        }
    }
}

impl PatternStripUi {
    pub fn selected_block_ids(&self) -> &HashSet<u64> {
        &self.selected_block_ids
    }

    pub fn has_selection(&self) -> bool {
        !self.selected_block_ids.is_empty()
    }

    pub fn clear_selection(&mut self) {
        self.selected_block_ids.clear();
    }

    pub fn set_selection(&mut self, block_ids: impl IntoIterator<Item = u64>) {
        self.selected_block_ids.clear();
        self.selected_block_ids.extend(block_ids);
    }

    pub fn prune_selection(&mut self, project: &Project) {
        self.selected_block_ids
            .retain(|id| project.pattern_block(*id).is_some());
    }

    pub fn take_open_block_request(&mut self) -> Option<u64> {
        self.open_block_request.take()
    }

    pub fn gesture_active(&self) -> bool {
        self.active_drag.is_some() || self.marquee.is_some()
    }

    /// Total vertical space used by `lane_count` pattern strips plus the add-lane row.
    pub fn pattern_lanes_area_height(lane_count: usize) -> f32 {
        let strips = if lane_count == 0 {
            PATTERN_STRIP_GAP + ADD_PATTERN_LANE_ROW_HEIGHT
        } else {
            PATTERN_STRIP_GAP
                + lane_count as f32 * PATTERN_STRIP_HEIGHT
                + (lane_count.saturating_sub(1)) as f32 * PATTERN_STRIP_LANE_GAP
                + PATTERN_STRIP_GAP
                + ADD_PATTERN_LANE_ROW_HEIGHT
        };
        PATTERN_PRIORITY_ROW_HEIGHT + strips
    }

    pub fn priority_row_rect(body: Rect, track_area_height: f32) -> Rect {
        let top = body.top() + track_area_height;
        Rect::from_min_max(
            Pos2::new(body.left(), top),
            Pos2::new(body.right(), top + PATTERN_PRIORITY_ROW_HEIGHT),
        )
    }

    /// Y offset of one pattern strip below track lanes (lane 0 = top / highest priority).
    pub fn strip_top(track_area_height: f32, lane_index: usize) -> f32 {
        track_area_height
            + PATTERN_PRIORITY_ROW_HEIGHT
            + PATTERN_STRIP_GAP
            + lane_index as f32 * (PATTERN_STRIP_HEIGHT + PATTERN_STRIP_LANE_GAP)
    }

    pub fn strip_rect(body: Rect, track_area_height: f32, lane_index: usize) -> Rect {
        let top = body.top() + Self::strip_top(track_area_height, lane_index);
        Rect::from_min_max(
            Pos2::new(body.left(), top),
            Pos2::new(body.right(), top + PATTERN_STRIP_HEIGHT),
        )
    }

    pub fn contains_y(body: Rect, track_area_height: f32, lane_index: usize, y: f32) -> bool {
        Self::strip_rect(body, track_area_height, lane_index).contains(Pos2::new(body.left(), y))
    }

    pub fn add_lane_row_rect(body: Rect, track_area_height: f32, lane_count: usize) -> Rect {
        let top = body.top()
            + Self::strip_top(track_area_height, lane_count.saturating_sub(1).max(0))
            + if lane_count == 0 {
                0.0
            } else {
                PATTERN_STRIP_HEIGHT + PATTERN_STRIP_GAP
            };
        Rect::from_min_max(
            Pos2::new(body.left(), top),
            Pos2::new(body.right(), top + ADD_PATTERN_LANE_ROW_HEIGHT),
        )
    }

    /// Timeline half of the play-priority divider between track lanes and pattern strips.
    pub fn paint_priority_timeline(
        painter: &egui::Painter,
        row: Rect,
        patterns_override: bool,
        theme: &ThemeColors,
    ) {
        let bg = if patterns_override {
            theme.accent.gamma_multiply(0.10)
        } else {
            theme.panel_bg.gamma_multiply(0.92)
        };
        painter.rect_filled(row, 0.0, bg);
        painter.line_segment(
            [row.left_top(), row.right_top()],
            egui::Stroke::new(1.0_f32, theme.separator),
        );
        painter.line_segment(
            [row.left_bottom(), row.right_bottom()],
            egui::Stroke::new(1.0_f32, theme.separator),
        );
        let hint_rect = Rect::from_min_max(
            Pos2::new(row.left() + TRACK_HEADER_WIDTH + 6.0, row.top()),
            row.max,
        );
        let hint = if patterns_override {
            "Patterns override playlist MIDI"
        } else {
            "Playlist MIDI wins - pattern rows are draft until bake or solo"
        };
        painter.with_clip_rect(hint_rect).text(
            Pos2::new(hint_rect.left(), hint_rect.center().y),
            egui::Align2::LEFT_CENTER,
            hint,
            egui::FontId::proportional(10.0),
            theme.text_muted,
        );
    }

    /// Header-column toggle: Playlist vs Patterns playback priority.
    pub fn show_priority_header(
        ui: &mut egui::Ui,
        header: Rect,
        clip: Rect,
        project: &mut Project,
        theme: &ThemeColors,
    ) {
        let painter = ui.painter().with_clip_rect(clip);
        let patterns_win = project.pattern_overrides_playlist;
        painter.rect_filled(header, 0.0, theme.track_header_bg);
        let gap = 3.0;
        let btn_w = (header.width() - gap * 3.0) / 2.0;
        let btn_h = header.height() - 8.0;
        let playlist_btn = Rect::from_center_size(
            Pos2::new(header.left() + gap + btn_w * 0.5, header.center().y),
            Vec2::new(btn_w, btn_h),
        );
        let patterns_btn = Rect::from_center_size(
            Pos2::new(header.right() - gap - btn_w * 0.5, header.center().y),
            Vec2::new(btn_w, btn_h),
        );

        for (rect, label, active, value) in [
            (playlist_btn, "Playlist", !patterns_win, false),
            (patterns_btn, "Patterns", patterns_win, true),
        ] {
            let fill = if active {
                theme.accent.gamma_multiply(0.35)
            } else {
                theme.widget_bg
            };
            let stroke = if active {
                theme.accent
            } else {
                theme.separator
            };
            painter.rect(
                rect,
                3.0,
                fill,
                egui::Stroke::new(1.0_f32, stroke),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(10.0),
                if active {
                    theme.text_primary
                } else {
                    theme.text_muted
                },
            );
            let id = ui.id().with(("pattern_play_priority", label));
            let response = ui.interact(rect, id, egui::Sense::click());
            if response.clicked() && project.pattern_overrides_playlist != value {
                project.pattern_overrides_playlist = value;
            }
            let hover = if value {
                "Pattern rows replace playlist MIDI in their windows"
            } else {
                "Playlist clips play; pattern rows are draft until bake or block solo"
            };
            response.on_hover_text(hover);
        }
    }

    /// Lane label in the fixed header column (side-by-side playlist layout).
    pub fn paint_lane_header(
        painter: &egui::Painter,
        header: Rect,
        lane_label: &str,
        theme: &ThemeColors,
    ) {
        painter.rect_filled(header, 0.0, theme.track_header_bg);
        painter.text(
            Pos2::new(header.left() + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            lane_label,
            egui::FontId::proportional(12.0),
            theme.track_header_text,
        );
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            egui::Stroke::new(1.0_f32, theme.separator),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        painter: &egui::Painter,
        strip: Rect,
        body: Rect,
        metrics: TimelineMetrics,
        total_beats: f32,
        beats_per_bar: f32,
        project: &Project,
        lane_id: u64,
        theme: &ThemeColors,
    ) {
        let blocks = project
            .pattern_lane(lane_id)
            .map(|lane| lane.blocks.as_slice())
            .unwrap_or(&[]);
        // `strip`/`body` are beat-mapped with a virtual left gutter (shifted), so
        // the visible timeline starts one header-width in from strip.left().
        let timeline_lane = Rect::from_min_max(
            Pos2::new(strip.left() + TRACK_HEADER_WIDTH, strip.top()),
            strip.max,
        );
        painter.rect_filled(timeline_lane, 0.0, theme.lane_bg.gamma_multiply(0.92));
        draw_timeline_grid_lines(
            painter,
            strip,
            metrics,
            total_beats,
            beats_per_bar,
            theme,
        );

        let raise_ids: HashSet<u64> = self
            .active_drag
            .as_ref()
            .map(|drag| drag.moving_block_ids().collect())
            .unwrap_or_default();

        let mut draw_order: Vec<&PatternBlock> = blocks.iter().collect();
        if !raise_ids.is_empty() {
            draw_order.sort_by_key(|block| raise_ids.contains(&block.id));
        }

        for block in draw_order {
            let block_rect = pattern_block_rect(body, strip, block, metrics);
            let selected = self.selected_block_ids.contains(&block.id);
            let ghosted = !project.pattern_overrides_playlist
                && block.has_override_notes()
                && !block.solo;
            let fill = if ghosted {
                if selected {
                    theme.pattern_block_fill_selected.gamma_multiply(0.42)
                } else {
                    theme.clip_ghosted.gamma_multiply(0.42)
                }
            } else if selected {
                theme.pattern_block_fill_selected
            } else {
                theme.pattern_block_fill
            };
            painter.rect(
                block_rect,
                4.0,
                fill,
                egui::Stroke::new(
                    1.5_f32,
                    if selected {
                        theme.pattern_block_stroke_selected
                    } else {
                        theme.pattern_block_stroke
                    },
                ),
                egui::StrokeKind::Inside,
            );

            let solo_rect = pattern_solo_button_rect(block_rect);
            let solo_fill = if block.solo {
                theme.pattern_block_solo
            } else {
                theme.widget_bg
            };
            painter.rect_filled(solo_rect, 2.0, solo_fill);
            painter.text(
                solo_rect.center(),
                egui::Align2::CENTER_CENTER,
                "S",
                egui::FontId::proportional(10.0),
                if block.solo {
                    theme.panel_bg
                } else {
                    theme.text_muted
                },
            );

            let label = format!("[P] {}", block.name);
            let label_clip = block_rect.shrink2(Vec2::new(4.0, 2.0));
            painter.with_clip_rect(label_clip).text(
                Pos2::new(label_clip.left() + 4.0, label_clip.top() + 2.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(11.0),
                theme.pattern_block_label,
            );
        }

        if let Some(marquee) = &self.marquee {
            let rect = marquee.rect();
            painter.rect_filled(rect, 0.0, theme.marquee_fill);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0_f32, theme.marquee_stroke),
                egui::StrokeKind::Outside,
            );
        }

        painter.line_segment(
            [
                Pos2::new(strip.left() + TRACK_HEADER_WIDTH, strip.bottom()),
                Pos2::new(strip.right(), strip.bottom()),
            ],
            egui::Stroke::new(1.0_f32, theme.separator),
        );

        let _ = lane_id;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn handle_pointer(
        &mut self,
        response: &Response,
        body: Rect,
        strip: Rect,
        lane_id: u64,
        metrics: TimelineMetrics,
        project: &mut Project,
        history: &mut EditHistory,
        clip_selection: &mut HashSet<u64>,
    ) {
        update_pattern_resize_hover_cursor(
            response,
            body,
            strip,
            lane_blocks(project, lane_id),
            metrics,
        );

        let primary_down = response
            .ctx
            .input(|input| input.pointer.button_down(egui::PointerButton::Primary));

        let end_drag = response.drag_stopped()
            || (!primary_down && (self.active_drag.is_some() || self.marquee.is_some()));
        if end_drag {
            if let Some(drag) = self.active_drag.take() {
                finish_pattern_drag(project, history, &mut self.selected_block_ids, &drag, self.drag_moved);
            }
            self.marquee = None;
            self.drag_moved = false;
        }

        let Some(pointer) = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos())
            .or_else(|| response.ctx.pointer_interact_pos())
        else {
            return;
        };

        if !strip.contains(pointer) && !strip.contains(
            response
                .ctx
                .input(|input| input.pointer.press_origin())
                .unwrap_or(pointer),
        ) {
            return;
        }

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

        if let Some(solo_block) = {
            let blocks = lane_blocks(project, lane_id);
            hit_test_solo_button(body, strip, blocks, press_pos, metrics)
        } {
            if response.clicked_by(egui::PointerButton::Primary) {
                let before = project.clone();
                project.toggle_pattern_block_solo(solo_block);
                history.push_before(before);
                clip_selection.clear();
                self.set_selection([solo_block]);
            }
            return;
        }

        if let Some(drag) = self.active_drag.clone() {
            if primary_down && (response.dragged() || response.drag_started()) {
                self.drag_moved = true;
                let current_beats = x_to_beat(body, pointer.x, metrics);
                if matches!(
                    drag.mode,
                    ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd
                ) {
                    response
                        .ctx
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                apply_pattern_drag(project, &drag, current_beats);
            }
            return;
        }

        if let Some(active_marquee) = self.marquee.as_mut() {
            if primary_down {
                active_marquee.set_current(pointer);
                self.selected_block_ids = select_blocks_in_rect(
                    body,
                    strip,
                    lane_blocks(project, lane_id),
                    active_marquee.rect(),
                    metrics,
                );
                clip_selection.clear();
            }
            return;
        }

        if response.drag_started_by(egui::PointerButton::Primary)
            && is_timeline_pointer(strip, press_pos)
        {
            let hit_block = {
                let blocks = lane_blocks(project, lane_id);
                hit_test_block(body, strip, blocks, press_pos, metrics).cloned()
            };
            if let Some(block) = hit_block {
                self.marquee = None;
                clip_selection.clear();

                let bounds = pattern_block_rect(body, strip, &block, metrics);
                let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);

                let already_selected = self.selected_block_ids.contains(&block.id);
                if !already_selected {
                    self.selected_block_ids.clear();
                    self.selected_block_ids.insert(block.id);
                }

                history.begin(project);

                let mut primary_id = block.id;
                let mut ignore_ids = Vec::new();
                if matches!(mode, ClipDragMode::Move) && shift_held {
                    let source_ids: Vec<u64> =
                        self.selected_block_ids.iter().copied().collect();
                    ignore_ids = source_ids.clone();
                    let created =
                        project.duplicate_pattern_blocks(&source_ids, 0.0, true);
                    if let Some((_, mapped_primary)) =
                        created.iter().find(|(src, _)| *src == block.id)
                    {
                        primary_id = *mapped_primary;
                    } else if let Some((_, first)) = created.first() {
                        primary_id = *first;
                    }
                    self.selected_block_ids.clear();
                    self.selected_block_ids
                        .extend(created.into_iter().map(|(_, id)| id));
                }

                let originals = match mode {
                    ClipDragMode::Move => self
                        .selected_block_ids
                        .iter()
                        .filter_map(|id| {
                            project.pattern_block(*id).map(|block| PatternBlockOriginal {
                                block_id: block.id,
                                start_beats: block.start_beats,
                                length_beats: block.length_beats,
                            })
                        })
                        .collect(),
                    ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd => project
                        .pattern_block(primary_id)
                        .map(|block| {
                            vec![PatternBlockOriginal {
                                block_id: block.id,
                                start_beats: block.start_beats,
                                length_beats: block.length_beats,
                            }]
                        })
                        .unwrap_or_default(),
                };

                self.active_drag = Some(PatternBlockDrag {
                    lane_id,
                    block_id: primary_id,
                    mode,
                    pointer_start_beats: x_to_beat(body, press_pos.x, metrics),
                    originals,
                    ignore_ids,
                });
                return;
            }

            if is_timeline_pointer(strip, press_pos) {
                self.active_drag = None;
                self.selected_block_ids.clear();
                clip_selection.clear();
                self.marquee = Some(MarqueeDrag::new(press_pos, pointer));
                let marquee_rect = self.marquee.as_ref().unwrap().rect();
                self.selected_block_ids = select_blocks_in_rect(
                    body,
                    strip,
                    lane_blocks(project, lane_id),
                    marquee_rect,
                    metrics,
                );
            }
        }

        if response.clicked_by(egui::PointerButton::Primary)
            && !response.dragged()
            && is_timeline_pointer(strip, pointer)
        {
            let clicked_block = {
                let blocks = lane_blocks(project, lane_id);
                hit_test_block(body, strip, blocks, pointer, metrics).map(|block| block.id)
            };
            if let Some(block_id) = clicked_block {
                if ctrl_or_cmd {
                    if !self.selected_block_ids.remove(&block_id) {
                        self.selected_block_ids.insert(block_id);
                    }
                } else {
                    self.selected_block_ids.clear();
                    self.selected_block_ids.insert(block_id);
                }
                clip_selection.clear();
            } else {
                let start = Project::snap_beats(x_to_beat(body, pointer.x, metrics).max(0.0));
                let before = project.clone();
                if let Some(block_id) =
                    project.add_pattern_block(lane_id, start, DEFAULT_CLIP_LENGTH_BEATS)
                {
                    history.push_before(before);
                    self.selected_block_ids.clear();
                    self.selected_block_ids.insert(block_id);
                    clip_selection.clear();
                }
            }
        }

        if response.double_clicked_by(egui::PointerButton::Primary) && strip.contains(pointer) {
            let block_id = {
                let blocks = lane_blocks(project, lane_id);
                hit_test_block(body, strip, blocks, pointer, metrics).map(|block| block.id)
            };
            if let Some(block_id) = block_id {
                self.selected_block_ids.clear();
                self.selected_block_ids.insert(block_id);
                clip_selection.clear();
                self.open_block_request = Some(block_id);
            }
        }
    }
}

fn lane_blocks<'a>(project: &'a Project, lane_id: u64) -> &'a [PatternBlock] {
    project
        .pattern_lane(lane_id)
        .map(|lane| lane.blocks.as_slice())
        .unwrap_or(&[])
}

fn pattern_block_rect(
    timeline: Rect,
    strip: Rect,
    block: &PatternBlock,
    metrics: TimelineMetrics,
) -> Rect {
    let left = timeline_x(timeline, block.start_beats, metrics);
    let right = timeline_x(timeline, block.end_beats(), metrics);
    Rect::from_min_max(
        Pos2::new(left + 1.0, strip.top() + 4.0),
        Pos2::new(right - SOLO_BUTTON_WIDTH - 2.0, strip.bottom() - 4.0),
    )
}

fn pattern_solo_button_rect(block_rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(block_rect.right() + 1.0, block_rect.top()),
        Pos2::new(block_rect.right() + SOLO_BUTTON_WIDTH, block_rect.bottom()),
    )
}

fn hit_test_block<'a>(
    timeline: Rect,
    strip: Rect,
    blocks: &'a [PatternBlock],
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<&'a PatternBlock> {
    blocks.iter().rev().find(|block| {
        let block_rect = pattern_block_rect(timeline, strip, block, metrics);
        block_rect.contains(pos) || pattern_solo_button_rect(block_rect).contains(pos)
    })
}

fn hit_test_solo_button(
    timeline: Rect,
    strip: Rect,
    blocks: &[PatternBlock],
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<u64> {
    blocks.iter().rev().find_map(|block| {
        let block_rect = pattern_block_rect(timeline, strip, block, metrics);
        let solo = pattern_solo_button_rect(block_rect);
        if solo.contains(pos) {
            Some(block.id)
        } else {
            None
        }
    })
}

fn select_blocks_in_rect(
    timeline: Rect,
    strip: Rect,
    blocks: &[PatternBlock],
    rect: Rect,
    metrics: TimelineMetrics,
) -> HashSet<u64> {
    blocks
        .iter()
        .filter(|block| {
            let block_rect = pattern_block_rect(timeline, strip, block, metrics);
            rect.intersects(block_rect)
        })
        .map(|block| block.id)
        .collect()
}

fn update_pattern_resize_hover_cursor(
    response: &Response,
    body: Rect,
    strip: Rect,
    blocks: &[PatternBlock],
    metrics: TimelineMetrics,
) {
    let Some(hover) = response.hover_pos() else {
        return;
    };
    if !strip.contains(hover) {
        return;
    }
    let Some(block) = hit_test_block(body, strip, blocks, hover, metrics) else {
        return;
    };
    let bounds = pattern_block_rect(body, strip, block, metrics);
    if clip_resize_mode(bounds, hover.x).is_some() {
        response
            .ctx
            .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn finish_pattern_drag(
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    drag: &PatternBlockDrag,
    drag_moved: bool,
) {
    if !drag.ignore_ids.is_empty() && !drag_moved {
        history.abort(project);
        selected.clear();
        selected.extend(drag.ignore_ids.iter().copied());
        return;
    }
    history.commit(project);
    if !drag_moved {
        selected.clear();
        selected.insert(drag.block_id);
    }
}

fn apply_pattern_drag(project: &mut Project, drag: &PatternBlockDrag, current_beats: f32) {
    match drag.mode {
        ClipDragMode::Move => {
            let Some(primary) = drag
                .originals
                .iter()
                .find(|original| original.block_id == drag.block_id)
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
                        original.block_id,
                        original.start_beats,
                        original.length_beats,
                    )
                })
                .collect();
            let snapped_delta = project.clamp_pattern_block_move_delta(
                drag.lane_id,
                &originals,
                desired_delta,
                &drag.ignore_ids,
            );
            for original in &drag.originals {
                if let Some(block) = project.pattern_block_mut(original.block_id) {
                    block.start_beats =
                        (original.start_beats + snapped_delta).max(0.0);
                }
            }
        }
        ClipDragMode::ResizeStart => {
            let Some(original) = drag.originals.first() else {
                return;
            };
            let end = original.start_beats + original.length_beats;
            let left_bound = project.pattern_block_resize_start_bound(
                drag.lane_id,
                drag.block_id,
                original.start_beats,
            );
            let new_start = Project::snap_beats(current_beats.max(0.0));
            let clamped_start = new_start
                .max(left_bound)
                .min(end - SNAP_BEATS)
                .max(0.0);
            let Some(block) = project.pattern_block_mut(drag.block_id) else {
                return;
            };
            block.length_beats = end - clamped_start;
            block.start_beats = clamped_start;
        }
        ClipDragMode::ResizeEnd => {
            let Some(original) = drag.originals.first() else {
                return;
            };
            let right_bound = project.pattern_block_resize_end_bound(
                drag.lane_id,
                drag.block_id,
                original.start_beats + original.length_beats,
            );
            let new_end = Project::snap_beats(current_beats.max(0.0));
            let clamped_end = new_end
                .min(right_bound)
                .max(original.start_beats + SNAP_BEATS);
            let Some(block) = project.pattern_block_mut(drag.block_id) else {
                return;
            };
            block.length_beats = clamped_end - block.start_beats;
        }
    }
}
