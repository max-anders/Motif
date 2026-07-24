mod clip;
mod instrument;
mod note;
mod project;
mod serde_b64;
mod track;

pub use clip::{MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
pub use instrument::{PluginFormat, TrackInstrument};
pub use note::Note;
pub use project::{
    Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
#[allow(unused_imports)] // public model surface for tests / future UI
pub use track::Track;
