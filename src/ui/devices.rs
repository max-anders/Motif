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
    FAVORITES_COLUMN_WIDTH,
};
use crate::ui::macro_panel::{show_macro_panel, MACRO_COLUMN_WIDTH};
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

const TILE_WIDTH: f32 = 146.0;
const TILE_HEIGHT: f32 = 74.0;
const TILE_ROUNDING: f32 = 4.0;
const TILE_INNER_MARGIN: f32 = 6.0;
const TILE_CONTENT_WIDTH: f32 = TILE_WIDTH - TILE_INNER_MARGIN * 2.0;
const TILE_CONTENT_HEIGHT: f32 = TILE_HEIGHT - TILE_INNER_MARGIN * 2.0;
const TILE_GAP: f32 = 8.0;
// Matches `with_solid_scrollbars` bar width so the lane sits flush above the bar.
const MINI_SCROLLBAR_WIDTH: f32 = 10.0;
const STRIP_PADDING: f32 = 8.0;
const TILE_TO_MODULATOR_GAP: f32 = 8.0;
/// Horizontal inset applied inside every dock column, so headers and content
/// line up on the same left edge from one column to the next.
const COLUMN_PADDING: f32 = 8.0;
const DEVICE_COLUMN_WIDTH: f32 = TILE_WIDTH + COLUMN_PADDING * 2.0;
/// Gap + hairline between dock columns. Exact: the dock row zeroes horizontal
/// item spacing so column widths sum to the panel width with no drift.
const COLUMN_SEP_WIDTH: f32 = 9.0;
/// LFO column bounds. This is the only dock column that absorbs panel resize.
const MOD_COLUMN_MIN_WIDTH: f32 = 300.0;
const MOD_COLUMN_DEFAULT_WIDTH: f32 = 360.0;
const MOD_COLUMN_MAX_WIDTH: f32 = 760.0;

/// `Frame::side_top_panel` inner margin (8 per side). `SidePanel` widths include
/// it, the dock's own layout math does not.
const PANEL_FRAME_H_MARGIN: f32 = 16.0;

/// Combined content width of the non-resizable columns
/// (macros, optional favorites, devices), excluding the panel frame margin.
/// Each column carries its own `COLUMN_PADDING`, so no extra edge padding here.
fn dock_fixed_content_width(favorites_open: bool) -> f32 {
    let favorites = if favorites_open {
        FAVORITES_COLUMN_WIDTH + COLUMN_SEP_WIDTH
    } else {
        0.0
    };
    MACRO_COLUMN_WIDTH + COLUMN_SEP_WIDTH + favorites + DEVICE_COLUMN_WIDTH
}

/// `(min, max)` panel width for the current column visibility.
///
/// Only the LFO column stretches, so with it closed both bounds collapse to one
/// exact width. `SidePanel` clamps its stored width into this range every frame,
/// which is what makes the dock snap back instead of staying wide.
fn dock_panel_width_bounds(favorites_open: bool, mods_open: bool) -> (f32, f32) {
    let fixed = dock_fixed_content_width(favorites_open) + PANEL_FRAME_H_MARGIN;
    if !mods_open {
        return (fixed, fixed);
    }
    let base = fixed + COLUMN_SEP_WIDTH;
    (base + MOD_COLUMN_MIN_WIDTH, base + MOD_COLUMN_MAX_WIDTH)
}

/// How the dock side panel should be sized for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockPanelWidth {
    pub default_width: f32,
    pub min_width: f32,
    pub max_width: f32,
    pub resizable: bool,
}

/// Vertical rule between dock columns, allocating exactly [`COLUMN_SEP_WIDTH`].
fn column_separator(ui: &mut Ui, height: f32, theme: &ThemeColors) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(COLUMN_SEP_WIDTH, height), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, theme.separator));
}

/// Keep a dock column's contents inside its own lane so a chip that asks for more
/// width than it was given can never paint over the neighbouring column.
/// Horizontal only: vertical scrolling and popups must stay unclipped.
fn clip_column_width(ui: &mut Ui) {
    let clip = ui.clip_rect();
    let lane = Rect::from_x_y_ranges(ui.max_rect().x_range(), clip.y_range());
    ui.set_clip_rect(clip.intersect(lane));
}

