//! Combined app settings persistence (`settings.json`): shortcuts + themes + plugin paths + project prefs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    clamp_undo_limit, DEFAULT_AUTOSAVE_INTERVAL_SECS, DEFAULT_UNDO_LIMIT,
};

use super::shortcuts::{ShortcutRegistry, StoredBinding};
use super::theme::{Theme, ThemeCatalog, DEFAULT_THEME_NAME};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    bindings: Vec<StoredBinding>,
    #[serde(default = "default_active_theme")]
    active_theme: String,
    #[serde(default)]
    themes: Vec<Theme>,
    #[serde(default)]
    plugin_extra_paths: Vec<PathBuf>,
    #[serde(default = "default_undo_limit")]
    undo_limit: usize,
    #[serde(default)]
    plugin_keys: PluginKeySettings,
    #[serde(default = "default_autosave_enabled")]
    autosave_enabled: bool,
    #[serde(default = "default_autosave_interval")]
    autosave_interval_secs: u32,
    #[serde(default)]
    recent_projects: Vec<PathBuf>,
}

fn default_active_theme() -> String {
    DEFAULT_THEME_NAME.to_string()
}

fn default_undo_limit() -> usize {
    DEFAULT_UNDO_LIMIT
}

fn default_forward_transport() -> bool {
    true
}

fn default_autosave_enabled() -> bool {
    true
}

fn default_autosave_interval() -> u32 {
    DEFAULT_AUTOSAVE_INTERVAL_SECS
}

/// Keyboard routing between plugin editor windows and Motif.
///
/// While a plugin editor is focused, Space normally goes to the plugin. When
/// forwarding is on, Motif grabs Space so it drives transport (play/pause).
/// A plugin that needs Space in its own UI can opt out via `overrides`
/// (keyed by the plugin `unique_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginKeySettings {
    /// Default when a plugin has no explicit override.
    #[serde(default = "default_forward_transport")]
    pub forward_transport_default: bool,
    /// Per-plugin override, keyed by plugin `unique_id`. `true` = forward Space.
    #[serde(default)]
    pub overrides: HashMap<String, bool>,
}

impl Default for PluginKeySettings {
    fn default() -> Self {
        Self {
            forward_transport_default: true,
            overrides: HashMap::new(),
        }
    }
}

impl PluginKeySettings {
    /// Effective "forward Space to Motif" value for a plugin.
    pub fn forward_transport_for(&self, unique_id: &str) -> bool {
        self.overrides
            .get(unique_id)
            .copied()
            .unwrap_or(self.forward_transport_default)
    }

    /// Set (or clear, when it matches the default) a per-plugin override.
    pub fn set_forward_transport_for(&mut self, unique_id: &str, forward: bool) {
        if forward == self.forward_transport_default {
            self.overrides.remove(unique_id);
        } else {
            self.overrides.insert(unique_id.to_string(), forward);
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub shortcuts: ShortcutRegistry,
    pub themes: ThemeCatalog,
    pub plugin_extra_paths: Vec<PathBuf>,
    pub undo_limit: usize,
    pub plugin_keys: PluginKeySettings,
    pub autosave_enabled: bool,
    pub autosave_interval_secs: u32,
    pub recent_projects: Vec<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: ShortcutRegistry::defaults(),
            themes: ThemeCatalog::default(),
            plugin_extra_paths: Vec::new(),
            undo_limit: DEFAULT_UNDO_LIMIT,
            plugin_keys: PluginKeySettings::default(),
            autosave_enabled: true,
            autosave_interval_secs: DEFAULT_AUTOSAVE_INTERVAL_SECS,
            recent_projects: Vec::new(),
        }
    }
}

impl AppSettings {
    pub fn load_or_defaults(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(json) => Self::from_json(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let file: SettingsFile = serde_json::from_str(json).map_err(|error| error.to_string())?;

        let mut shortcuts = if file.bindings.is_empty() {
            ShortcutRegistry::defaults()
        } else {
            ShortcutRegistry::from_stored(file.bindings)
                .unwrap_or_else(|_| ShortcutRegistry::defaults())
        };
        shortcuts.ensure_default_actions();

        let themes = ThemeCatalog::from_stored(file.active_theme, file.themes);
        Ok(Self {
            shortcuts,
            themes,
            plugin_extra_paths: file.plugin_extra_paths,
            undo_limit: clamp_undo_limit(file.undo_limit),
            plugin_keys: file.plugin_keys,
            autosave_enabled: file.autosave_enabled,
            autosave_interval_secs: file.autosave_interval_secs.max(30),
            recent_projects: file.recent_projects,
        })
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        let (active_theme, themes) = self.themes.stored();
        let file = SettingsFile {
            bindings: self.shortcuts.to_stored()?,
            active_theme,
            themes,
            plugin_extra_paths: self.plugin_extra_paths.clone(),
            undo_limit: clamp_undo_limit(self.undo_limit),
            plugin_keys: self.plugin_keys.clone(),
            autosave_enabled: self.autosave_enabled,
            autosave_interval_secs: self.autosave_interval_secs.max(30),
            recent_projects: self.recent_projects.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
        fs::write(path, json).map_err(|error| error.to_string())
    }
}
