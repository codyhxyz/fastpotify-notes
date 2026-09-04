//! What you have to say about the playing song, in a side panel that
//! follows it.

use egui::{Align, CornerRadius, Frame, Layout, Margin};

use crate::app::App;
use crate::model::Action;
use crate::theme::{self, Icon};

use super::widgets;

/// Height of one line in the editor, for filling the panel with rows.
const LINE_HEIGHT: f32 = 19.0;

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let panel = egui::Panel::right("notes-panel")
        .resizable(true)
        .default_size(app.settings.notes_width)
        .size_range(theme::SIDE_PANEL_MIN_WIDTH..=640.0)
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        let window_controls = super::window_controls_reservation(
            ui.ctx(),
            app.show_queue_panel,
            app.show_lyrics_panel,
            app.show_notes_panel,
            ui.available_width(),
        );
        ui.add_space(window_controls.notes_top);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            theme::text(ui, "Notes", theme::bold(18.0), palette.text);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(ui, Icon::X, 18.0, palette.secondary, palette.text, "Close")
                    .clicked()
                {
                    app.actions.push(Action::ToggleNotesPanel);
                }
            });
        });
        ui.add_space(8.0);
        contents(app, ui);
    });
    let width = response.response.rect.width();
    if (app.settings.notes_width - width).abs() > 1.0 {
        app.settings.notes_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let Some(now) = app.now_playing() else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::SquarePen,
            "Nothing playing",
            "Play a song to write about it.",
        );
        return;
    };
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.vertical(|ui| {
            theme::text(ui, &now.title, theme::medium(13.0), palette.text);
            if !now.subtitle.is_empty() {
                theme::text(ui, &now.subtitle, theme::regular(12.0), palette.secondary);
            }
        });
    });
    ui.add_space(8.0);

    // The times written in the note, each a way back to that moment.
    // Scanned when the text changes, not on every frame.
    if !app.note_chips.is_empty() {
        let chips = app.note_chips.clone();
        ui.horizontal_wrapped(|ui| {
            ui.add_space(4.0);
            for (label, at_ms) in chips {
                if theme::pill_button(ui, &palette, &label, false).clicked() {
                    app.actions.push(Action::Seek(at_ms));
                }
            }
        });
        ui.add_space(8.0);
    }

    let space = ui.available_height();
    let rows = (((space - 24.0) / LINE_HEIGHT).floor() as usize).max(3);
    egui::ScrollArea::vertical()
        .id_salt("note-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .fill(palette.surface)
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut app.note_buffer)
                            .id(egui::Id::new("note-editor"))
                            .hint_text(
                                egui::RichText::new("Write about this song").color(palette.dim),
                            )
                            .font(theme::regular(14.0))
                            .text_color(palette.text)
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .desired_rows(rows),
                    );
                    // The editor owns the string because egui needs it to;
                    // every other change still goes through an action.
                    if response.changed() {
                        app.actions.push(Action::NoteEdited);
                    }
                });
        });
}