/// Allocate one dock column of exactly `width` and run `contents` inside its
/// padded, clipped lane. `contents` receives the usable content width, so every
/// column derives its chip width the same way and they share one left edge.
///
/// The inset is on the left only: the matching gap on the right stays inside the
/// column so a scrollbar has a lane of its own instead of sitting on a chip.
fn dock_column<R>(
    ui: &mut Ui,
    width: f32,
    height: f32,
    item_spacing: Vec2,
    contents: impl FnOnce(&mut Ui, f32) -> R,
) -> R {
    let content_width = (width - COLUMN_PADDING * 2.0).max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        Layout::top_down(Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing = item_spacing;
            clip_column_width(ui);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: COLUMN_PADDING as i8,
                    ..egui::Margin::ZERO
                })
                .show(ui, |ui| {
                    let lane_width = content_width + COLUMN_PADDING;
                    ui.set_min_width(lane_width);
                    ui.set_max_width(lane_width);
                    contents(ui, content_width)
                })
                .inner
        },
    )
    .inner
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainLayout {
    Page,
    Dock,
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
enum DeviceTileAction {
    None,
    Select,
    ToggleMods,
    ToggleBypass,
    Remove,
    OpenEditor,
    CloseEditor,
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
    /// `(track_id, target_key)` whose LFO column is open in the dock. At most one
    /// slot at a time; `None` (the default) keeps the dock at its narrow width.
    mod_panel_open: Option<(u64, u64)>,
    /// LFO column width the user dragged to. Persisted separately from the panel
    /// width so other columns appearing does not resize the LFO editor.
    mod_column_width: f32,
    /// Fixed-column width this frame's plan was built from, for reading drags back.
    dock_fixed_width: f32,
    /// Column layout the panel was last sized for; a change re-pins its width.
    dock_last_layout: Option<(u32, bool)>,
    /// Columns the panel was sized for this frame.
    dock_columns: DockColumns,
}

/// Optional dock columns, resolved once per frame while planning the panel width.
///
/// Rendering follows this snapshot rather than live selection state, so a click
/// never paints a column the panel has no room for yet - that mismatch is what
/// made the LFO curve flash at the wrong width for a frame when favorites appeared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DockColumns {
    /// Target key the favorites column shows, or `None` when it is hidden.
    favorites_target: Option<u64>,
    /// Target key the LFO column shows, or `None` when it is hidden.
    mod_target: Option<u64>,
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
            mod_panel_open: None,
            mod_column_width: MOD_COLUMN_DEFAULT_WIDTH,
            dock_fixed_width: dock_fixed_content_width(false) + PANEL_FRAME_H_MARGIN,
            dock_last_layout: None,
            dock_columns: DockColumns::default(),
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
    fn dock_mod_target(&self, track_id: u64) -> Option<u64> {
        self.mod_panel_open
            .filter(|(open_track, _)| *open_track == track_id)
            .map(|(_, target_key)| target_key)
    }

    pub fn selected_target_key(&self, track_id: u64) -> u64 {
        self.selected_modulator_target
            .get(&track_id)
            .copied()
            .unwrap_or(INSTRUMENT_MOD_TARGET_KEY)
    }

    /// Which optional columns the dock wants, from current selection state.
    fn dock_target_columns(
        &self,
        track: Option<&Track>,
        settings: &AppSettings,
    ) -> DockColumns {
        let Some(track) = track else {
            return DockColumns::default();
        };
        let key = normalize_modulator_target_key(track, self.selected_target_key(track.id));
        DockColumns {
            favorites_target: favorites_column_visible(track, key, settings).then_some(key),
            mod_target: self.dock_mod_target(track.id),
        }
    }

    /// Width plan for the dock side panel this frame.
    ///
    /// The LFO column owns all of the panel's slack, so its width is what gets
    /// remembered: when the favorites column appears or disappears the panel
    /// grows or shrinks by that column instead of the LFO editor absorbing it.
    pub fn dock_panel_width(
        &mut self,
        track: Option<&Track>,
        settings: &AppSettings,
    ) -> DockPanelWidth {
        let columns = self.dock_target_columns(track, settings);
        self.dock_columns = columns;
        let favorites_open = columns.favorites_target.is_some();
        let mods_open = columns.mod_target.is_some();

        let (min_width, max_width) = dock_panel_width_bounds(favorites_open, mods_open);
        self.dock_fixed_width = dock_fixed_content_width(favorites_open) + PANEL_FRAME_H_MARGIN;

        let layout = (self.dock_fixed_width.to_bits(), mods_open);
        let layout_changed = self.dock_last_layout.replace(layout) != Some(layout);

        if !mods_open {
            return DockPanelWidth {
                default_width: min_width,
                min_width,
                max_width,
                resizable: false,
            };
        }

        let target = (self.dock_fixed_width + COLUMN_SEP_WIDTH + self.mod_column_width)
            .clamp(min_width, max_width);
        if layout_changed {
            // Pin for the one frame a column appears/disappears, so the panel
            // absorbs the change instead of the LFO column. The range reopens
            // next frame for dragging.
            DockPanelWidth {
                default_width: target,
                min_width: target,
                max_width: target,
                resizable: true,
            }
        } else {
            DockPanelWidth {
                default_width: target,
                min_width,
                max_width,
                resizable: true,
            }
        }
    }

    /// Feed the panel's realised width back after `SidePanel::show`, so a user
    /// resize drag updates the remembered LFO column width.
    pub fn note_dock_panel_width(&mut self, panel_width: f32) {
        if self.mod_panel_open.is_none() {
            return;
        }
        self.mod_column_width = (panel_width - self.dock_fixed_width - COLUMN_SEP_WIDTH)
            .clamp(MOD_COLUMN_MIN_WIDTH, MOD_COLUMN_MAX_WIDTH);
    }

    /// Open the LFO column on this slot, or close it when it is already the open one.
    fn toggle_mod_panel_for_target(&mut self, track_id: u64, target_key: u64) {
        if self.mod_panel_open == Some((track_id, target_key)) {
            self.mod_panel_open = None;
        } else {
            self.mod_panel_open = Some((track_id, target_key));
            self.selected_modulator_target.insert(track_id, target_key);
        }
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
        if layout == ChainLayout::Page {
            ui.label(
                RichText::new(format!("Track devices: {}", track.name))
                    .color(theme.track_header_text)
                    .strong(),
            );
            ui.add_space(4.0);
        }

        let selected_key =
            normalize_modulator_target_key(track, self.selected_target_key(track.id));
        let show_favorites = favorites_column_visible(track, selected_key, settings);
        let target_filter = target_filter_from_device_key(selected_key);

        match layout {
            ChainLayout::Page => {
                ui.add_space(STRIP_PADDING);
                egui::ScrollArea::vertical()
                    .id_salt(("devices_fx_grid", track.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        show_macro_panel(
                            ui,
                            project,
                            engine,
                            history,
                            settings,
                            &mut settings_dirty,
                            track,
                            theme,
                            MACRO_COLUMN_WIDTH,
                        );
                        if show_favorites {
                            ui.add_space(TILE_TO_MODULATOR_GAP);
                            settings_dirty |= show_favorites_panel(
                                ui,
                                project,
                                engine,
                                history,
                                settings,
                                track,
                                target_filter,
                                theme,
                                FAVORITES_COLUMN_WIDTH,
                            );
                        }
                        ui.add_space(TILE_TO_MODULATOR_GAP);
                        let mut mod_settings_dirty = false;
                        self.show_modulator_panel_for_track(
                            ui,
                            project,
                            engine,
                            history,
                            settings,
                            track,
                            theme,
                            ModulatorLayout::Wide,
                            None,
                            &mut mod_settings_dirty,
                        );
                        settings_dirty |= mod_settings_dirty;
                        ui.add_space(TILE_TO_MODULATOR_GAP);
                        ui.spacing_mut().item_spacing = Vec2::new(TILE_GAP, TILE_GAP);
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(STRIP_PADDING);
                            settings_dirty |= self.paint_device_chain_tiles(
                                ui,
                                project,
                                engine,
                                catalog,
                                history,
                                device_errors,
                                settings,
                                track,
                                theme,
                                ChainLayout::Page,
                            );
                        });
                    });
            }
            ChainLayout::Dock => {
                // Which columns to paint is decided by `dock_panel_width`, before the
                // panel is sized. Re-reading live state here would paint a column the
                // panel has no width for yet, for one frame.
                let columns = self.dock_columns;
                let body_height = ui.available_height();
                let total_width = ui.available_width();
                // Zeroed so column widths sum exactly to the panel width; each column
                // restores normal spacing for its own contents.
                let item_spacing = ui.spacing().item_spacing;
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    dock_column(ui, DEVICE_COLUMN_WIDTH, body_height, item_spacing, |ui, _| {
                        ui.label(
                            RichText::new("Devices")
                                .small()
                                .strong()
                                .color(theme.track_header_text),
                        );
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt(("devices_dock_chain", track.id))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_width(TILE_WIDTH);
                                ui.spacing_mut().item_spacing.y = TILE_GAP;
                                settings_dirty |= self.paint_device_chain_tiles(
                                    ui,
                                    project,
                                    engine,
                                    catalog,
                                    history,
                                    device_errors,
                                    settings,
                                    track,
                                    theme,
                                    ChainLayout::Dock,
                                );
                            });
                    });

                    if let Some(mod_target) = columns.mod_target {
                        column_separator(ui, body_height, theme);
                        // The LFO column is the only one that absorbs panel resize.
                        let mod_column_width = (total_width
                            - dock_fixed_content_width(columns.favorites_target.is_some())
                            - COLUMN_SEP_WIDTH)
                            .clamp(MOD_COLUMN_MIN_WIDTH, MOD_COLUMN_MAX_WIDTH);
                        let mod_filter = target_filter_from_device_key(
                            normalize_modulator_target_key(track, mod_target),
                        );
                        dock_column(
                            ui,
                            mod_column_width,
                            body_height,
                            item_spacing,
                            |ui, content_width| {
                                let mut mod_settings_dirty = false;
                                show_modulator_panel(
                                    ui,
                                    project,
                                    track,
                                    track.id,
                                    mod_filter,
                                    ModulatorLayout::Compact,
                                    Some(content_width),
                                    engine,
                                    history,
                                    settings,
                                    &mut mod_settings_dirty,
                                    theme,
                                );
                                settings_dirty |= mod_settings_dirty;
                            },
                        );
                    }

                    if let Some(favorites_target) = columns.favorites_target {
                        column_separator(ui, body_height, theme);
                        let target_filter = target_filter_from_device_key(
                            normalize_modulator_target_key(track, favorites_target),
                        );
                        dock_column(
                            ui,
                            FAVORITES_COLUMN_WIDTH,
                            body_height,
                            item_spacing,
                            |ui, content_width| {
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
                            },
                        );
                    }

                    column_separator(ui, body_height, theme);
                    dock_column(
                        ui,
                        MACRO_COLUMN_WIDTH,
                        body_height,
                        item_spacing,
                        |ui, content_width| {
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
                        },
                    );
                });
            }
        }
        settings_dirty
    }

    #[allow(clippy::too_many_arguments)]
    fn show_modulator_panel_for_track(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        settings: &mut AppSettings,
        track: &Track,
        theme: &ThemeColors,
        mod_layout: ModulatorLayout,
        fixed_content_width: Option<f32>,
        settings_dirty: &mut bool,
    ) {
        let selected_key = self
            .selected_modulator_target
            .entry(track.id)
            .or_insert(INSTRUMENT_MOD_TARGET_KEY);
        *selected_key = normalize_modulator_target_key(track, *selected_key);
        let target_filter = target_filter_from_device_key(*selected_key);
        show_modulator_panel(
            ui,
            project,
            track,
            track.id,
            target_filter,
            mod_layout,
            fixed_content_width,
            engine,
            history,
            settings,
            settings_dirty,
            theme,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_device_chain_tiles(
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
        // Dock: one narrow vertical column, Mod buttons own the LFO column.
        // Page: tiles wrap horizontally, Mod buttons only move the selection.
        let tiles_stack_vertically = layout == ChainLayout::Dock;
        let mod_button_toggles_panel = layout == ChainLayout::Dock;
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
        let open_mod_target = self.dock_mod_target(track.id);

        let instrument_selected =
            current_target == INSTRUMENT_MOD_TARGET_KEY;
        let instrument_mod_active =
            mod_button_toggles_panel && open_mod_target == Some(INSTRUMENT_MOD_TARGET_KEY);
        let instrument_mod_count =
            modulator_count_for_target(track, TargetFilter::Instrument);
        let (_, instrument_action) = instrument_tile(
            ui,
            project,
            track,
            engine,
            catalog,
            history,
            settings,
            &mut settings_dirty,
            &mut self.change_instrument_search,
            &mut self.plugin_editor_request,
            track.id,
            theme,
            instrument_selected,
            instrument_mod_active,
            instrument_mod_count,
        );
        match instrument_action {
            DeviceTileAction::Select => {
                self.selected_modulator_target
                    .insert(track.id, INSTRUMENT_MOD_TARGET_KEY);
            }
            DeviceTileAction::ToggleMods => {
                if mod_button_toggles_panel {
                    self.toggle_mod_panel_for_target(track.id, INSTRUMENT_MOD_TARGET_KEY);
                } else {
                    self.selected_modulator_target
                        .insert(track.id, INSTRUMENT_MOD_TARGET_KEY);
                }
            }
            _ => {}
        }

        let mut drag_from: Option<usize> = None;
        let mut drag_to: Option<usize> = None;

        for (index, device) in track.devices.iter().enumerate() {
            let status = device_errors.get(&(track.id, device.id)).map(String::as_str);
            let editor_open = engine.plugin_editor_is_open(PluginRef::device(track.id, device.id));
            let slot_ready = engine.plugin_slot_ready(PluginRef::device(track.id, device.id));

            let payload = (track.id, index);
            let drag_id = Id::new(("device_tile_drag", track.id, device.id));
            let device_id = device.id;
            let device_name = device.name.clone();
            let device_unique_id = device.unique_id.clone();
            let device_bypassed = device.bypassed;
            let device_selected =
                current_target == device_id;
            let device_mod_active =
                mod_button_toggles_panel && open_mod_target == Some(device_id);
            let device_mod_count =
                modulator_count_for_target(track, TargetFilter::Device { device_id });

            let (tile_response, action) = device_tile_contents(
                ui,
                device,
                status,
                editor_open,
                slot_ready,
                theme,
                drag_id,
                payload,
                device_selected,
                device_mod_active,
                device_mod_count,
            );
            let tile_response = tile_response.on_hover_text("Right-click for favorite params");
            tile_response.context_menu(|ui| {
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
                if let Some(hovered) = tile_response.dnd_hover_payload::<(u64, usize)>() {
                    if hovered.0 == track.id {
                        let rect = tile_response.rect;
                        // The dock stacks tiles vertically and the page wraps them
                        // horizontally, so hit-test and draw along the flow axis.
                        let insert_idx = if tiles_stack_vertically {
                            if pointer.y < rect.center().y { index } else { index + 1 }
                        } else if pointer.x < rect.center().x {
                            index
                        } else {
                            index + 1
                        };
                        let before = insert_idx == index;
                        let stroke = Stroke::new(2.0_f32, theme.accent);
                        if tiles_stack_vertically {
                            let y = if before { rect.top() } else { rect.bottom() };
                            ui.painter().hline(rect.x_range(), y, stroke);
                        } else {
                            let x = if before { rect.left() } else { rect.right() };
                            ui.painter().vline(x, rect.y_range(), stroke);
                        }
                        if let Some(released) =
                            tile_response.dnd_release_payload::<(u64, usize)>()
                        {
                            if released.0 == track.id {
                                drag_from = Some(released.1);
                                drag_to = Some(insert_idx);
                            }
                        }
                    }
                }
            }

            match action {
                DeviceTileAction::None => {}
                DeviceTileAction::Select => {
                    self.selected_modulator_target.insert(track.id, device_id);
                }
                DeviceTileAction::ToggleMods => {
                    if mod_button_toggles_panel {
                        self.toggle_mod_panel_for_target(track.id, device_id);
                    } else {
                        self.selected_modulator_target.insert(track.id, device_id);
                    }
                }
                DeviceTileAction::ToggleBypass => {
                    history.push_before(project.clone());
                    project.set_device_bypass(track.id, device_id, !device_bypassed);
                }
                DeviceTileAction::Remove => {
                    history.push_before(project.clone());
                    project.remove_device(track.id, device_id);
                    if self
                        .selected_modulator_target
                        .get(&track.id)
                        .copied()
                        == Some(device_id)
                    {
                        self.selected_modulator_target
                            .insert(track.id, INSTRUMENT_MOD_TARGET_KEY);
                    }
                    if self.mod_panel_open == Some((track.id, device_id)) {
                        self.mod_panel_open = None;
                    }
                }
                DeviceTileAction::OpenEditor => {
                    self.plugin_editor_request = Some(PluginEditorRequest::Open {
                        track_id: track.id,
                        device_id: Some(device_id),
                        title: device_name.clone(),
                    });
                }
                DeviceTileAction::CloseEditor => {
                    self.plugin_editor_request = Some(PluginEditorRequest::Close {
                        track_id: track.id,
                        device_id: Some(device_id),
                    });
                }
            }
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

        add_fx_tile(ui, project, catalog, &mut self.add_fx_search, track.id, theme);
        settings_dirty
    }
}

fn mod_tile_button(ui: &mut Ui, active: bool, theme: &ThemeColors) -> bool {
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
    ui.add(
        egui::Button::new(RichText::new("Mod").small().strong().color(text))
            .fill(fill)
            .stroke(Stroke::new(1.0_f32, stroke))
            .corner_radius(3.0)
            .min_size(Vec2::new(30.0, 18.0)),
    )
    .on_hover_text(if active {
        "Hide modulators for this device"
    } else {
        "Show LFO / MSEG modulators"
    })
    .clicked()
}

fn device_tile_action_button(
    ui: &mut Ui,
    label: &str,
    theme: &ThemeColors,
    hover: &str,
) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).small().color(theme.button_text))
            .fill(theme.widget_bg)
            .stroke(Stroke::new(1.0_f32, theme.separator))
            .corner_radius(3.0)
            .min_size(Vec2::new(30.0, 18.0)),
    )
    .on_hover_text(hover)
    .clicked()
}

