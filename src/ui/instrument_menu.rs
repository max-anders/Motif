//! Shared instrument picker (Add track / change track instrument).

use egui::Ui;

use crate::engine::{CatalogEntry, PluginCatalog};
use crate::model::TrackInstrument;

#[derive(Debug, Clone)]
pub enum InstrumentChoice {
    BuiltInPiano,
    Plugin(CatalogEntry),
}

/// Searchable list: built-in piano + catalog instruments.
/// Returns a choice when the user picks one.
/// When `focus_search` is true, the search field requests keyboard focus.
pub fn show_instrument_picker(
    ui: &mut Ui,
    catalog: &PluginCatalog,
    search: &mut String,
    id_salt: &str,
    focus_search: bool,
) -> Option<InstrumentChoice> {
    let mut choice = None;

    ui.horizontal(|ui| {
        ui.label("Search");
        let response = ui.add(
            egui::TextEdit::singleline(search)
                .id_salt(format!("{id_salt}_search"))
                .desired_width(180.0)
                .hint_text("Name or vendor"),
        );
        if focus_search {
            response.request_focus();
        }
    });
    ui.add_space(4.0);

    if ui
        .selectable_label(false, "Built-in Piano")
        .on_hover_text("Soft Motif piano synth")
        .clicked()
    {
        choice = Some(InstrumentChoice::BuiltInPiano);
    }

    ui.separator();
    if catalog.instrument_count() == 0 {
        ui.label("No scanned instruments. Open Settings -> Plugin Manager and Rescan.");
    } else {
        egui::ScrollArea::vertical()
            .id_salt(format!("{id_salt}_list"))
            .max_height(240.0)
            .show(ui, |ui| {
                for entry in catalog.filtered(search) {
                    let label = format!(
                        "{}  [{}]  - {}",
                        entry.name,
                        entry.format_badge(),
                        entry.vendor
                    );
                    if ui.selectable_label(false, label).clicked() {
                        choice = Some(InstrumentChoice::Plugin(entry.clone()));
                    }
                }
            });
    }

    choice
}

pub fn choice_to_instrument(choice: InstrumentChoice) -> TrackInstrument {
    match choice {
        InstrumentChoice::BuiltInPiano => TrackInstrument::BuiltInPiano,
        InstrumentChoice::Plugin(entry) => entry.to_instrument(),
    }
}

pub fn track_name_for_choice(choice: &InstrumentChoice, fallback_number: usize) -> String {
    match choice {
        InstrumentChoice::BuiltInPiano => format!("Track {fallback_number}"),
        InstrumentChoice::Plugin(entry) => entry.name.clone(),
    }
}

/// Searchable list of insert-FX candidates (effects only — no built-in entry).
/// Returns the chosen catalog entry when the user picks one.
/// When `focus_search` is true, the search field requests keyboard focus.
pub fn show_effect_picker(
    ui: &mut Ui,
    catalog: &PluginCatalog,
    search: &mut String,
    id_salt: &str,
    focus_search: bool,
) -> Option<CatalogEntry> {
    let mut choice = None;

    ui.horizontal(|ui| {
        ui.label("Search");
        let response = ui.add(
            egui::TextEdit::singleline(search)
                .id_salt(format!("{id_salt}_search"))
                .desired_width(180.0)
                .hint_text("Name or vendor"),
        );
        if focus_search {
            response.request_focus();
        }
    });
    ui.add_space(4.0);

    if catalog.effect_count() == 0 {
        ui.label("No scanned effects. Open Settings -> Plugin Manager and Rescan.");
    } else {
        egui::ScrollArea::vertical()
            .id_salt(format!("{id_salt}_list"))
            .max_height(240.0)
            .show(ui, |ui| {
                for entry in catalog.filtered_effects(search) {
                    let label = format!(
                        "{}  [{}]  - {}",
                        entry.name,
                        entry.format_badge(),
                        entry.vendor
                    );
                    if ui.selectable_label(false, label).clicked() {
                        choice = Some(entry.clone());
                    }
                }
            });
    }

    choice
}
