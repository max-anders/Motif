use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use egui::containers::scroll_area::ScrollBarVisibility;
use egui::{Align, Align2, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};

use crate::engine::{DawEngine, DecodedAudio, PluginCatalog, PluginRef};
use crate::model::{Device, EditHistory, Project, Track, TrackInstrument};
use crate::ui::instrument_menu::{
    choice_to_instrument, show_effect_picker, show_instrument_picker, InstrumentChoice,
    MENU_LIST_MAX_HEIGHT,
};
use crate::ui::app_settings::AppSettings;
use crate::ui::favorites_panel::{
    favorites_column_visible, show_favorites_menu, show_favorites_panel, unique_id_for_target,
};
use crate::ui::macro_panel::show_macro_panel;
use crate::ui::modulator::{
    modulator_count_for_target, normalize_modulator_target_key, show_modulator_panel,
    target_filter_from_device_key, ModulatorLayout, TargetFilter, INSTRUMENT_MOD_TARGET_KEY,
};
use crate::ui::playlist::{
    draw_lane_timeline, draw_marquee, handle_single_track_clip_pointer, ms_toggle_button,
    track_header_row, ClipDrag, LANE_HEIGHT, MarqueeDrag, PluginEditorRequest, TRACK_HEADER_WIDTH,
};
use crate::ui::theme::ThemeColors;
use crate::ui::track_rename::TrackRenameUi;
use crate::ui::timeline::{
    apply_horizontal_wheel_controls, arrangement_beat_width_bounds, draw_loop_region, draw_playhead,
    draw_playback_anchor, draw_ruler, handle_loop_region_pointer, handle_timeline_playhead_pointer,
    hit_test_loop_edge, timeline_body_rect, with_solid_scrollbars, LoopEdge, TimelineMetrics,
    DEFAULT_BEAT_WIDTH, RULER_HEIGHT,
};

const STRIP_ROUNDING: f32 = 4.0;
const STRIP_INNER_MARGIN: f32 = 6.0;
const STRIP_GAP: f32 = 8.0;
const STRIP_HEADER_ROW_HEIGHT: f32 = 22.0;
const STRIP_META_LINE_HEIGHT: f32 = 14.0;
const STRIP_EXPANDED_BODY_HEIGHT: f32 = 96.0;
const STRIP_BUTTON_HEIGHT: f32 = 18.0;
// Matches `with_solid_scrollbars` bar width so the lane sits flush above the bar.
const MINI_SCROLLBAR_WIDTH: f32 = 10.0;
const SECTION_GAP: f32 = 8.0;
/// Horizontal inset inside the dock column.
const COLUMN_PADDING: f32 = 8.0;
/// Single-column dock width bounds (content, excluding frame margin).
const DOCK_MIN_WIDTH: f32 = 220.0;
const DOCK_DEFAULT_WIDTH: f32 = 280.0;
const DOCK_MAX_WIDTH: f32 = 760.0;

/// `Frame::side_top_panel` inner margin (8 per side). `SidePanel` widths include
/// it, the dock's own layout math does not.
const PANEL_FRAME_H_MARGIN: f32 = 16.0;

fn dock_panel_width_bounds() -> (f32, f32, f32) {
    (
        DOCK_MIN_WIDTH + PANEL_FRAME_H_MARGIN,
        DOCK_DEFAULT_WIDTH + PANEL_FRAME_H_MARGIN,
        DOCK_MAX_WIDTH + PANEL_FRAME_H_MARGIN,
    )
}

fn strip_collapsed_height(meta_lines: usize) -> f32 {
    STRIP_INNER_MARGIN * 2.0
        + STRIP_HEADER_ROW_HEIGHT
        + meta_lines as f32 * STRIP_META_LINE_HEIGHT
}

