use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use egui::containers::scroll_area::ScrollBarVisibility;
use egui::{Align, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};

use crate::engine::{DawEngine, DecodedAudio, PluginCatalog, PluginRef};
use crate::model::{Device, EditHistory, Project, Track, TrackInstrument};
use crate::ui::automation::AutomationUi;
use crate::ui::instrument_menu::{
    choice_to_instrument, show_effect_picker, show_instrument_picker, InstrumentChoice,
    MENU_LIST_MAX_HEIGHT,
};
use crate::ui::playlist::{
    draw_lane_timeline, draw_marquee, handle_single_track_clip_pointer, ms_toggle_button,
    track_header_row, ClipDrag, LANE_HEIGHT, MarqueeDrag, PluginEditorRequest, TRACK_HEADER_WIDTH,
};
use crate::ui::theme::ThemeColors;
use crate::ui::timeline::{
    apply_horizontal_wheel_controls, draw_loop_region, draw_playhead, draw_ruler,
    handle_loop_region_pointer, handle_timeline_playhead_pointer, hit_test_loop_edge,
    timeline_body_rect, with_solid_scrollbars, LoopEdge, TimelineMetrics, DEFAULT_BEAT_WIDTH,
    RULER_HEIGHT,
};

const TILE_WIDTH: f32 = 146.0;
const TILE_HEIGHT: f32 = 74.0;
const TILE_ROUNDING: f32 = 4.0;
const TILE_INNER_MARGIN: f32 = 6.0;
const TILE_CONTENT_WIDTH: f32 = TILE_WIDTH - TILE_INNER_MARGIN * 2.0;
const TILE_CONTENT_HEIGHT: f32 = TILE_HEIGHT - TILE_INNER_MARGIN * 2.0;
const TILE_GAP: f32 = 8.0;
const STRIP_HEADER_HEIGHT: f32 = 28.0;
const STRIP_PADDING: f32 = 8.0;
/// Default height for the bottom device strip (header + one tile row + scrollbar).
pub const DEVICES_STRIP_HEIGHT: f32 =
    STRIP_HEADER_HEIGHT + TILE_HEIGHT + STRIP_PADDING + MINI_SCROLLBAR_WIDTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChainLayout {
    Page,
    Strip,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DevicesStripOutput {
    pub expand: bool,
    pub hide: bool,
}
// Ruler + one lane + the horizontal scrollbar strip; no vertical scroll, no gap.
const MINI_PLAYLIST_HEIGHT: f32 = RULER_HEIGHT + LANE_HEIGHT + MINI_SCROLLBAR_WIDTH;
// Matches `with_solid_scrollbars` bar width so the lane sits flush above the bar.
const MINI_SCROLLBAR_WIDTH: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceTileAction {
    None,
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
    mini_drag_moved: bool,
    automation: AutomationUi,
    automation_strip_expanded: bool,
}

impl Default for DevicesUi {
    fn default() -> Self {
        Self {
            add_fx_search: String::new(),
            change_instrument_search: String::new(),
            plugin_editor_request: None,
            delete_track_request: None,
            hovered_track_header: None,
            open_clip_request: None,
            mini_selected_clip_ids: HashSet::new(),
            mini_active_drag: None,
            mini_marquee: None,
            mini_dragging_playhead: false,
            mini_dragging_loop_edge: None,
            mini_beat_width: DEFAULT_BEAT_WIDTH,
            mini_scroll_offset: Vec2::ZERO,
            mini_drag_moved: false,
            automation: AutomationUi::default(),
            automation_strip_expanded: false,
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

    pub fn hovered_track_header(&self) -> Option<u64> {
        self.hovered_track_header
    }

    pub fn take_open_clip_request(&mut self) -> Option<u64> {
        self.open_clip_request.take()
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
        theme: &ThemeColors,
    ) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);
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
                                            &mut self.hovered_track_header,
                                            "devices",
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
                        self.automation.show_page_section(
                            ui,
                            project,
                            track_id,
                            &track_snapshot,
                            engine,
                            history,
                            self.mini_beat_width,
                            self.mini_scroll_offset,
                            theme,
                        );
                        ui.add_space(8.0);
                        self.show_device_chain(
                            ui,
                            project,
                            engine,
                            catalog,
                            history,
                            device_errors,
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

        self.automation.show_strip_section(
            ui,
            &mut self.automation_strip_expanded,
            project,
            track_id,
            &track_snapshot,
            engine,
            history,
            self.mini_beat_width,
            self.mini_scroll_offset,
            theme,
        );
        if self.automation_strip_expanded {
            ui.add_space(4.0);
        }

        self.show_device_chain(
            ui,
            project,
            engine,
            catalog,
            history,
            device_errors,
            &track_snapshot,
            theme,
            ChainLayout::Strip,
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
        apply_horizontal_wheel_controls(
            &mini_ui,
            mini_rect,
            &mut self.mini_beat_width,
            &mut self.mini_scroll_offset.x,
        );

        let metrics = TimelineMetrics {
            beat_width: self.mini_beat_width,
        };
        let total_beats = project.arrangement_length_beats();
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
                            true,
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
                        project.track_audible(track),
                        project.bpm,
                        decoded_audio,
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
        track: &Track,
        theme: &ThemeColors,
        layout: ChainLayout,
    ) {
        if layout == ChainLayout::Page {
            ui.label(
                RichText::new(format!("Track devices: {}", track.name))
                    .color(theme.track_header_text)
                    .strong(),
            );
            ui.add_space(4.0);
        }

        match layout {
            ChainLayout::Page => {
                ui.add_space(STRIP_PADDING);
                egui::ScrollArea::vertical()
                    .id_salt(("devices_fx_grid", track.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(TILE_GAP, TILE_GAP);
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(STRIP_PADDING);
                            self.paint_device_chain_tiles(
                                ui,
                                project,
                                engine,
                                catalog,
                                history,
                                device_errors,
                                track,
                                theme,
                            );
                        });
                    });
            }
            ChainLayout::Strip => {
                ui.add_space(STRIP_PADDING);
                egui::ScrollArea::horizontal()
                    .id_salt(("devices_fx_strip", track.id))
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(TILE_GAP, TILE_GAP);
                        ui.horizontal(|ui| {
                            ui.add_space(STRIP_PADDING);
                            self.paint_device_chain_tiles(
                                ui,
                                project,
                                engine,
                                catalog,
                                history,
                                device_errors,
                                track,
                                theme,
                            );
                        });
                    });
            }
        }
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
        track: &Track,
        theme: &ThemeColors,
    ) {
        instrument_tile(
            ui,
            project,
            track,
            engine,
            catalog,
            history,
            &mut self.change_instrument_search,
            &mut self.plugin_editor_request,
            track.id,
            theme,
        );

        let mut drag_from: Option<usize> = None;
        let mut drag_to: Option<usize> = None;

        for (index, device) in track.devices.iter().enumerate() {
            let status = device_errors.get(&(track.id, device.id)).map(String::as_str);
            let editor_open = engine.plugin_editor_is_open(PluginRef::device(track.id, device.id));
            let slot_ready = engine.plugin_slot_ready(PluginRef::device(track.id, device.id));

            // Drag handle only -- wrapping the whole tile in `dnd_drag_source` overlays
            // Sense::drag + Grab cursor on Edit/Byp/x and steals their clicks.
            let payload = (track.id, index);
            let drag_id = Id::new(("device_tile_drag", track.id, device.id));
            let (tile_response, action) = device_tile_contents(
                ui,
                device,
                status,
                editor_open,
                slot_ready,
                theme,
                drag_id,
                payload,
            );

            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                if let Some(hovered) = tile_response.dnd_hover_payload::<(u64, usize)>() {
                    if hovered.0 == track.id {
                        let insert_idx = if pointer.x < tile_response.rect.center().x {
                            index
                        } else {
                            index + 1
                        };
                        // Light insert cue while reordering.
                        let x = if insert_idx == index {
                            tile_response.rect.left()
                        } else {
                            tile_response.rect.right()
                        };
                        ui.painter().vline(
                            x,
                            tile_response.rect.y_range(),
                            Stroke::new(2.0, theme.accent),
                        );
                        if let Some(released) = tile_response.dnd_release_payload::<(u64, usize)>() {
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
                DeviceTileAction::ToggleBypass => {
                    history.push_before(project.clone());
                    project.set_device_bypass(track.id, device.id, !device.bypassed);
                }
                DeviceTileAction::Remove => {
                    history.push_before(project.clone());
                    project.remove_device(track.id, device.id);
                }
                DeviceTileAction::OpenEditor => {
                    self.plugin_editor_request = Some(PluginEditorRequest::Open {
                        track_id: track.id,
                        device_id: Some(device.id),
                        title: device.name.clone(),
                    });
                }
                DeviceTileAction::CloseEditor => {
                    self.plugin_editor_request = Some(PluginEditorRequest::Close {
                        track_id: track.id,
                        device_id: Some(device.id),
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

        add_fx_tile(ui, project, catalog, &mut self.add_fx_search, track.id);
    }
}

#[allow(clippy::too_many_arguments)]
fn instrument_tile(
    ui: &mut Ui,
    project: &mut Project,
    track: &Track,
    engine: &dyn DawEngine,
    catalog: &PluginCatalog,
    history: &mut EditHistory,
    change_instrument_search: &mut String,
    plugin_editor_request: &mut Option<PluginEditorRequest>,
    track_id: u64,
    theme: &ThemeColors,
) {
    let is_plugin = matches!(track.instrument, TrackInstrument::Plugin { .. });
    let editor_open = engine.plugin_editor_is_open(PluginRef::instrument(track_id));
    let slot_ready = engine.plugin_slot_ready(PluginRef::instrument(track_id));
    let track_name = track.name.clone();

    let (tile_response, mut tile_ui) = device_tile_shell(
        ui,
        theme.widget_bg_active,
        theme.accent,
        ("devices_instrument_tile", track_id),
    );
    tile_ui.label(RichText::new("Instrument").color(theme.text_muted).small());
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
        if is_plugin {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let label = if editor_open {
                    "Close"
                } else if slot_ready {
                    "Edit"
                } else {
                    "..."
                };
                if ui
                    .add_enabled(editor_open || slot_ready, egui::Button::new(label).small())
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
            });
        }
    });

    tile_response
        .on_hover_text("Right-click to change instrument")
        .context_menu(|ui| {
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
}

fn device_tile_shell(
    ui: &mut Ui,
    fill: egui::Color32,
    stroke: egui::Color32,
    id_salt: impl std::hash::Hash,
) -> (egui::Response, Ui) {
    // Hover is enough for drop-target `contains_pointer`; drag lives on the grip only.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(TILE_WIDTH, TILE_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, TILE_ROUNDING, fill);
    ui.painter().rect_stroke(
        rect,
        TILE_ROUNDING,
        Stroke::new(1.0_f32, stroke),
        egui::StrokeKind::Inside,
    );

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
    );
    tile_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            RichText::new(truncate_label(&device.name, 14))
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
            if ms_toggle_button(ui, "Byp", device.bypassed, theme) {
                action = DeviceTileAction::ToggleBypass;
            }
            if ui
                .small_button("x")
                .on_hover_text("Remove device")
                .clicked()
            {
                action = DeviceTileAction::Remove;
            }
            let label = if editor_open {
                "Close"
            } else if slot_ready {
                "Edit"
            } else {
                "..."
            };
            if ui
                .add_enabled(editor_open || slot_ready, egui::Button::new(label).small())
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
        });
    });
    (tile_response, action)
}

fn add_fx_tile(
    ui: &mut Ui,
    project: &mut Project,
    catalog: &PluginCatalog,
    add_fx_search: &mut String,
    track_id: u64,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(TILE_WIDTH, TILE_HEIGHT),
        Layout::top_down(Align::Center),
        |ui| {
            ui.centered_and_justified(|ui| {
                ui.menu_button("+ Add FX", |ui| {
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
                });
            });
        },
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
