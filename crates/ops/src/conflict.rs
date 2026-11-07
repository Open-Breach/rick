//! Name conflicts: a copy or move that would land on an existing name asks
//! once — Skip, Overwrite, Rename, Cancel, apply to all remaining — and
//! pauses until the answer comes.
//!
//! Jobs ask through [`Conflicts`], which the application owns: it drains the
//! new questions into the model, the UI shows the dialogue for the first,
//! and the answer comes back through [`Conflicts::answer`]. A job whose
//! question is never answered ends when its token is cancelled, as any job.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use runtime::{JobCtx, oneshot};
use tracing::debug;
use types::{Conflict, ConflictAnswer, ConflictId, Error, Location, Progress, Resolution};

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

    /// Asks about `target` on behalf of a job and waits for the answer. A
    /// question dropped unanswered (the application went away) is a
    /// `Cancel`.
    pub(crate) async fn ask(
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
pub(crate) fn name_of(loc: &Location) -> String {
    loc.path.file_name().map_or_else(
        || loc.path.to_string(),
        |n| String::from_utf8_lossy(n).into_owned(),
    )
}

/// What one job remembers from "apply to all remaining".
#[derive(Debug, Default)]
pub(crate) struct Policy {
    all: Option<Resolution>,
}

/// Where an item goes after its conflict, if anywhere.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Landing {
    /// Put it at this location; `overwrite` says an existing entry may be
    /// replaced (file) or merged into (folder).
    At { to: Location, overwrite: bool },
    /// Leave it out.
    Skip,
}

impl Policy {
    /// Decides where `from` lands when going to `to`, asking through
    /// `conflicts` when `to` is taken and no standing answer applies.
    ///
    /// # Errors
    ///
    /// `Error::Cancelled` when the answer is Cancel; the provider's error
    /// when `to` cannot be examined.
    pub(crate) async fn land(
        &mut self,
        conflicts: &Conflicts,
        job: &JobCtx,
        mounts: &vfs::Mounts,
        from: &Location,
        to: &Location,
        remaining: u64,
    ) -> Result<Landing, Error> {
        let provider = mounts.resolve(to)?;
        let mut target = to.clone();
        loop {
            let existing = match provider.stat(&target).await {
                Ok(entry) => entry,
                Err(Error::NotFound) => {
                    return Ok(Landing::At {
                        to: target,
                        overwrite: false,
                    });
                }
                Err(e) => return Err(e),
            };
            let is_dir = existing.kind == types::EntryKind::Dir;
            let resolution = if let Some(standing) = &self.all {
                standing.clone()
            } else {
                let answer = conflicts.ask(job, from, &target, is_dir, remaining).await;
                if answer.apply_to_all {
                    self.all = Some(answer.resolution.clone());
                }
                answer.resolution
            };
            match resolution {
                Resolution::Skip => return Ok(Landing::Skip),
                Resolution::Cancel => return Err(Error::Cancelled),
                Resolution::Overwrite => {
                    return Ok(Landing::At {
                        to: target,
                        overwrite: true,
                    });
                }
                Resolution::Rename(name) => {
                    let dir = Location::new(to.provider, crate::edit::parent(&to.path));
                    let proposed = if self.all.is_some() {
                        // Applied to all: number each remaining name itself.
                        free_name(&*provider, &dir, &name_of(to)).await
                    } else {
                        name
                    };
                    target = crate::inside(&dir, proposed.as_bytes());
                    // An explicit new name that is taken too asks again.
                    if self.all.is_some() {
                        return Ok(Landing::At {
                            to: target,
                            overwrite: false,
                        });
                    }
                }
            }
        }
    }
}

/// `name (2)`, `name (3)`, … the first that does not exist in `dir`.
async fn free_name(provider: &dyn ext::Provider, dir: &Location, name: &str) -> String {
    for n in 2..10_000u32 {
        let candidate = types::numbered_name(name, n);
        if provider
            .stat(&crate::inside(dir, candidate.as_bytes()))
            .await
            .is_err()
        {
            return candidate;
        }
    }
    types::numbered_name(name, 10_000)
}
