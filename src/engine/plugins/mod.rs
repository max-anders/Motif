//! CLAP / VST3 scan, cache, load, and editor hosting (truce-rack).

mod catalog;
mod editor;
#[cfg(target_os = "linux")]
mod editor_window;
mod host;

pub use catalog::{CatalogEntry, EntryCategory, PluginCatalog, PLUGIN_CACHE_FILE};
pub use editor::{EditorPoll, PluginEditorHost, PluginRef};
pub use host::{load_and_activate, HostedPlugin};

#[cfg(not(target_os = "linux"))]
pub use editor::HostX11;
#[cfg(target_os = "linux")]
pub use editor_window::{init_xlib_threads, HostX11};
