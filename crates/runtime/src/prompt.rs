//! Where an extension asks the user a question while it works, and the
//! application answers: the host-key and passphrase dialogues of a
//! connection that opens under a listing, where no `UiHost` is in hand.
//!
//! The queue works like [`Conflicts`](crate::Conflicts): the asker puts a
//! [`Question`] in, rings the waker and awaits a `oneshot`; the application
//! takes the new questions, shows one dialogue at a time and answers by
//! id. A question dropped unanswered (the application went away, or no
//! application is listening) is [`Answer::Cancelled`], so a provider
//! without a user in front of it never trusts or guesses anything.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;
use tracing::debug;
use types::{Answer, Question, QuestionId, QuestionKind};

/// Rings when a new question is waiting, so the UI repaints.
pub type PromptWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Inner {
    next: u64,
    /// Questions asked since the application last looked.
    fresh: Vec<Question>,
    /// The askers waiting, by question.
    waiting: HashMap<QuestionId, oneshot::Sender<Answer>>,
}

/// The question queue: extensions ask, the application answers.
#[derive(Default)]
pub struct Prompts {
    inner: Mutex<Inner>,
    waker: Mutex<Option<PromptWaker>>,
}

impl std::fmt::Debug for Prompts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Prompts")
            .field("fresh", &inner.fresh.len())
            .field("waiting", &inner.waiting.len())
            .finish()
    }
}

impl Prompts {
    /// No questions yet.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Plain data; a panic elsewhere leaves it usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the function called after every new question.
    pub fn set_waker(&self, waker: PromptWaker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// The questions asked since the last call.
    pub fn take_new(&self) -> Vec<Question> {
        std::mem::take(&mut self.lock().fresh)
    }

    /// Answers a question. Returns `false` when nobody waits on that id
    /// any more.
    pub fn answer(&self, id: QuestionId, answer: Answer) -> bool {
        let sender = self.lock().waiting.remove(&id);
        match sender {
            Some(sender) => sender.send(answer).is_ok(),
            None => false,
        }
    }

    /// Whether an asker waits on an answer right now.
    #[must_use]
    pub fn is_waiting(&self, id: QuestionId) -> bool {
        self.lock().waiting.contains_key(&id)
    }

    /// Asks `kind` under `title` with `text` and waits for the answer;
    /// `warning` styles the dialogue as one to read twice. A question
    /// nobody answers (the application went away) is
    /// [`Answer::Cancelled`].
    pub async fn ask(
        &self,
        title: impl Into<String>,
        text: impl Into<String>,
        kind: QuestionKind,
        warning: bool,
    ) -> Answer {
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut inner = self.lock();
            inner.next += 1;
            let id = QuestionId(inner.next);
            inner.waiting.insert(id, tx);
            inner.fresh.push(Question {
                id,
                title: title.into(),
                text: text.into(),
                kind,
                warning,
            });
            id
        };
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = waker {
            wake();
        }
        debug!(%id, "asking the user");
        rx.await.unwrap_or(Answer::Cancelled)
    }

    /// Asks a choice: the index of the option picked, or `None` when the
    /// dialogue was dismissed.
    pub async fn choose(
        &self,
        title: impl Into<String>,
        text: impl Into<String>,
        options: Vec<String>,
        default: usize,
        warning: bool,
    ) -> Option<usize> {
        self.ask(
            title,
            text,
            QuestionKind::Choice { options, default },
            warning,
        )
        .await
        .choice()
    }

    /// Asks for a secret: what was typed and whether to remember it, or
    /// `None` when the dialogue was dismissed.
    pub async fn secret(
        &self,
        title: impl Into<String>,
        text: impl Into<String>,
        remember: Option<bool>,
    ) -> Option<(String, bool)> {
        match self
            .ask(title, text, QuestionKind::Secret { remember }, false)
            .await
        {
            Answer::Secret { secret, remember } => Some((secret, remember)),
            _ => None,
        }
    }

    /// Answers every question with `answer_with` as soon as it is asked:
    /// what a test, or a headless host, installs as the waker. The answers
    /// are given from the waker, so an asker that awaits sees its answer
    /// at once.
    pub fn answer_with(
        self: &Arc<Self>,
        answer_with: impl Fn(&Question) -> Answer + Send + Sync + 'static,
    ) {
        let weak = Arc::downgrade(self);
        self.set_waker(Arc::new(move || {
            if let Some(prompts) = weak.upgrade() {
                for question in prompts.take_new() {
                    let answer = answer_with(&question);
                    prompts.answer(question.id, answer);
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_waits_for_its_answer_and_a_dropped_one_is_cancelled() {
        let prompts = Prompts::new();
        crate::block_on(async {
            let asker = Arc::clone(&prompts);
            let asking = tokio::spawn(async move {
                asker
                    .choose(
                        "Trust?",
                        "fingerprint",
                        vec!["Yes".into(), "No".into()],
                        1,
                        false,
                    )
                    .await
            });
            let question = loop {
                let fresh = prompts.take_new();
                if let Some(q) = fresh.into_iter().next() {
                    break q;
                }
                tokio::task::yield_now().await;
            };
            assert_eq!(question.title, "Trust?");
            assert!(matches!(
                question.kind,
                QuestionKind::Choice { default: 1, .. }
            ));
            assert!(prompts.is_waiting(question.id));
            assert!(prompts.answer(question.id, Answer::Choice(0)));
            assert!(!prompts.answer(question.id, Answer::Choice(0)));
            assert_eq!(asking.await.ok(), Some(Some(0)));

            // Nobody answers: the asker is told so once the question is
            // dropped.
            let asker = Arc::clone(&prompts);
            let asking =
                tokio::spawn(async move { asker.secret("Password", "for x", Some(false)).await });
            let question = loop {
                if let Some(q) = prompts.take_new().into_iter().next() {
                    break q;
                }
                tokio::task::yield_now().await;
            };
            prompts.lock().waiting.remove(&question.id);
            assert_eq!(asking.await.ok(), Some(None));
        });
    }

    #[test]
    fn a_scripted_answerer_answers_from_the_waker() {
        let prompts = Prompts::new();
        prompts.answer_with(|q| match &q.kind {
            QuestionKind::Secret { .. } => Answer::Secret {
                secret: "hunter2".into(),
                remember: true,
            },
            QuestionKind::Choice { .. } => Answer::Cancelled,
        });
        crate::block_on(async {
            assert_eq!(
                prompts.secret("Password", "", Some(false)).await,
                Some(("hunter2".to_owned(), true))
            );
            assert_eq!(
                prompts
                    .choose("Trust?", "", vec!["Yes".into()], 0, true)
                    .await,
                None
            );
        });
        assert_eq!(prompts.take_new(), Vec::<types::Question>::new());
        assert!(format!("{prompts:?}").contains("waiting: 0"));
    }
}
