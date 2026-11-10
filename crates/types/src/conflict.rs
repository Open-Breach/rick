//! A name conflict a copy or move job met, and the answer it waits for.
//!
//! The job pauses at the first existing name and asks once; the answer may
//! hold for every remaining conflict of that job. These values cross from
//! the operations to the application, the model and the UI, which is why
//! they live here.

use std::fmt;
use std::time::SystemTime;

use crate::{JobId, Location};

/// Identity of one conflict, unique within a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConflictId(pub u64);

impl fmt::Display for ConflictId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conflict {}", self.0)
    }
}

/// What the dialogue shows of one side of a conflict, the existing entry
/// or the one that arrives, so the two can be compared before choosing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictSide {
    /// Whether it is a folder.
    pub is_dir: bool,
    /// Its size in bytes, when it is known.
    pub size: Option<u64>,
    /// When it was last modified, when that is known.
    pub modified: Option<SystemTime>,
}

/// What a job knows about a conflict when it asks: the two sides, the
/// name *Keep both* would give, and where the conflict stands among the
/// job's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictDetails {
    /// The entry that is already there.
    pub existing: ConflictSide,
    /// The entry that would land on it.
    pub incoming: ConflictSide,
    /// The first free numbered name beside the existing entry, `name
    /// (2).ext` or the next; `None` proposes `name (2).ext` unchecked.
    pub proposed: Option<String>,
    /// This conflict's number and the number of conflicts in all, `(3,
    /// 12)`, when the job counted them before it began.
    pub position: Option<(u64, u64)>,
    /// How many conflicts are still to come after this one; where the job
    /// did not count them, how many items are.
    pub remaining: u64,
}

/// An existing name a job would land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// This conflict.
    pub id: ConflictId,
    /// The job that waits for the answer.
    pub job: JobId,
    /// What is being copied or moved.
    pub source: Location,
    /// The existing entry it would replace.
    pub target: Location,
    /// The existing entry: a folder (overwrite merges) or a file
    /// (overwrite replaces), its size and its date.
    pub existing: ConflictSide,
    /// The entry that arrives.
    pub incoming: ConflictSide,
    /// The name *Keep both* gives the arriving entry: `name (2).ext`, or
    /// the next free number.
    pub proposed: String,
    /// This conflict's number and the job's number of conflicts, when the
    /// job counted them.
    pub position: Option<(u64, u64)>,
    /// How many conflicts of the job are still to come after this one;
    /// how many items, where the job did not count its conflicts.
    pub remaining: u64,
}

impl Conflict {
    /// Whether a folder meets a folder: the two merge, and there is no
    /// keeping both.
    #[must_use]
    pub fn between_folders(&self) -> bool {
        self.existing.is_dir && self.incoming.is_dir
    }
}

/// What to do with a conflicting item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Leave the existing entry; the item is not copied or moved.
    Skip,
    /// Replace a file, merge into a folder.
    Overwrite,
    /// Put the item under this name instead: *Keep both*, with the
    /// conflict's proposed name.
    Rename(String),
    /// End the job with `Error::Cancelled`.
    Cancel,
}

/// The dialogue's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAnswer {
    /// What to do with this item.
    pub resolution: Resolution,
    /// Apply the same to every remaining conflict of the job. A `Rename`
    /// applied to all numbers each remaining name the same way, and does
    /// not answer for two folders, which merge or are skipped.
    pub apply_to_all: bool,
}

impl ConflictAnswer {
    /// An answer for this one item.
    #[must_use]
    pub fn once(resolution: Resolution) -> Self {
        Self {
            resolution,
            apply_to_all: false,
        }
    }

    /// An answer for every remaining conflict of the job.
    #[must_use]
    pub fn for_all(resolution: Resolution) -> Self {
        Self {
            resolution,
            apply_to_all: true,
        }
    }
}

/// `name (2)`, `name (3)`, … with the extension kept after the number:
/// `report (2).pdf`. `n` is the number; `1` gives the name itself.
#[must_use]
pub fn numbered_name(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.to_owned();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered_name("a.txt", 2), "a (2).txt");
        assert_eq!(numbered_name("a.txt", 1), "a.txt");
        assert_eq!(numbered_name("Makefile", 3), "Makefile (3)");
        assert_eq!(numbered_name(".hidden", 2), ".hidden (2)");
        assert_eq!(numbered_name("x.tar.gz", 2), "x.tar (2).gz");
    }

    #[test]
    fn answers_say_whether_they_hold_for_the_rest() {
        assert!(!ConflictAnswer::once(Resolution::Skip).apply_to_all);
        assert!(ConflictAnswer::for_all(Resolution::Overwrite).apply_to_all);
        assert_eq!(ConflictId(3).to_string(), "conflict 3");
    }
}
