//! Mixer view: channel strips over the same Track model (gain, pan, M/S, meters).
//!
//! Every strip (channel or master) reserves an *exact* `STRIP_WIDTH x
//! strip_height` footprint up front via [`Ui::allocate_exact_size`], then
//! draws its contents into a detached child `Ui` clipped to that rect. This
//! matters: egui's flow layouts grow a container to fit whatever a child
//! widget actually rendered (e.g. a slider's appended `.text()` label
//! wrapping in a narrow column), and previous versions of this view let that
//! leak out into the parent, so strips silently drifted apart in width and
//! height depending on the exact value shown. Pre-reserving the footprint
//! and clipping content to it makes that structurally impossible: whatever
//! happens inside a strip, its footprint in the mixer row is always
//! identical.
//!
//! `strip_height` tracks the panel's available height (floored at
//! [`STRIP_HEIGHT_MIN`]); any extra vertical space goes to the fader/meter
//! so strips fill the mixer view instead of sitting in a short top band.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::ops::RangeInclusive;

use egui::{
    epaint::TextShape, Align, Align2, Color32, Context, FontFamily, FontId, Id, Layout, Pos2,
    Rect, Sense, Stroke, Ui, UiBuilder, Vec2,
};

use crate::engine::DawEngine;
use crate::model::{EditHistory, Project, MAX_GAIN_DB, MIN_GAIN_DB};
use crate::ui::playlist::ms_toggle_button;
use crate::ui::theme::ThemeColors;

/// Compact strip width (FL-style narrow channel).
const STRIP_WIDTH: f32 = 58.0;
/// Design / minimum strip height; taller panels grow strips from here.
const STRIP_HEIGHT_MIN: f32 = 320.0;
const STRIP_ROUNDING: f32 = 4.0;
const STRIP_MARGIN: f32 = 4.0;
const STRIP_GAP: f32 = 4.0;
/// Inset of the strip row from the mixer panel edges.
const MIXER_PADDING_LEFT: f32 = 8.0;
const MIXER_PADDING_BOTTOM: f32 = 8.0;

const TOP_BAR_HEIGHT: f32 = 2.0;
const HEADER_HEIGHT: f32 = 88.0;
const CONTROL_ROW_HEIGHT: f32 = 18.0;
/// Fader track height at [`STRIP_HEIGHT_MIN`]; grows 1:1 with strip height.
const FADER_HEIGHT_MIN: f32 = 140.0;
const FADER_TRACK_WIDTH: f32 = 14.0;
const FADER_VALUE_HEIGHT: f32 = 12.0;
/// Gain readout under the fader — smaller than body text so the rail dominates.
const FADER_VALUE_FONT: f32 = 8.0;
const METER_WIDTH: f32 = 4.0;
const METER_GAP: f32 = 2.0;
const METER_DECAY: f32 = 0.85;
/// Snap gain to unity when the fader is within this many dB of 0.
const GAIN_UNITY_SNAP_DB: f32 = 0.6;
const PAN_KNOB_SIZE: f32 = 26.0;
const PAN_ROW_HEIGHT: f32 = PAN_KNOB_SIZE + 2.0;
const PAN_SNAP: f32 = 0.04;
const FOOTER_ROW_HEIGHT: f32 = 22.0;
const VERTICAL_NAME_FONT: f32 = 9.0;

/// Default bottom-panel height as a fraction of the editor area (below transport).
pub const MIXER_PANEL_DEFAULT_FRACTION: f32 = 0.5;
/// Snap targets while dragging the panel resize handle.
pub const MIXER_PANEL_SNAP_HALF: f32 = 0.5;
pub const MIXER_PANEL_SNAP_FULL: f32 = 0.92;
/// Drag below this fraction on release closes the panel.
pub const MIXER_PANEL_CLOSE_FRACTION: f32 = 0.18;
pub const MIXER_PANEL_MIN_HEIGHT: f32 = 240.0;
const MIXER_PANEL_SNAP_EPS: f32 = 0.06;

/// Clamp a stored mixer height fraction to sane bounds.
pub fn clamp_mixer_panel_fraction(fraction: f32) -> f32 {
    fraction.clamp(MIXER_PANEL_CLOSE_FRACTION, MIXER_PANEL_SNAP_FULL)
}

