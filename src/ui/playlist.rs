use std::collections::HashSet;

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{MidiClip, Project, SNAP_BEATS, DEFAULT_CLIP_LENGTH_BEATS, MAX_PITCH, MIN_PITCH};
use crate::ui::timeline::{
    apply_horizontal_wheel_controls, draw_playhead, draw_ruler, draw_timeline_grid_lines,
    handle_timeline_playhead_pointer, is_timeline_pointer, ruler_rect, timeline_body_rect,
    timeline_x, x_to_beat, TimelineMetrics, DEFAULT_BEAT_WIDTH,
    RULER_HEIGHT, TIMELINE_GUTTER_WIDTH,
};

const TRACK_HEADER_WIDTH: f32 = TIMELINE_GUTTER_WIDTH;
const LANE_HEIGHT: f32 = 72.0;
const RESIZE_HANDLE_PX: f32 = 10.0;
const PLAYLIST_BG: Color32 = Color32::from_rgb(18, 18, 22);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipDragMode {
    Move,
    ResizeStart,
    ResizeEnd,
}

#[derive(Debug, Clone)]
struct ClipDrag {
    clip_id: u64,
    mode: ClipDragMode,
    pointer_start_beats: f32,
    original_start: f32,
    original_length: f32,
}

pub struct PlaylistUi {
    selected_clip_ids: HashSet<u64>,
    active_drag: Option<ClipDrag>,
    dragging_playhead: bool,
    beat_width: f32,
    scroll_offset: Vec2,
    /// Set when user clicks a clip without dragging (consumed by app).
    open_clip_request: Option<u64>,
    /// True if pointer moved enough during drag to count as a drag, not a click.
    drag_moved: bool,
}

impl Default for PlaylistUi {
    fn default() -> Self {
        Self {
            selected_clip_ids: HashSet::new(),
            active_drag: None,
            dragging_playhead: false,
            beat_width: DEFAULT_BEAT_WIDTH,
            scroll_offset: Vec2::ZERO,
            open_clip_request: None,
            drag_moved: false,
        }
    }
}

impl PlaylistUi {
    pub fn selected_clip_ids(&self) -> &HashSet<u64> {
        &self.selected_clip_ids
    }

    pub fn take_open_clip_request(&mut self) -> Option<u64> {
        self.open_clip_request.take()
    }

    pub fn clear_selection(&mut self) {
        self.selected_clip_ids.clear();
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
    ) {
        // CentralPanel uses Frame::NONE; paint the full panel so nothing shows through.
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, PLAYLIST_BG);

        ui.horizontal(|ui| {
            if ui.button("Add track").clicked() {
                let number = project.tracks.len() + 1;
                project.add_track(&format!("Track {number}"));
            }
        });
        ui.add_space(4.0);

        let viewport_rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(viewport_rect, 0.0, PLAYLIST_BG);
        apply_horizontal_wheel_controls(
            ui,
            viewport_rect,
            &mut self.beat_width,
            &mut self.scroll_offset.x,
        );

        let metrics = TimelineMetrics {
            beat_width: self.beat_width,
        };
        let total_beats = project.loop_end_beats.max(4.0);
        let lane_count = project.tracks.len().max(1);
        let content_height = RULER_HEIGHT + lane_count as f32 * LANE_HEIGHT;
        let content_width = TRACK_HEADER_WIDTH + total_beats * metrics.beat_width;
        let viewport = ui.available_size();
        let canvas_size = Vec2::new(
            content_width.max(viewport.x),
            content_height.max(viewport.y),
        );

        let output = egui::ScrollArea::both()
            .id_salt("playlist_canvas")
            .scroll_offset(self.scroll_offset)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_size(canvas_size);
                let (response, painter) =
                    ui.allocate_painter(canvas_size, Sense::click_and_drag());
                let rect = response.rect;
                painter.rect_filled(rect, 0.0, PLAYLIST_BG);
                let ruler = ruler_rect(rect);
                let body = timeline_body_rect(rect);

                if handle_timeline_playhead_pointer(
                    &response,
                    ruler,
                    body,
                    metrics,
                    engine,
                    &mut self.dragging_playhead,
                    0.0,
                    true,
                ) {
                    // Playhead handled; clip interactions skipped this frame when scrubbing.
                } else {
                    handle_clip_pointer(
                        &response,
                        body,
                        metrics,
                        project,
                        &mut self.selected_clip_ids,
                        &mut self.active_drag,
                        &mut self.open_clip_request,
                        &mut self.drag_moved,
                    );
                }

                draw_ruler(
                    &painter,
                    ruler,
                    body,
                    metrics,
                    total_beats,
                    project.beats_per_bar,
                );

