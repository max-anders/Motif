//! CLAP / VST3 scan, cache, load, and editor hosting (truce-rack).

mod catalog;
mod editor;
#[cfg(target_os = "linux")]
mod editor_window;
mod host;

pub use catalog::{CatalogEntry, EntryCategory, PluginCatalog, PLUGIN_CACHE_FILE};
pub use editor::{EditorCloseBinding, EditorPoll, PluginEditorHost, PluginRef};
pub use host::{load_and_activate, HostedPlugin, PluginParamInfo};

/// Plugins whose Linux GUI abort()s the host process (in-process hosting).
/// Vital 1.6.4 bgfx EGL is the known case on NVIDIA; identity match is name/id.
pub fn plugin_gui_may_abort_host(unique_id: &str, name: &str) -> bool {
    let id = unique_id.to_ascii_lowercase();
    let nm = name.to_ascii_lowercase();
    id.contains("vital") || nm.contains("vital")
}

/// True when `main` applied MOTIF_PLUGIN_GL=software (Mesa llvmpipe for plugin EGL).
pub fn plugin_gl_software_enabled() -> bool {
    std::env::var("LIBGL_ALWAYS_SOFTWARE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::plugin_gui_may_abort_host;

    #[test]
    fn vital_identities_are_flagged() {
        assert!(plugin_gui_may_abort_host("com.vital.synth", "Vital"));
        assert!(plugin_gui_may_abort_host("Vital.vst3", "Track 1"));
        assert!(!plugin_gui_may_abort_host("org.surge-synth.surge", "Surge XT"));
    }
}

#[cfg(not(target_os = "linux"))]
pub use editor::HostX11;
#[cfg(target_os = "linux")]
pub use editor_window::{init_xlib_threads, HostX11};
