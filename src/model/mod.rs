mod clip;
mod note;
mod project;
mod track;

pub use clip::{MidiClip, DEFAULT_CLIP_LENGTH_BEATS};
pub use note::Note;
pub use project::{
    Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
pub use track::Track;
