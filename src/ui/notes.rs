//! What you have to say about the playing song, in a side panel that
//! follows it.

use egui::{Align, CornerRadius, Frame, Layout, Margin};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::Action;
use crate::theme::{self, Icon};

use super::widgets;

/// The size the note is written at.
const NOTE_SIZE: f32 = 14.0;

pub fn side_panel(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let fit = super::yielding_panel(
        ui.ctx(),
        "notes-panel",
        theme::SIDE_PANEL_MIN_WIDTH..=640.0,
        app.settings.notes_width,
        ui.available_width() - super::topbar::least_width(ui.ctx()),
    );
    let panel = egui::Panel::right("notes-panel")
        .resizable(true)
        .default_size(app.settings.notes_width)
        .size_range(fit.range.clone())
        .show_separator_line(false)
        .frame(
            Frame::new()
                .fill(palette.panel)
                .inner_margin(Margin::symmetric(12, 12)),
        );
    let response = panel.show(ui, |ui| {
        // The panel is the lyrics panel's twin and sits where it would,
        // under the same window controls.
        let window_controls =
            super::window_controls_reservation(ui.ctx(), false, true, ui.available_width());
        ui.add_space(window_controls.lyrics_top);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            theme::text(
                ui,
                gettext(app.locale, "Notes"),
                theme::bold(18.0),
                palette.text,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::icon_button(
                    ui,
                    Icon::X,
                    18.0,
                    palette.secondary,
                    palette.text,
                    &gettext(app.locale, "Close"),
                )
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
    if (app.settings.notes_width - width).abs() > 1.0
        && super::panel_width_chosen(ui.ctx(), "notes-panel", &fit)
    {
        app.settings.notes_width = width;
        app.actions.push(Action::SettingsChanged);
    }
}

fn contents(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    let Some(now) = app.now_playing() else {
        widgets::empty_state(
            ui,
            &palette,
            Icon::SquarePen,
            &gettext(locale, "Nothing playing"),
            &gettext(locale, "Play a song to write about it."),
        );
        return;
    };
    if crate::notes::track_id(&now.uri).is_none() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::SquarePen,
            &gettext(locale, "Notes are for songs"),
            &gettext(locale, "Play one to write about it."),
        );
        return;
    }
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.vertical(|ui| {
            theme::text(ui, &now.title, theme::medium(13.0), palette.text);
            if !now.subtitle.is_empty() {
                theme::text(ui, &now.subtitle, theme::regular(12.0), palette.secondary);
            }
        });
    });
    if let Some(hint) = app.notes_sync_hint() {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            theme::text(ui, hint, theme::regular(11.5), palette.secondary);
        });
    }
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

    // Enough rows to reach the bottom of the panel, so the editor is the
    // whole rest of it and a click anywhere in it lands in the text.
    let space = ui.available_height();
    let line = ui.fonts_mut(|fonts| fonts.row_height(&theme::regular(NOTE_SIZE)));
    let rows = (((space - 20.0) / line).floor() as usize).max(3);
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
                                egui::RichText::new(gettext(locale, "Write about this song"))
                                    .color(palette.dim),
                            )
                            .font(theme::regular(NOTE_SIZE))
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
