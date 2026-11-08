//! Name conflicts: where a copy or move that would land on an existing
//! name decides what to do — Skip, Overwrite, Rename, Cancel, apply to all
//! remaining — through the dialogue the [`Conflicts`] queue of
//! `runtime` serves.
//!
//! Jobs ask through the queue their [`JobCtx`] carries; the application
//! drains the new questions into the model, the UI shows the dialogue for
//! the first, and the answer comes back through [`Conflicts::answer`].
//! [`Policy`] remembers an "apply to all" answer for the rest of one job.

use runtime::JobCtx;
pub use runtime::{ConflictWaker, Conflicts, name_of};
use types::{Error, Location, Resolution};

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
