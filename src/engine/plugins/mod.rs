//! CLAP / VST3 scan, cache, and load helpers (truce-rack).

mod catalog;
mod host;

pub use catalog::{
    CatalogEntry, PluginCatalog, PLUGIN_CACHE_FILE,
};
pub use host::{load_and_activate, HostedPlugin};
