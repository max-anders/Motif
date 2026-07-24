//! Project file I/O, data-dir paths, recent list helpers, and crash-recovery backups.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::Project;

pub const PROJECT_EXTENSION: &str = "motif";
pub const PROJECT_FORMAT: &str = "motif";
pub const PROJECT_FORMAT_VERSION: u32 = 2;
pub const MAX_RECENT_PROJECTS: usize = 12;
pub const DEFAULT_AUTOSAVE_INTERVAL_SECS: u32 = 180;

const RECOVERY_PROJECT_FILE: &str = "unsaved.motif";
const RECOVERY_META_FILE: &str = "unsaved.meta.json";
const LEGACY_PROJECT_FILE: &str = "project.json";

/// On-disk wrapper around [`Project`] for future format evolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEnvelope {
    pub format: String,
    pub version: u32,
    pub project: Project,
}

impl ProjectEnvelope {
    pub fn new(project: Project) -> Self {
        Self {
            format: PROJECT_FORMAT.to_string(),
            version: PROJECT_FORMAT_VERSION,
            project,
        }
    }
}

/// Metadata written beside the recovery project file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryMeta {
    pub original_path: Option<PathBuf>,
    pub project_name: String,
    pub saved_at_unix: u64,
}

/// Prefer XDG data home (`~/.local/share/motif` on Linux).
pub fn data_dir() -> Result<PathBuf, String> {
    let dirs = directories::ProjectDirs::from("com", "max-anders", "motif")
        .ok_or_else(|| "could not resolve Motif data directory".to_string())?;
    let path = dirs.data_dir().to_path_buf();
    fs::create_dir_all(&path).map_err(|error| format!("create data dir: {error}"))?;
    Ok(path)
}

pub fn projects_dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join("projects");
    fs::create_dir_all(&path).map_err(|error| format!("create projects dir: {error}"))?;
    Ok(path)
}

pub fn recovery_dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join("recovery");
    fs::create_dir_all(&path).map_err(|error| format!("create recovery dir: {error}"))?;
    Ok(path)
}

pub fn recovery_file() -> Result<PathBuf, String> {
    Ok(recovery_dir()?.join(RECOVERY_PROJECT_FILE))
}

pub fn recovery_meta_file() -> Result<PathBuf, String> {
    Ok(recovery_dir()?.join(RECOVERY_META_FILE))
}

pub fn legacy_project_path() -> PathBuf {
    PathBuf::from(LEGACY_PROJECT_FILE)
}

pub fn project_display_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled")
        .to_string()
}

pub fn ensure_motif_extension(path: PathBuf) -> PathBuf {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case(PROJECT_EXTENSION) => path,
        _ => path.with_extension(PROJECT_EXTENSION),
    }
}

pub fn save_project_to(path: &Path, project: &Project) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| format!("create parent dir: {error}"))?;
        }
    }
    let envelope = ProjectEnvelope::new(project.clone());
    let json = serde_json::to_string_pretty(&envelope).map_err(|error| error.to_string())?;
    fs::write(path, json).map_err(|error| format!("write {}: {error}", path.display()))
}

pub fn load_project_from(path: &Path) -> Result<Project, String> {
    let json = fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Project::from_json(&json).map_err(|error| format!("parse {}: {error}", path.display()))
}

/// Push `path` to the front of recent projects (dedupe, cap length).
pub fn push_recent(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|existing| existing != &path);
    recent.insert(0, path);
    if recent.len() > MAX_RECENT_PROJECTS {
        recent.truncate(MAX_RECENT_PROJECTS);
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn write_recovery(
    project: &Project,
    original_path: Option<&Path>,
    project_name: &str,
) -> Result<(), String> {
    let path = recovery_file()?;
    save_project_to(&path, project)?;
    let meta = RecoveryMeta {
        original_path: original_path.map(|p| p.to_path_buf()),
        project_name: project_name.to_string(),
        saved_at_unix: unix_now(),
    };
    let meta_path = recovery_meta_file()?;
    let json = serde_json::to_string_pretty(&meta).map_err(|error| error.to_string())?;
    fs::write(&meta_path, json).map_err(|error| format!("write recovery meta: {error}"))
}

pub fn clear_recovery() -> Result<(), String> {
    let project = recovery_file()?;
    let meta = recovery_meta_file()?;
    if project.exists() {
        fs::remove_file(&project).map_err(|error| format!("remove recovery: {error}"))?;
    }
    if meta.exists() {
        fs::remove_file(&meta).map_err(|error| format!("remove recovery meta: {error}"))?;
    }
    Ok(())
}

pub fn load_recovery_meta() -> Option<RecoveryMeta> {
    let meta_path = recovery_meta_file().ok()?;
    let project_path = recovery_file().ok()?;
    if !meta_path.exists() || !project_path.exists() {
        return None;
    }
    let json = fs::read_to_string(&meta_path).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn load_recovery_project() -> Result<Project, String> {
    load_project_from(&recovery_file()?)
}

pub fn format_unix_time(unix: u64) -> String {
    // Keep it simple and locale-free: YYYY-MM-DD HH:MM UTC-ish from epoch secs.
    // Avoid chrono dependency; good enough for a restore prompt.
    let secs = unix as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    // Civil date from Unix day (Howard Hinnant algorithm, simplified).
    let (y, m, d) = unix_days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{min:02}")
}

fn unix_days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_mute_solo_persist_in_motif_json() {
        let mut project = Project::default();
        project.tracks[0].muted = true;
        project.tracks[0].solo = true;
        let json = serde_json::to_string(&crate::model::persistence::ProjectEnvelope::new(
            project.clone(),
        ))
        .unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert!(loaded.tracks[0].muted);
        assert!(loaded.tracks[0].solo);
    }

    #[test]
    fn envelope_round_trip() {
        let project = Project::default();
        let json = serde_json::to_string_pretty(&ProjectEnvelope::new(project.clone())).unwrap();
        let loaded = Project::from_json(&json).unwrap();
        assert_eq!(loaded.bpm, project.bpm);
        assert_eq!(loaded.tracks.len(), project.tracks.len());
    }

    #[test]
    fn push_recent_dedupes_and_caps() {
        let mut recent = Vec::new();
        for i in 0..15 {
            push_recent(&mut recent, PathBuf::from(format!("/p/{i}.motif")));
        }
        assert_eq!(recent.len(), MAX_RECENT_PROJECTS);
        assert_eq!(recent[0], PathBuf::from("/p/14.motif"));
        push_recent(&mut recent, PathBuf::from("/p/10.motif"));
        assert_eq!(recent[0], PathBuf::from("/p/10.motif"));
        assert_eq!(recent.iter().filter(|p| p.ends_with("10.motif")).count(), 1);
    }

    #[test]
    fn ensure_motif_extension_adds_suffix() {
        assert_eq!(
            ensure_motif_extension(PathBuf::from("/tmp/song")),
            PathBuf::from("/tmp/song.motif")
        );
        assert_eq!(
            ensure_motif_extension(PathBuf::from("/tmp/song.motif")),
            PathBuf::from("/tmp/song.motif")
        );
    }
}
