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
use std::ops::RangeInclusive;

use egui::{Align, Align2, Color32, FontFamily, FontId, Id, Layout, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, Vec2};

use crate::engine::DawEngine;
use crate::model::{EditHistory, Project, MAX_GAIN_DB, MIN_GAIN_DB};
use crate::ui::playlist::ms_toggle_button;
use crate::ui::theme::ThemeColors;

const STRIP_WIDTH: f32 = 132.0;
/// Design / minimum strip height; taller panels grow strips from here.
const STRIP_HEIGHT_MIN: f32 = 352.0;
const STRIP_ROUNDING: f32 = 6.0;
const STRIP_MARGIN: f32 = 10.0;
const STRIP_GAP: f32 = 10.0;
/// Inset of the strip row from the mixer panel edges.
const MIXER_PADDING_LEFT: f32 = 12.0;
const MIXER_PADDING_BOTTOM: f32 = 12.0;

const TOP_BAR_HEIGHT: f32 = 3.0;
const CONTROL_ROW_HEIGHT: f32 = 24.0;
/// Fader track height at [`STRIP_HEIGHT_MIN`]; grows 1:1 with strip height.
const FADER_HEIGHT_MIN: f32 = 148.0;
const FADER_TRACK_WIDTH: f32 = 22.0;
const FADER_VALUE_HEIGHT: f32 = 14.0;
/// Gain readout under the fader — smaller than body text so the rail dominates.
const FADER_VALUE_FONT: f32 = 9.0;
const METER_WIDTH: f32 = 8.0;
const METER_GAP: f32 = 3.0;
const METER_DECAY: f32 = 0.85;
const PAN_ROW_HEIGHT: f32 = 20.0;
const PAN_LABEL_WIDTH: f32 = 22.0;
const PAN_SLIDER_WIDTH: f32 = 48.0;
const PAN_VALUE_WIDTH: f32 = 28.0;
const PAN_VALUE_FONT: f32 = 9.0;
const FOOTER_ROW_HEIGHT: f32 = 28.0;

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
        ui.heading("Mixer");
        ui.label(
            egui::RichText::new(
                "Same track object as playlist / inspector — gain, pan, M/S, meters.",
            )
            .color(theme.text_muted)
            .small(),
        );
        ui.add_space(10.0);

        self.decay_meters(engine);

        // Fill the panel: strip chrome stays fixed, fader/meter absorb the delta.
        let strip_height =
            (ui.available_height() - MIXER_PADDING_BOTTOM).max(STRIP_HEIGHT_MIN);
        let fader_height = FADER_HEIGHT_MIN + (strip_height - STRIP_HEIGHT_MIN);

        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_height(strip_height);
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
                let header = strip_header(
                    ui,
                    &track.name,
                    track.instrument.display_name(),
                    theme.track_header_text,
                    theme.text_muted,
                );
                clicked = header.clicked();

                ui.add_space(6.0);
                fixed_height_row(ui, CONTROL_ROW_HEIGHT, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
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

                ui.add_space(8.0);
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
                        t.gain_db = gain;
                    }
                });

                ui.add_space(8.0);
                let mut pan = track.pan;
                let pan_response = pan_row(ui, &mut pan, theme);
                apply_slider_with_history(history, project, &pan_response, |project| {
                    if let Some(t) = project.track_mut(track_id) {
                        t.pan = pan;
                    }
                });

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(4.0));
                fixed_height_row(ui, FOOTER_ROW_HEIGHT, |ui| {
                    ui.vertical(|ui| {
                        footer_chip(
                            ui,
                            &format!("{} send{}", track.sends.len(), plural(track.sends.len())),
                            theme.text_muted,
                        );
                        footer_chip(
                            ui,
                            &format!("{} fx", track.devices.len()),
                            theme.text_muted,
                        );
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
                strip_header(ui, "Master", "Stereo bus", theme.accent, theme.text_muted);

                ui.add_space(6.0);
                fixed_height_row(ui, CONTROL_ROW_HEIGHT, |_ui| {});

                ui.add_space(8.0);
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
                    project.master_gain_db = gain;
                });

                ui.add_space(8.0);
                fixed_height_row(ui, PAN_ROW_HEIGHT, |_ui| {});

                ui.add_space(6.0);
                ui.add(egui::Separator::default().spacing(4.0));
                fixed_height_row(ui, FOOTER_ROW_HEIGHT, |ui| {
                    ui.vertical(|ui| {
                        footer_chip(ui, "output", theme.text_muted);
                        footer_chip(ui, "stereo out", theme.text_muted);
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

/// Name (truncated, clickable) + subtitle line. Same two-line shape for every strip.
fn strip_header(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    title_color: Color32,
    subtitle_color: Color32,
) -> egui::Response {
    let title_response = ui.add(
        egui::Label::new(egui::RichText::new(title).color(title_color).strong())
            .truncate()
            .sense(Sense::click()),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(subtitle)
                .color(subtitle_color)
                .small(),
        )
        .truncate(),
    );
    title_response
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

/// Vertical fader + stereo meter, with a manually-painted (never-wrapping) dB readout.
///
/// egui vertical sliders take their *length* from [`egui::style::Spacing::slider_width`],
/// not from [`Ui::add_sized`] — set that before adding the slider or the rail stays
/// at the default short length while the strip grows around it.
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

    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = METER_GAP;
            // Vertical rail length (see module note above).
            ui.spacing_mut().slider_width = fader_height;

            let slider = egui::Slider::new(value, range)
                .vertical()
                .show_value(false)
                .trailing_fill(true);
            // Thickness x length; length is driven by slider_width set above.
            slider_response =
                Some(ui.add_sized(Vec2::new(FADER_TRACK_WIDTH, fader_height), slider));

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
            &format!("{value:+.1}"),
            FADER_VALUE_FONT,
            theme.text_muted,
        );
    });

    slider_response.expect("slider is always added above")
}

/// Compact pan control: fixed-width label, fixed-width slider, painted readout.
fn pan_row(ui: &mut Ui, pan: &mut f32, theme: &ThemeColors) -> egui::Response {
    let mut slider_response = None;
    fixed_height_row(ui, PAN_ROW_HEIGHT, |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_sized(
                Vec2::new(PAN_LABEL_WIDTH, PAN_ROW_HEIGHT),
                egui::Label::new(
                    egui::RichText::new("Pan")
                        .color(theme.text_muted)
                        .small(),
                ),
            );
            let slider = egui::Slider::new(pan, -1.0..=1.0).show_value(false);
            slider_response = Some(
                ui.add_sized(Vec2::new(PAN_SLIDER_WIDTH, PAN_ROW_HEIGHT * 0.7), slider),
            );
            painted_value(
                ui,
                Vec2::new(PAN_VALUE_WIDTH, PAN_ROW_HEIGHT),
                &pan_label(*pan),
                PAN_VALUE_FONT,
                theme.text_muted,
            );
        });
    });
    slider_response.expect("slider is always added above")
}

fn pan_label(pan: f32) -> String {
    let amount = (pan.abs() * 100.0).round() as i32;
    if amount == 0 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{amount}")
    } else {
        format!("R{amount}")
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
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
    if response.changed() {
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
