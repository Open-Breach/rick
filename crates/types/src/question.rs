//! A question an extension asks the user while it works, and the answer it
//! waits for: an unknown host key to trust or refuse, a passphrase for an
//! encrypted key, a password the keychain does not hold.
//!
//! Actions ask through the `UiHost` they run with; a provider or a
//! connector has none in hand when a connection opens under a listing, so
//! it asks through the prompt queue of `runtime` instead and the
//! application shows the dialogue. These values cross from the extension
//! to the application, the model and the UI, which is why they live here.

use std::fmt;

/// Identity of one question, unique within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QuestionId(pub u64);

impl fmt::Display for QuestionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "question {}", self.0)
    }
}

/// What kind of answer a question wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionKind {
    /// One of a few options, each a button; `default` is the one Enter
    /// picks. Escape and closing the dialogue answer
    /// [`Answer::Cancelled`].
    Choice {
        /// The options, in display order; the last is usually *Cancel*.
        options: Vec<String>,
        /// The index of the option Enter picks.
        default: usize,
    },
    /// A secret typed masked: a password or a passphrase. With `remember`
    /// set, the dialogue shows a *Remember in the keychain* checkbox
    /// starting at that value.
    Secret {
        /// Whether to offer remembering, and the checkbox's initial state.
        remember: Option<bool>,
    },
}

/// A question an extension asked and the application shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// This question.
    pub id: QuestionId,
    /// The dialogue's heading: `Unknown host key`, `Passphrase for
    /// ~/.ssh/id_ed25519`.
    pub title: String,
    /// The body: the key type and fingerprint, what the password is for.
    pub text: String,
    /// What kind of answer it wants.
    pub kind: QuestionKind,
    /// A warning the user should read twice (a changed host key): the
    /// dialogue is styled as one and Enter picks nothing but the default.
    pub warning: bool,
}

/// The dialogue's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The option at this index of a [`QuestionKind::Choice`].
    Choice(usize),
    /// The secret typed into a [`QuestionKind::Secret`], and whether the
    /// *Remember* checkbox was ticked.
    Secret {
        /// What was typed.
        secret: String,
        /// Whether to keep it in the keychain.
        remember: bool,
    },
    /// The dialogue was dismissed, or the application went away before an
    /// answer came.
    Cancelled,
}

impl Answer {
    /// The chosen index of a choice; `None` for anything else.
    #[must_use]
    pub fn choice(&self) -> Option<usize> {
        match self {
            Self::Choice(index) => Some(*index),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_read_back() {
        assert_eq!(Answer::Choice(2).choice(), Some(2));
        assert_eq!(Answer::Cancelled.choice(), None);
        assert_eq!(
            Answer::Secret {
                secret: "x".into(),
                remember: true
            }
            .choice(),
            None
        );
        assert_eq!(QuestionId(3).to_string(), "question 3");
    }
}
