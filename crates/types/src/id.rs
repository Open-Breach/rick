//! Identities: providers, extensions, entries and jobs.

use std::fmt;
use std::sync::{Mutex, PoisonError};

/// Which extension provides a tree: `"local"`, `"archive"`, `"sftp"`, `"s3"`,
/// `"trash"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderId(pub &'static str);

impl ProviderId {
    /// The local filesystem, the provider every OS path belongs to.
    pub const LOCAL: Self = Self("local");
    /// Archives browsed in place: a location whose path is the OS path of
    /// the archive file followed by the path inside it.
    pub const ARCHIVE: Self = Self("archive");
    /// Rick's own virtual locations, `rick://trash`, `rick://favorites` and
    /// the like: one provider whose first-level directories are the views
    /// extensions register (`ext::Registry::view`).
    pub const RICK: Self = Self("rick");

    /// The id for a provider name read at run time, from a settings file
    /// say. The name is interned: the first call for a name leaks one copy
    /// of it, every later call returns that copy, so the set of leaked
    /// strings is the set of distinct provider names ever read.
    #[must_use]
    pub fn intern(name: &str) -> Self {
        static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
        let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = names.iter().find(|n| **n == name) {
            return Self(existing);
        }
        let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
        names.push(leaked);
        Self(leaked)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Identity of an extension, as declared in its manifest: `"git"`,
/// `"preview-pdf"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExtId(pub &'static str);

impl fmt::Display for ExtId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Stable identity of an entry inside one provider.
///
/// The local provider packs `(device, inode)` into `key`; other providers
/// choose their own 128-bit key. An `EntryId` is never derived from a path
/// string, so renames and moves keep selection, history and decorations
/// intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId {
    /// The provider that owns the entry.
    pub provider: ProviderId,
    /// Provider-scoped key, unique within `provider`.
    pub key: u128,
}

impl EntryId {
    /// Builds an id from a provider and its 128-bit key.
    #[must_use]
    pub const fn new(provider: ProviderId, key: u128) -> Self {
        Self { provider, key }
    }

    /// Builds the local-filesystem key from a device and inode number.
    #[must_use]
    pub const fn from_device_inode(provider: ProviderId, device: u64, inode: u64) -> Self {
        Self {
            provider,
            key: ((device as u128) << 64) | inode as u128,
        }
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:032x}", self.provider, self.key)
    }
}

/// Identity of a running or finished job, unique within one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_inode_round_trips_through_key() {
        let id = EntryId::from_device_inode(ProviderId("local"), 0xDEAD, 0xBEEF);
        assert_eq!(id.key >> 64, 0xDEAD);
        assert_eq!(id.key & u128::from(u64::MAX), 0xBEEF);
        assert_eq!(id.to_string(), "local:000000000000dead000000000000beef");
    }

    #[test]
    fn interned_names_compare_equal_to_literals() {
        assert_eq!(ProviderId::intern("local"), ProviderId("local"));
        let a = ProviderId::intern("made-up");
        let b = ProviderId::intern("made-up");
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.0, b.0), "one copy per name");
        assert_ne!(a, ProviderId("local"));
    }

    #[test]
    fn ids_display_their_name() {
        assert_eq!(ProviderId("sftp").to_string(), "sftp");
        assert_eq!(ProviderId::LOCAL, ProviderId("local"));
        assert_eq!(ProviderId::ARCHIVE.to_string(), "archive");
        assert_eq!(ProviderId::RICK.to_string(), "rick");
        assert_eq!(ExtId("git").to_string(), "git");
        assert_eq!(JobId(7).to_string(), "job#7");
    }
}
