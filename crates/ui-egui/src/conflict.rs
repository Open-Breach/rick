//! The paste-conflict dialogue (SPEC.md *Dialogues*, *Execution*): a copy,
//! move, drop, extraction or undo would land on an existing name. The
//! title is the question, *Replace file?*, with its place among the job's
//! conflicts; the two entries are shown as cards, to compare before
//! choosing; the choices are Skip, Replace (Merge between two folders) and
//! Keep both, the default, with *Do the same for the remaining* while
//! conflicts remain, and Cancel for the whole operation. The job waits
//! until the answer goes back through `Command::ResolveConflict`.

use std::path::Path;
use std::time::SystemTime;

use app::{App, Command};
use egui::{FontId, Rect, Sense, pos2, vec2};
use ext::t;
use types::{Conflict, ConflictAnswer, ConflictId, ConflictSide, Location, Resolution};

use crate::shell::Request;
use crate::theme::{self, CARD_RADIUS, ICON, TEXT_BODY, TEXT_SECONDARY};
use crate::widgets::ButtonKind;
use crate::{dialog, format, icons, integration, widgets};

/// The dialogue's width; it never grows for a long name.
pub const WIDTH: f32 = 520.0;
/// The side of a card's icon tile.
const TILE: f32 = 36.0;

/// What the dialogue holds between frames.
#[derive(Debug, Default)]
pub struct DialogState {
    /// The conflict the checkbox belongs to.
    for_conflict: Option<ConflictId>,
    /// Do the same for the remaining conflicts.
    pub all: bool,
}

/// The three forms of the dialogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// A file meets a file: Skip, Replace, Keep both.
    Files,
    /// A file and a folder of one name: asked about as two files are.
    Mixed,
    /// A folder meets a folder: Skip and Merge.
    Folders,
}

impl Form {
    fn of(conflict: &Conflict) -> Self {
        match (conflict.existing.is_dir, conflict.incoming.is_dir) {
            (true, true) => Self::Folders,
            (false, false) => Self::Files,
            _ => Self::Mixed,
        }
    }
}

/// The last component of a location, lossy.
fn name_of(location: &Location) -> String {
    location.path.file_name().map_or_else(
        || location.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// The title: the question, and the conflict's place when the job has
/// several.
fn title(conflict: &Conflict, form: Form) -> String {
    let question = match (form, conflict.existing.is_dir) {
        (Form::Folders, _) => t!("conflict-merge-folder"),
        (_, true) => t!("conflict-replace-folder"),
        (_, false) => t!("conflict-replace-file"),
    };
    match conflict.position {
        Some((index, total)) if total > 1 => t!(
            "conflict-title-counted",
            question = question,
            index = index,
            total = total
        ),
        _ => question,
    }
}

/// The secondary line: which folder the name is already in, the home
/// directory written `~`.
fn subtitle(conflict: &Conflict, home: &Path) -> String {
    let folder = conflict.target.path.parent().map_or_else(String::new, |p| {
        integration::tilde(Path::new(p.to_string_lossy().as_ref()), home)
    });
    if conflict.existing.is_dir {
        t!("conflict-folder-already-in", folder = folder)
    } else {
        t!("conflict-file-already-in", folder = folder)
    }
}

/// A side's facts under its name: the size and the modified time, a
/// folder's time alone.
fn facts(side: &ConflictSide, now: SystemTime) -> String {
    let when = format::brief(side.modified, now);
    match (side.size, when.is_empty()) {
        (Some(bytes), false) => format!("{}  ·  {when}", format::size(Some(bytes))),
        (Some(bytes), true) => format::size(Some(bytes)),
        (None, _) => when,
    }
}

/// One of the two cards: the caption in capitals over a row with the
/// icon tile, the name on one line — cut with an ellipsis — and the facts
/// under it. The arriving entry's card has an `accent/focus` border.
fn card(ui: &mut egui::Ui, caption: &str, arriving: bool, name: &str, side: &ConflictSide) {
    let scheme = theme::scheme(ui);
    let stroke = if arriving {
        egui::Stroke::new(1.5, scheme.accent_focus)
    } else {
        egui::Stroke::new(1.0, scheme.border_default)
    };
    egui::Frame::new()
        .fill(scheme.bg_card)
        .stroke(stroke)
        .corner_radius(CARD_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            widgets::caps_heading(ui, caption);
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), TILE), Sense::hover());
            let tile = Rect::from_min_size(row.min, vec2(TILE, TILE));
            ui.painter()
                .rect_filled(tile, CARD_RADIUS, scheme.bg_subtle);
            let glyph = if side.is_dir {
                icons::FOLDER
            } else {
                icons::by_extension(name)
            };
            ui.painter().text(
                tile.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(ICON),
                scheme.icon_default,
            );
            let x = tile.right() + 10.0;
            let width = row.right() - x;
            widgets::line(
                ui,
                pos2(x, row.top() + 10.0),
                name,
                FontId::proportional(TEXT_BODY),
                scheme.text_primary,
                width,
            );
            let facts = facts(side, SystemTime::now());
            if !facts.is_empty() {
                widgets::line(
                    ui,
                    pos2(x, row.top() + 27.0),
                    &facts,
                    FontId::proportional(TEXT_SECONDARY),
                    scheme.text_secondary,
                    width,
                );
            }
        });
}

