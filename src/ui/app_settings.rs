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
    /// Per-plugin starred parameters, keyed by plugin `unique_id`.
    #[serde(default)]
    plugin_favorites: HashMap<String, Vec<PluginFavoriteParam>>,
    /// Per-plugin last-tweaked parameters (MRU), keyed by plugin `unique_id`.
    #[serde(default)]
    plugin_last_tweaked: HashMap<String, Vec<PluginFavoriteParam>>,
    #[serde(default = "default_autosave_enabled")]
    autosave_enabled: bool,
    #[serde(default = "default_autosave_interval")]
    autosave_interval_secs: u32,
    #[serde(default = "default_metronome_enabled")]
    metronome_enabled: bool,
    #[serde(default)]
    recent_projects: Vec<PathBuf>,
    #[serde(default)]
    recent_samples: Vec<PathBuf>,
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

fn default_metronome_enabled() -> bool {
    true
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

/// One starred plugin parameter (display cache + stable param id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginFavoriteParam {
    pub param_id: u32,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub shortcuts: ShortcutRegistry,
    pub themes: ThemeCatalog,
    pub plugin_extra_paths: Vec<PathBuf>,
    pub undo_limit: usize,
    pub plugin_keys: PluginKeySettings,
    /// Per-plugin starred parameters, keyed by plugin `unique_id`.
    pub plugin_favorites: HashMap<String, Vec<PluginFavoriteParam>>,
    /// Per-plugin last-tweaked parameters (MRU), keyed by plugin `unique_id`.
    pub plugin_last_tweaked: HashMap<String, Vec<PluginFavoriteParam>>,
    pub autosave_enabled: bool,
    pub autosave_interval_secs: u32,
    pub metronome_enabled: bool,
    pub recent_projects: Vec<PathBuf>,
    pub recent_samples: Vec<PathBuf>,
}

/// Cap for recently imported sample paths (add browser Samples tab).
pub const MAX_RECENT_SAMPLES: usize = 20;

/// Cap for last-tweaked plugin parameters per `unique_id`.
pub const MAX_LAST_TWEAKED_PARAMS: usize = 8;

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: ShortcutRegistry::defaults(),
            themes: ThemeCatalog::default(),
            plugin_extra_paths: Vec::new(),
            undo_limit: DEFAULT_UNDO_LIMIT,
            plugin_keys: PluginKeySettings::default(),
            plugin_favorites: HashMap::new(),
            plugin_last_tweaked: HashMap::new(),
            autosave_enabled: true,
            autosave_interval_secs: DEFAULT_AUTOSAVE_INTERVAL_SECS,
            metronome_enabled: true,
            recent_projects: Vec::new(),
            recent_samples: Vec::new(),
        }
    }
}

