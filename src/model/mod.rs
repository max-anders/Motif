mod clip;
mod clipboard;
mod history;
mod instrument;
mod note;
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
pub use note::Note;
pub use project::{Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS};
#[allow(unused_imports)] // public model surface for tests / future UI
pub use track::Track;