/// How the dock side panel should be sized for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockPanelWidth {
    pub default_width: f32,
    pub min_width: f32,
    pub max_width: f32,
    pub resizable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainLayout {
    Page,
    Dock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevicesView {
    Patch,
    Detail { track_id: u64, target_key: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineSectionKind {
    Macros,
    Lfo,
    Fav,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DevicesStripOutput {
    pub expand: bool,
    pub hide: bool,
    /// Favorites (or other settings) changed; app should persist `settings.json`.
    pub settings_dirty: bool,
}
// Ruler + one lane + the horizontal scrollbar strip; no vertical scroll, no gap.
const MINI_PLAYLIST_HEIGHT: f32 = RULER_HEIGHT + LANE_HEIGHT + MINI_SCROLLBAR_WIDTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceStripAction {
    None,
    Select,
    ToggleBypass,
    Remove,
    OpenEditor,
    CloseEditor,
    ToggleInline(InlineSectionKind),
    ToggleBodyExpand,
    OpenDetail,
}

pub struct DevicesUi {
    add_fx_search: String,
    change_instrument_search: String,
    /// Open/close native plugin editor (consumed by app).
    plugin_editor_request: Option<PluginEditorRequest>,
    /// Delete track request (consumed by app).
    delete_track_request: Option<u64>,
    /// Duplicate track request (consumed by app).
    duplicate_track_request: Option<u64>,
    /// Track header currently under the pointer (for Delete track shortcut).
    hovered_track_header: Option<u64>,
    /// Open clip request (consumed by app for piano-roll transition).
    open_clip_request: Option<u64>,
    mini_selected_clip_ids: HashSet<u64>,
    mini_active_drag: Option<ClipDrag>,
    mini_marquee: Option<MarqueeDrag>,
    mini_dragging_playhead: bool,
    mini_dragging_loop_edge: Option<LoopEdge>,
    mini_beat_width: f32,
    mini_scroll_offset: Vec2,
    mini_timeline_view_w: f32,
    mini_drag_moved: bool,
    /// Selected modulator target per track (`0` = instrument, else FX device id).
    selected_modulator_target: HashMap<u64, u64>,
    /// Patch list vs full-column detail for one device.
    view: DevicesView,
    /// In-list expanded plugin/placeholder body for one device.
    expanded_body: Option<(u64, u64)>,
    /// Inline Macros / LFO / Favorites section under one device strip.
    inline_section: Option<(u64, u64, InlineSectionKind)>,
    /// Remembered dock panel width (content side of frame margin).
    dock_column_width: f32,
}

impl Default for DevicesUi {
    fn default() -> Self {
        Self {
            add_fx_search: String::new(),
            change_instrument_search: String::new(),
            plugin_editor_request: None,
            delete_track_request: None,
            duplicate_track_request: None,
            hovered_track_header: None,
            open_clip_request: None,
            mini_selected_clip_ids: HashSet::new(),
            mini_active_drag: None,
            mini_marquee: None,
            mini_dragging_playhead: false,
            mini_dragging_loop_edge: None,
            mini_beat_width: DEFAULT_BEAT_WIDTH,
            mini_scroll_offset: Vec2::ZERO,
            mini_timeline_view_w: 0.0,
            mini_drag_moved: false,
            selected_modulator_target: HashMap::new(),
            view: DevicesView::Patch,
            expanded_body: None,
            inline_section: None,
            dock_column_width: DOCK_DEFAULT_WIDTH,
        }
    }
}

impl DevicesUi {
    pub fn take_plugin_editor_request(&mut self) -> Option<PluginEditorRequest> {
        self.plugin_editor_request.take()
    }

    pub fn take_delete_track_request(&mut self) -> Option<u64> {
        self.delete_track_request.take()
    }

    pub fn take_duplicate_track_request(&mut self) -> Option<u64> {
        self.duplicate_track_request.take()
    }

    pub fn hovered_track_header(&self) -> Option<u64> {
        self.hovered_track_header
    }

    pub fn take_open_clip_request(&mut self) -> Option<u64> {
        self.open_clip_request.take()
    }

    /// Target key whose LFO column is open for `track_id`, if any.
    fn sanitize_view_state(&mut self, track_id: u64, track: &Track) {
        let valid_target = |key: u64| {
            key == INSTRUMENT_MOD_TARGET_KEY
                || track.devices.iter().any(|device| device.id == key)
        };
        if let DevicesView::Detail {
            track_id: detail_track,
            target_key,
        } = self.view
        {
            if detail_track != track_id || !valid_target(target_key) {
                self.view = DevicesView::Patch;
            }
        }
        if let Some((tid, key)) = self.expanded_body {
            if tid != track_id || !valid_target(key) {
                self.expanded_body = None;
            }
        }
        if let Some((tid, key, _)) = self.inline_section {
            if tid != track_id || !valid_target(key) {
                self.inline_section = None;
            }
        }
    }

    fn toggle_inline_section(&mut self, track_id: u64, target_key: u64, kind: InlineSectionKind) {
        self.selected_modulator_target.insert(track_id, target_key);
        if self.inline_section == Some((track_id, target_key, kind)) {
            self.inline_section = None;
        } else {
            self.inline_section = Some((track_id, target_key, kind));
        }
    }

    fn toggle_body_expand(&mut self, track_id: u64, target_key: u64) {
        self.selected_modulator_target.insert(track_id, target_key);
        if self.expanded_body == Some((track_id, target_key)) {
            self.expanded_body = None;
        } else {
            self.expanded_body = Some((track_id, target_key));
        }
    }

    fn open_detail(&mut self, track_id: u64, target_key: u64) {
        self.selected_modulator_target.insert(track_id, target_key);
        self.view = DevicesView::Detail {
            track_id,
            target_key,
        };
        self.inline_section = None;
        self.expanded_body = None;
    }

    fn back_to_patch(&mut self) {
        self.view = DevicesView::Patch;
    }

    pub fn selected_target_key(&self, track_id: u64) -> u64 {
        self.selected_modulator_target
            .get(&track_id)
            .copied()
            .unwrap_or(INSTRUMENT_MOD_TARGET_KEY)
    }

    /// Width plan for the dock side panel this frame.
    pub fn dock_panel_width(
        &mut self,
        _track: Option<&Track>,
        _settings: &AppSettings,
    ) -> DockPanelWidth {
        let (min_width, _, max_width) = dock_panel_width_bounds();
        let default_width = self.dock_column_width.clamp(min_width, max_width);
        DockPanelWidth {
            default_width,
            min_width,
            max_width,
            resizable: true,
        }
    }

    /// Feed the panel's realised width back after `SidePanel::show`.
    pub fn note_dock_panel_width(&mut self, panel_width: f32) {
        let (min_width, _, max_width) = dock_panel_width_bounds();
        self.dock_column_width = (panel_width - PANEL_FRAME_H_MARGIN).clamp(
            min_width - PANEL_FRAME_H_MARGIN,
            max_width - PANEL_FRAME_H_MARGIN,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        device_errors: &HashMap<(u64, u64), String>,
        selected_track: &mut Option<u64>,
        decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
        settings: &mut AppSettings,
        theme: &ThemeColors,
        track_rename: &mut TrackRenameUi,
    ) -> bool {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);
        let mut settings_dirty = false;
        self.hovered_track_header = None;
        if selected_track.and_then(|id| project.track(id)).is_none() {
            *selected_track = project.tracks.first().map(|track| track.id);
        }

        ui.horizontal(|ui| {
            let panel_height = ui.available_height();
            ui.allocate_ui_with_layout(
                Vec2::new(TRACK_HEADER_WIDTH, panel_height),
                Layout::top_down(Align::Min),
                |ui| {
                    // Align headers with the mini-playlist lane (below ruler strip).
                    ui.add_space(RULER_HEIGHT);
                    let headers_height = (panel_height - RULER_HEIGHT).max(0.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(TRACK_HEADER_WIDTH, headers_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            egui::ScrollArea::vertical()
                                .id_salt("devices_track_headers")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    let track_ids: Vec<u64> = project.tracks.iter().map(|t| t.id).collect();
                                    for track_id in track_ids {
                                        let Some(track_snapshot) = project.track(track_id).cloned() else {
                                            continue;
                                        };
                                        let (header, _) = ui.allocate_exact_size(
                                            Vec2::new(TRACK_HEADER_WIDTH, LANE_HEIGHT),
                                            Sense::hover(),
                                        );
                                        let mut select_request = None;
                                        // `track_header_row` allocates its M/S controls via
                                        // `allocate_ui_at_rect`, which rewinds a top-down cursor to
                                        // the controls' rect near the row top. On this real layout
                                        // column that would drag the next header up over this one
                                        // (overlap + clipping). Run it in a child UI pinned to the
                                        // row rect so only the `allocate_exact_size` above advances
                                        // the parent cursor.
                                        let mut row_ui = ui.new_child(
                                            UiBuilder::new()
                                                .id_salt(("devices_header_row", track_id))
                                                .max_rect(header)
                                                .layout(Layout::top_down(Align::Min)),
                                        );
                                        row_ui.set_clip_rect(header);
                                        track_header_row(
                                            &mut row_ui,
                                            header,
                                            header,
                                            &track_snapshot,
                                            track_id,
                                            project,
                                            engine,
                                            catalog,
                                            history,
                                            &mut self.change_instrument_search,
                                            None,
                                            theme,
                                            *selected_track == Some(track_id),
                                            &mut select_request,
                                            &mut self.plugin_editor_request,
                                            &mut self.delete_track_request,
                                            &mut self.duplicate_track_request,
                                            &mut self.hovered_track_header,
                                            None,
                                            "devices",
                                            track_rename,
                                        );
                                        if let Some(id) = select_request {
                                            *selected_track = Some(id);
                                        }
                                    }
                                });
                        },
                    );
                },
            );

            ui.separator();

            ui.allocate_ui_with_layout(ui.available_size(), Layout::top_down(Align::Min), |ui| {
                if let Some(track_id) = *selected_track {
                    if let Some(track_snapshot) = project.track(track_id).cloned() {
                        self.mini_selected_clip_ids.retain(|clip_id| {
                            track_snapshot.clips.iter().any(|clip| clip.id() == *clip_id)
                        });
                        self.show_mini_playlist(
                            ui,
                            project,
                            engine,
                            history,
                            &track_snapshot,
                            decoded_audio,
                            theme,
                        );
                        ui.add_space(8.0);
                        settings_dirty |= self.show_device_chain(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            settings,
                            &track_snapshot,
                            theme,
                            ChainLayout::Page,
                        );
                    }
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("No tracks in project").color(theme.text_muted));
                    });
                }
            });
        });
        settings_dirty
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show_strip(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        device_errors: &HashMap<(u64, u64), String>,
        selected_track: &mut Option<u64>,
        settings: &mut AppSettings,
        theme: &ThemeColors,
    ) -> DevicesStripOutput {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);
        let mut output = DevicesStripOutput::default();

        if selected_track.and_then(|id| project.track(id)).is_none() {
            *selected_track = project.tracks.first().map(|track| track.id);
        }

        let Some(track_id) = *selected_track else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No tracks in project").color(theme.text_muted));
            });
            return output;
        };

        let Some(track_snapshot) = project.track(track_id).cloned() else {
            return output;
        };

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&track_snapshot.name)
                    .color(theme.track_header_text)
                    .strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Hide").clicked() {
                    output.hide = true;
                }
                if ui.button("Expand").clicked() {
                    output.expand = true;
                }
            });
        });
        ui.add_space(4.0);

        output.settings_dirty = self.show_device_chain(
            ui,
            project,
            engine,
            catalog,
            history,
            device_errors,
            settings,
            &track_snapshot,
            theme,
            ChainLayout::Dock,
        );

        output
    }
}

