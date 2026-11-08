//! The question dialogue: an extension asked the user something while it
//! works — an unknown host key to trust, a changed one to refuse, a
//! passphrase for an encrypted key, a password the keychain does not hold
//! — and waits for the answer, which goes back through
//! `Command::AnswerQuestion`. One question at a time, the oldest first; a
//! warning (a changed host key) is styled as one and Enter picks nothing
//! but its default, *Cancel*.

use app::{App, Command};
use types::{Answer, QuestionId, QuestionKind};

use crate::shell::Request;

/// What the dialogue holds between frames.
#[derive(Debug, Default)]
pub struct DialogState {
    /// The question the field and checkbox belong to.
    for_question: Option<QuestionId>,
    /// The secret as typed.
    pub secret: String,
    /// The *Remember* checkbox.
    pub remember: bool,
    /// Take the keyboard on the next frame.
    focus: bool,
}

/// Shows the dialogue for the oldest open question, if any.
pub fn show(ctx: &egui::Context, app: &App, state: &mut DialogState, requests: &mut Vec<Request>) {
    let Some(question) = app.model().questions.first() else {
        state.for_question = None;
        return;
    };
    if state.for_question != Some(question.id) {
        state.for_question = Some(question.id);
        state.secret.clear();
        state.remember = match &question.kind {
            QuestionKind::Secret { remember } => remember.unwrap_or(false),
            QuestionKind::Choice { .. } => false,
        };
        state.focus = true;
    }
    let mut answer: Option<Answer> = None;
    egui::Modal::new(egui::Id::new("question")).show(ctx, |ui| {
        ui.set_width(460.0);
        let heading = egui::RichText::new(&question.title).heading();
        if question.warning {
            ui.label(heading.color(ui.visuals().warn_fg_color));
        } else {
            ui.label(heading);
        }
        ui.add_space(4.0);
        ui.label(&question.text);
        ui.add_space(8.0);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        match &question.kind {
            QuestionKind::Choice { options, default } => {
                ui.horizontal(|ui| {
                    for (index, option) in options.iter().enumerate() {
                        let button = if index == *default {
                            egui::Button::new(egui::RichText::new(option).strong())
                        } else {
                            egui::Button::new(option)
                        };
                        if ui.add(button).clicked() {
                            answer = Some(Answer::Choice(index));
                        }
                    }
                });
                if enter && answer.is_none() {
                    answer = Some(Answer::Choice(*default));
                }
            }
            QuestionKind::Secret { remember } => {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut state.secret)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                if state.focus {
                    field.request_focus();
                    state.focus = false;
                }
                if remember.is_some() {
                    ui.checkbox(&mut state.remember, "Remember in the keychain");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() || (enter && field.lost_focus()) {
                        answer = Some(Answer::Secret {
                            secret: state.secret.clone(),
                            remember: state.remember,
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(Answer::Cancelled);
                    }
                });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(Answer::Cancelled);
        }
    });
    if let Some(answer) = answer {
        requests.push(Request::Command(Command::AnswerQuestion {
            id: question.id,
            answer,
        }));
        state.for_question = None;
    }
}
