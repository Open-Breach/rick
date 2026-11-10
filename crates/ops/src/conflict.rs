//! Name conflicts: where a copy or move that would land on an existing
//! name decides what to do — Skip, Replace (Merge between two folders),
//! Keep both, Cancel, the same for the remaining — through the dialogue
//! the [`Conflicts`] queue of `runtime` serves (SPEC.md *Execution*).
//!
//! Jobs ask through the queue their [`JobCtx`] carries; the application
//! drains the new questions into the model, the UI shows the dialogue for
//! the first, and the answer comes back through [`Conflicts::answer`].
//! [`Policy`] remembers a "do the same for the remaining" answer for the
//! rest of one job, and counts the job's conflicts so the dialogue can say
//! "3 of 12".

use runtime::JobCtx;
pub use runtime::{ConflictWaker, Conflicts, name_of};
use types::{ConflictDetails, ConflictSide, Entry, EntryKind, Error, Location, Resolution};

/// What one job remembers from "do the same for the remaining", and where
/// it stands among its conflicts.
#[derive(Debug, Default)]
pub(crate) struct Policy {
    all: Option<Resolution>,
    /// The conflicts counted before the job began.
    total: u64,
    /// The conflicts met so far, asked about or answered by `all`.
    met: u64,
}

/// How many of `pairs` would land on an existing name: what the job
/// compares with the destination before it brings anything, so the first
/// question already says how many there are. A target that cannot be
/// examined is not counted; the job meets its error when it gets there.
pub(crate) async fn count(mounts: &vfs::Mounts, pairs: &[(Location, Location)]) -> u64 {
    let mut conflicts = 0;
    for (_, to) in pairs {
        let Ok(provider) = mounts.resolve(to) else {
            continue;
        };
        if provider.stat(to).await.is_ok() {
            conflicts += 1;
        }
    }
    conflicts
}

/// One side of a conflict as the dialogue shows it.
fn side(entry: &Entry) -> ConflictSide {
    let is_dir = entry.kind == EntryKind::Dir;
    ConflictSide {
        is_dir,
        // A folder's size is not known without counting it.
        size: if is_dir { None } else { entry.size },
        modified: entry.modified,
    }
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
    /// A policy for a job with `total` conflicts, as [`count`] found them.
    pub(crate) fn counted(total: u64) -> Self {
        Self {
            total,
            ..Self::default()
        }
    }

    /// Decides where `from` lands when going to `to`, asking through
    /// `conflicts` when `to` is taken and no standing answer applies. The
    /// question carries both sides, the first free numbered name for
    /// *Keep both*, and the conflict's place among the job's. A standing
    /// *Keep both* does not answer for two folders: they are asked about.
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
    ) -> Result<Landing, Error> {
        let provider = mounts.resolve(to)?;
        let dir = Location::new(to.provider, crate::edit::parent(&to.path));
        let mut target = to.clone();
        let mut counted = false;
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
            if !counted {
                counted = true;
                self.met += 1;
                // A name taken since the count still has its number.
                self.total = self.total.max(self.met);
            }
            let incoming = match mounts.resolve(from) {
                Ok(source) => source.stat(from).await.map(|e| side(&e)).ok(),
                Err(_) => None,
            }
            .unwrap_or_default();
            let existing = side(&existing);
            let folders = existing.is_dir && incoming.is_dir;
            let standing = self
                .all
                .clone()
                .filter(|all| !(folders && matches!(all, Resolution::Rename(_))));
            let held = standing.is_some();
            let resolution = if let Some(standing) = standing {
                standing
            } else {
                let details = ConflictDetails {
                    existing,
                    incoming,
                    proposed: Some(free_name(&*provider, &dir, &name_of(to)).await),
                    position: Some((self.met, self.total)),
                    remaining: self.total - self.met,
                };
                let answer = conflicts.ask(job, from, &target, details).await;
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
                    let applies_to_all = held || self.all.is_some();
                    let proposed = if applies_to_all {
                        // For all: number each remaining name itself.
                        free_name(&*provider, &dir, &name_of(to)).await
                    } else {
                        name
                    };
                    target = crate::inside(&dir, proposed.as_bytes());
                    // A name given for this one item that is taken too
                    // asks again.
                    if applies_to_all {
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
