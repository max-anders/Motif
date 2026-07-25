use egui::{RichText, Stroke, Ui};

use crate::engine::{DawEngine, EnginePerformance};
use crate::model::{Project, MIN_LOOP_SPAN_BEATS};

pub struct TransportUi;

/// Wall-clock playhead time from musical position (MM:SS.mmm).
pub(crate) fn format_playhead_time(beats: f32, beats_per_second: f32) -> String {
    let total_seconds = (beats / beats_per_second.max(f32::EPSILON)).max(0.0);
    let minutes = (total_seconds / 60.0).floor() as u32;
    let seconds = total_seconds % 60.0;
    let whole_seconds = seconds.floor() as u32;
    let millis = (seconds.fract() * 1000.0).round() as u32;
    format!("{minutes:02}:{whole_seconds:02}.{millis:03}")
}

fn show_time_box(ui: &mut Ui, beats: f32, beats_per_second: f32) {
    let text = format_playhead_time(beats, beats_per_second);
    egui::Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .stroke(Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(15.0)
                    .strong(),
            );
        });
}

fn show_perf_strip(ui: &mut Ui, perf: EnginePerformance) {
    let warn = perf.cpu_percent >= 80.0 || perf.xruns > 0 || perf.lock_skips > 0;
    let color = if warn {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().weak_text_color()
    };

    let rate_k = if perf.sample_rate_hz >= 1000 {
        format!("{}k", (perf.sample_rate_hz + 500) / 1000)
    } else {
        format!("{}Hz", perf.sample_rate_hz)
    };
    let buf = if perf.buffer_frames > 0 {
        format!("{} @ {rate_k}", perf.buffer_frames)
    } else {
        format!("-- @ {rate_k}")
    };
    let latency = if perf.latency_ms > 0.0 {
        format!("~{:.1} ms", perf.latency_ms)
    } else {
        String::from("~-- ms")
    };
    let text = format!(
        "CPU {:.1}%  |  {buf}  |  {latency}  |  xruns {}  |  locks {}",
        perf.cpu_percent, perf.xruns, perf.lock_skips
    );

    ui.label(RichText::new(text).monospace().size(12.0).color(color))
        .on_hover_text(
            "Audio callback load vs buffer period. \
             xruns = callbacks that overran the budget. \
             locks = plugin try_lock skips on the audio thread (dropout risk).",
        );
}

impl TransportUi {
    pub fn show(
        ui: &mut Ui,
        project: &mut Project,
        engine: &mut dyn DawEngine,
    ) -> bool {
        let mut metronome_changed = false;
        ui.horizontal(|ui| {
            let play_label = if engine.is_playing() { "Pause" } else { "Play" };
            let play_resp = ui
                .button(play_label)
                .on_hover_text(
                    "Space: pause returns to the start mark (triangle on the ruler).\n\
                     Shift+Space (or Shift+click): pause and leave the playhead.",
                );
            if play_resp.clicked() {
                let shift = ui.input(|i| i.modifiers.shift);
                if shift && engine.is_playing() {
                    engine.pause_in_place();
                } else {
                    engine.toggle_playback();
                }
            }
            if ui.button("Stop").clicked() {
                engine.stop();
                engine.seek_beats(0.0);
            }

            ui.separator();

            show_time_box(ui, engine.current_beats(), project.beats_per_second());

            ui.separator();

            let mut metronome = engine.metronome_enabled();
            if ui.checkbox(&mut metronome, "Metronome").changed() {
                engine.set_metronome_enabled(metronome);
                metronome_changed = true;
            }

            ui.separator();

            ui.label(RichText::new("BPM").strong());
            let mut bpm = project.bpm;
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .speed(0.5)
                        .range(40.0..=240.0),
                )
                .changed()
            {
                project.bpm = bpm;
                engine.set_beats_per_second(project.beats_per_second());
            }

            ui.separator();

            let mut loop_enabled = project.loop_enabled;
            if ui
                .toggle_value(&mut loop_enabled, "Loop")
                .on_hover_text(
                    "Cycle playback inside the loop region. Off = play through to the end. \
                     Shortcut: L (remappable in Settings).",
                )
                .changed()
            {
                project.loop_enabled = loop_enabled;
            }
            if project.loop_enabled {
                ui.label(RichText::new("start").weak());
                let mut loop_start = project.loop_start_beats;
                if ui
                    .add(
                        egui::DragValue::new(&mut loop_start)
                            .speed(0.25)
                            .range(0.0..=252.0),
                    )
                    .changed()
                {
                    project.loop_start_beats =
                        loop_start.clamp(0.0, project.loop_end_beats - MIN_LOOP_SPAN_BEATS);
                }
                ui.label(RichText::new("end").weak());
                let mut loop_end = project.loop_end_beats;
                if ui
                    .add(
                        egui::DragValue::new(&mut loop_end)
                            .speed(0.25)
                            .range(1.0..=256.0),
                    )
                    .changed()
                {
                    project.loop_end_beats =
                        loop_end.max(project.loop_start_beats + MIN_LOOP_SPAN_BEATS);
                }
            }

            ui.separator();

            let bar = (engine.current_beats() / project.beats_per_bar).floor() as i32 + 1;
            let beat_in_bar = engine.current_beats().rem_euclid(project.beats_per_bar) + 1.0;
            ui.label(format!(
                "Position: bar {:.0} beat {:.2} ({:.2} beats)",
                bar,
                beat_in_bar,
                engine.current_beats()
            ));

            ui.separator();
            show_perf_strip(ui, engine.performance());
        });
        metronome_changed
    }
}

#[cfg(test)]
mod tests {
    use super::format_playhead_time;

    #[test]
    fn format_playhead_time_zero() {
        assert_eq!(format_playhead_time(0.0, 2.0), "00:00.000");
    }

    #[test]
    fn format_playhead_time_with_millis() {
        // 2 beats/s -> 1.5 beats = 0.75 s
        assert_eq!(format_playhead_time(1.5, 2.0), "00:00.750");
    }

    #[test]
    fn format_playhead_time_minutes() {
        // 120 bpm -> 2 beats/s; 240 beats = 120 s = 2:00
        assert_eq!(format_playhead_time(240.0, 2.0), "02:00.000");
    }
}
