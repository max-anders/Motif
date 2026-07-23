mod app;
mod engine;
mod model;
mod ui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 760.0])
            .with_title("Motif"),
        ..Default::default()
    };

    eframe::run_native(
        "Motif",
        native_options,
        Box::new(|cc| Ok(Box::new(app::DawApp::new(cc)))),
    )
}
