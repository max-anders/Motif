mod app;
mod engine;
mod model;
mod ui;

fn main() -> eframe::Result<()> {
    #[cfg(target_os = "linux")]
    engine::plugins::init_xlib_threads();

    // winit 0.29+ removed WINIT_UNIX_BACKEND. We force X11 so Motif and CLAP/VST3
    // editors share XWayland under Hyprland. Override: MOTIF_UNIX_BACKEND=wayland.
    #[cfg(target_os = "linux")]
    let prefer_wayland = std::env::var("MOTIF_UNIX_BACKEND")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false);

    // On the forced-X11 path, winit derives the HiDPI scale from `Xft.dpi`, which
    // under Hyprland/XWayland often differs from the Wayland-native scale and
    // makes the UI look zoomed. Pin it to 1.0 unless the user set it explicitly.
    // Override: WINIT_X11_SCALE_FACTOR=1.5 (a float) or `randr` for auto-detect.
    #[cfg(target_os = "linux")]
    if !prefer_wayland && std::env::var_os("WINIT_X11_SCALE_FACTOR").is_none() {
        std::env::set_var("WINIT_X11_SCALE_FACTOR", "1");
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 760.0])
            .with_title("Motif"),
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(move |builder| {
            if !prefer_wayland {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11();
            }
        })),
        ..Default::default()
    };

    eframe::run_native(
        "Motif",
        native_options,
        Box::new(|cc| Ok(Box::new(app::DawApp::new(cc)))),
    )
}
