//! Combined app settings persistence (`settings.json`): shortcuts + themes.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
}

fn default_active_theme() -> String {
    DEFAULT_THEME_NAME.to_string()
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub shortcuts: ShortcutRegistry,
    pub themes: ThemeCatalog,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            shortcuts: ShortcutRegistry::defaults(),
            themes: ThemeCatalog::default(),
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
        let file: SettingsFile =
            serde_json::from_str(json).map_err(|error| error.to_string())?;

        let mut shortcuts = if file.bindings.is_empty() {
            ShortcutRegistry::defaults()
        } else {
            ShortcutRegistry::from_stored(file.bindings)
                .unwrap_or_else(|_| ShortcutRegistry::defaults())
        };
        shortcuts.ensure_default_actions();

        let themes = ThemeCatalog::from_stored(file.active_theme, file.themes);
        Ok(Self { shortcuts, themes })
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        let (active_theme, themes) = self.themes.stored();
        let file = SettingsFile {
            bindings: self.shortcuts.to_stored()?,
            active_theme,
            themes,
        };
        let json = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
        fs::write(path, json).map_err(|error| error.to_string())
    }
}
