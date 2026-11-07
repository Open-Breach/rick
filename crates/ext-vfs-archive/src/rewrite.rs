//! The plan of a rewrite: what a write, a rename or a removal does to the
//! entries of an archive, applied by each format's writer.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::index::is_inside;

/// One change to an archive's entries.
#[derive(Debug, Clone)]
pub enum Edit {
    /// Drop the entry at this path and, for a folder, everything below it.
    Remove(Vec<u8>),
    /// Move the entry, and everything below it, from one path to another.
    Rename { from: Vec<u8>, to: Vec<u8> },
    /// Add a file, or replace the one at this path, from a file on disk.
    AddFile {
        path: Vec<u8>,
        source: PathBuf,
        modified: SystemTime,
    },
    /// Add a folder entry.
    AddDir { path: Vec<u8> },
}

/// The edits of one rewrite, in order.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// The edits.
    pub edits: Vec<Edit>,
}

impl Plan {
    /// A plan of one edit.
    #[must_use]
    pub fn one(edit: Edit) -> Self {
        Self { edits: vec![edit] }
    }

    /// Where an existing entry goes: its path, its renamed path, or `None`
    /// when it is removed or replaced by an addition.
    #[must_use]
    pub fn new_path(&self, path: &[u8]) -> Option<Vec<u8>> {
        let mut current = path.to_vec();
        for edit in &self.edits {
            match edit {
                Edit::Remove(prefix) if is_inside(&current, prefix) => return None,
                Edit::Rename { from, to } if is_inside(&current, from) => {
                    let rest = current[from.len()..].to_vec();
                    current = [to.as_slice(), rest.as_slice()].concat();
                }
                Edit::AddFile { path, .. } | Edit::AddDir { path } if *path == current => {
                    return None;
                }
                _ => {}
            }
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_maps_old_paths_to_new_ones() {
        let plan = Plan {
            edits: vec![
                Edit::Remove(b"gone".to_vec()),
                Edit::Rename {
                    from: b"a".to_vec(),
                    to: b"b".to_vec(),
                },
                Edit::AddFile {
                    path: b"keep/new.txt".to_vec(),
                    source: PathBuf::from("/tmp/x"),
                    modified: SystemTime::UNIX_EPOCH,
                },
            ],
        };
        assert_eq!(plan.new_path(b"gone/x"), None);
        assert_eq!(plan.new_path(b"gone"), None);
        assert_eq!(plan.new_path(b"a/x/y"), Some(b"b/x/y".to_vec()));
        assert_eq!(plan.new_path(b"a"), Some(b"b".to_vec()));
        assert_eq!(plan.new_path(b"ab"), Some(b"ab".to_vec()));
        assert_eq!(plan.new_path(b"keep/new.txt"), None, "replaced");
        assert_eq!(
            plan.new_path(b"keep/old.txt"),
            Some(b"keep/old.txt".to_vec())
        );
    }
}