                let playhead = engine.current_beats();
                draw_playhead(
                    &painter,
                    ruler,
                    body,
                    metrics,
                    playhead,
                    true,
                );

                for (index, track) in project.tracks.iter().enumerate() {
                    let lane_top = body.top() + index as f32 * LANE_HEIGHT;
                    let lane_rect = Rect::from_min_max(
                        Pos2::new(body.left(), lane_top),
                        Pos2::new(body.right(), lane_top + LANE_HEIGHT),
                    );
                    draw_lane(
                        &painter,
                        lane_rect,
                        body,
                        metrics,
                        total_beats,
                        project.beats_per_bar,
                        track.name.as_str(),
                        &track.clips,
                        &self.selected_clip_ids,
                    );
                }
            });

        self.scroll_offset = output.state.offset;

        if self.active_drag.is_none() && !self.drag_moved {
            // click-without-drag handled in handle_clip_pointer
        }
        if self.active_drag.is_none() {
            self.drag_moved = false;
        }
    }
}

fn draw_lane(
    painter: &egui::Painter,
    lane: Rect,
    timeline: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
    track_name: &str,
    clips: &[MidiClip],
    selected: &HashSet<u64>,
) {
    let header = Rect::from_min_max(
        lane.min,
        Pos2::new(lane.left() + TRACK_HEADER_WIDTH, lane.bottom()),
    );
    painter.rect_filled(header, 0.0, Color32::from_rgb(40, 40, 50));
    painter.text(
        Pos2::new(header.left() + 6.0, header.center().y),
        egui::Align2::LEFT_CENTER,
        track_name,
        egui::FontId::proportional(12.0),
        Color32::from_rgb(210, 210, 220),
    );

    let timeline_lane = Rect::from_min_max(
        Pos2::new(lane.left() + TRACK_HEADER_WIDTH, lane.top()),
        lane.max,
    );
    painter.rect_filled(timeline_lane, 0.0, Color32::from_rgb(22, 22, 28));
    draw_timeline_grid_lines(
        painter,
        timeline_lane,
        metrics,
        total_beats,
        beats_per_bar,
    );

    for clip in clips {
        let clip_rect = clip_block_rect(timeline, lane, clip, metrics);
        let is_selected = selected.contains(&clip.id);
        let fill = if is_selected {
            Color32::from_rgb(100, 170, 255)
        } else {
            Color32::from_rgb(60, 110, 180)
        };
        painter.rect(
            clip_rect,
            4.0,
            fill,
            egui::Stroke::new(
                1.5_f32,
                if is_selected {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(140, 180, 230)
                },
            ),
            egui::StrokeKind::Inside,
        );

        painter.text(
            Pos2::new(clip_rect.left() + 6.0, clip_rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::proportional(11.0),
            Color32::from_rgb(240, 240, 250),
        );

        draw_clip_note_preview(painter, clip_rect, clip);
    }

    painter.line_segment(
        [Pos2::new(lane.left(), lane.bottom()), Pos2::new(lane.right(), lane.bottom())],
        egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 55, 68)),
    );
}

fn clip_block_rect(timeline: Rect, lane: Rect, clip: &MidiClip, metrics: TimelineMetrics) -> Rect {
    let left = timeline_x(timeline, clip.start_beats, metrics);
    let right = timeline_x(timeline, clip.end_beats(), metrics);
    Rect::from_min_max(
        Pos2::new(left + 1.0, lane.top() + 4.0),
        Pos2::new(right - 1.0, lane.bottom() - 4.0),
    )
}

fn draw_clip_note_preview(painter: &egui::Painter, clip_rect: Rect, clip: &MidiClip) {
    if clip.notes.is_empty() {
        return;
    }
    let preview_top = clip_rect.top() + 20.0;
    let preview_height = (clip_rect.height() - 24.0).max(8.0);
    let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;

    for note in &clip.notes {
        let rel_start = note.start_beats / clip.length_beats.max(SNAP_BEATS);
        let rel_end = note.end_beats() / clip.length_beats.max(SNAP_BEATS);
        let x0 = clip_rect.left() + 4.0 + rel_start * (clip_rect.width() - 8.0);
        let x1 = clip_rect.left() + 4.0 + rel_end * (clip_rect.width() - 8.0);
        let pitch_norm = (note.pitch as f32 - MIN_PITCH as f32) / pitch_span;
        let y = preview_top + (1.0 - pitch_norm) * preview_height;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(x0, y),
                Pos2::new(x1.max(x0 + 2.0), y + 3.0),
            ),
            1.0,
            Color32::from_rgba_unmultiplied(255, 255, 255, 120),
        );
    }
}