#[allow(clippy::too_many_arguments)]
fn instrument_tile(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    engine: &dyn DawEngine,
    catalog: &PluginCatalog,
    history: &mut EditHistory,
    settings: &mut AppSettings,
    settings_dirty: &mut bool,
    change_instrument_search: &mut String,
    plugin_editor_request: &mut Option<PluginEditorRequest>,
    track_id: u64,
    theme: &ThemeColors,
    selected: bool,
    mod_button_active: bool,
    mod_count: usize,
) -> (egui::Response, DeviceTileAction) {
    let mut action = DeviceTileAction::None;
    let is_plugin = matches!(track.instrument, TrackInstrument::Plugin { .. });
    let editor_open = engine.plugin_editor_is_open(PluginRef::instrument(track_id));
    let slot_ready = engine.plugin_slot_ready(PluginRef::instrument(track_id));
    let track_name = track.name.clone();

    let (tile_response, mut tile_ui) = device_tile_shell(
        ui,
        theme.widget_bg_active,
        theme.accent,
        ("devices_instrument_tile", track_id),
        selected,
        mod_count,
        theme,
    );
    tile_ui.horizontal(|ui| {
        ui.label(
            RichText::new("INST")
                .small()
                .strong()
                .monospace()
                .color(theme.accent),
        );
    });
    tile_ui.label(
        RichText::new(truncate_label(track.instrument.display_name(), 16))
            .color(theme.track_header_text)
            .strong()
            .small(),
    );
    tile_ui.label(
        RichText::new(track.instrument.format_badge().unwrap_or("Piano"))
            .color(theme.text_muted)
            .small()
            .monospace(),
    );
    tile_ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if mod_tile_button(ui, mod_button_active, theme) {
                action = DeviceTileAction::ToggleMods;
            }
            if is_plugin {
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
                            .min_size(Vec2::new(34.0, 18.0)),
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
                    *plugin_editor_request = Some(if editor_open {
                        PluginEditorRequest::Close {
                            track_id,
                            device_id: None,
                        }
                    } else {
                        PluginEditorRequest::Open {
                            track_id,
                            device_id: None,
                            title: track_name,
                        }
                    });
                }
            }
        });
    });

    let instrument_unique_id = unique_id_for_target(track, INSTRUMENT_MOD_TARGET_KEY)
        .unwrap_or_default()
        .to_string();
    let response =
        tile_response.on_hover_text("Right-click for favorites / change instrument.");
    response.context_menu(|ui| {
        if !instrument_unique_id.is_empty() {
            *settings_dirty |= show_favorites_menu(
                ui,
                settings,
                engine,
                theme,
                track_id,
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
            change_instrument_search,
            &format!("devfx_chg_{track_id}"),
            false,
            MENU_LIST_MAX_HEIGHT,
        ) {
            let rename = match &choice {
                InstrumentChoice::Plugin(entry) => Some(entry.name.clone()),
                InstrumentChoice::BuiltInPiano => None,
            };
            let instrument = choice_to_instrument(choice);
            history.push_before(project.clone());
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
    });
    // Right-click selects too, so a param starred from the context menu lands on
    // the slot the favorites column is showing.
    if action == DeviceTileAction::None && (response.clicked() || response.secondary_clicked()) {
        action = DeviceTileAction::Select;
    }
    (response, action)
}

fn device_tile_shell(
    ui: &mut Ui,
    fill: egui::Color32,
    stroke: egui::Color32,
    id_salt: impl std::hash::Hash,
    selected: bool,
    mod_count: usize,
    theme: &ThemeColors,
) -> (egui::Response, Ui) {
    let stroke_color = if selected { theme.accent } else { stroke };
    let stroke_width = if selected { 2.0_f32 } else { 1.0_f32 };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(TILE_WIDTH, TILE_HEIGHT), Sense::click());
    ui.painter().rect_filled(rect, TILE_ROUNDING, fill);
    ui.painter().rect_stroke(
        rect,
        TILE_ROUNDING,
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

    let content_rect = rect.shrink(TILE_INNER_MARGIN);
    let mut content_ui = ui.new_child(
        UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(content_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    content_ui.set_clip_rect(content_rect);
    content_ui.set_min_size(Vec2::new(TILE_CONTENT_WIDTH, TILE_CONTENT_HEIGHT));
    content_ui.set_max_size(Vec2::new(TILE_CONTENT_WIDTH, TILE_CONTENT_HEIGHT));
    (response, content_ui)
}

fn device_tile_contents(
    ui: &mut Ui,
    device: &Device,
    status: Option<&str>,
    editor_open: bool,
    slot_ready: bool,
    theme: &ThemeColors,
    drag_id: Id,
    drag_payload: (u64, usize),
    selected: bool,
    mod_button_active: bool,
    mod_count: usize,
) -> (egui::Response, DeviceTileAction) {
    let mut action = DeviceTileAction::None;
    let fill = if device.bypassed {
        theme.widget_bg
    } else {
        theme.widget_bg_active
    };
    let (tile_response, mut tile_ui) = device_tile_shell(
        ui,
        fill,
        theme.separator,
        ("device_tile", device.id),
        selected,
        mod_count,
        theme,
    );
    tile_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
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
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Narrow grip only: whole-tile dnd overlays Grab and steals Edit clicks.
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
        });
    });
    tile_ui.label(
        RichText::new(device.format_badge())
            .color(theme.text_muted)
            .small()
            .monospace(),
    );
    if let Some(status) = status {
        tile_ui.label(
            RichText::new(truncate_label(status, 22))
                .color(theme.accent_warning)
                .small(),
        );
    }
    tile_ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if mod_tile_button(ui, mod_button_active, theme) {
                action = DeviceTileAction::ToggleMods;
            }
            if ms_toggle_button(ui, "Byp", device.bypassed, theme) {
                action = DeviceTileAction::ToggleBypass;
            }
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
                        .min_size(Vec2::new(34.0, 18.0)),
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
                    DeviceTileAction::CloseEditor
                } else {
                    DeviceTileAction::OpenEditor
                };
            }
            if device_tile_action_button(ui, "x", theme, "Remove device") {
                action = DeviceTileAction::Remove;
            }
        });
    });
    // Right-click selects too, so a param starred from the context menu lands on
    // the slot the favorites column is showing.
    if action == DeviceTileAction::None
        && (tile_response.clicked() || tile_response.secondary_clicked())
    {
        action = DeviceTileAction::Select;
    }
    (tile_response, action)
}