/// Nearest snap target after the user releases the resize handle.
pub fn snap_mixer_panel_fraction(fraction: f32) -> f32 {
    let fraction = clamp_mixer_panel_fraction(fraction);
    let snaps = [MIXER_PANEL_SNAP_HALF, MIXER_PANEL_SNAP_FULL];
    let mut best = snaps[0];
    let mut best_dist = (fraction - best).abs();
    for snap in snaps {
        let dist = (fraction - snap).abs();
        if dist < best_dist {
            best = snap;
            best_dist = dist;
        }
    }
    if best_dist <= MIXER_PANEL_SNAP_EPS {
        best
    } else {
        fraction
    }
}

/// Stable egui id for the bottom mixer `TopBottomPanel` (and its resize grip).
pub const MIXER_PANEL_ID: &str = "mixer_panel";

#[derive(Debug, Default)]
pub struct MixerPanelResize {
    tracking: bool,
}

impl MixerPanelResize {
    pub fn panel_id() -> Id {
        Id::new(MIXER_PANEL_ID)
    }

    /// True while the user is dragging the panel's top resize grip.
    pub fn is_resize_dragging(ctx: &Context) -> bool {
        let resize_id = Self::panel_id().with("__resize");
        ctx.read_response(resize_id)
            .is_some_and(|response| response.dragged())
    }

    /// Write egui's persisted panel height so content cannot expand the panel.
    ///
    /// egui `TopBottomPanel` stores the *content* rect as `PanelState`; if content
    /// asks for more height than allocated, the panel grows every frame. Pinning
    /// the stored height from our remembered fraction stops that feedback loop.
    pub fn force_height(ctx: &Context, available: Rect, height: f32) {
        let height = height.clamp(0.0, available.height().max(0.0));
        let rect = Rect::from_min_max(
            Pos2::new(available.left(), available.bottom() - height),
            available.max,
        );
        ctx.data_mut(|data| {
            data.insert_persisted(
                Self::panel_id(),
                egui::containers::panel::PanelState { rect },
            );
        });
    }

    /// Call after `TopBottomPanel::show` each frame. Returns `true` when settings
    /// should be persisted (snap on resize release).
    pub fn note_height(
        &mut self,
        ctx: &Context,
        height: f32,
        available_height: f32,
        fraction: &mut f32,
        open: &mut bool,
    ) -> bool {
        if available_height <= 1.0 {
            return false;
        }

        let dragging = Self::is_resize_dragging(ctx);
        let mut save = false;
        if self.tracking && !dragging {
            let raw = height / available_height;
            if raw < MIXER_PANEL_CLOSE_FRACTION {
                *open = false;
            } else {
                *fraction = snap_mixer_panel_fraction(raw);
                save = true;
            }
            self.tracking = false;
        } else if dragging {
            self.tracking = true;
            *fraction = clamp_mixer_panel_fraction(height / available_height);
        }
        save
    }
}

#[derive(Debug, Default)]
pub struct MixerUi {
    /// Displayed peak levels with UI-side decay: track_id -> (l, r).
    displayed: HashMap<u64, (f32, f32)>,
    master_displayed: (f32, f32),
}