/// The captions of the two cards, by what each side is.
fn captions(conflict: &Conflict) -> (String, String) {
    let existing = if conflict.existing.is_dir {
        t!("conflict-existing-folder")
    } else {
        t!("conflict-existing-file")
    };
    let arriving = if conflict.incoming.is_dir {
        t!("conflict-new-folder")
    } else {
        t!("conflict-new-file")
    };
    (existing, arriving)
}

/// The footer: Cancel at the left, and at the right Skip, Replace or
/// Merge, and Keep both. The filled button is the one Enter chooses:
/// Keep both, or Skip between two folders.
fn footer(ui: &mut egui::Ui, form: Form) -> Option<Resolution> {
    // Both sides of the band report into one cell.
    let answer = std::cell::Cell::new(None);
    dialog::footer(
        ui,
        |ui| {
            if widgets::button(ui, &t!("cancel"), false, true).clicked() {
                answer.set(Some(Resolution::Cancel));
            }
        },
        |ui| {
            // Laid out from the right edge inwards.
            if form != Form::Folders
                && widgets::button(ui, &t!("conflict-keep-both"), true, true).clicked()
            {
                answer.set(Some(keep_both()));
            }
            let overwrite = if form == Form::Folders {
                t!("conflict-merge")
            } else {
                t!("conflict-replace")
            };
            if widgets::caution_button(ui, &overwrite, true).clicked() {
                answer.set(Some(Resolution::Overwrite));
            }
            let skip = if form == Form::Folders {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            };
            if widgets::button_of(ui, None, &t!("skip"), skip, true).clicked() {
                answer.set(Some(Resolution::Skip));
            }
        },
    );
    answer.into_inner()
}

/// *Keep both* before the job's proposed name is filled in.
fn keep_both() -> Resolution {
    Resolution::Rename(String::new())
}

/// Shows the dialogue for the oldest open conflict, if any.
pub fn show(ctx: &egui::Context, app: &App, state: &mut DialogState, requests: &mut Vec<Request>) {
    let Some(conflict) = app.model().conflicts.first() else {
        state.for_conflict = None;
        return;
    };
    if state.for_conflict != Some(conflict.id) {
        state.for_conflict = Some(conflict.id);
        state.all = false;
    }
    let form = Form::of(conflict);
    let home = app.platform().home_dir();
    let mut answer: Option<Resolution> = None;
    dialog::modal(ctx, egui::Id::new("paste-conflict"), WIDTH, |ui| {
        if dialog::band(ui, None, &title(conflict, form), &subtitle(conflict, &home)) {
            answer = Some(Resolution::Cancel);
        }
        dialog::body(ui, |ui| {
            let (existing, arriving) = captions(conflict);
            card(
                ui,
                &existing,
                false,
                &name_of(&conflict.target),
                &conflict.existing,
            );
            card(
                ui,
                &arriving,
                true,
                &name_of(&conflict.source),
                &conflict.incoming,
            );
            if conflict.remaining > 0 {
                let label = if conflict.position.is_some() {
                    t!("conflict-same-for-remaining", count = conflict.remaining)
                } else {
                    t!("conflict-same-for-all-remaining")
                };
                if widgets::checkbox(ui, state.all, &label).clicked() {
                    state.all = !state.all;
                }
            }
        });
        if let Some(chosen) = footer(ui, form) {
            answer = Some(chosen);
        }
        // Enter takes the choice that loses nothing; Escape cancels.
        if answer.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            answer = Some(if form == Form::Folders {
                Resolution::Skip
            } else {
                keep_both()
            });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Resolution::Cancel);
        }
    });
    if let Some(resolution) = answer {
        // Keep both lands under the name the job proposed, the first free
        // numbered one.
        let resolution = match resolution {
            Resolution::Rename(_) => Resolution::Rename(conflict.proposed.clone()),
            other => other,
        };
        requests.push(Request::Command(Command::ResolveConflict {
            id: conflict.id,
            answer: ConflictAnswer {
                resolution,
                apply_to_all: state.all,
            },
        }));
        state.for_conflict = None;
    }
}
