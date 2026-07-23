mod note;
mod project;

pub use note::Note;
pub use project::{
    Project, DEFAULT_NOTE_DURATION_BEATS, MAX_PITCH, MIN_PITCH, SNAP_BEATS,
};
