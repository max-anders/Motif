mod clip;
mod clipboard;
mod history;
mod instrument;
mod mixer;
mod note;
mod persistence;
mod project;
mod serde_b64;
mod track;

pub use clip::{MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
#[allow(unused_imports)] // public clipboard surface for app / tests
pub use clipboard::{ClipboardClip, ClipboardNote, EditClipboard};
pub use history::{
    clamp_undo_limit, EditHistory, DEFAULT_UNDO_LIMIT, MAX_UNDO_LIMIT, MIN_UNDO_LIMIT,
};
pub use instrument::{PluginFormat, TrackInstrument};
#[allow(unused_imports)] // public mixer surface for UI / engine / tests
pub use mixer::{
    db_to_linear, pan_gains, Device, Macro, Send, MAX_GAIN_DB, MIN_GAIN_DB,
};
pub use note::Note;
pub use persistence::{
    clear_recovery, ensure_motif_extension, format_unix_time, legacy_project_path, load_project_from,
    load_recovery_meta, load_recovery_project, project_display_name, projects_dir, push_recent,
    save_project_to, write_recovery, RecoveryMeta, DEFAULT_AUTOSAVE_INTERVAL_SECS, PROJECT_EXTENSION,
};
pub use project::{Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS};
#[allow(unused_imports)] // public model surface for tests / future UI
pub use track::Track;
