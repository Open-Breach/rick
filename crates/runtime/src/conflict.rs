//! Name conflicts: a job that would land on an existing name asks once —
//! Skip, Overwrite, Rename, Cancel, apply to all remaining — and pauses
//! until the answer comes.
//!
//! Jobs ask through the [`Conflicts`] queue their [`JobCtx`] carries, which
//! the application owns through its [`JobRunner`](crate::JobRunner): it
//! drains the new questions into the model, the UI shows the dialogue for
//! the first, and the answer comes back through [`Conflicts::answer`]. A job
//! whose question is never answered ends when its token is cancelled, as
//! any job. A plugin's job (an extraction, say) asks the same way as the
//! kernel's copies, so one dialogue serves every operation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;
use tracing::debug;
use types::{Conflict, ConflictAnswer, ConflictId, Location, Progress, Resolution};

use crate::JobCtx;

/// Called whenever a job asks, so the application can wake its loop.
pub type ConflictWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Inner {
    next: u64,
    /// Asked and not yet taken by the application.
    fresh: Vec<Conflict>,
    /// Asked and not yet answered, by id.
    waiting: HashMap<ConflictId, oneshot::Sender<ConflictAnswer>>,
}

/// Where jobs ask about name conflicts and the application answers.
#[derive(Default)]
pub struct Conflicts {
    inner: Mutex<Inner>,
    waker: Mutex<Option<ConflictWaker>>,
}

impl std::fmt::Debug for Conflicts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Conflicts")
            .field("fresh", &inner.fresh.len())
            .field("waiting", &inner.waiting.len())
            .finish()
    }
}

impl Conflicts {
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
    pub fn set_waker(&self, waker: ConflictWaker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// The questions asked since the last call.
    pub fn take_new(&self) -> Vec<Conflict> {
        std::mem::take(&mut self.lock().fresh)
    }

    /// Answers a question. Returns `false` when no job waits on that id.
    pub fn answer(&self, id: ConflictId, answer: ConflictAnswer) -> bool {
        let sender = self.lock().waiting.remove(&id);
        match sender {
            Some(sender) => sender.send(answer).is_ok(),
            None => false,
        }
    }

    /// Whether a job waits on an answer right now.
    #[must_use]
    pub fn is_waiting(&self, id: ConflictId) -> bool {
        self.lock().waiting.contains_key(&id)
    }

    /// Asks about `target` on behalf of `job` and waits for the answer:
    /// `source` is what would land there, `target_is_dir` what is there
    /// now, `remaining` how many items follow this one (for "apply to
    /// all"). The job's progress says it is waiting meanwhile. A question
    /// dropped unanswered (the application went away) is a `Cancel`.
    pub async fn ask(
        &self,
        job: &JobCtx,
        source: &Location,
        target: &Location,
        target_is_dir: bool,
        remaining: u64,
    ) -> ConflictAnswer {
        let name = name_of(target);
        let (tx, rx) = oneshot::channel();
        let conflict = {
            let mut inner = self.lock();
            inner.next += 1;
            let id = ConflictId(inner.next);
            inner.waiting.insert(id, tx);
            let conflict = Conflict {
                id,
                job: job.id,
                source: source.clone(),
                target: target.clone(),
                target_is_dir,
                proposed: types::numbered_name(&name, 2),
                remaining,
            };
            inner.fresh.push(conflict.clone());
            conflict
        };
        job.progress
            .report(Progress::Message(format!("{name} exists; waiting")));
        let waker = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(wake) = waker {
            wake();
        }
        debug!(%conflict.id, %target, "asking about a conflict");
        rx.await.unwrap_or(ConflictAnswer::once(Resolution::Cancel))
    }
}

/// The last component of a location, lossy.
#[must_use]
pub fn name_of(loc: &Location) -> String {
    loc.path.file_name().map_or_else(
        || loc.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}
