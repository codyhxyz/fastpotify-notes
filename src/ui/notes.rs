//! What you have to say about the playing song, in a side panel that
//! follows it.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, UiBuilder, Vec2, pos2, vec2};

use crate::app::App;
use crate::model::{Action, Page};
use crate::theme::{self, Icon};

use super::widgets;

/// The size the note is written at.
const NOTE_SIZE: f32 = 14.0;
const ROW_HEIGHT: f32 = 64.0;
const COVER: f32 = 40.0;
/// Room on the right of a row for when the note was last written in.
const WHEN_WIDTH: f32 = 96.0;

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
                if theme::pill_button(ui, &palette, "All notes", false).clicked() {
                    app.actions.push(Action::Open(Page::Notes));
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

    // Enough rows to reach the bottom of the panel, so the editor is the
    // whole rest of it and a click anywhere in it lands in the text.
    let space = ui.available_height();
    let line = ui.ctx().fonts_mut(|fonts| fonts.row_height(&theme::regular(NOTE_SIZE)));
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
                                egui::RichText::new("Write about this song").color(palette.dim),
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

/// What a click on a row asked for.
enum Picked {
    Play,
    Delete,
}

/// Every note written here, newest first, with a search over all of it.
pub fn page(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        theme::text(ui, "Notes", theme::bold(28.0), palette.text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let width = ui.available_width().clamp(160.0, 280.0);
            widgets::search_field(
                ui,
                &palette,
                egui::Id::new("notes-search"),
                &mut app.notes_query,
                "Search your notes",
                width,
            );
        });
    });
    ui.add_space(12.0);

    if app.notes.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::SquarePen,
            "No notes yet",
            "Open the Notes panel while a song plays and start writing.",
        );
        return;
    }

    let now = jiff::Timestamp::now();
    let query = app.notes_query.trim().to_lowercase();
    let mut picked: Option<(String, Picked)> = None;
    let mut shown = 0;
    for (uri, note) in app.notes.newest_first() {
        if !matches(note, &query) {
            continue;
        }
        shown += 1;
        if let Some(pick) = row(ui, &palette, uri, note, now) {
            picked = Some((uri.to_string(), pick));
        }
    }
    if shown == 0 {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            "Nothing found",
            "No note mentions that.",
        );
    }
    match picked {
        Some((uri, Picked::Play)) => {
            app.actions.push(Action::PlayUris {
                uris: vec![uri],
                index: 0,
            });
            app.actions.push(Action::ShowNotesPanel);
        }
        Some((uri, Picked::Delete)) => app.actions.push(Action::DeleteNote(uri)),
        None => {}
    }
}

/// Whether a note answers the search, over everything it holds. The list
/// is tens to hundreds of notes, so this runs over the loaded list rather
/// than through an index.
fn matches(note: &crate::notes::Note, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let holds = |text: &str| text.to_lowercase().contains(query);
    holds(&note.text)
        || holds(&note.track.title)
        || holds(&note.track.album)
        || note.track.artists.iter().any(|artist| holds(artist))
}

fn row(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    uri: &str,
    note: &crate::notes::Note,
    now: jiff::Timestamp,
) -> Option<Picked> {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    if ui.rect_contains_pointer(rect) {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            palette
                .surface_hover
                .gamma_multiply(if palette.dark { 0.7 } else { 1.0 }),
        );
    }
    let art = Rect::from_center_size(
        pos2(rect.left() + 8.0 + COVER / 2.0, rect.center().y),
        Vec2::splat(COVER),
    );
    widgets::paint_cover(
        ui,
        palette,
        note.track.art_url.as_deref(),
        art,
        4.0,
        Icon::Music,
    );

    let mut column = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(art.right() + 12.0, rect.top() + 7.0),
                pos2(rect.right() - WHEN_WIDTH, rect.bottom() - 7.0),
            ))
            .layout(Layout::top_down(Align::Min)),
    );
    column.spacing_mut().item_spacing.y = 1.0;
    let title = if note.track.title.is_empty() {
        uri
    } else {
        note.track.title.as_str()
    };
    theme::text(&mut column, title, theme::medium(14.0), palette.text);
    if !note.track.artists.is_empty() {
        theme::text(
            &mut column,
            note.track.artists.join(", "),
            theme::regular(12.5),
            palette.secondary,
        );
    }
    theme::text(
        &mut column,
        note.summary(),
        theme::regular(12.5),
        palette.dim,
    );

    let mut when = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.right() - WHEN_WIDTH, rect.top()),
                pos2(rect.right() - 10.0, rect.bottom()),
            ))
            .layout(Layout::right_to_left(Align::Center)),
    );
    theme::text(
        &mut when,
        crate::util::format_relative_date(&note.updated_at, now),
        theme::regular(12.0),
        palette.secondary,
    );

    let mut picked = response.clicked().then_some(Picked::Play);
    egui::Popup::context_menu(&response)
        .frame(widgets::menu_frame(palette))
        .show(|ui| {
            ui.set_min_width(180.0);
            if widgets::menu_item(ui, palette, Some(Icon::Trash), "Delete note") {
                picked = Some(Picked::Delete);
                ui.close();
            }
        });
    picked
}
