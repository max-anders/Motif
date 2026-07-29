mod automation;
mod audio_clip;
mod clip;
mod pattern;
mod clipboard;
mod history;
mod instrument;
mod mixer;
mod modulator;
mod note;
mod persistence;
mod project;
mod serde_b64;
mod track;

#[allow(unused_imports)] // public automation surface for UI / engine / tests
pub use automation::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveKind,
};
#[allow(unused_imports)] // public audio clip surface for engine / app
pub use audio_clip::AudioClip;
#[allow(unused_imports)] // public clip surfaces for app / tests
pub use clip::{Clip, MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
#[allow(unused_imports)] // public clipboard surface for app / tests
pub use clipboard::{ClipboardClip, ClipboardNote, EditClipboard};
pub use history::{
    clamp_undo_limit, EditHistory, DEFAULT_UNDO_LIMIT, MAX_UNDO_LIMIT, MIN_UNDO_LIMIT,
};
pub use instrument::{PluginFormat, TrackInstrument};
#[allow(unused_imports)] // public mixer surface for UI / engine / tests
pub use mixer::{
    db_to_linear, pan_gains, Device, Macro, MacroMapping, MacroTarget, Send, MAX_GAIN_DB,
    MIN_GAIN_DB,
};
#[allow(unused_imports)] // public modulator surface for UI / engine / tests
pub use modulator::{LfoModulator, LfoRate, LfoShape};
pub use note::Note;
#[allow(unused_imports)] // public pattern surface for engine / UI / tests
pub use pattern::{
    PatternBlock, PatternLane, PatternRowMode, PatternTrackContent, ResolvedMidiNote,
};
pub use persistence::{
    clear_recovery, ensure_motif_extension, format_unix_time, legacy_project_path, load_project_from,
    load_recovery_meta, load_recovery_project, project_display_name, projects_dir, push_recent,
    save_project_to, write_recovery, RecoveryMeta, DEFAULT_AUTOSAVE_INTERVAL_SECS, PROJECT_EXTENSION,
};
pub use project::{
    BakeError, Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_LOOP_SPAN_BEATS, MIN_PITCH,
    SNAP_BEATS,
};
#[allow(unused_imports)] // public model surface for tests / future UI
pub use track::Track;