fn hit_test_clip<'a>(
    timeline: Rect,
    lane: Rect,
    clips: &'a [MidiClip],
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<&'a MidiClip> {
    clips.iter().rev().find(|clip| {
        clip_block_rect(timeline, lane, clip, metrics).contains(pos)
    })
}

fn clip_resize_mode(bounds: Rect, pointer_x: f32) -> Option<ClipDragMode> {
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

fn lane_rect_for_track(body: Rect, track_index: usize) -> Rect {
    let lane_top = body.top() + track_index as f32 * LANE_HEIGHT;
    Rect::from_min_max(
        Pos2::new(body.left(), lane_top),
        Pos2::new(body.right(), lane_top + LANE_HEIGHT),
    )
}

fn handle_clip_pointer(
    response: &Response,
    body: Rect,
    metrics: TimelineMetrics,
    project: &mut Project,
    selected: &mut HashSet<u64>,
    active_drag: &mut Option<ClipDrag>,
    open_clip_request: &mut Option<u64>,
    drag_moved: &mut bool,
) {
    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            if let Some(drag) = active_drag.take() {
                if !*drag_moved {
                    selected.clear();
                    selected.insert(drag.clip_id);
                    *open_clip_request = Some(drag.clip_id);
                }
            }
            *drag_moved = false;
        }
        return;
    };

    let press_pos = response
        .ctx
        .input(|input| input.pointer.press_origin())
        .unwrap_or(pointer);

    if let Some(drag) = active_drag.clone() {
        if response.dragged() {
            *drag_moved = true;
            let current_beats = x_to_beat(body, pointer.x, metrics);
            apply_clip_drag(project, &drag, current_beats);
        }
        if response.drag_stopped() {
            if !*drag_moved {
                selected.clear();
                selected.insert(drag.clip_id);
                *open_clip_request = Some(drag.clip_id);
            }
            *active_drag = None;
            *drag_moved = false;
        }
        return;
    }

    if !body.contains(pointer) {
        return;
    }

    // Find which track/lane was hit
    let track_index = ((press_pos.y - body.top()) / LANE_HEIGHT).floor() as usize;
    if track_index >= project.tracks.len() {
        return;
    }
    let track_id = project.tracks[track_index].id;
    let lane = lane_rect_for_track(body, track_index);

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(lane, press_pos)
    {
        if let Some(clip) = hit_test_clip(body, lane, &project.tracks[track_index].clips, press_pos, metrics).cloned() {
            let bounds = clip_block_rect(body, lane, &clip, metrics);
            let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);
            selected.clear();
            selected.insert(clip.id);
            *active_drag = Some(ClipDrag {
                clip_id: clip.id,
                mode,
                pointer_start_beats: x_to_beat(body, press_pos.x, metrics),
                original_start: clip.start_beats,
                original_length: clip.length_beats,
            });
            return;
        }

        // Empty lane: create clip
        if is_timeline_pointer(lane, press_pos) {
            let start = Project::snap_beats(x_to_beat(body, press_pos.x, metrics).max(0.0));
            if let Some(clip_id) = project.add_clip_to_track(track_id, start, DEFAULT_CLIP_LENGTH_BEATS) {
                selected.clear();
                selected.insert(clip_id);
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && is_timeline_pointer(lane, pointer)
    {
        if let Some(clip) = hit_test_clip(body, lane, &project.tracks[track_index].clips, pointer, metrics) {
            selected.clear();
            selected.insert(clip.id);
            *open_clip_request = Some(clip.id);
        }
    }
}

fn apply_clip_drag(project: &mut Project, drag: &ClipDrag, current_beats: f32) {
    let Some(clip) = project.clip_mut(drag.clip_id) else {
        return;
    };

    match drag.mode {
        ClipDragMode::Move => {
            let delta = Project::snap_beats(current_beats - drag.pointer_start_beats);
            let new_start = (drag.original_start + delta).max(0.0);
            clip.set_start_beats(new_start);
        }
        ClipDragMode::ResizeStart => {
            let new_start = Project::snap_beats(current_beats.max(0.0));
            let end = drag.original_start + drag.original_length;
            let clamped_start = new_start.min(end - SNAP_BEATS);
            clip.set_start_beats(clamped_start);
            clip.set_length_beats(end - clamped_start);
        }
        ClipDragMode::ResizeEnd => {
            let new_end = Project::snap_beats(current_beats.max(0.0));
            clip.set_start_beats(drag.original_start);
            clip.set_length_beats((new_end - drag.original_start).max(SNAP_BEATS));
        }
    }
}