impl DevicesUi {
    #[allow(clippy::too_many_arguments)]
    fn show_mini_playlist(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        track: &Track,
        decoded_audio: &HashMap<PathBuf, Arc<DecodedAudio>>,
        theme: &ThemeColors,
    ) {
        let (mini_rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), MINI_PLAYLIST_HEIGHT),
            Sense::hover(),
        );
        ui.painter().rect_filled(mini_rect, 4.0, theme.panel_bg);
        ui.painter().rect_stroke(
            mini_rect,
            4.0,
            Stroke::new(1.0_f32, theme.separator),
            egui::StrokeKind::Inside,
        );

        let mut mini_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(("devices_mini_root", track.id))
                .max_rect(mini_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        mini_ui.set_clip_rect(mini_rect);
        let total_beats = project.arrangement_length_beats();
        let timeline_view_w = if self.mini_timeline_view_w > 0.0 {
            self.mini_timeline_view_w
        } else {
            mini_rect.width().max(1.0)
        };
        let (min_beat_width, max_beat_width) =
            arrangement_beat_width_bounds(timeline_view_w, total_beats);
        self.mini_beat_width = self
            .mini_beat_width
            .clamp(min_beat_width, max_beat_width);
        apply_horizontal_wheel_controls(
            &mini_ui,
            mini_rect,
            &mut self.mini_beat_width,
            &mut self.mini_scroll_offset.x,
            min_beat_width,
            max_beat_width,
            0.0,
        );

        let metrics = TimelineMetrics {
            beat_width: self.mini_beat_width,
        };
        let content_width = total_beats * metrics.beat_width;
        // Exactly ruler + one lane tall: single-track lane never scrolls vertically,
        // so this keeps the lane flush against the horizontal scrollbar (no dead gap).
        let canvas_size = Vec2::new(
            content_width.max(mini_rect.width()),
            RULER_HEIGHT + LANE_HEIGHT,
        );
        let scroll = self.mini_scroll_offset;

        let output = with_solid_scrollbars(&mut mini_ui, theme, |ui| {
            // Horizontal-only: one track means no vertical scrollbar is ever needed.
            egui::ScrollArea::horizontal()
                .id_salt(("devices_mini_playlist", track.id))
                .auto_shrink([false, false])
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                .scroll_offset(scroll)
                .show(ui, |ui| {
                    ui.set_min_size(canvas_size);
                    let (response, painter) = ui.allocate_painter(canvas_size, Sense::click_and_drag());
                    let content = response.rect;
                    // `draw_lane_timeline`/ruler helpers include a fixed left gutter width.
                    // Shift the virtual timeline left so that gutter aligns with the left
                    // column we already render outside this mini-canvas.
                    let timeline_rect = content.translate(Vec2::new(-TRACK_HEADER_WIDTH, 0.0));
                    let body = timeline_body_rect(timeline_rect);
                    let ruler = Rect::from_min_max(
                        timeline_rect.min,
                        Pos2::new(timeline_rect.right(), timeline_rect.top() + RULER_HEIGHT),
                    );
                    let lane = Rect::from_min_max(
                        Pos2::new(body.left(), body.top()),
                        Pos2::new(body.right(), body.top() + LANE_HEIGHT),
                    );

                    // Pointer first so loop/clip/playhead mutations paint this frame.
                    let gesture_active =
                        self.mini_active_drag.is_some() || self.mini_marquee.is_some();
                    let loop_handled = !gesture_active
                        && handle_loop_region_pointer(
                            &response,
                            ruler,
                            body,
                            metrics,
                            project,
                            &mut self.mini_dragging_loop_edge,
                        );
                    if loop_handled {
                        // Loop edge drag owns the pointer this frame.
                    } else if !gesture_active
                        && handle_timeline_playhead_pointer(
                            &response,
                            ruler,
                            lane,
                            metrics,
                            engine,
                            &mut self.mini_dragging_playhead,
                            0.0,
                        )
                    {
                        // Playhead scrub; skip clip picks while dragging.
                    } else {
                        handle_single_track_clip_pointer(
                            &response,
                            timeline_rect,
                            lane,
                            track.id,
                            &track.clips,
                            metrics,
                            project,
                            history,
                            &mut self.mini_selected_clip_ids,
                            &mut self.mini_active_drag,
                            &mut self.mini_marquee,
                            &mut self.open_clip_request,
                            &mut self.mini_drag_moved,
                        );
                    }

                    let raise_clip_ids: HashSet<u64> = self
                        .mini_active_drag
                        .as_ref()
                        .map(|drag| drag.moving_clip_ids().collect())
                        .unwrap_or_default();
                    let timeline_painter = painter.with_clip_rect(content);
                    draw_lane_timeline(
                        &timeline_painter,
                        lane,
                        body,
                        metrics,
                        total_beats,
                        project.beats_per_bar,
                        &track.clips,
                        &self.mini_selected_clip_ids,
                        &raise_clip_ids,
                        project.track_audible(track),
                        project.bpm,
                        decoded_audio,
                        &project.pattern_override_windows_for_track(track.id),
                        theme,
                    );
                    draw_ruler(
                        &timeline_painter,
                        ruler,
                        ruler,
                        metrics,
                        total_beats,
                        project.beats_per_bar,
                        theme,
                    );
                    if let Some((loop_start, loop_end)) = project.loop_span() {
                        let hover_edge = response.hover_pos().and_then(|pos| {
                            hit_test_loop_edge(ruler, body, metrics, loop_start, loop_end, pos)
                        });
                        let highlighted = self.mini_dragging_loop_edge.or(hover_edge);
                        draw_loop_region(
                            &timeline_painter,
                            ruler,
                            lane,
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
                        &timeline_painter,
                        ruler,
                        lane,
                        metrics,
                        anchor,
                        playhead,
                        true,
                        theme,
                    );
                    draw_playhead(
                        &timeline_painter,
                        ruler,
                        lane,
                        metrics,
                        playhead,
                        true,
                        theme,
                    );

                    if let Some(marquee) = &self.mini_marquee {
                        draw_marquee(&timeline_painter, marquee.rect(), theme);
                    }
                })
        });

        self.mini_scroll_offset = output.state.offset;
        self.mini_timeline_view_w = output.inner_rect.width().max(1.0);
        if self.mini_active_drag.is_none() && self.mini_marquee.is_none() {
            self.mini_drag_moved = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_device_chain(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        device_errors: &HashMap<(u64, u64), String>,
        settings: &mut AppSettings,
        track: &Track,
        theme: &ThemeColors,
        layout: ChainLayout,
    ) -> bool {
        let mut settings_dirty = false;
        self.sanitize_view_state(track.id, track);

        if layout == ChainLayout::Page {
            ui.label(
                RichText::new(format!("Track devices: {}", track.name))
                    .color(theme.track_header_text)
                    .strong(),
            );
            ui.add_space(4.0);
        } else {
            ui.label(
                RichText::new("Devices")
                    .small()
                    .strong()
                    .color(theme.track_header_text),
            );
            ui.add_space(4.0);
        }

        let content_width = (ui.available_width() - COLUMN_PADDING * 2.0).max(120.0);
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: COLUMN_PADDING as i8,
                right: COLUMN_PADDING as i8,
                ..egui::Margin::ZERO
            })
            .show(ui, |ui| {
                ui.set_width(content_width + COLUMN_PADDING * 2.0);
                let scroll_id = match layout {
                    ChainLayout::Page => ("devices_patch_page", track.id),
                    ChainLayout::Dock => ("devices_patch_dock", track.id),
                };
                egui::ScrollArea::vertical()
                    .id_salt(scroll_id)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(content_width);
                        ui.spacing_mut().item_spacing.y = STRIP_GAP;
                        settings_dirty |= self.paint_device_patch(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
                            settings,
                            track,
                            theme,
                            content_width,
                        );
                    });
            });
        settings_dirty
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_device_patch(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        device_errors: &HashMap<(u64, u64), String>,
        settings: &mut AppSettings,
        track: &Track,
        theme: &ThemeColors,
        content_width: f32,
    ) -> bool {
        if let DevicesView::Detail {
            track_id,
            target_key,
        } = self.view
        {
            if track_id == track.id {
                return self.paint_device_detail(
                    ui,
                    project,
                    engine,
                    history,
                    settings,
                    track,
                    theme,
                    content_width,
                    target_key,
                );
            }
            self.view = DevicesView::Patch;
        }

        let mut settings_dirty = false;
        {
            let key = self
                .selected_modulator_target
                .entry(track.id)
                .or_insert(INSTRUMENT_MOD_TARGET_KEY);
            *key = normalize_modulator_target_key(track, *key);
        }

        let current_target = self
            .selected_modulator_target
            .get(&track.id)
            .copied()
            .unwrap_or(INSTRUMENT_MOD_TARGET_KEY);

        let instrument_selected = current_target == INSTRUMENT_MOD_TARGET_KEY;
        let instrument_mod_count =
            modulator_count_for_target(track, TargetFilter::Instrument);
        let instrument_action = self.paint_instrument_strip(
            ui,
            project,
            track,
            engine,
            catalog,
            history,
            settings,
            &mut settings_dirty,
            theme,
            content_width,
            instrument_selected,
            instrument_mod_count,
        );
        self.handle_strip_action(
            project,
            history,
            track.id,
            INSTRUMENT_MOD_TARGET_KEY,
            instrument_action,
            None,
            Some(track.name.clone()),
        );
        settings_dirty |= self.paint_strip_inline_section(
            ui,
            project,
            engine,
            history,
            settings,
            track,
            theme,
            content_width,
            INSTRUMENT_MOD_TARGET_KEY,
        );

        let mut drag_from: Option<usize> = None;
        let mut drag_to: Option<usize> = None;

        for (index, device) in track.devices.iter().enumerate() {
            let status = device_errors.get(&(track.id, device.id)).map(String::as_str);
            let editor_open = engine.plugin_editor_is_open(PluginRef::device(track.id, device.id));
            let slot_ready = engine.plugin_slot_ready(PluginRef::device(track.id, device.id));
            let payload = (track.id, index);
            let drag_id = Id::new(("device_strip_drag", track.id, device.id));
            let device_id = device.id;
            let device_name = device.name.clone();
            let device_unique_id = device.unique_id.clone();
            let device_bypassed = device.bypassed;
            let device_selected = current_target == device_id;
            let device_mod_count =
                modulator_count_for_target(track, TargetFilter::Device { device_id });

            let (strip_response, action) = self.paint_fx_strip(
                ui,
                track,
                device,
                engine,
                status,
                editor_open,
                slot_ready,
                theme,
                content_width,
                drag_id,
                payload,
                device_selected,
                device_mod_count,
            );
            let strip_response = strip_response.on_hover_text("Right-click for favorite params");
            strip_response.context_menu(|ui| {
                settings_dirty |= show_favorites_menu(
                    ui,
                    settings,
                    engine,
                    theme,
                    track.id,
                    Some(device_id),
                    &device_unique_id,
                );
            });

            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                if let Some(hovered) = strip_response.dnd_hover_payload::<(u64, usize)>() {
                    if hovered.0 == track.id {
                        let rect = strip_response.rect;
                        let insert_idx = if pointer.y < rect.center().y {
                            index
                        } else {
                            index + 1
                        };
                        let before = insert_idx == index;
                        let stroke = Stroke::new(2.0_f32, theme.accent);
                        let y = if before { rect.top() } else { rect.bottom() };
                        ui.painter().hline(rect.x_range(), y, stroke);
                        if let Some(released) =
                            strip_response.dnd_release_payload::<(u64, usize)>()
                        {
                            if released.0 == track.id {
                                drag_from = Some(released.1);
                                drag_to = Some(insert_idx);
                            }
                        }
                    }
                }
            }

            self.handle_strip_action(
                project,
                history,
                track.id,
                device_id,
                action,
                Some(device_bypassed),
                Some(device_name),
            );
            settings_dirty |= self.paint_strip_inline_section(
                ui,
                project,
                engine,
                history,
                settings,
                track,
                theme,
                content_width,
                device_id,
            );
        }

        if let (Some(from), Some(mut to)) = (drag_from, drag_to) {
            if from < to {
                to -= 1;
            }
            if from != to {
                history.begin(project);
                project.move_device(track.id, from, to);
                history.commit(project);
            }
        }

        add_fx_strip(ui, project, catalog, &mut self.add_fx_search, track.id, theme, content_width);
        settings_dirty
    }

    fn handle_strip_action(
        &mut self,
        project: &mut Project,
        history: &mut EditHistory,
        track_id: u64,
        target_key: u64,
        action: DeviceStripAction,
        bypassed: Option<bool>,
        device_name: Option<String>,
    ) {
        match action {
            DeviceStripAction::None => {}
            DeviceStripAction::Select => {
                self.selected_modulator_target.insert(track_id, target_key);
            }
            DeviceStripAction::ToggleInline(kind) => {
                self.toggle_inline_section(track_id, target_key, kind);
            }
            DeviceStripAction::ToggleBodyExpand => {
                self.toggle_body_expand(track_id, target_key);
            }
            DeviceStripAction::OpenDetail => {
                self.open_detail(track_id, target_key);
            }
            DeviceStripAction::ToggleBypass => {
                if let Some(bypassed) = bypassed {
                    history.push_before(project.clone());
                    project.set_device_bypass(track_id, target_key, !bypassed);
                }
            }
            DeviceStripAction::Remove => {
                history.push_before(project.clone());
                project.remove_device(track_id, target_key);
                if self
                    .selected_modulator_target
                    .get(&track_id)
                    .copied()
                    == Some(target_key)
                {
                    self.selected_modulator_target
                        .insert(track_id, INSTRUMENT_MOD_TARGET_KEY);
                }
                if self.expanded_body == Some((track_id, target_key)) {
                    self.expanded_body = None;
                }
                if self.inline_section.is_some_and(|(tid, key, _)| tid == track_id && key == target_key)
                {
                    self.inline_section = None;
                }
                if let DevicesView::Detail {
                    track_id: detail_track,
                    target_key: detail_key,
                } = self.view
                {
                    if detail_track == track_id && detail_key == target_key {
                        self.view = DevicesView::Patch;
                    }
                }
            }
            DeviceStripAction::OpenEditor => {
                self.plugin_editor_request = Some(PluginEditorRequest::Open {
                    track_id,
                    device_id: if target_key == INSTRUMENT_MOD_TARGET_KEY {
                        None
                    } else {
                        Some(target_key)
                    },
                    title: device_name.unwrap_or_else(|| "Plugin".to_string()),
                });
            }
            DeviceStripAction::CloseEditor => {
                self.plugin_editor_request = Some(PluginEditorRequest::Close {
                    track_id,
                    device_id: if target_key == INSTRUMENT_MOD_TARGET_KEY {
                        None
                    } else {
                        Some(target_key)
                    },
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_strip_inline_section(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        settings: &mut AppSettings,
        track: &Track,
        theme: &ThemeColors,
        content_width: f32,
        target_key: u64,
    ) -> bool {
        let Some((track_id, key, kind)) = self.inline_section else {
            return false;
        };
        if track_id != track.id || key != target_key {
            return false;
        }

        let mut settings_dirty = false;
        ui.add_space(SECTION_GAP);
        match kind {
            InlineSectionKind::Macros => {
                show_macro_panel(
                    ui,
                    project,
                    engine,
                    history,
                    settings,
                    &mut settings_dirty,
                    track,
                    theme,
                    content_width,
                );
            }
            InlineSectionKind::Lfo => {
                let target_filter = target_filter_from_device_key(target_key);
                show_modulator_panel(
                    ui,
                    project,
                    track,
                    track.id,
                    target_filter,
                    ModulatorLayout::Compact,
                    Some(content_width),
                    engine,
                    history,
                    settings,
                    &mut settings_dirty,
                    theme,
                );
            }
            InlineSectionKind::Fav => {
                if favorites_column_visible(track, target_key, settings) {
                    let target_filter = target_filter_from_device_key(target_key);
                    settings_dirty |= show_favorites_panel(
                        ui,
                        project,
                        engine,
                        history,
                        settings,
                        track,
                        target_filter,
                        theme,
                        content_width,
                    );
                } else {
                    ui.label(
                        RichText::new("No favorite params for this device")
                            .small()
                            .color(theme.text_muted),
                    );
                }
            }
        }
        settings_dirty
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_device_detail(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        settings: &mut AppSettings,
        track: &Track,
        theme: &ThemeColors,
        content_width: f32,
        target_key: u64,
    ) -> bool {
        let mut settings_dirty = false;
        if ui.button("< Back to patch").clicked() {
            self.back_to_patch();
            return settings_dirty;
        }
        ui.add_space(4.0);

        let label = detail_target_label(track, target_key);
        ui.label(
            RichText::new(label)
                .strong()
                .color(theme.track_header_text),
        );
        ui.add_space(SECTION_GAP);

        paint_device_body_placeholder(
            ui,
            track,
            engine,
            theme,
            content_width,
            target_key,
            &mut self.plugin_editor_request,
        );
        ui.add_space(SECTION_GAP);

        show_macro_panel(
            ui,
            project,
            engine,
            history,
            settings,
            &mut settings_dirty,
            track,
            theme,
            content_width,
        );
        ui.add_space(SECTION_GAP);

        let target_filter = target_filter_from_device_key(target_key);
        show_modulator_panel(
            ui,
            project,
            track,
            track.id,
            target_filter,
            ModulatorLayout::Compact,
            Some(content_width),
            engine,
            history,
            settings,
            &mut settings_dirty,
            theme,
        );

        if favorites_column_visible(track, target_key, settings) {
            ui.add_space(SECTION_GAP);
            settings_dirty |= show_favorites_panel(
                ui,
                project,
                engine,
                history,
                settings,
                track,
                target_filter,
                theme,
                content_width,
            );
        }
        settings_dirty
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_instrument_strip(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        track: &Track,
        engine: &dyn DawEngine,
        catalog: &PluginCatalog,
        history: &mut EditHistory,
        settings: &mut AppSettings,
        settings_dirty: &mut bool,
        theme: &ThemeColors,
        content_width: f32,
        selected: bool,
        mod_count: usize,
    ) -> DeviceStripAction {
        let target_key = INSTRUMENT_MOD_TARGET_KEY;
        let body_expanded = self.expanded_body == Some((track.id, target_key));
        let inline_macros = self.inline_section == Some((track.id, target_key, InlineSectionKind::Macros));
        let inline_lfo = self.inline_section == Some((track.id, target_key, InlineSectionKind::Lfo));
        let inline_fav = self.inline_section == Some((track.id, target_key, InlineSectionKind::Fav));
        let is_plugin = matches!(track.instrument, TrackInstrument::Plugin { .. });
        let editor_open = engine.plugin_editor_is_open(PluginRef::instrument(track.id));
        let slot_ready = engine.plugin_slot_ready(PluginRef::instrument(track.id));
        let fav_available = unique_id_for_target(track, target_key).is_some();
        let mut action = DeviceStripAction::None;

        let meta_lines = 0usize;
        let (strip_response, mut strip_ui, _total_height) = device_strip_shell(
            ui,
            theme.widget_bg_active,
            theme.accent,
            ("devices_instrument_strip", track.id),
            selected,
            mod_count,
            theme,
            content_width,
            body_expanded,
            meta_lines,
        );

        strip_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_height(STRIP_HEADER_ROW_HEIGHT);
            ui.label(
                RichText::new("INST")
                    .small()
                    .strong()
                    .monospace()
                    .color(theme.accent),
            );
            ui.label(
                RichText::new(truncate_label(track.instrument.display_name(), 14))
                    .color(theme.track_header_text)
                    .strong()
                    .small(),
            );
            ui.label(
                RichText::new(track.instrument.format_badge().unwrap_or("Piano"))
                    .color(theme.text_muted)
                    .small()
                    .monospace(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if strip_expand_button(ui, body_expanded, theme) {
                    action = DeviceStripAction::ToggleBodyExpand;
                }
                let button_action = paint_strip_buttons(
                    ui,
                    theme,
                    inline_macros,
                    inline_lfo,
                    inline_fav && fav_available,
                    is_plugin,
                    editor_open,
                    slot_ready,
                    false,
                    false,
                );
                if button_action != DeviceStripAction::None {
                    action = button_action;
                }
            });
        });
        if body_expanded {
            paint_expanded_body_in_strip(
                &mut strip_ui,
                track,
                engine,
                theme,
                content_width,
                target_key,
                &mut self.plugin_editor_request,
            );
        }

        let instrument_unique_id = unique_id_for_target(track, target_key)
            .unwrap_or_default()
            .to_string();
        let response =
            strip_response.on_hover_text("Right-click for favorites / change instrument.");
        response.context_menu(|ui| {
            if !instrument_unique_id.is_empty() {
                *settings_dirty |= show_favorites_menu(
                    ui,
                    settings,
                    engine,
                    theme,
                    track.id,
                    None,
                    &instrument_unique_id,
                );
                ui.separator();
            }
            ui.label("Change instrument");
            ui.separator();
            if let Some(choice) = show_instrument_picker(
                ui,
                catalog,
                &mut self.change_instrument_search,
                &format!("devfx_chg_{}", track.id),
                false,
                MENU_LIST_MAX_HEIGHT,
            ) {
                let rename = match &choice {
                    InstrumentChoice::Plugin(entry) => Some(entry.name.clone()),
                    InstrumentChoice::BuiltInPiano => None,
                };
                let instrument = choice_to_instrument(choice);
                history.push_before(project.clone());
                if let Some(track_mut) = project.track_mut(track.id) {
                    if let Some(name) = rename {
                        track_mut.name = name;
                    }
                    track_mut.plugin_state = None;
                    track_mut.instrument = instrument;
                }
                self.change_instrument_search.clear();
                ui.close_menu();
            }
        });
        if action == DeviceStripAction::None
            && (response.clicked() || response.secondary_clicked())
        {
            action = DeviceStripAction::Select;
        }
        action
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_fx_strip(
        &mut self,
        ui: &mut Ui,
        track: &Track,
        device: &Device,
        engine: &dyn DawEngine,
        status: Option<&str>,
        editor_open: bool,
        slot_ready: bool,
        theme: &ThemeColors,
        content_width: f32,
        drag_id: Id,
        drag_payload: (u64, usize),
        selected: bool,
        mod_count: usize,
    ) -> (egui::Response, DeviceStripAction) {
        let target_key = device.id;
        let body_expanded = self.expanded_body == Some((drag_payload.0, target_key));
        let inline_macros =
            self.inline_section == Some((drag_payload.0, target_key, InlineSectionKind::Macros));
        let inline_lfo =
            self.inline_section == Some((drag_payload.0, target_key, InlineSectionKind::Lfo));
        let inline_fav =
            self.inline_section == Some((drag_payload.0, target_key, InlineSectionKind::Fav));

        let fill = if device.bypassed {
            theme.widget_bg
        } else {
            theme.widget_bg_active
        };
        let meta_lines = usize::from(status.is_some());
        let (strip_response, mut strip_ui, _total_height) = device_strip_shell(
            ui,
            fill,
            theme.separator,
            ("device_strip", device.id),
            selected,
            mod_count,
            theme,
            content_width,
            body_expanded,
            meta_lines,
        );
        let mut action = DeviceStripAction::None;
        strip_ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_height(STRIP_HEADER_ROW_HEIGHT);
            ui.label(
                RichText::new("FX")
                    .small()
                    .strong()
                    .monospace()
                    .color(theme.text_muted),
            );
            ui.label(
                RichText::new(truncate_label(&device.name, 12))
                    .color(theme.track_header_text)
                    .strong()
                    .small(),
            );
            ui.label(
                RichText::new(device.format_badge())
                    .color(theme.text_muted)
                    .small()
                    .monospace(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.dnd_drag_source(drag_id, drag_payload, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new("::")
                                .color(theme.text_muted)
                                .monospace()
                                .small(),
                        )
                        .sense(Sense::hover()),
                    )
                    .on_hover_text("Drag to reorder");
                });
                if strip_expand_button(ui, body_expanded, theme) {
                    action = DeviceStripAction::ToggleBodyExpand;
                }
                let button_action = paint_strip_buttons(
                    ui,
                    theme,
                    inline_macros,
                    inline_lfo,
                    inline_fav,
                    true,
                    editor_open,
                    slot_ready,
                    device.bypassed,
                    true,
                );
                if button_action != DeviceStripAction::None {
                    action = button_action;
                }
            });
        });
        if let Some(status) = status {
            strip_ui.label(
                RichText::new(truncate_label(status, 28))
                    .color(theme.accent_warning)
                    .small(),
            );
        }
        if body_expanded {
            paint_expanded_body_in_strip(
                &mut strip_ui,
                track,
                engine,
                theme,
                content_width,
                target_key,
                &mut self.plugin_editor_request,
            );
        }
        if action == DeviceStripAction::None
            && (strip_response.clicked() || strip_response.secondary_clicked())
        {
            action = DeviceStripAction::Select;
        }
        (strip_response, action)
    }
}

