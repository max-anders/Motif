//! Pattern rack: per-playlist-track rows for drafting section MIDI overrides.
//! Step grids are painted inline; melody opens the slim piano-roll row editor.

use egui::{Align2, Button, Pos2, Rect, RichText, Sense, Ui, Vec2};

use crate::engine::DawEngine;
use crate::model::{EditHistory, Note, PatternRowMode, Project};
use crate::ui::note_preview::{draw_note_preview, NotePreviewStyle};
use crate::ui::pattern_row_editor::paint_inline_step_strip;
use crate::ui::theme::ThemeColors;

const ROW_HEIGHT: f32 = 56.0;
const LABEL_WIDTH: f32 = 120.0;
const MELODY_BTN_WIDTH: f32 = 58.0;
const ROW_GAP: f32 = 2.0;

#[derive(Debug, Default)]
pub struct PatternRackUi {
    selected_track_id: Option<u64>,
    /// In-progress step drag-paint: `(track_id, on/off)`.
    step_paint: Option<(u64, bool)>,
}

pub enum PatternRackAction {
    None,
    Status(String),
    /// Open the slim piano-roll editor for this row.
    OpenMelody(u64),
    /// Bake pattern MIDI into playlist clips for this block.
    Bake,
}

impl PatternRackUi {
    pub fn clear_selection(&mut self) {
        self.selected_track_id = None;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        block_id: u64,
        project: &mut Project,
        engine: &dyn DawEngine,
        history: &mut EditHistory,
        selected_track: &mut Option<u64>,
        theme: &ThemeColors,
    ) -> PatternRackAction {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme.panel_bg);

        let Some(block) = project.pattern_block(block_id).cloned() else {
            return PatternRackAction::None;
        };

        ui.heading(format!("Pattern: {}", block.name));
        ui.label(
            RichText::new(format!(
                "{:.1} beats at {:.1} on the arrangement",
                block.length_beats, block.start_beats
            ))
            .color(theme.text_muted)
            .small(),
        );
        ui.label(
            RichText::new(
                "Paint steps inline on each row; Melody opens the piano-roll editor. Empty rows are off (playlist MIDI unchanged). Lower-lane rows dim when a higher lane wins the same track.",
            )
            .color(theme.text_muted)
            .small(),
        );
        let mut action = PatternRackAction::None;
        ui.horizontal(|ui| {
            if ui.button("Bake to playlist").clicked() {
                action = PatternRackAction::Bake;
            }
            ui.label(
                RichText::new("Commits pattern MIDI inside this section; removes the pattern block.")
                    .color(theme.text_muted)
                    .small(),
            );
        });
        ui.add_space(6.0);