fn add_fx_tile(
    ui: &mut Ui,
    project: &mut Project,
    catalog: &PluginCatalog,
    add_fx_search: &mut String,
    track_id: u64,
    theme: &ThemeColors,
) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(TILE_WIDTH, TILE_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, TILE_ROUNDING, theme.panel_bg);
    ui.painter().rect_stroke(
        rect,
        TILE_ROUNDING,
        Stroke::new(1.0_f32, theme.separator.gamma_multiply(0.85)),
        egui::StrokeKind::Inside,
    );
    ui.allocate_ui_at_rect(rect.shrink(TILE_INNER_MARGIN), |ui| {
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
    fn dock_width_is_pinned_when_lfo_column_closed() {
        for favorites_open in [false, true] {
            let (min, max) = dock_panel_width_bounds(favorites_open, false);
            assert_eq!(min, max, "closed LFO column must leave no resize slack");
        }
    }

    #[test]
    fn lfo_column_is_the_only_resizable_one() {
        let (closed_min, closed_max) = dock_panel_width_bounds(true, false);
        assert_eq!(closed_min, closed_max);

        let (open_min, open_max) = dock_panel_width_bounds(true, true);
        assert!(open_min > closed_min);
        assert_eq!(open_max - open_min, MOD_COLUMN_MAX_WIDTH - MOD_COLUMN_MIN_WIDTH);
    }

    #[test]
    fn favorites_column_widens_dock_by_exactly_its_column() {
        let hidden = dock_panel_width_bounds(false, false).0;
        let shown = dock_panel_width_bounds(true, false).0;
        assert_eq!(shown - hidden, FAVORITES_COLUMN_WIDTH + COLUMN_SEP_WIDTH);
    }

    /// A drag is read back off the panel, and the favorites column appearing
    /// then moves the panel edge instead of squeezing the LFO editor.
    #[test]
    fn lfo_column_keeps_its_width_when_favorites_appear() {
        let mut devices = DevicesUi::default();
        devices.mod_panel_open = Some((1, 7));

        let narrow_fixed = dock_fixed_content_width(false) + PANEL_FRAME_H_MARGIN;
        devices.dock_fixed_width = narrow_fixed;
        let dragged_lfo = MOD_COLUMN_DEFAULT_WIDTH + 120.0;
        devices.note_dock_panel_width(narrow_fixed + COLUMN_SEP_WIDTH + dragged_lfo);
        assert_eq!(devices.mod_column_width, dragged_lfo);

        let wide_fixed = dock_fixed_content_width(true) + PANEL_FRAME_H_MARGIN;
        assert_eq!(
            wide_fixed - narrow_fixed,
            FAVORITES_COLUMN_WIDTH + COLUMN_SEP_WIDTH
        );
        devices.dock_fixed_width = wide_fixed;
        devices.note_dock_panel_width(wide_fixed + COLUMN_SEP_WIDTH + dragged_lfo);
        assert_eq!(devices.mod_column_width, dragged_lfo);
    }

    #[test]
    fn closed_lfo_column_ignores_panel_width_feedback() {
        let mut devices = DevicesUi::default();
        devices.note_dock_panel_width(4000.0);
        assert_eq!(devices.mod_column_width, MOD_COLUMN_DEFAULT_WIDTH);
    }

    /// The panel is sized from `dock_columns` and the dock renders exactly those
    /// columns. If the two diverge the LFO column paints at the wrong width for
    /// a frame, which reads as the curve popping.
    #[test]
    fn planned_width_matches_the_columns_recorded_for_rendering() {
        let project = Project::default();
        let track = project.tracks.first().expect("default project has a track");
        let settings = AppSettings::default();
        let mut devices = DevicesUi::default();
        devices.mod_panel_open = Some((track.id, INSTRUMENT_MOD_TARGET_KEY));

        let plan = devices.dock_panel_width(Some(track), &settings);
        let columns = devices.dock_columns;
        assert_eq!(columns.mod_target, Some(INSTRUMENT_MOD_TARGET_KEY));

        let lfo_width = plan.default_width
            - dock_fixed_content_width(columns.favorites_target.is_some())
            - PANEL_FRAME_H_MARGIN
            - COLUMN_SEP_WIDTH;
        assert_eq!(lfo_width, MOD_COLUMN_DEFAULT_WIDTH);
    }

    /// Columns inset their contents by `COLUMN_PADDING` so headers and chips
    /// share one left edge; the devices column must still fit a full tile.
    #[test]
    fn device_column_fits_a_tile_inside_the_shared_padding() {
        assert_eq!(DEVICE_COLUMN_WIDTH - COLUMN_PADDING * 2.0, TILE_WIDTH);
    }
}
