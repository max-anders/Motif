//! Right-side inspector: properties facet of the selected Track.

use egui::Ui;

use crate::model::{EditHistory, Project, MAX_GAIN_DB, MIN_GAIN_DB};
use crate::ui::macro_panel::macro_target_label;
use crate::ui::theme::ThemeColors;
use crate::ui::track_rename::apply_track_name_if_changed;

/// Draw the inspector contents for `selected_track` into an already-opened side panel.
pub fn show_inspector(
    ui: &mut Ui,
    project: &mut Project,
    history: &mut EditHistory,
    selected_track: Option<u64>,
    theme: &ThemeColors,
) {
    ui.heading("Inspector");
    ui.label(
        egui::RichText::new("Properties of the selected track")
            .color(theme.text_muted)
            .small(),
    );
    ui.separator();

    let Some(track_id) = selected_track else {
        ui.label(
            egui::RichText::new("No track selected.\nClick a playlist header or mixer strip.")
                .color(theme.text_muted),
        );
        return;
    };

    let Some(track) = project.track(track_id).cloned() else {
        ui.label(
            egui::RichText::new("Selected track no longer exists.")
                .color(theme.accent_warning),
        );
        return;
    };

    ui.label(egui::RichText::new("Name").strong());
    let mut name = track.name.clone();
    let name_response = ui.add(
        egui::TextEdit::singleline(&mut name).desired_width(ui.available_width()),
    );
    if name_response.lost_focus() && name != track.name {
        apply_track_name_if_changed(history, project, track_id, &track.name, &name);
    }

    ui.label(
        egui::RichText::new(format!("Id {}", track.id))
            .color(theme.text_muted)
            .small()
            .monospace(),
    );
    ui.label(
        egui::RichText::new(track.instrument.display_name())
            .color(theme.text_muted),
    );
    if let Some(badge) = track.instrument.format_badge() {
        ui.label(egui::RichText::new(badge).color(theme.accent).small());
    }

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Mixer").strong());

    let mut gain = track.gain_db;
    let gain_response = ui.add(egui::Slider::new(&mut gain, MIN_GAIN_DB..=MAX_GAIN_DB).text("Gain (dB)"));
    apply_prop_slider(history, project, &gain_response, |project| {
        if let Some(t) = project.track_mut(track_id) {
            t.gain_db = gain;
        }
    });

    let mut pan = track.pan;
    let pan_response = ui.add(egui::Slider::new(&mut pan, -1.0..=1.0).text("Pan"));
    apply_prop_slider(history, project, &pan_response, |project| {
        if let Some(t) = project.track_mut(track_id) {
            t.pan = pan;
        }
    });

    ui.horizontal(|ui| {
        ui.label(format!("Mute: {}", if track.muted { "on" } else { "off" }));
        ui.label(format!("Solo: {}", if track.solo { "on" } else { "off" }));
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Sends").strong());
    if track.sends.is_empty() {
        ui.label(
            egui::RichText::new("(none — scaffolding only)")
                .color(theme.text_muted)
                .italics(),
        );
    } else {
        for (i, send) in track.sends.iter().enumerate() {
            let target = send
                .target_track
                .map(|id| format!("track {id}"))
                .unwrap_or_else(|| String::from("unassigned"));
            ui.label(format!(
                "{}. {}  {:.1} dB  {}",
                i + 1,
                target,
                send.level_db,
                if send.enabled { "on" } else { "off" }
            ));
        }
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Devices").strong());
    if track.devices.is_empty() {
        ui.label(
            egui::RichText::new("(none — insert FX not hosted yet)")
                .color(theme.text_muted)
                .italics(),
        );
    } else {
        for device in &track.devices {
            let bypass = if device.bypassed { " bypassed" } else { "" };
            ui.label(format!("#{} {}{bypass}", device.id, device.name));
        }
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Macros").strong());
    if track.macros.is_empty() {
        ui.label(
            egui::RichText::new("(none)")
                .color(theme.text_muted)
                .italics(),
        );
    } else {
        for m in &track.macros {
            ui.label(format!("#{} {}: {:.2}", m.id, m.name, m.value));
            for mapping in &m.mappings {
                let dest = if !mapping.param_name.is_empty() {
                    mapping.param_name.clone()
                } else {
                    macro_target_label(&track, &mapping.target)
                };
                ui.label(
                    egui::RichText::new(format!(
                        "  -> {dest} [{:.2}..{:.2}]",
                        mapping.min, mapping.max
                    ))
                    .small()
                    .color(theme.text_muted),
                );
            }
        }
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Routing").strong());
    ui.label(
        egui::RichText::new("Track -> Master (fixed for now)")
            .color(theme.text_muted)
            .italics(),
    );
}

fn apply_prop_slider(
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
