//! Performance center view: callback CPU history + per-track DSP table.

use std::collections::{HashMap, VecDeque};

use egui::{
    Align2, Color32, FontFamily, FontId, Grid, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2,
};

use crate::engine::{DawEngine, EnginePerformance, TrackVoiceKind};
use crate::model::Project;
use crate::ui::theme::ThemeColors;

/// Samples kept for the CPU graph (~5 s at 60 fps).
const CPU_HISTORY_LEN: usize = 300;
const GRAPH_HEIGHT: f32 = 120.0;

#[derive(Debug, Default)]
pub struct PerformanceUi {
    cpu_history: VecDeque<f32>,
}

impl PerformanceUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        project: &Project,
        engine: &dyn DawEngine,
        theme: &ThemeColors,
    ) {
        ui.ctx().request_repaint();
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);

        let summary = engine.performance();
        self.cpu_history.push_back(summary.cpu_percent);
        while self.cpu_history.len() > CPU_HISTORY_LEN {
            self.cpu_history.pop_front();
        }

        ui.heading("Performance");
        ui.label(
            RichText::new(
                "Live audio-thread load. Transport strip shows the same CPU / buffer / xrun totals.",
            )
            .color(theme.text_muted),
        );
        ui.add_space(8.0);

        show_summary_row(ui, engine, summary, theme);
        ui.add_space(12.0);

        ui.label(RichText::new("Callback CPU (recent)").strong());
        ui.add_space(4.0);
        draw_cpu_graph(ui, &self.cpu_history, summary.cpu_percent, theme);
        ui.add_space(12.0);

        ui.label(RichText::new("Per-track DSP (latest callback)").strong());
        ui.label(
            RichText::new("Sorted by total ms. Voice = instrument; FX = insert chain; Samples = audio clips.")
                .color(theme.text_muted)
                .small(),
        );
        ui.add_space(4.0);

        let mut rows = engine.track_performance();
        rows.sort_by(|a, b| {
            b.total_ms
                .partial_cmp(&a.total_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let track_names: HashMap<u64, &str> = project
            .tracks
            .iter()
            .map(|t| (t.id, t.name.as_str()))
            .collect();
        let instruments: HashMap<u64, String> = project
            .tracks
            .iter()
            .map(|t| {
                let badge = t
                    .instrument
                    .format_badge()
                    .map(|b| format!(" [{b}]"))
                    .unwrap_or_default();
                (
                    t.id,
                    format!("{}{badge}", t.instrument.display_name()),
                )
            })
            .collect();
        let device_counts: HashMap<u64, usize> = project
            .tracks
            .iter()
            .map(|t| (t.id, t.devices.len()))
            .collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Grid::new("perf_track_grid")
                    .striped(true)
                    .num_columns(8)
                    .spacing([12.0, 4.0])
                    .min_col_width(48.0)
                    .show(ui, |ui| {
                        header_cell(ui, "Track", theme);
                        header_cell(ui, "Instrument", theme);
                        header_cell(ui, "Kind", theme);
                        header_cell(ui, "Voices", theme);
                        header_cell(ui, "Voice ms", theme);
                        header_cell(ui, "FX ms", theme);
                        header_cell(ui, "Total ms", theme);
                        header_cell(ui, "Locks", theme);
                        ui.end_row();

                        if rows.is_empty() {
                            ui.label(
                                RichText::new("No track voices on the audio thread yet.")
                                    .color(theme.text_muted),
                            );
                            ui.end_row();
                            return;
                        }

                        let budget_ms = if summary.sample_rate_hz > 0
                            && summary.buffer_frames > 0
                        {
                            (summary.buffer_frames as f32 / summary.sample_rate_hz as f32) * 1000.0
                        } else {
                            0.0
                        };

                        for row in &rows {
                            let name = track_names
                                .get(&row.track_id)
                                .copied()
                                .unwrap_or("?");
                            let instrument = instruments
                                .get(&row.track_id)
                                .map(String::as_str)
                                .unwrap_or("-");
                            let fx_n = device_counts.get(&row.track_id).copied().unwrap_or(0);
                            let hot = budget_ms > 0.0 && row.total_ms >= budget_ms * 0.25;
                            let warn = row.lock_skips > 0
                                || (budget_ms > 0.0 && row.total_ms >= budget_ms * 0.5);
                            let value_color = if warn {
                                theme.accent_warning
                            } else if hot {
                                theme.accent
                            } else {
                                theme.text_primary
                            };

                            ui.label(RichText::new(name).color(theme.text_primary));
                            ui.label(
                                RichText::new(instrument)
                                    .color(theme.text_muted)
                                    .small(),
                            );
                            ui.label(RichText::new(row.voice_kind.label()).monospace());
                            let voices = if row.voice_kind == TrackVoiceKind::Piano {
                                format!("{}", row.active_voices)
                            } else if fx_n > 0 {
                                format!("- / {fx_n}fx")
                            } else {
                                String::from("-")
                            };
                            ui.label(RichText::new(voices).monospace());
                            ui.label(
                                RichText::new(format!("{:.2}", row.voice_ms))
                                    .monospace()
                                    .color(value_color),
                            );
                            ui.label(
                                RichText::new(format!("{:.2}", row.fx_ms))
                                    .monospace()
                                    .color(value_color),
                            );
                            ui.label(
                                RichText::new(format!("{:.2}", row.total_ms))
                                    .monospace()
                                    .strong()
                                    .color(value_color),
                            );
                            ui.label(
                                RichText::new(format!("{}", row.lock_skips))
                                    .monospace()
                                    .color(if row.lock_skips > 0 {
                                        theme.accent_warning
                                    } else {
                                        theme.text_muted
                                    }),
                            );
                            ui.end_row();
                        }
                    });
            });
    }
}