        let track_ids: Vec<u64> = project.tracks.iter().map(|track| track.id).collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for track_id in track_ids {
                    let track_name = project
                        .track(track_id)
                        .map(|track| track.name.clone())
                        .unwrap_or_default();
                    if track_name.is_empty() {
                        continue;
                    }
                    let notes: Vec<Note> = project
                        .pattern_block(block_id)
                        .and_then(|block| block.track_content(track_id))
                        .map(|row| row.notes.clone())
                        .unwrap_or_default();
                    let has_data = !notes.is_empty();
                    let row_selected = self.selected_track_id == Some(track_id);
                    let suppressed = has_data
                        && project.pattern_row_suppressed_by_higher_lane(block_id, track_id);
                    let step_row = matches!(
                        project.pattern_row_mode(block_id, track_id),
                        PatternRowMode::Step
                    );

                    let (row_rect, row_response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    let painter = ui.painter_at(row_rect);

                    let bg = if suppressed {
                        theme.pattern_rack_row_suppressed
                    } else if row_selected {
                        theme.pattern_rack_row_selected
                    } else if has_data {
                        theme.pattern_rack_row_bg
                    } else {
                        theme.pattern_rack_row_inactive
                    };
                    painter.rect_filled(row_rect, 2.0, bg);
                    if row_selected {
                        painter.rect_stroke(
                            row_rect,
                            2.0,
                            egui::Stroke::new(1.0_f32, theme.pattern_block_stroke_selected),
                            egui::StrokeKind::Inside,
                        );
                    }

                    let label_rect = Rect::from_min_max(
                        Pos2::new(row_rect.left() + 8.0, row_rect.top()),
                        Pos2::new(row_rect.left() + LABEL_WIDTH, row_rect.bottom()),
                    );
                    let label_response =
                        ui.interact(label_rect, ui.id().with(track_id).with("label"), Sense::click());
                    let label_color = if has_data {
                        theme.track_header_text
                    } else {
                        theme.text_muted
                    };
                    painter.text(
                        Pos2::new(label_rect.left() + 4.0, label_rect.center().y - 6.0),
                        Align2::LEFT_CENTER,
                        &track_name,
                        egui::FontId::proportional(12.0),
                        label_color,
                    );
                    let status_label = if !has_data {
                        "off"
                    } else if suppressed {
                        "muted"
                    } else {
                        "active"
                    };
                    painter.text(
                        Pos2::new(label_rect.left() + 4.0, label_rect.center().y + 8.0),
                        Align2::LEFT_CENTER,
                        status_label,
                        egui::FontId::proportional(10.0),
                        theme.text_muted,
                    );

                    if label_response.clicked() {
                        self.selected_track_id = Some(track_id);
                        *selected_track = Some(track_id);
                    }

                    let melody_btn_rect = Rect::from_min_max(
                        Pos2::new(label_rect.right() + 4.0, row_rect.top() + 6.0),
                        Pos2::new(
                            label_rect.right() + 4.0 + MELODY_BTN_WIDTH,
                            row_rect.bottom() - 6.0,
                        ),
                    );
                    let melody_response = ui.put(
                        melody_btn_rect,
                        Button::new(
                            RichText::new("Melody")
                                .size(11.0)
                                .color(theme.track_header_text),
                        )
                        .min_size(Vec2::new(MELODY_BTN_WIDTH - 4.0, melody_btn_rect.height())),
                    );
                    if melody_response.clicked() {
                        self.selected_track_id = Some(track_id);
                        *selected_track = Some(track_id);
                        project.set_pattern_row_mode(
                            block_id,
                            track_id,
                            Some(PatternRowMode::Melody),
                        );
                        action = PatternRackAction::OpenMelody(track_id);
                    }

                    let content_rect = Rect::from_min_max(
                        Pos2::new(melody_btn_rect.right() + 6.0, row_rect.top() + 4.0),
                        Pos2::new(row_rect.right() - 6.0, row_rect.bottom() - 4.0),
                    );
                    painter.rect_filled(content_rect, 2.0, theme.pattern_rack_content_bg);

                    if step_row {
                        paint_inline_step_strip(
                            ui,
                            content_rect,
                            block_id,
                            track_id,
                            &block,
                            project,
                            engine,
                            history,
                            &mut self.step_paint,
                            theme,
                        );
                    } else if has_data {
                        draw_note_preview(
                            &painter,
                            content_rect,
                            &notes,
                            block.length_beats,
                            theme,
                            &NotePreviewStyle::rack_row(),
                        );
                        let preview_response = ui.interact(
                            content_rect,
                            ui.id().with(track_id).with("preview"),
                            Sense::click(),
                        );
                        if preview_response.clicked() {
                            self.selected_track_id = Some(track_id);
                            *selected_track = Some(track_id);
                            action = PatternRackAction::OpenMelody(track_id);
                        }
                    } else {
                        painter.text(
                            content_rect.center(),
                            Align2::CENTER_CENTER,
                            "melody row - use Melody",
                            egui::FontId::proportional(11.0),
                            theme.text_muted,
                        );
                    }

                    if row_response.clicked()
                        && !label_response.hovered()
                        && !melody_response.hovered()
                    {
                        self.selected_track_id = Some(track_id);
                        *selected_track = Some(track_id);
                    }

                    ui.add_space(ROW_GAP);
                }
            });

        action
    }
}