impl AppSettings {
    /// Push a sample path to the front of the recent list (dedupe, cap).
    pub fn push_recent_sample(&mut self, path: PathBuf) {
        self.recent_samples.retain(|existing| existing != &path);
        self.recent_samples.insert(0, path);
        if self.recent_samples.len() > MAX_RECENT_SAMPLES {
            self.recent_samples.truncate(MAX_RECENT_SAMPLES);
        }
    }

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
            plugin_favorites: file.plugin_favorites,
            plugin_last_tweaked: file.plugin_last_tweaked,
            autosave_enabled: file.autosave_enabled,
            autosave_interval_secs: file.autosave_interval_secs.max(30),
            metronome_enabled: file.metronome_enabled,
            recent_projects: file.recent_projects,
            recent_samples: file.recent_samples,
        })
    }

    /// Last-tweaked MRU for a plugin identity (newest first; empty when none).
    pub fn last_tweaked_for(&self, unique_id: &str) -> &[PluginFavoriteParam] {
        self.plugin_last_tweaked
            .get(unique_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Move `param_id` to the front of the last-tweaked MRU (dedupe, cap).
    /// Returns true when the list changed.
    pub fn touch_param(
        &mut self,
        unique_id: &str,
        param_id: u32,
        name: impl Into<String>,
    ) -> bool {
        if unique_id.is_empty() {
            return false;
        }
        let name = name.into();
        let list = self
            .plugin_last_tweaked
            .entry(unique_id.to_string())
            .or_default();
        let previous = list.clone();
        list.retain(|entry| entry.param_id != param_id);
        list.insert(
            0,
            PluginFavoriteParam {
                param_id,
                name,
            },
        );
        if list.len() > MAX_LAST_TWEAKED_PARAMS {
            list.truncate(MAX_LAST_TWEAKED_PARAMS);
        }
        *list != previous
    }

    /// Favorites list for a plugin identity (empty when none).
    pub fn favorites_for(&self, unique_id: &str) -> &[PluginFavoriteParam] {
        self.plugin_favorites
            .get(unique_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Add a favorite for `unique_id` (no-op if already present). Returns true if changed.
    pub fn add_favorite(&mut self, unique_id: &str, param_id: u32, name: impl Into<String>) -> bool {
        if unique_id.is_empty() {
            return false;
        }
        let list = self
            .plugin_favorites
            .entry(unique_id.to_string())
            .or_default();
        if list.iter().any(|fav| fav.param_id == param_id) {
            return false;
        }
        list.push(PluginFavoriteParam {
            param_id,
            name: name.into(),
        });
        true
    }

    /// Remove a favorite. Returns true if something was removed.
    pub fn remove_favorite(&mut self, unique_id: &str, param_id: u32) -> bool {
        let Some(list) = self.plugin_favorites.get_mut(unique_id) else {
            return false;
        };
        let before = list.len();
        list.retain(|fav| fav.param_id != param_id);
        let changed = list.len() != before;
        if list.is_empty() {
            self.plugin_favorites.remove(unique_id);
        }
        changed
    }

    /// True when `param_id` is already starred for this plugin.
    pub fn has_favorite(&self, unique_id: &str, param_id: u32) -> bool {
        self.plugin_favorites
            .get(unique_id)
            .is_some_and(|list| list.iter().any(|fav| fav.param_id == param_id))
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
            plugin_favorites: self.plugin_favorites.clone(),
            plugin_last_tweaked: self.plugin_last_tweaked.clone(),
            autosave_enabled: self.autosave_enabled,
            autosave_interval_secs: self.autosave_interval_secs.max(30),
            metronome_enabled: self.metronome_enabled,
            recent_projects: self.recent_projects.clone(),
            recent_samples: self.recent_samples.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
        fs::write(path, json).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_favorites_add_dedupe_remove() {
        let mut settings = AppSettings::default();
        assert!(settings.add_favorite("com.example.filter", 3, "Cutoff"));
        assert!(!settings.add_favorite("com.example.filter", 3, "Cutoff"));
        assert_eq!(settings.favorites_for("com.example.filter").len(), 1);
        assert!(settings.has_favorite("com.example.filter", 3));
        assert!(settings.remove_favorite("com.example.filter", 3));
        assert!(settings.favorites_for("com.example.filter").is_empty());
    }

    #[test]
    fn plugin_favorites_round_trip_json() {
        let mut settings = AppSettings::default();
        settings.add_favorite("uid.a", 1, "Res");
        let json = {
            let (active_theme, themes) = settings.themes.stored();
            let file = SettingsFile {
                bindings: Vec::new(),
                active_theme,
                themes,
                plugin_extra_paths: Vec::new(),
                undo_limit: settings.undo_limit,
                plugin_keys: settings.plugin_keys.clone(),
                plugin_favorites: settings.plugin_favorites.clone(),
                plugin_last_tweaked: settings.plugin_last_tweaked.clone(),
                autosave_enabled: true,
                autosave_interval_secs: 60,
                metronome_enabled: true,
                recent_projects: Vec::new(),
                recent_samples: Vec::new(),
            };
            serde_json::to_string(&file).unwrap()
        };
        let loaded = AppSettings::from_json(&json).unwrap();
        assert_eq!(loaded.favorites_for("uid.a")[0].param_id, 1);
        assert_eq!(loaded.favorites_for("uid.a")[0].name, "Res");
    }

    #[test]
    fn plugin_last_tweaked_mru_dedupe_cap() {
        let mut settings = AppSettings::default();
        assert!(settings.touch_param("com.example.synth", 1, "Cut"));
        assert!(settings.touch_param("com.example.synth", 2, "Res"));
        assert!(settings.touch_param("com.example.synth", 1, "Cutoff"));
        let list = settings.last_tweaked_for("com.example.synth");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].param_id, 1);
        assert_eq!(list[0].name, "Cutoff");
        assert_eq!(list[1].param_id, 2);

        for id in 10..(10 + MAX_LAST_TWEAKED_PARAMS as u32) {
            assert!(settings.touch_param("com.example.synth", id, format!("P{id}")));
        }
        let list = settings.last_tweaked_for("com.example.synth");
        assert_eq!(list.len(), MAX_LAST_TWEAKED_PARAMS);
        assert_eq!(list[0].param_id, 10 + MAX_LAST_TWEAKED_PARAMS as u32 - 1);
        assert!(!list.iter().any(|e| e.param_id == 1 || e.param_id == 2));
    }

    #[test]
    fn plugin_last_tweaked_round_trip_json() {
        let mut settings = AppSettings::default();
        settings.touch_param("uid.b", 7, "Drive");
        let json = {
            let (active_theme, themes) = settings.themes.stored();
            let file = SettingsFile {
                bindings: Vec::new(),
                active_theme,
                themes,
                plugin_extra_paths: Vec::new(),
                undo_limit: settings.undo_limit,
                plugin_keys: settings.plugin_keys.clone(),
                plugin_favorites: HashMap::new(),
                plugin_last_tweaked: settings.plugin_last_tweaked.clone(),
                autosave_enabled: true,
                autosave_interval_secs: 60,
                metronome_enabled: true,
                recent_projects: Vec::new(),
                recent_samples: Vec::new(),
            };
            serde_json::to_string(&file).unwrap()
        };
        let loaded = AppSettings::from_json(&json).unwrap();
        assert_eq!(loaded.last_tweaked_for("uid.b")[0].param_id, 7);
        assert_eq!(loaded.last_tweaked_for("uid.b")[0].name, "Drive");
    }
}