impl MixerUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        selected_track: &mut Option<u64>,
        theme: &ThemeColors,
    ) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Drag the top edge to resize (snaps half / full).")
                    .color(theme.text_muted)
                    .small(),
            );
        });
        ui.add_space(4.0);

        self.decay_meters(engine);

        // Measure AFTER the chrome above so strip min-height cannot exceed the
        // panel and feed egui's content-sized PanelState growth loop.
        let strip_height = (ui.available_height() - MIXER_PADDING_BOTTOM).max(0.0);
        let fader_height = if strip_height >= STRIP_HEIGHT_MIN {
            FADER_HEIGHT_MIN + (strip_height - STRIP_HEIGHT_MIN)
        } else {
            (FADER_HEIGHT_MIN * (strip_height / STRIP_HEIGHT_MIN)).max(48.0)
        };

        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Never request more than the remaining panel space.
                ui.set_min_height(strip_height.min(ui.available_height()));
                ui.horizontal_top(|ui| {
                    ui.add_space(MIXER_PADDING_LEFT);
                    ui.spacing_mut().item_spacing.x = STRIP_GAP;
                    let track_ids: Vec<u64> = project.tracks.iter().map(|t| t.id).collect();
                    for track_id in track_ids {
                        let selected = *selected_track == Some(track_id);
                        let (peak_l, peak_r) =
                            self.displayed.get(&track_id).copied().unwrap_or((0.0, 0.0));
                        self.channel_strip(
                            ui,
                            project,
                            engine,
                            history,
                            selected_track,
                            track_id,
                            selected,
                            peak_l,
                            peak_r,
                            strip_height,
                            fader_height,
                            theme,
                        );
                    }

                    ui.add_space(4.0);
                    let (ml, mr) = self.master_displayed;
                    self.master_strip(
                        ui,
                        project,
                        history,
                        ml,
                        mr,
                        strip_height,
                        fader_height,
                        theme,
                    );
                });
                ui.add_space(MIXER_PADDING_BOTTOM);
            });
    }

    fn decay_meters(&mut self, engine: &dyn DawEngine) {
        let levels = engine.meter_levels();
        let live: HashMap<u64, (f32, f32)> = levels
            .into_iter()
            .map(|(id, l, r)| (id, (l, r)))
            .collect();
        for (id, (l, r)) in &live {
            let prev = self.displayed.get(id).copied().unwrap_or((0.0, 0.0));
            self.displayed.insert(
                *id,
                (l.max(prev.0 * METER_DECAY), r.max(prev.1 * METER_DECAY)),
            );
        }
        for (id, (l, r)) in self.displayed.iter_mut() {
            if !live.contains_key(id) {
                *l *= METER_DECAY;
                *r *= METER_DECAY;
            }
        }
        self.displayed.retain(|_, (l, r)| *l > 0.001 || *r > 0.001);

        let (raw_l, raw_r) = engine.master_meter();
        self.master_displayed = (
            raw_l.max(self.master_displayed.0 * METER_DECAY),
            raw_r.max(self.master_displayed.1 * METER_DECAY),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn channel_strip(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
        history: &mut EditHistory,
        selected_track: &mut Option<u64>,
        track_id: u64,
        selected: bool,
        peak_l: f32,
        peak_r: f32,
        strip_height: f32,
        fader_height: f32,
        theme: &ThemeColors,
    ) {
        let Some(track) = project.tracks.iter().find(|t| t.id == track_id).cloned() else {
            return;
        };

        let mut clicked = false;
        strip_container(
            ui,
            Id::new(("mixer_strip", track_id)),
            strip_height,
            selected,
            selected,
            theme,
            |ui| {
                let header = vertical_strip_header(
                    ui,
                    &track.name,
                    track.instrument.display_name(),
                    theme.track_header_text,
                    theme.text_muted,
                );
                clicked = header.clicked();

                fixed_height_row(ui, CONTROL_ROW_HEIGHT, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ms_toggle_button(ui, "M", track.muted, theme) {
                            history.push_before(project.clone());
                            let exclusive = ui.input(|i| i.modifiers.shift);
                            if exclusive {
                                project.exclusive_mute(track_id);
                            } else if let Some(t) = project.track_mut(track_id) {
                                t.muted = !t.muted;
                            }
                            engine.all_notes_off();
                        }
                        if ms_toggle_button(ui, "S", track.solo, theme) {
                            history.push_before(project.clone());
                            let exclusive = ui.input(|i| i.modifiers.shift);
                            if exclusive {
                                project.exclusive_solo(track_id);
                            } else if let Some(t) = project.track_mut(track_id) {
                                t.solo = !t.solo;
                            }
                            engine.all_notes_off();
                        }
                    });
                });

                ui.add_space(4.0);
                let mut gain = track.gain_db;
                let response = fader_section(
                    ui,
                    &mut gain,
                    MIN_GAIN_DB..=MAX_GAIN_DB,
                    peak_l,
                    peak_r,
                    fader_height,
                    theme,
                );
                apply_slider_with_history(history, project, &response, |project| {
                    if let Some(t) = project.track_mut(track_id) {
                        t.gain_db = snap_gain_db(gain);
                    }
                });

                ui.add_space(4.0);
                let mut pan = track.pan;
                let pan_response = pan_knob(ui, &mut pan, theme);
                apply_slider_with_history(history, project, &pan_response, |project| {
                    if let Some(t) = project.track_mut(track_id) {
                        t.pan = snap_pan(pan);
                    }
                });

                ui.add_space(4.0);
                fixed_height_row(ui, FOOTER_ROW_HEIGHT, |ui| {
                    ui.vertical_centered(|ui| {
                        footer_chip(
                            ui,
                            &format!("{}s", track.sends.len()),
                            theme.text_muted,
                        );
                        footer_chip(ui, &format!("{}fx", track.devices.len()), theme.text_muted);
                    });
                });
            },
        );

        if clicked {
            *selected_track = Some(track_id);
        }
    }

    fn master_strip(
        &mut self,
        ui: &mut Ui,
        project: &mut Project,
        history: &mut EditHistory,
        peak_l: f32,
        peak_r: f32,
        strip_height: f32,
        fader_height: f32,
        theme: &ThemeColors,
    ) {
        strip_container(
            ui,
            Id::new("mixer_strip_master"),
            strip_height,
            true,
            false,
            theme,
            |ui| {
                vertical_strip_header(ui, "Master", "Out", theme.accent, theme.text_muted);

                fixed_height_row(ui, CONTROL_ROW_HEIGHT, |_ui| {});

                ui.add_space(4.0);
                let mut gain = project.master_gain_db;
                let response = fader_section(
                    ui,
                    &mut gain,
                    MIN_GAIN_DB..=MAX_GAIN_DB,
                    peak_l,
                    peak_r,
                    fader_height,
                    theme,
                );
                apply_slider_with_history(history, project, &response, |project| {
                    project.master_gain_db = snap_gain_db(gain);
                });

                ui.add_space(4.0);
                fixed_height_row(ui, PAN_ROW_HEIGHT, |_ui| {});

                ui.add_space(4.0);
                fixed_height_row(ui, FOOTER_ROW_HEIGHT, |ui| {
                    ui.vertical_centered(|ui| {
                        footer_chip(ui, "out", theme.text_muted);
                    });
                });
            },
        );
    }
}