fn header_cell(ui: &mut Ui, text: &str, theme: &ThemeColors) {
    ui.label(RichText::new(text).strong().color(theme.text_muted).small());
}

fn show_summary_row(
    ui: &mut Ui,
    engine: &dyn DawEngine,
    summary: EnginePerformance,
    theme: &ThemeColors,
) {
    let device = engine
        .audio_device_name()
        .unwrap_or_else(|| String::from("(no device)"));
    let (pending_inst, pending_fx) = engine.pending_plugin_loads();
    let rate_k = if summary.sample_rate_hz >= 1000 {
        format!("{}k", (summary.sample_rate_hz + 500) / 1000)
    } else {
        format!("{} Hz", summary.sample_rate_hz)
    };
    let buf = if summary.buffer_frames > 0 {
        format!("{} frames @ {rate_k}", summary.buffer_frames)
    } else {
        format!("buffer pending @ {rate_k}")
    };
    let latency = if summary.latency_ms > 0.0 {
        format!("{:.1} ms", summary.latency_ms)
    } else {
        String::from("--")
    };
    let warn = summary.cpu_percent >= 80.0 || summary.xruns > 0 || summary.lock_skips > 0;
    let cpu_color = if warn {
        theme.accent_warning
    } else {
        theme.text_primary
    };

    ui.horizontal_wrapped(|ui| {
        metric_chip(ui, "CPU", format!("{:.1}%", summary.cpu_percent), cpu_color, theme);
        metric_chip(ui, "Buffer", buf, theme.text_primary, theme);
        metric_chip(ui, "Latency", latency, theme.text_primary, theme);
        metric_chip(
            ui,
            "Xruns",
            format!("{}", summary.xruns),
            if summary.xruns > 0 {
                theme.accent_warning
            } else {
                theme.text_primary
            },
            theme,
        );
        metric_chip(
            ui,
            "Lock skips",
            format!("{}", summary.lock_skips),
            if summary.lock_skips > 0 {
                theme.accent_warning
            } else {
                theme.text_primary
            },
            theme,
        );
        metric_chip(
            ui,
            "Pending loads",
            format!("{pending_inst} inst / {pending_fx} fx"),
            if pending_inst + pending_fx > 0 {
                theme.accent
            } else {
                theme.text_primary
            },
            theme,
        );
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("Output: {device}"))
            .color(theme.text_muted)
            .small(),
    );
}

fn metric_chip(ui: &mut Ui, label: &str, value: String, value_color: Color32, theme: &ThemeColors) {
    egui::Frame::new()
        .fill(theme.widget_bg)
        .stroke(Stroke::new(1.0_f32, theme.separator))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .corner_radius(4.0)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(label).small().color(theme.text_muted));
                ui.label(RichText::new(value).monospace().strong().color(value_color));
            });
        });
    ui.add_space(6.0);
}

fn draw_cpu_graph(ui: &mut Ui, history: &VecDeque<f32>, current: f32, theme: &ThemeColors) {
    let (rect, _response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), GRAPH_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme.widget_bg);
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0_f32, theme.separator),
        egui::StrokeKind::Inside,
    );

    let pad = 8.0;
    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + pad, rect.top() + pad),
        Pos2::new(rect.right() - pad, rect.bottom() - pad),
    );
    if plot.width() <= 1.0 || plot.height() <= 1.0 {
        return;
    }

    // Reference lines at 50% and 100%.
    for pct in [50.0_f32, 100.0] {
        let y = plot.bottom() - (pct / 100.0).clamp(0.0, 1.25) / 1.25 * plot.height();
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, theme.separator),
        );
        painter.text(
            Pos2::new(plot.left() + 2.0, y - 2.0),
            Align2::LEFT_BOTTOM,
            format!("{pct:.0}%"),
            FontId::new(10.0, FontFamily::Proportional),
            theme.text_muted,
        );
    }

    if history.len() < 2 {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "Collecting samples...",
            FontId::new(12.0, FontFamily::Proportional),
            theme.text_muted,
        );
        return;
    }

    let max_pct = history
        .iter()
        .copied()
        .fold(100.0_f32, f32::max)
        .max(current)
        .max(1.0);
    let y_scale = max_pct.max(100.0) * 1.05;

    let n = history.len().saturating_sub(1).max(1) as f32;
    let mut points: Vec<Pos2> = Vec::with_capacity(history.len());
    for (i, value) in history.iter().enumerate() {
        let x = plot.left() + (i as f32 / n) * plot.width();
        let y = plot.bottom() - (value / y_scale).clamp(0.0, 1.0) * plot.height();
        points.push(Pos2::new(x, y));
    }

    let line_color = if current >= 80.0 {
        theme.accent_warning
    } else {
        theme.accent
    };
    painter.add(Shape::line(points, Stroke::new(1.5_f32, line_color)));

    painter.text(
        Pos2::new(rect.right() - 8.0, rect.top() + 8.0),
        Align2::RIGHT_TOP,
        format!("{current:.1}%"),
        FontId::new(12.0, FontFamily::Monospace),
        line_color,
    );
}
