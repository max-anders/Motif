use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::engine::{DawEngine, PluginCatalog, PluginRef};
use crate::model::{
    EditHistory, MidiClip, Project, Track, TrackInstrument, DEFAULT_CLIP_LENGTH_BEATS, MAX_PITCH,
    MIN_PITCH, SNAP_BEATS,
};
use crate::ui::instrument_menu::{
    choice_to_instrument, show_instrument_picker, track_name_for_choice, InstrumentChoice,
};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_horizontal_wheel_controls, daw_editor_scroll_area, draw_loop_region, draw_playhead,
    draw_ruler, draw_timeline_grid_lines, handle_timeline_playhead_pointer, is_timeline_pointer,
    timeline_body_rect, timeline_x, with_solid_scrollbars, x_to_beat, TimelineMetrics,
    DEFAULT_BEAT_WIDTH, RULER_HEIGHT, TIMELINE_GUTTER_WIDTH,
};

pub(crate) const TRACK_HEADER_WIDTH: f32 = TIMELINE_GUTTER_WIDTH;
pub(crate) const LANE_HEIGHT: f32 = 72.0;
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
pub(crate) struct ClipDrag {
    /// Primary clip under the pointer (resize target / open-on-double-click id).
    clip_id: u64,
    /// Track owning the primary clip; used to keep track selection in sync.
    track_id: u64,
    mode: ClipDragMode,
    pointer_start_beats: f32,
    originals: Vec<ClipOriginal>,
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

pub struct PlaylistUi {
    selected_clip_ids: HashSet<u64>,
    active_drag: Option<ClipDrag>,
    dragging_playhead: bool,
    beat_width: f32,
    scroll_offset: Vec2,
    /// Set when user clicks a clip without dragging (consumed by app).
    open_clip_request: Option<u64>,
    /// Open/close native plugin editor (consumed by app).
    plugin_editor_request: Option<PluginEditorRequest>,
    /// Delete track (consumed by app for piano-roll / engine cleanup).
    delete_track_request: Option<u64>,
    /// True if pointer moved enough during drag to count as a drag, not a click.
    drag_moved: bool,
    add_track_search: String,
    change_instrument_search: String,
    /// Last instrument load errors for display on lanes.
    instrument_errors: HashMap<u64, String>,
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
            plugin_editor_request: None,
            delete_track_request: None,
            drag_moved: false,
            add_track_search: String::new(),
            change_instrument_search: String::new(),
            instrument_errors: HashMap::new(),
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

    pub fn take_plugin_editor_request(&mut self) -> Option<PluginEditorRequest> {
        self.plugin_editor_request.take()
    }

    pub fn take_delete_track_request(&mut self) -> Option<u64> {
        self.delete_track_request.take()
    }

    pub fn clear_selection(&mut self) {
        self.selected_clip_ids.clear();
    }

    pub fn set_selection(&mut self, clip_ids: impl IntoIterator<Item = u64>) {
        self.selected_clip_ids.clear();
        self.selected_clip_ids.extend(clip_ids);
    }

    pub fn prune_selection(&mut self, project: &Project) {
        self.selected_clip_ids
            .retain(|id| project.clip(*id).is_some());
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
        theme: &ThemeColors,
    ) {
        // CentralPanel uses Frame::NONE; paint the full panel so nothing shows through.
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);

        ui.horizontal(|ui| {
            egui::menu::menu_button(ui, "Add track", |ui| {
                if let Some(choice) =
                    show_instrument_picker(ui, catalog, &mut self.add_track_search, "add_track")
                {
                    let number = project.tracks.len() + 1;
                    let name = track_name_for_choice(&choice, number);
                    let instrument = choice_to_instrument(choice);
                    history.push_before(project.clone());
                    project.add_track(&name, instrument);
                    self.add_track_search.clear();
                    ui.close_menu();
                }
            });
        });
        ui.add_space(4.0);

        let viewport_rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(viewport_rect, 0.0, theme.panel_bg);
        apply_horizontal_wheel_controls(
            ui,
            viewport_rect,
            &mut self.beat_width,
            &mut self.scroll_offset.x,
        );