/// Reserves an exact `STRIP_WIDTH x strip_height` footprint in `ui`, paints the
/// strip chrome (rounded card + top accent bar), then runs `add_contents`
/// inside a *detached* child `Ui` clipped to the card's interior. Detached
/// means: whatever `add_contents` draws can never grow the reserved
/// footprint, so every strip occupies identical space in the mixer row.
fn strip_container(
    ui: &mut Ui,
    id_salt: Id,
    strip_height: f32,
    accent_top: bool,
    selected: bool,
    theme: &ThemeColors,
    add_contents: impl FnOnce(&mut Ui),
) {
    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(STRIP_WIDTH, strip_height), Sense::hover());

    let fill = if selected {
        theme.widget_bg_active
    } else {
        theme.track_header_bg
    };
    let stroke_color = if selected { theme.accent } else { theme.separator };
    let painter = ui.painter();
    painter.rect_filled(rect, STRIP_ROUNDING, fill);
    painter.rect_stroke(
        rect,
        STRIP_ROUNDING,
        Stroke::new(1.0_f32, stroke_color),
        egui::StrokeKind::Inside,
    );

    let bar_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOP_BAR_HEIGHT));
    painter.rect_filled(
        bar_rect,
        egui::CornerRadius {
            nw: STRIP_ROUNDING as u8,
            ne: STRIP_ROUNDING as u8,
            sw: 0,
            se: 0,
        },
        if accent_top { theme.accent } else { stroke_color },
    );

    let content_rect = Rect::from_min_max(
        Pos2::new(rect.min.x + STRIP_MARGIN, rect.min.y + TOP_BAR_HEIGHT + 6.0),
        Pos2::new(rect.max.x - STRIP_MARGIN, rect.max.y - STRIP_MARGIN),
    );

    let mut content_ui = ui.new_child(
        UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(content_rect)
            .layout(Layout::top_down(Align::LEFT)),
    );
    content_ui.set_clip_rect(content_rect);
    add_contents(&mut content_ui);
}

/// Vertical track name + tiny subtitle, FL-style compact header.
fn vertical_strip_header(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    title_color: Color32,
    subtitle_color: Color32,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, HEADER_HEIGHT), Sense::click());
    paint_vertical_label(ui, rect, title, title_color, VERTICAL_NAME_FONT);
    let sub_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - 10.0),
        rect.max,
    );
    ui.painter().text(
        sub_rect.center(),
        Align2::CENTER_CENTER,
        truncate_chars(subtitle, 8),
        FontId::new(7.0, FontFamily::Proportional),
        subtitle_color,
    );
    response.on_hover_text(format!("{title}\n{subtitle}"))
}

