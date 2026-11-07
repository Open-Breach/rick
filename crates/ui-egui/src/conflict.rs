//! The conflict dialogue: a copy, move, drag and drop or undo would land on
//! an existing name. Skip, Overwrite, Rename (with a proposed `name (2)`),
//! Apply to all remaining, Cancel. The job waits until the answer goes back
//! through `Command::ResolveConflict`.

use app::{App, Command};
use types::{ConflictAnswer, ConflictId, Resolution};

use crate::shell::Request;

/// What the dialogue holds between frames.
#[derive(Debug, Default)]
pub struct DialogState {
    /// The conflict the text field and checkbox belong to.
    for_conflict: Option<ConflictId>,
    /// The Rename field.
    pub name: String,
    /// Apply to all remaining.
    pub all: bool,
}

/// Shows the dialogue for the oldest open conflict, if any.
pub fn show(ctx: &egui::Context, app: &App, state: &mut DialogState, requests: &mut Vec<Request>) {
    let Some(conflict) = app.model().conflicts.first() else {
        state.for_conflict = None;
        return;
    };
    if state.for_conflict != Some(conflict.id) {
        state.for_conflict = Some(conflict.id);
        state.name.clone_from(&conflict.proposed);
        state.all = false;
    }
    let name = conflict.target.path.file_name().map_or_else(
        || conflict.target.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    );
    let folder = conflict
        .target
        .path
        .parent()
        .map_or_else(String::new, |p| p.to_string_lossy().into_owned());
    let mut answer: Option<Resolution> = None;
    egui::Modal::new(egui::Id::new("conflict")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.heading(format!("{name} already exists"));
        ui.label(format!(
            "{} is already in {folder}.",
            if conflict.target_is_dir {
                "A folder with this name"
            } else {
                "A file with this name"
            }
        ));
        let what = if conflict.target_is_dir {
            "Overwrite merges the folders; Skip leaves the existing one."
        } else {
            "Overwrite replaces the existing file; Skip leaves it."
        };
        ui.weak(what);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Rename to");
            let output = egui::TextEdit::singleline(&mut state.name)
                .desired_width(220.0)
                .show(ui);
            if output.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                answer = Some(Resolution::Rename(state.name.trim().to_owned()));
            }
            let free = !state.name.trim().is_empty() && state.name.trim() != name;
            if ui.add_enabled(free, egui::Button::new("Rename")).clicked() {
                answer = Some(Resolution::Rename(state.name.trim().to_owned()));
            }
        });
        if conflict.remaining > 0 {
            ui.checkbox(
                &mut state.all,
                format!("Apply to all remaining ({} more)", conflict.remaining),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Skip").clicked() {
                answer = Some(Resolution::Skip);
            }
            if ui
                .button(if conflict.target_is_dir {
                    "Overwrite (merge)"
                } else {
                    "Overwrite"
                })
                .clicked()
            {
                answer = Some(Resolution::Overwrite);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    answer = Some(Resolution::Cancel);
                }
            });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Resolution::Cancel);
        }
    });
    if let Some(resolution) = answer {
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