fn strip_button(
    ui: &mut Ui,
    label: &str,
    active: bool,
    theme: &ThemeColors,
    hover: &str,
) -> bool {
    let fill = if active { theme.accent } else { theme.widget_bg };
    let stroke = if active { theme.accent } else { theme.separator };
    let text = if active { theme.panel_bg } else { theme.button_text };
    ui.add(
        egui::Button::new(RichText::new(label).small().strong().color(text))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .corner_radius(3.0)
            .min_size(Vec2::new(22.0, STRIP_BUTTON_HEIGHT)),
    )
    .on_hover_text(hover)
    .clicked()
}

fn strip_expand_button(ui: &mut Ui, expanded: bool, _theme: &ThemeColors) -> bool {
    let label = if expanded { "v" } else { ">" };
    ui.small_button(label).on_hover_text(if expanded {
        "Collapse device body"
    } else {
        "Expand device body"
    }).clicked()
}

#[allow(clippy::too_many_arguments)]
fn paint_strip_buttons(
    ui: &mut Ui,
    theme: &ThemeColors,
    macros_active: bool,
    lfo_active: bool,
    fav_active: bool,
    show_plugin_edit: bool,
    editor_open: bool,
    slot_ready: bool,
    bypassed: bool,
    is_fx: bool,
) -> DeviceStripAction {
    let mut action = DeviceStripAction::None;
    if strip_button(ui, "M", macros_active, theme, "Show macros") {
        action = DeviceStripAction::ToggleInline(InlineSectionKind::Macros);
    }
    if strip_button(ui, "L", lfo_active, theme, "Show LFO / MSEG modulators") {
        action = DeviceStripAction::ToggleInline(InlineSectionKind::Lfo);
    }
    if strip_button(ui, "*", fav_active, theme, "Show favorite params") {
        action = DeviceStripAction::ToggleInline(InlineSectionKind::Fav);
    }
    if strip_button(ui, "Det", false, theme, "Open detail view") {
        action = DeviceStripAction::OpenDetail;
    }
    if show_plugin_edit {
        let label = if editor_open {
            "Close"
        } else if slot_ready {
            "Edit"
        } else {
            "..."
        };
        if ui
            .add_enabled(
                editor_open || slot_ready,
                egui::Button::new(RichText::new(label).small().color(theme.button_text))
                    .fill(theme.widget_bg)
                    .stroke(Stroke::new(1.0_f32, theme.separator))
                    .corner_radius(3.0)
                    .min_size(Vec2::new(30.0, STRIP_BUTTON_HEIGHT)),
            )
            .on_hover_text(if editor_open {
                "Close plugin editor"
            } else if slot_ready {
                "Open plugin editor"
            } else {
                "Loading..."
            })
            .clicked()
        {
            action = if editor_open {
                DeviceStripAction::CloseEditor
            } else {
                DeviceStripAction::OpenEditor
            };
        }
    }
    if is_fx && ms_toggle_button(ui, "Byp", bypassed, theme) {
        action = DeviceStripAction::ToggleBypass;
    }
    if is_fx && strip_button(ui, "x", false, theme, "Remove device") {
        action = DeviceStripAction::Remove;
    }
    action
}