/// Runs `add_contents` inside a child UI whose reported height is forced to
/// `height` regardless of what it draws, so optional rows (e.g. master has no
/// M/S buttons) still reserve identical vertical space to their channel-strip
/// counterpart.
fn fixed_height_row(ui: &mut Ui, height: f32, add_contents: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    ui.allocate_ui(Vec2::new(width, height), |ui| {
        ui.set_min_height(height);
        add_contents(ui);
    });
}

/// Vertical fader + stereo meter, centered in the strip, with a 0 dB tick and snap.
fn fader_section(
    ui: &mut Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    peak_l: f32,
    peak_r: f32,
    fader_height: f32,
    theme: &ThemeColors,
) -> egui::Response {
    let mut slider_response = None;
    let column_width = FADER_TRACK_WIDTH + METER_GAP + METER_WIDTH * 2.0;

    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = METER_GAP;
            ui.spacing_mut().slider_width = fader_height;

            let slider = egui::Slider::new(value, range.clone())
                .vertical()
                .show_value(false)
                .trailing_fill(true);
            let response =
                ui.add_sized(Vec2::new(FADER_TRACK_WIDTH, fader_height), slider);
            paint_unity_tick(ui, response.rect, &range, theme);
            slider_response = Some(response);

            let (meter_rect, _) = ui.allocate_exact_size(
                Vec2::new(METER_WIDTH * 2.0, fader_height),
                Sense::hover(),
            );
            draw_stereo_meter(ui, meter_rect, peak_l, peak_r, theme);
        });

        ui.add_space(2.0);
        painted_value(
            ui,
            Vec2::new(column_width, FADER_VALUE_HEIGHT),
            &format!("{:.1}", snap_gain_db(*value)),
            FADER_VALUE_FONT,
            theme.text_muted,
        );
    });

    if slider_response
        .as_ref()
        .is_some_and(|r| r.changed() || r.drag_stopped())
    {
        *value = snap_gain_db(*value);
    }

    slider_response.expect("slider is always added above")
}

/// Rotary pan knob (compact, centered).
fn pan_knob(ui: &mut Ui, pan: &mut f32, theme: &ThemeColors) -> egui::Response {
    let mut knob_response = None;
    fixed_height_row(ui, PAN_ROW_HEIGHT, |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            let size = Vec2::splat(PAN_KNOB_SIZE);
            let (rect, mut response) = ui.allocate_exact_size(size, Sense::drag());
            let center = rect.center();
            let radius = rect.width() * 0.42;
            let painter = ui.painter();

            painter.circle_filled(center, radius, theme.widget_bg);
            painter.circle_stroke(center, radius, Stroke::new(1.0_f32, theme.separator));

            let angle = pan_to_angle(*pan);
            let indicator = center + Vec2::angled(angle) * radius * 0.72;
            painter.line_segment(
                [center, indicator],
                Stroke::new(1.5_f32, theme.track_header_text),
            );

            if response.dragged() {
                let delta = response.drag_delta();
                *pan = (*pan + delta.x * 0.01 - delta.y * 0.01).clamp(-1.0, 1.0);
                *pan = snap_pan(*pan);
                response.mark_changed();
            }
            if response.double_clicked() {
                *pan = 0.0;
                response.mark_changed();
            }

            knob_response = Some(response.on_hover_text(format!("Pan: {}", pan_label(*pan))));
        });
    });
    knob_response.expect("knob is always added above")
}

fn pan_to_angle(pan: f32) -> f32 {
    // -1 (full L) .. +1 (full R), 12 o'clock = center.
    -FRAC_PI_2 + pan.clamp(-1.0, 1.0) * FRAC_PI_2 * 0.85
}

fn snap_gain_db(db: f32) -> f32 {
    if db.abs() <= GAIN_UNITY_SNAP_DB {
        0.0
    } else {
        db.clamp(MIN_GAIN_DB, MAX_GAIN_DB)
    }
}

fn snap_pan(pan: f32) -> f32 {
    if pan.abs() <= PAN_SNAP {
        0.0
    } else {
        pan.clamp(-1.0, 1.0)
    }
}

fn paint_unity_tick(
    ui: &Ui,
    slider_rect: Rect,
    range: &RangeInclusive<f32>,
    theme: &ThemeColors,
) {
    let min = *range.start();
    let max = *range.end();
    if !(min..=max).contains(&0.0) {
        return;
    }
    let t = (0.0 - min) / (max - min);
    let y = slider_rect.top() + (1.0 - t) * slider_rect.height();
    let tick = Stroke::new(1.0_f32, theme.accent.gamma_multiply(0.85));
    ui.painter().line_segment(
        [
            Pos2::new(slider_rect.left() - 2.0, y),
            Pos2::new(slider_rect.right() + 2.0, y),
        ],
        tick,
    );
}