        let metrics = TimelineMetrics {
            beat_width: self.beat_width,
        };
        let total_beats = project.arrangement_length_beats();
        let loop_span = project.loop_span();
        let lane_count = project.tracks.len().max(1);
        let content_height = RULER_HEIGHT + lane_count as f32 * LANE_HEIGHT;
        let content_width = TRACK_HEADER_WIDTH + total_beats * metrics.beat_width;
        let viewport = ui.available_size();
        let canvas_size = Vec2::new(
            content_width.max(viewport.x),
            content_height.max(viewport.y),
        );

        // Offset used to place content this frame (wheel updates apply next frame).
        let scroll = self.scroll_offset;

        let output = with_solid_scrollbars(ui, theme, |ui| {
            daw_editor_scroll_area("playlist_canvas")
                .scroll_offset(scroll)
                .show(ui, |ui| {
                    ui.set_min_size(canvas_size);
                    let (response, painter) =
                        ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    painter.rect_filled(content, 0.0, theme.panel_bg);
                    let body = timeline_body_rect(content);

                    // Visible viewport in screen space. Ruler stays pinned to the top
                    // (follows horizontal scroll); track headers stay pinned to the left
                    // (follow vertical scroll) - same sticky chrome as the piano roll.
                    let viewport = Rect::from_min_size(content.min + scroll, viewport_rect.size())
                        .intersect(ui.clip_rect());
                    let sticky_ruler = Rect::from_min_max(
                        viewport.min,
                        Pos2::new(viewport.right(), viewport.top() + RULER_HEIGHT),
                    );
                    let ruler_timeline = Rect::from_min_max(
                        Pos2::new(content.left(), sticky_ruler.top()),
                        Pos2::new(content.right(), sticky_ruler.bottom()),
                    );
                    let sticky_headers = Rect::from_min_max(
                        Pos2::new(viewport.left(), sticky_ruler.bottom()),
                        Pos2::new(viewport.left() + TRACK_HEADER_WIDTH, viewport.bottom()),
                    );

                    // When scrolled, content-space timeline hit tests treat the sticky
                    // header column as beats; skip body seeks there (ruler still works).
                    let on_sticky_headers = response
                        .interact_pointer_pos()
                        .is_some_and(|pos| sticky_headers.contains(pos));
                    let allow_playhead = self.dragging_playhead || !on_sticky_headers;

                    if allow_playhead
                        && handle_timeline_playhead_pointer(
                            &response,
                            sticky_ruler,
                            body,
                            metrics,
                            engine,
                            &mut self.dragging_playhead,
                            0.0,
                            true,
                        )
                    {
                        // Playhead handled; clip interactions skipped this frame when scrubbing.
                    } else {
                        handle_clip_pointer(
                            &response,
                            body,
                            sticky_headers,
                            metrics,
                            project,
                            history,
                            &mut self.selected_clip_ids,
                            &mut self.active_drag,
                            &mut self.open_clip_request,
                            &mut self.drag_moved,
                            selected_track,
                        );
                    }

                    // Keep scrolled content out of the sticky header / ruler strips.
                    let timeline_clip = Rect::from_min_max(
                        Pos2::new(sticky_headers.right(), sticky_ruler.bottom()),
                        content.max,
                    );
                    let timeline_painter = painter.with_clip_rect(timeline_clip);

                    for (index, track) in project.tracks.iter().enumerate() {
                        let lane_top = body.top() + index as f32 * LANE_HEIGHT;
                        let lane_rect = Rect::from_min_max(
                            Pos2::new(body.left(), lane_top),
                            Pos2::new(body.right(), lane_top + LANE_HEIGHT),
                        );
                        let audible = project.track_audible(track);
                        draw_lane_timeline(
                            &timeline_painter,
                            lane_rect,
                            body,
                            metrics,
                            total_beats,
                            project.beats_per_bar,
                            &track.clips,
                            &self.selected_clip_ids,
                            audible,
                            theme,
                        );
                    }

                    // Sticky chrome on top of scrolled content.
                    let track_ids: Vec<u64> = project.tracks.iter().map(|t| t.id).collect();
                    for (index, track) in project.tracks.iter().enumerate() {
                        let lane_top = body.top() + index as f32 * LANE_HEIGHT;
                        let header = Rect::from_min_max(
                            Pos2::new(sticky_headers.left(), lane_top),
                            Pos2::new(sticky_headers.right(), lane_top + LANE_HEIGHT),
                        );
                        draw_track_header(
                            &painter.with_clip_rect(sticky_headers),
                            header,
                            track.name.as_str(),
                            track.instrument.display_name(),
                            track.instrument.format_badge(),
                            self.instrument_errors.get(&track.id).map(String::as_str),
                            theme,
                        );
                    }

                    draw_ruler(
                        &painter.with_clip_rect(sticky_ruler),
                        sticky_ruler,
                        ruler_timeline,
                        metrics,
                        total_beats,
                        project.beats_per_bar,
                        theme,
                    );

                    // Loop region + playhead, clipped to the right of track headers.
                    let playhead_clip = Rect::from_min_max(
                        Pos2::new(sticky_headers.right(), sticky_ruler.top()),
                        content.max,
                    );
                    if let Some((loop_start, loop_end)) = loop_span {
                        draw_loop_region(
                            &painter.with_clip_rect(playhead_clip),
                            sticky_ruler,
                            body,
                            metrics,
                            loop_start,
                            loop_end,
                            theme,
                        );
                    }
                    let playhead = engine.current_beats();
                    draw_playhead(
                        &painter.with_clip_rect(playhead_clip),
                        sticky_ruler,
                        body,
                        metrics,
                        playhead,
                        true,
                        theme,
                    );

                    // Track header controls (M/S, context menu).
                    for (index, track_id) in track_ids.into_iter().enumerate() {
                        let lane_top = body.top() + index as f32 * LANE_HEIGHT;
                        let header = Rect::from_min_max(
                            Pos2::new(sticky_headers.left(), lane_top),
                            Pos2::new(sticky_headers.right(), lane_top + LANE_HEIGHT),
                        );
                        let track_snapshot = project
                            .tracks
                            .iter()
                            .find(|t| t.id == track_id)
                            .cloned();
                        let Some(track_snapshot) = track_snapshot else {
                            continue;
                        };
                        track_header_row(
                            ui,
                            header,
                            sticky_headers,
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
                            "playlist",
                        );
                    }
                })
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
    id_scope: &'static str,
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
    if header_response.clicked() {
        *select_track_request = Some(track_id);
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
                if let Some(track) = project.track_mut(track_id) {
                    track.muted = !track.muted;
                }
                engine.all_notes_off();
            }
            if ms_toggle_button(ui, "S", solo, theme) {
                history.push_before(project.clone());
                if let Some(track) = project.track_mut(track_id) {
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
        ui.label("Change instrument");
        ui.separator();
        if let Some(choice) = show_instrument_picker(
            ui,
            catalog,
            change_instrument_search,
            &format!("chg_{track_id}"),
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
        if ui.button("Solo").clicked() {
            history.push_before(project.clone());
            if let Some(track) = project.track_mut(track_id) {
                track.solo = !track.solo;
            }
            engine.all_notes_off();
            ui.close_menu();
        }
        ui.separator();
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

pub(crate) fn draw_lane_timeline(
    painter: &egui::Painter,
    lane: Rect,
    timeline: Rect,
    metrics: TimelineMetrics,
    total_beats: f32,
    beats_per_bar: f32,
    clips: &[MidiClip],
    selected: &HashSet<u64>,
    audible: bool,
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

    for clip in clips {
        let clip_rect = clip_block_rect(timeline, lane, clip, metrics);
        let is_selected = selected.contains(&clip.id);
        let fill = if is_selected {
            theme.clip_fill_selected
        } else {
            theme.clip_fill
        };
        painter.rect(
            clip_rect,
            4.0,
            fill,
            egui::Stroke::new(
                1.5_f32,
                if is_selected {
                    theme.clip_stroke_selected
                } else {
                    theme.clip_stroke
                },
            ),
            egui::StrokeKind::Inside,
        );

        painter.text(
            Pos2::new(clip_rect.left() + 6.0, clip_rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            &clip.name,
            egui::FontId::proportional(11.0),
            theme.clip_label,
        );

        draw_clip_note_preview(painter, clip_rect, clip, theme);
    }

    painter.line_segment(
        [
            Pos2::new(lane.left() + TRACK_HEADER_WIDTH, lane.bottom()),
            Pos2::new(lane.right(), lane.bottom()),
        ],
        egui::Stroke::new(1.0_f32, theme.separator),
    );
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
    clip: &MidiClip,
    metrics: TimelineMetrics,
) -> Rect {
    let left = timeline_x(timeline, clip.start_beats, metrics);
    let right = timeline_x(timeline, clip.end_beats(), metrics);
    Rect::from_min_max(
        Pos2::new(left + 1.0, lane.top() + 4.0),
        Pos2::new(right - 1.0, lane.bottom() - 4.0),
    )
}

fn draw_clip_note_preview(
    painter: &egui::Painter,
    clip_rect: Rect,
    clip: &MidiClip,
    theme: &ThemeColors,
) {
    if clip.notes.is_empty() {
        return;
    }
    let preview_top = clip_rect.top() + 20.0;
    let preview_height = (clip_rect.height() - 24.0).max(8.0);
    let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
    let length = clip.length_beats.max(SNAP_BEATS);
    // Clip notes to the block so resize does not paint past the edges.
    let clipped = painter.with_clip_rect(clip_rect);

    for note in &clip.notes {
        if note.start_beats >= length {
            continue;
        }
        let rel_start = note.start_beats / length;
        let rel_end = note.end_beats().min(length) / length;
        let x0 = clip_rect.left() + 4.0 + rel_start * (clip_rect.width() - 8.0);
        let x1 = clip_rect.left() + 4.0 + rel_end * (clip_rect.width() - 8.0);
        let pitch_norm = (note.pitch as f32 - MIN_PITCH as f32) / pitch_span;
        let y = preview_top + (1.0 - pitch_norm) * preview_height;
        clipped.rect_filled(
            Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1.max(x0 + 2.0), y + 3.0)),
            1.0,
            theme.clip_note_preview,
        );
    }
}

pub(crate) fn hit_test_clip<'a>(
    timeline: Rect,
    lane: Rect,
    clips: &'a [MidiClip],
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<&'a MidiClip> {
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
    project: &Project,
    metrics: TimelineMetrics,
) {
    let Some(hover) = response.hover_pos() else {
        return;
    };
    if !body.contains(hover) || sticky_headers.contains(hover) {
        return;
    }
    let track_index = ((hover.y - body.top()) / LANE_HEIGHT).floor() as usize;
    if track_index >= project.tracks.len() {
        return;
    }
    let lane = lane_rect_for_track(body, track_index);
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

fn lane_rect_for_track(body: Rect, track_index: usize) -> Rect {
    let lane_top = body.top() + track_index as f32 * LANE_HEIGHT;
    Rect::from_min_max(
        Pos2::new(body.left(), lane_top),
        Pos2::new(body.right(), lane_top + LANE_HEIGHT),
    )
}

#[allow(clippy::too_many_arguments)]
fn handle_clip_pointer(
    response: &Response,
    body: Rect,
    sticky_headers: Rect,
    metrics: TimelineMetrics,
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    active_drag: &mut Option<ClipDrag>,
    open_clip_request: &mut Option<u64>,
    drag_moved: &mut bool,
    selected_track: &mut Option<u64>,
) {
    update_clip_resize_hover_cursor(response, body, sticky_headers, project, metrics);

    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            if let Some(drag) = active_drag.take() {
                history.commit(project);
                if !*drag_moved {
                    selected.clear();
                    selected.insert(drag.clip_id);
                }
                *selected_track = Some(drag.track_id);
            }
            *drag_moved = false;
        }
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
        if response.dragged() {
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
        if response.drag_stopped() {
            history.commit(project);
            if !*drag_moved {
                selected.clear();
                selected.insert(drag.clip_id);
            }
            *selected_track = Some(drag.track_id);
            *active_drag = None;
            *drag_moved = false;
        }
        return;
    }

    // Sticky track headers own this column; do not treat it as empty-lane / clip hits.
    if sticky_headers.contains(press_pos) || sticky_headers.contains(pointer) {
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
        if let Some(clip) = hit_test_clip(
            body,
            lane,
            &project.tracks[track_index].clips,
            press_pos,
            metrics,
        )
        .cloned()
        {
            let bounds = clip_block_rect(body, lane, &clip, metrics);
            let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);

            let already_selected = selected.contains(&clip.id);
            if !already_selected {
                selected.clear();
                selected.insert(clip.id);
            }
            *selected_track = Some(track_id);

            // Snapshot before Shift-duplicate so one undo covers dup+move.
            history.begin(project);

            let mut primary_id = clip.id;
            // Shift+Move: leave originals, drag duplicates (same as piano-roll notes).
            if matches!(mode, ClipDragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected.iter().copied().collect();
                let new_ids = project.duplicate_clips(&source_ids, 0.0);
                if let Some(mapped_primary) = source_ids
                    .iter()
                    .position(|id| *id == clip.id)
                    .and_then(|index| new_ids.get(index).copied())
                {
                    primary_id = mapped_primary;
                } else if let Some(first) = new_ids.first().copied() {
                    primary_id = first;
                }
                selected.clear();
                selected.extend(new_ids);
            }

            let originals = match mode {
                ClipDragMode::Move => selected
                    .iter()
                    .filter_map(|id| {
                        project.clip(*id).map(|c| ClipOriginal {
                            clip_id: c.id,
                            start_beats: c.start_beats,
                            length_beats: c.length_beats,
                        })
                    })
                    .collect(),
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd => project
                    .clip(primary_id)
                    .map(|c| {
                        vec![ClipOriginal {
                            clip_id: c.id,
                            start_beats: c.start_beats,
                            length_beats: c.length_beats,
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
            });
            return;
        }

        // Empty lane: create clip
        if is_timeline_pointer(lane, press_pos) {
            let start = Project::snap_beats(x_to_beat(body, press_pos.x, metrics).max(0.0));
            let before = project.clone();
            if let Some(clip_id) =
                project.add_clip_to_track(track_id, start, DEFAULT_CLIP_LENGTH_BEATS)
            {
                history.push_before(before);
                selected.clear();
                selected.insert(clip_id);
                *selected_track = Some(track_id);
            }
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
                if !selected.remove(&clip.id) {
                    selected.insert(clip.id);
                }
            } else {
                selected.clear();
                selected.insert(clip.id);
            }
            *selected_track = Some(track_id);
        }
    }

    if response.double_clicked_by(egui::PointerButton::Primary) && body.contains(pointer) {
        if let Some(clip_id) = hit_test_clip_id(body, project, pointer, metrics) {
            selected.clear();
            selected.insert(clip_id);
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
    clips: &[MidiClip],
    metrics: TimelineMetrics,
    project: &mut Project,
    history: &mut EditHistory,
    selected: &mut HashSet<u64>,
    active_drag: &mut Option<ClipDrag>,
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

    let Some(pointer) = response.interact_pointer_pos() else {
        if response.drag_stopped() {
            if let Some(drag) = active_drag.take() {
                history.commit(project);
                if !*drag_moved {
                    selected.clear();
                    selected.insert(drag.clip_id);
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
    let (shift_held, ctrl_or_cmd) = response.ctx.input(|input| {
        (
            input.modifiers.shift,
            input.modifiers.ctrl || input.modifiers.command || input.modifiers.mac_cmd,
        )
    });

    if let Some(drag) = active_drag.clone() {
        if response.dragged() {
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
        if response.drag_stopped() {
            history.commit(project);
            if !*drag_moved {
                selected.clear();
                selected.insert(drag.clip_id);
            }
            *active_drag = None;
            *drag_moved = false;
        }
        return;
    }

    if !body.contains(pointer) {
        return;
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && is_timeline_pointer(lane, press_pos)
    {
        if let Some(clip) = hit_test_clip(body, lane, clips, press_pos, metrics).cloned() {
            let bounds = clip_block_rect(body, lane, &clip, metrics);
            let mode = clip_resize_mode(bounds, press_pos.x).unwrap_or(ClipDragMode::Move);

            let already_selected = selected.contains(&clip.id);
            if !already_selected {
                selected.clear();
                selected.insert(clip.id);
            }

            history.begin(project);

            let mut primary_id = clip.id;
            if matches!(mode, ClipDragMode::Move) && shift_held {
                let source_ids: Vec<u64> = selected.iter().copied().collect();
                let new_ids = project.duplicate_clips(&source_ids, 0.0);
                if let Some(mapped_primary) = source_ids
                    .iter()
                    .position(|id| *id == clip.id)
                    .and_then(|index| new_ids.get(index).copied())
                {
                    primary_id = mapped_primary;
                } else if let Some(first) = new_ids.first().copied() {
                    primary_id = first;
                }
                selected.clear();
                selected.extend(new_ids);
            }

            let originals = match mode {
                ClipDragMode::Move => selected
                    .iter()
                    .filter_map(|id| {
                        project.clip(*id).map(|c| ClipOriginal {
                            clip_id: c.id,
                            start_beats: c.start_beats,
                            length_beats: c.length_beats,
                        })
                    })
                    .collect(),
                ClipDragMode::ResizeStart | ClipDragMode::ResizeEnd => project
                    .clip(primary_id)
                    .map(|c| {
                        vec![ClipOriginal {
                            clip_id: c.id,
                            start_beats: c.start_beats,
                            length_beats: c.length_beats,
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
            });
            return;
        }

        if is_timeline_pointer(lane, press_pos) {
            let start = Project::snap_beats(x_to_beat(body, press_pos.x, metrics).max(0.0));
            let before = project.clone();
            if let Some(clip_id) = project.add_clip_to_track(track_id, start, DEFAULT_CLIP_LENGTH_BEATS) {
                history.push_before(before);
                selected.clear();
                selected.insert(clip_id);
            }
        }
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && !response.dragged()
        && is_timeline_pointer(lane, pointer)
    {
        if let Some(clip) = hit_test_clip(body, lane, clips, pointer, metrics) {
            if ctrl_or_cmd {
                if !selected.remove(&clip.id) {
                    selected.insert(clip.id);
                }
            } else {
                selected.clear();
                selected.insert(clip.id);
            }
        }
    }

    if response.double_clicked_by(egui::PointerButton::Primary) && body.contains(pointer) {
        if let Some(clip) = hit_test_clip(body, lane, clips, pointer, metrics) {
            selected.clear();
            selected.insert(clip.id);
            *open_clip_request = Some(clip.id);
        }
    }
}

fn hit_test_clip_id(
    body: Rect,
    project: &Project,
    pos: Pos2,
    metrics: TimelineMetrics,
) -> Option<u64> {
    let track_index = ((pos.y - body.top()) / LANE_HEIGHT).floor() as usize;
    if track_index >= project.tracks.len() {
        return None;
    }
    let lane = lane_rect_for_track(body, track_index);
    hit_test_clip(body, lane, &project.tracks[track_index].clips, pos, metrics).map(|clip| clip.id)
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
            let mut snapped_delta =
                Project::snap_beats(primary.start_beats + raw_delta).max(0.0) - primary.start_beats;
            let min_start = drag
                .originals
                .iter()
                .map(|original| original.start_beats)
                .fold(f32::INFINITY, f32::min);
            if min_start + snapped_delta < 0.0 {
                snapped_delta = -min_start;
            }
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
            let Some(clip) = project.clip_mut(drag.clip_id) else {
                return;
            };
            let new_start = Project::snap_beats(current_beats.max(0.0));
            let end = original.start_beats + original.length_beats;
            let clamped_start = new_start.min(end - SNAP_BEATS);
            clip.set_start_beats(clamped_start);
            clip.set_length_beats(end - clamped_start);
        }
        ClipDragMode::ResizeEnd => {
            let Some(original) = drag.originals.first() else {
                return;
            };
            let Some(clip) = project.clip_mut(drag.clip_id) else {
                return;
            };
            let new_end = Project::snap_beats(current_beats.max(0.0));
            clip.set_start_beats(original.start_beats);
            clip.set_length_beats((new_end - original.start_beats).max(SNAP_BEATS));
        }
    }
}
