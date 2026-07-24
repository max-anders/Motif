//! Scan CLAP/VST3 instruments and cache results next to settings.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use truce_rack::clap::ClapScanner;
use truce_rack::core::info::PluginCategory;
use truce_rack::core::scanner::PluginScanner;
use truce_rack::vst3::Vst3Scanner;

use crate::model::{PluginFormat, TrackInstrument};

pub const PLUGIN_CACHE_FILE: &str = "plugin_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub vendor: String,
    pub unique_id: String,
    pub format: PluginFormat,
    pub path: PathBuf,
    pub accepts_midi: bool,
    /// Scanner-reported custom editor; confirm after load via the instance.
    #[serde(default)]
    pub has_editor: bool,
}

impl CatalogEntry {
    pub fn to_instrument(&self) -> TrackInstrument {
        TrackInstrument::Plugin {
            format: self.format,
            unique_id: self.unique_id.clone(),
            name: self.name.clone(),
        }
    }

    pub fn format_badge(&self) -> &'static str {
        self.format.label()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    scanned_at_unix: u64,
    extra_paths: Vec<PathBuf>,
    entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone)]
pub struct PluginCatalog {
    pub entries: Vec<CatalogEntry>,
    pub scanned_at_unix: Option<u64>,
    pub last_error: Option<String>,
    pub extra_paths: Vec<PathBuf>,
}

impl Default for PluginCatalog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            scanned_at_unix: None,
            last_error: None,
            extra_paths: Vec::new(),
        }
    }
}

impl PluginCatalog {
    pub fn load_or_defaults(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(json) => Self::from_cache_json(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn from_cache_json(json: &str) -> Result<Self, String> {
        let file: CacheFile = serde_json::from_str(json).map_err(|error| error.to_string())?;
        Ok(Self {
            entries: file.entries,
            scanned_at_unix: Some(file.scanned_at_unix),
            last_error: None,
            extra_paths: file.extra_paths,
        })
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        let scanned_at_unix = self.scanned_at_unix.unwrap_or_else(now_unix);
        let file = CacheFile {
            scanned_at_unix,
            extra_paths: self.extra_paths.clone(),
            entries: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
        fs::write(path, json).map_err(|error| error.to_string())
    }

    pub fn instrument_count(&self) -> usize {
        self.entries.len()
    }

    pub fn find(&self, format: PluginFormat, unique_id: &str) -> Option<&CatalogEntry> {
        self.entries
            .iter()
            .find(|entry| entry.format == format && entry.unique_id == unique_id)
    }

    pub fn filtered(&self, query: &str) -> Vec<&CatalogEntry> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.entries.iter().collect();
        }
        self.entries
            .iter()
            .filter(|entry| {
                entry.name.to_lowercase().contains(&q)
                    || entry.vendor.to_lowercase().contains(&q)
                    || entry.format.as_str().contains(&q)
            })
            .collect()
    }

    /// Full rescan of default OS paths plus `extra_paths`. Instruments only.
    pub fn rescan(&mut self) {
        let mut entries = Vec::new();
        let mut errors = Vec::new();

        match scan_clap_instruments(&[]) {
            Ok(mut found) => entries.append(&mut found),
            Err(error) => errors.push(format!("CLAP: {error}")),
        }
        match scan_vst3_instruments(&[]) {
            Ok(mut found) => entries.append(&mut found),
            Err(error) => errors.push(format!("VST3: {error}")),
        }

        let extra = self.extra_paths.clone();
        for path in &extra {
            if !path.exists() {
                errors.push(format!("Missing path: {}", path.display()));
                continue;
            }
            // yabridge chainloaders abort the host process on failed Wine bridge —
            // never dlopen them from Motif (in-process).
            if is_yabridge_path(path) {
                errors.push(format!(
                    "Skipped yabridge path {} (Windows VST3 bridge cannot be scanned/loaded in-process; use a native Linux CLAP/VST3)",
                    path.display()
                ));
                continue;
            }
            match scan_clap_instruments(&[path.clone()]) {
                Ok(mut found) => entries.append(&mut found),
                Err(error) => errors.push(format!("CLAP {}: {error}", path.display())),
            }
            match scan_vst3_instruments(&[path.clone()]) {
                Ok(mut found) => entries.append(&mut found),
                Err(error) => errors.push(format!("VST3 {}: {error}", path.display())),
            }
        }

        dedupe_entries(&mut entries);
        entries.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.format.as_str().cmp(b.format.as_str()))
        });

        self.entries = entries;
        self.scanned_at_unix = Some(now_unix());
        self.last_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        };
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_yabridge_path(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("yabridge"))
    })
}

fn keep_catalog_candidate(
    format: PluginFormat,
    category: PluginCategory,
    accepts_midi: bool,
) -> bool {
    match format {
        // truce-rack currently tags every VST3 as Effect; keep MIDI-capable modules.
        PluginFormat::Vst3 => {
            matches!(category, PluginCategory::Instrument)
                || accepts_midi
                || matches!(category, PluginCategory::Effect)
        }
        PluginFormat::Clap => matches!(category, PluginCategory::Instrument),
    }
}

fn scan_clap_instruments(extra_only: &[PathBuf]) -> Result<Vec<CatalogEntry>, String> {
    let scanner = ClapScanner::new();
    let infos = if extra_only.is_empty() {
        scanner.scan().map_err(|e| e.to_string())?
    } else {
        let mut all = Vec::new();
        for path in extra_only {
            if is_yabridge_path(path) {
                continue;
            }
            all.extend(scanner.scan_path(path).map_err(|e| e.to_string())?);
        }
        all
    };
    Ok(infos
        .into_iter()
        .filter(|info| !is_yabridge_path(&info.path))
        .filter_map(|info| {
            let format = PluginFormat::from_rack_format(info.format)?;
            if !keep_catalog_candidate(format, info.category, info.accepts_midi) {
                return None;
            }
            Some(CatalogEntry {
                name: info.name,
                vendor: info.vendor,
                unique_id: info.unique_id,
                format,
                path: info.path,
                accepts_midi: info.accepts_midi,
                has_editor: info.has_editor,
            })
        })
        .collect())
}

fn scan_vst3_instruments(extra_only: &[PathBuf]) -> Result<Vec<CatalogEntry>, String> {
    let scanner = Vst3Scanner::new();
    let infos = if extra_only.is_empty() {
        scanner.scan().map_err(|e| e.to_string())?
    } else {
        let mut all = Vec::new();
        for path in extra_only {
            if is_yabridge_path(path) {
                continue;
            }
            all.extend(scanner.scan_path(path).map_err(|e| e.to_string())?);
        }
        all
    };
    Ok(infos
        .into_iter()
        .filter(|info| !is_yabridge_path(&info.path))
        .filter_map(|info| {
            let format = PluginFormat::from_rack_format(info.format)?;
            if !keep_catalog_candidate(format, info.category, info.accepts_midi) {
                return None;
            }
            Some(CatalogEntry {
                name: info.name,
                vendor: info.vendor,
                unique_id: info.unique_id,
                format,
                path: info.path,
                accepts_midi: info.accepts_midi,
                has_editor: info.has_editor,
            })
        })
        .collect())
}

fn dedupe_entries(entries: &mut Vec<CatalogEntry>) {
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert((entry.format, entry.unique_id.clone())));
}
