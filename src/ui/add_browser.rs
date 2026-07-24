//! Unified add browser modal: Instruments / FX / Samples.
//!
//! Opened by shortcuts that preselect a tab. Selecting an item creates a track,
//! adds insert FX, or imports a sample (recent list or Browse...).

use std::path::{Path, PathBuf};

use egui::{Align2, Context, Key, Ui, Vec2, Window};

use crate::engine::{CatalogEntry, PluginCatalog};
use crate::ui::instrument_menu::{
    show_effect_picker, show_instrument_picker, InstrumentChoice,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserTab {
    #[default]
    Instruments,
    Fx,
    Samples,
}

#[derive(Debug, Clone)]
pub enum AddBrowserAction {
    CreateTrack(InstrumentChoice),
    AddEffect(CatalogEntry),
    AddSample(PathBuf),
    BrowseSample,
    Close,
}

#[derive(Debug, Default)]
pub struct AddBrowserUi {
    tab: BrowserTab,
    instrument_search: String,
    effect_search: String,
    sample_search: String,
    /// Request focus on the active tab's search field once after open / tab switch.
    focus_search: bool,
}

impl AddBrowserUi {
    /// Call when a shortcut (or toolbar) opens the browser so the tab is set
    /// and search autofocuses on the next frame.
    pub fn prepare_open(&mut self, tab: BrowserTab) {
        self.tab = tab;
        self.focus_search = true;
        self.clear_searches();
    }

    /// Show the add browser modal. `open` is `Some(_)` while visible.
    pub fn show(
        &mut self,
        ctx: &Context,
        open: &mut Option<BrowserTab>,
        catalog: &PluginCatalog,
        recent_samples: &[PathBuf],
    ) -> Option<AddBrowserAction> {
        if open.is_none() {
            return None;
        }

        let mut action = None;
        let mut still_open = true;

        // Escape while search is focused: global shortcut poll is suppressed by
        // wants_keyboard_input, so handle close here.
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            *open = None;
            self.focus_search = false;
            self.clear_searches();
            return Some(AddBrowserAction::Close);
        }

        const BROWSER_SIZE: Vec2 = Vec2::new(440.0, 480.0);
        Window::new("Add")
            .collapsible(false)
            .resizable(false)
            .default_size(BROWSER_SIZE)
            .min_size(BROWSER_SIZE)
            .max_size(BROWSER_SIZE)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut still_open)
            .show(ctx, |ui| {
                // Fill the fixed frame so tab switches do not reflow the window.
                ui.set_min_size(ui.available_size());
                action = self.show_contents(ui, catalog, recent_samples);
            });

        if !still_open {
            *open = None;
            self.focus_search = false;
            self.clear_searches();
            if action.is_none() {
                action = Some(AddBrowserAction::Close);
            }
        }

        if matches!(
            action,
            Some(
                AddBrowserAction::CreateTrack(_)
                    | AddBrowserAction::AddEffect(_)
                    | AddBrowserAction::AddSample(_)
                    | AddBrowserAction::BrowseSample
                    | AddBrowserAction::Close
            )
        ) {
            *open = None;
            self.focus_search = false;
            self.clear_searches();
        } else if open.is_some() {
            // Keep open Option in sync with the active tab (for debugging / reopen).
            *open = Some(self.tab);
        }

        action
    }

    fn clear_searches(&mut self) {
        self.instrument_search.clear();
        self.effect_search.clear();
        self.sample_search.clear();
    }

    fn show_contents(
        &mut self,
        ui: &mut Ui,
        catalog: &PluginCatalog,
        recent_samples: &[PathBuf],
    ) -> Option<AddBrowserAction> {
        let mut action = None;
        let focus = self.focus_search;
        if focus {
            self.focus_search = false;
        }

        ui.horizontal(|ui| {
            for (tab, label) in [
                (BrowserTab::Instruments, "Instruments"),
                (BrowserTab::Fx, "FX"),
                (BrowserTab::Samples, "Samples"),
            ] {
                if ui.selectable_label(self.tab == tab, label).clicked() {
                    self.tab = tab;
                    self.focus_search = true;
                }
            }
        });
        ui.add_space(8.0);

        match self.tab {
            BrowserTab::Instruments => {
                ui.label("Create a new track with the selected instrument.");
                ui.add_space(4.0);
                let list_height = ui.available_height();
                if let Some(choice) = show_instrument_picker(
                    ui,
                    catalog,
                    &mut self.instrument_search,
                    "add_browser_inst",
                    focus,
                    list_height,
                ) {
                    action = Some(AddBrowserAction::CreateTrack(choice));
                }
            }
            BrowserTab::Fx => {
                ui.label("Add an insert effect to the selected track.");
                ui.add_space(4.0);
                let list_height = ui.available_height();
                if let Some(entry) = show_effect_picker(
                    ui,
                    catalog,
                    &mut self.effect_search,
                    "add_browser_fx",
                    focus,
                    list_height,
                ) {
                    action = Some(AddBrowserAction::AddEffect(entry));
                }
            }
            BrowserTab::Samples => {
                ui.label("Add an audio clip to the selected track.");
                ui.add_space(4.0);
                action = self.show_samples_tab(ui, recent_samples, focus);
            }
        }

        action
    }

    fn show_samples_tab(
        &mut self,
        ui: &mut Ui,
        recent_samples: &[PathBuf],
        focus_search: bool,
    ) -> Option<AddBrowserAction> {
        let mut action = None;

        ui.horizontal(|ui| {
            ui.label("Search");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.sample_search)
                    .id_salt("add_browser_sample_search")
                    .desired_width(180.0)
                    .hint_text("File name"),
            );
            if focus_search {
                response.request_focus();
            }
            if ui.button("Browse...").clicked() {
                action = Some(AddBrowserAction::BrowseSample);
            }
        });
        ui.add_space(4.0);

        let query = self.sample_search.trim().to_lowercase();
        let filtered: Vec<&PathBuf> = recent_samples
            .iter()
            .filter(|path| {
                if query.is_empty() {
                    return true;
                }
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_lowercase().contains(&query))
                    .unwrap_or(false)
                    || path
                        .to_str()
                        .map(|s| s.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .collect();

        ui.strong("Recent samples");
        ui.add_space(4.0);

        let list_height = ui.available_height();
        if filtered.is_empty() {
            if recent_samples.is_empty() {
                ui.weak("No recent samples yet. Use Browse... to import a file.");
            } else {
                ui.weak("No matches.");
            }
        } else {
            egui::ScrollArea::vertical()
                .id_salt("add_browser_sample_list")
                .max_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for path in filtered {
                        let label = sample_label(path);
                        let exists = path.exists();
                        let response =
                            ui.add_enabled(exists, egui::SelectableLabel::new(false, &label));
                        if !exists {
                            response.on_hover_text("File missing");
                        } else if response.clicked() {
                            action = Some(AddBrowserAction::AddSample(path.clone()));
                        }
                    }
                });
        }

        action
    }
}

fn sample_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| path.display().to_string())
}