fn device_strip_shell(
    ui: &mut Ui,
    fill: egui::Color32,
    stroke: egui::Color32,
    id_salt: impl std::hash::Hash,
    selected: bool,
    mod_count: usize,
    theme: &ThemeColors,
    content_width: f32,
    body_expanded: bool,
    meta_lines: usize,
) -> (egui::Response, Ui, f32) {
    let body_height = if body_expanded {
        STRIP_EXPANDED_BODY_HEIGHT
    } else {
        0.0
    };
    let total_height = strip_collapsed_height(meta_lines) + body_height;
    let stroke_color = if selected { theme.accent } else { stroke };
    let stroke_width = if selected { 2.0_f32 } else { 1.0_f32 };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(content_width, total_height), Sense::click());
    ui.painter().rect_filled(rect, STRIP_ROUNDING, fill);
    ui.painter().rect_stroke(
        rect,
        STRIP_ROUNDING,
        Stroke::new(stroke_width, stroke_color),
        egui::StrokeKind::Inside,
    );

    if mod_count > 0 {
        let badge_center = Pos2::new(rect.right() - 10.0, rect.top() + 10.0);
        ui.painter()
            .circle_filled(badge_center, 8.0, theme.accent);
        ui.painter().text(
            badge_center,
            Align2::CENTER_CENTER,
            format!("{mod_count}"),
            egui::FontId::proportional(9.0),
            theme.panel_bg,
        );
    }

    let content_rect = rect.shrink(STRIP_INNER_MARGIN);
    let mut content_ui = ui.new_child(
        UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(content_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    content_ui.set_clip_rect(content_rect);
    content_ui.set_width(content_width - STRIP_INNER_MARGIN * 2.0);
    (response, content_ui, total_height)
}

fn detail_target_label(track: &Track, target_key: u64) -> String {
    if target_key == INSTRUMENT_MOD_TARGET_KEY {
        format!("Instrument: {}", track.instrument.display_name())
    } else {
        track
            .devices
            .iter()
            .find(|device| device.id == target_key)
            .map(|device| format!("FX: {}", device.name))
            .unwrap_or_else(|| "FX".to_string())
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_device_body_placeholder(
    ui: &mut Ui,
    track: &Track,
    engine: &dyn DawEngine,
    theme: &ThemeColors,
    content_width: f32,
    target_key: u64,
    plugin_editor_request: &mut Option<PluginEditorRequest>,
) {
    let (body_rect, _) = ui.allocate_exact_size(
        Vec2::new(content_width, STRIP_EXPANDED_BODY_HEIGHT),
        Sense::hover(),
    );
    ui.painter().rect_filled(body_rect, STRIP_ROUNDING, theme.widget_bg);
    ui.painter().rect_stroke(
        body_rect,
        STRIP_ROUNDING,
        Stroke::new(1.0_f32, theme.separator),
        egui::StrokeKind::Inside,
    );
    ui.allocate_ui_at_rect(body_rect.shrink(STRIP_INNER_MARGIN), |ui| {
        if target_key == INSTRUMENT_MOD_TARGET_KEY {
            match &track.instrument {
                TrackInstrument::Plugin { .. } => {
                    ui.label(
                        RichText::new("VST/CLAP instrument")
                            .small()
                            .color(theme.text_muted),
                    );
                    ui.label(
                        RichText::new("Use Edit for the native plugin window.")
                            .small()
                            .color(theme.text_muted),
                    );
                }
                TrackInstrument::BuiltInPiano => {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Built-in Piano")
                                .small()
                                .color(theme.text_muted),
                        );
                    });
                }
            }
        } else if let Some(device) = track.devices.iter().find(|d| d.id == target_key) {
            ui.label(
                RichText::new(truncate_label(&device.name, 24))
                    .strong()
                    .small()
                    .color(theme.track_header_text),
            );
            ui.label(
                RichText::new(device.format_badge())
                    .small()
                    .monospace()
                    .color(theme.text_muted),
            );
            ui.label(
                RichText::new("Use Edit for the native plugin window.")
                    .small()
                    .color(theme.text_muted),
            );
        }
        let _ = engine;
        let _ = plugin_editor_request;
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_expanded_body_in_strip(
    ui: &mut Ui,
    track: &Track,
    engine: &dyn DawEngine,
    theme: &ThemeColors,
    content_width: f32,
    target_key: u64,
    plugin_editor_request: &mut Option<PluginEditorRequest>,
) {
    ui.add_space(4.0);
    paint_device_body_placeholder(
        ui,
        track,
        engine,
        theme,
        content_width - STRIP_INNER_MARGIN * 2.0,
        target_key,
        plugin_editor_request,
    );
}

fn add_fx_strip(
    ui: &mut Ui,
    project: &mut Project,
    catalog: &PluginCatalog,
    add_fx_search: &mut String,
    track_id: u64,
    theme: &ThemeColors,
    content_width: f32,
) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(content_width, strip_collapsed_height(0) * 0.6),
        Sense::hover(),
    );
    ui.painter().rect_filled(rect, STRIP_ROUNDING, theme.panel_bg);
    ui.painter().rect_stroke(
        rect,
        STRIP_ROUNDING,
        Stroke::new(1.0_f32, theme.separator.gamma_multiply(0.85)),
        egui::StrokeKind::Inside,
    );
    ui.allocate_ui_at_rect(rect.shrink(STRIP_INNER_MARGIN), |ui| {
        ui.centered_and_justified(|ui| {
            ui.menu_button(
                RichText::new("+ Add FX").small().color(theme.text_muted),
                |ui| {
                    if let Some(entry) = show_effect_picker(
                        ui,
                        catalog,
                        add_fx_search,
                        &format!("devfx_add_{track_id}"),
                        false,
                        MENU_LIST_MAX_HEIGHT,
                    ) {
                        project.add_device(track_id, entry.format, &entry.unique_id, &entry.name);
                        add_fx_search.clear();
                        ui.close_menu();
                    }
                },
            );
        });
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_width_is_always_resizable() {
        let (min, default, max) = dock_panel_width_bounds();
        assert!(min < max);
        assert!(default >= min && default <= max);
        let mut devices = DevicesUi::default();
        let plan = devices.dock_panel_width(None, &AppSettings::default());
        assert!(plan.resizable);
        assert_eq!(plan.min_width, min);
        assert_eq!(plan.max_width, max);
    }

    #[test]
    fn note_dock_panel_width_clamps_remembered_width() {
        let mut devices = DevicesUi::default();
        let (_, _, max) = dock_panel_width_bounds();
        devices.note_dock_panel_width(max + 500.0);
        assert!(devices.dock_column_width <= DOCK_MAX_WIDTH);
        let (min, _, _) = dock_panel_width_bounds();
        devices.note_dock_panel_width(min - 100.0);
        assert!(devices.dock_column_width >= DOCK_MIN_WIDTH);
    }
}
