//! The `UiHost` an immediate-mode UI can offer: answers are given before an
//! action runs, questions it asks without an answer are recorded for the
//! next frame.
//!
//! Actions call [`UiHost::prompt`] and [`UiHost::confirm`] expecting a
//! blocking dialogue. egui cannot block inside a frame, so the shell runs an
//! action twice when it has to: the first run gets `None` or `false` and
//! leaves the question in [`EguiHost::take_asked`]; the shell shows the
//! dialogue, and when the user answers it stores the answer and runs the
//! action again, which now gets the answer at once.

use std::sync::{Mutex, MutexGuard, PoisonError};

use ext::{Form, Level, UiHost, Values};

/// A question an action asked and nobody had answered yet.
#[derive(Debug, Clone)]
pub enum Ask {
    /// A line of text.
    Prompt {
        /// Dialogue title.
        title: String,
        /// Suggested text.
        default: String,
    },
    /// Yes or no.
    Confirm {
        /// Dialogue title.
        title: String,
        /// The question.
        text: String,
    },
    /// A form: fields with a live preview.
    Form(Form),
}

impl Ask {
    /// The dialogue's title.
    #[must_use]
    pub fn title(&self) -> &str {
        match self {
            Self::Prompt { title, .. } | Self::Confirm { title, .. } => title,
            Self::Form(form) => &form.title,
        }
    }
}

#[derive(Default)]
struct Inner {
    prompt_answer: Option<String>,
    confirm_answer: Option<bool>,
    form_answer: Option<Values>,
    asked: Option<Ask>,
    notices: Vec<(Level, String)>,
    shown: Vec<(String, String)>,
}

/// The shell's [`UiHost`].
#[derive(Default)]
pub struct EguiHost {
    inner: Mutex<Inner>,
}

impl EguiHost {
    /// A host with no answers queued.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Plain data; a panic elsewhere leaves it usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The answer the next `prompt` call gets.
    pub fn answer_prompt(&self, text: String) {
        self.lock().prompt_answer = Some(text);
    }

    /// The answer the next `confirm` call gets.
    pub fn answer_confirm(&self, yes: bool) {
        self.lock().confirm_answer = Some(yes);
    }

    /// The values the next `form` call gets.
    pub fn answer_form(&self, values: Values) {
        self.lock().form_answer = Some(values);
    }

    /// Drops any queued answer.
    pub fn clear_answers(&self) {
        let mut inner = self.lock();
        inner.prompt_answer = None;
        inner.confirm_answer = None;
        inner.form_answer = None;
    }

    /// The question an action asked since the last call, if any.
    pub fn take_asked(&self) -> Option<Ask> {
        self.lock().asked.take()
    }

    /// The notifications since the last call.
    pub fn take_notices(&self) -> Vec<(Level, String)> {
        std::mem::take(&mut self.lock().notices)
    }

    /// The texts actions asked to show since the last call, as (title,
    /// text).
    pub fn take_shown(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.lock().shown)
    }
}

impl std::fmt::Debug for EguiHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("EguiHost")
            .field("asked", &inner.asked)
            .field("notices", &inner.notices.len())
            .finish_non_exhaustive()
    }
}

impl UiHost for EguiHost {
    fn notify(&self, level: Level, text: &str) {
        self.lock().notices.push((level, text.to_owned()));
    }

    fn confirm(&self, title: &str, text: &str) -> bool {
        let mut inner = self.lock();
        if let Some(answer) = inner.confirm_answer.take() {
            return answer;
        }
        inner.asked = Some(Ask::Confirm {
            title: title.to_owned(),
            text: text.to_owned(),
        });
        false
    }

    fn prompt(&self, title: &str, default: &str) -> Option<String> {
        let mut inner = self.lock();
        if let Some(answer) = inner.prompt_answer.take() {
            return Some(answer);
        }
        inner.asked = Some(Ask::Prompt {
            title: title.to_owned(),
            default: default.to_owned(),
        });
        None
    }

    fn show(&self, title: &str, text: &str) {
        self.lock().shown.push((title.to_owned(), text.to_owned()));
    }

    fn form(&self, form: Form) -> Option<Values> {
        let mut inner = self.lock();
        if let Some(values) = inner.form_answer.take() {
            return Some(values);
        }
        inner.asked = Some(Ask::Form(form));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unanswered_questions_are_recorded_and_answers_are_consumed_once() {
        let host = EguiHost::new();
        assert_eq!(host.prompt("Rename", "a.txt"), None);
        assert!(matches!(
            host.take_asked(),
            Some(Ask::Prompt { title, default }) if title == "Rename" && default == "a.txt"
        ));
        assert!(host.take_asked().is_none());

        host.answer_prompt("b.txt".into());
        assert_eq!(host.prompt("Rename", "a.txt").as_deref(), Some("b.txt"));
        assert_eq!(
            host.prompt("Rename", "a.txt"),
            None,
            "an answer serves one call"
        );

        assert!(!host.confirm("Delete", "Really?"));
        assert!(matches!(host.take_asked(), Some(Ask::Confirm { .. })));
        host.answer_confirm(true);
        assert!(host.confirm("Delete", "Really?"));

        host.notify(Level::Info, "hi");
        assert_eq!(host.take_notices(), vec![(Level::Info, "hi".to_owned())]);
        assert_eq!(host.take_notices(), Vec::new());

        let form = Form::new("Compress", "OK").field(ext::Field::text("name", "Name", "a.zip", ""));
        assert!(host.form(form.clone()).is_none());
        assert!(matches!(host.take_asked(), Some(Ask::Form(f)) if f.title == "Compress"));
        host.answer_form(form.defaults());
        assert_eq!(
            host.form(form).map(|v| v.text("name").to_owned()),
            Some("a.zip".to_owned())
        );
    }
}