/// Horizontal label rotated 90 deg counter-clockwise for FL-style strip names.
fn paint_vertical_label(ui: &Ui, rect: Rect, text: &str, color: Color32, font_size: f32) {
    let label = truncate_chars(text, 14);
    let font_id = FontId::new(font_size, FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(label, font_id, color);
    let size = galley.size();
    let center = rect.center();
    // Pivot is the galley top-left; -90 deg puts the string along the strip.
    let pos = Pos2::new(center.x + size.y * 0.5, center.y - size.x * 0.5);
    let shape = TextShape::new(pos, galley, color)
        .with_angle(-FRAC_PI_2)
        .with_override_text_color(color);
    ui.painter().add(shape);
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn pan_label(pan: f32) -> String {
    let pan = snap_pan(pan);
    let amount = (pan.abs() * 100.0).round() as i32;
    if amount == 0 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{amount}")
    } else {
        format!("R{amount}")
    }
}

/// Single small muted line in the footer badge column (sends / fx / output info).
fn footer_chip(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .color(color)
                .small()
                .monospace(),
        )
        .truncate(),
    );
}

/// Draws `text` centered in an exactly-`size` rect. Never wraps and never
/// changes the layout size, however long the string is — this is what keeps
/// every strip's width/height identical regardless of the numeric value shown.
fn painted_value(ui: &mut Ui, size: Vec2, text: &str, font_size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::new(font_size, FontFamily::Monospace),
        color,
    );
}

/// Coalesced undo for drag gestures; discrete undo for click-to-set.
fn apply_slider_with_history(
    history: &mut EditHistory,
    project: &mut Project,
    response: &egui::Response,
    apply: impl FnOnce(&mut Project),
) {
    if !response.changed() && !response.drag_started() && !response.drag_stopped() {
        return;
    }
    if response.drag_started() {
        history.begin(project);
    } else if response.changed() && !response.dragged() {
        history.push_before(project.clone());
    }
    if response.changed() || response.drag_stopped() {
        apply(project);
    }
    if response.drag_stopped() {
        history.commit(project);
    }
}

fn draw_stereo_meter(ui: &Ui, rect: Rect, peak_l: f32, peak_r: f32, theme: &ThemeColors) {
    let gap = 2.0;
    let half_w = (rect.width() - gap) * 0.5;
    let left = Rect::from_min_size(rect.min, Vec2::new(half_w, rect.height()));
    let right = Rect::from_min_size(
        Pos2::new(rect.min.x + half_w + gap, rect.min.y),
        Vec2::new(half_w, rect.height()),
    );
    paint_meter_bar(ui, left, peak_l, theme);
    paint_meter_bar(ui, right, peak_r, theme);
}

fn paint_meter_bar(ui: &Ui, rect: Rect, peak: f32, theme: &ThemeColors) {
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, theme.meter_bg);
    let level = peak.clamp(0.0, 1.0);
    if level <= 0.0 {
        return;
    }
    let fill_h = rect.height() * level;
    let fill = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - fill_h),
        rect.max,
    );
    let color = if level > 0.9 {
        theme.meter_high
    } else {
        theme.meter_low
    };
    painter.rect_filled(fill, 2.0, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_snaps_near_unity() {
        assert_eq!(snap_gain_db(0.0), 0.0);
        assert_eq!(snap_gain_db(0.5), 0.0);
        assert_eq!(snap_gain_db(-0.6), 0.0);
        assert!((snap_gain_db(1.0) - 1.0).abs() < f32::EPSILON);
        assert!((snap_gain_db(-2.0) - (-2.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn pan_snaps_to_center() {
        assert_eq!(snap_pan(0.0), 0.0);
        assert_eq!(snap_pan(0.03), 0.0);
        assert!((snap_pan(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn mixer_panel_fraction_snaps_to_half_and_full() {
        assert_eq!(snap_mixer_panel_fraction(0.48), MIXER_PANEL_SNAP_HALF);
        assert_eq!(snap_mixer_panel_fraction(0.9), MIXER_PANEL_SNAP_FULL);
        assert!((snap_mixer_panel_fraction(0.62) - 0.62).abs() < f32::EPSILON);
    }
}
