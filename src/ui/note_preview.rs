//! Shared mini MIDI note previews (playlist clip thumbnails, pattern rack rows).

use egui::{Painter, Pos2, Rect};

use crate::model::{Note, MAX_PITCH, MIN_PITCH, SNAP_BEATS};
use crate::ui::theme::ThemeColors;

/// Layout tuning for note preview bars inside a rect.
#[derive(Debug, Clone, Copy)]
pub struct NotePreviewStyle {
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub horizontal_padding: f32,
    pub bar_height: f32,
    pub min_bar_width: f32,
    pub corner_radius: f32,
}

impl NotePreviewStyle {
    /// Playlist MIDI clip thumbnail (label occupies the top band).
    pub fn clip_thumbnail() -> Self {
        Self {
            top_inset: 20.0,
            bottom_inset: 4.0,
            horizontal_padding: 4.0,
            bar_height: 3.0,
            min_bar_width: 2.0,
            corner_radius: 1.0,
        }
    }

    /// Pattern rack row content area (no overlaid clip label).
    pub fn rack_row() -> Self {
        Self {
            top_inset: 4.0,
            bottom_inset: 4.0,
            horizontal_padding: 4.0,
            bar_height: 4.0,
            min_bar_width: 2.0,
            corner_radius: 1.0,
        }
    }
}

/// Draw horizontal pitch bars for pattern-local or clip-local notes over `length_beats`.
pub fn draw_note_preview(
    painter: &Painter,
    rect: Rect,
    notes: &[Note],
    length_beats: f32,
    theme: &ThemeColors,
    style: &NotePreviewStyle,
) {
    if notes.is_empty() {
        return;
    }

    let preview_top = rect.top() + style.top_inset;
    let preview_height = (rect.height() - style.top_inset - style.bottom_inset).max(8.0);
    let pitch_span = (MAX_PITCH - MIN_PITCH + 1) as f32;
    let length = length_beats.max(SNAP_BEATS);
    let inner_width = (rect.width() - style.horizontal_padding * 2.0).max(1.0);
    let clipped = painter.with_clip_rect(rect);

    for note in notes {
        if note.start_beats >= length {
            continue;
        }
        let rel_start = note.start_beats / length;
        let rel_end = note.end_beats().min(length) / length;
        let x0 = rect.left() + style.horizontal_padding + rel_start * inner_width;
        let x1 = rect.left() + style.horizontal_padding + rel_end * inner_width;
        let pitch_norm = (note.pitch as f32 - MIN_PITCH as f32) / pitch_span;
        let y = preview_top + (1.0 - pitch_norm) * preview_height;
        clipped.rect_filled(
            Rect::from_min_max(
                Pos2::new(x0, y),
                Pos2::new(
                    x1.max(x0 + style.min_bar_width),
                    y + style.bar_height,
                ),
            ),
            style.corner_radius,
            theme.clip_note_preview,
        );
    }
}
