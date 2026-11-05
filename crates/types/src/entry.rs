//! Directory entries and their attributes.

use std::fmt;
use std::time::SystemTime;

use crate::{EntryId, VPath};

bitflags::bitflags! {
    /// Attribute flags of an entry, as far as the provider knows them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Attrs: u32 {
        /// Hidden by platform convention (dot-file, Windows hidden attribute).
        const HIDDEN = 1;
        /// Marked as a system entry by the platform.
        const SYSTEM = 2;
        /// Not writable by the current user.
        const READONLY = 4;
        /// Executable by the current user.
        const EXECUTABLE = 8;
    }
}

/// A media type such as `text/plain` or `image/png`.
///
/// Stored as the type/subtype essence without parameters; filled lazily by
/// sniffing, so most entries carry `None` until something asks.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mime(Box<str>);

impl Mime {
    /// Wraps a media type string. The value is not validated.
    #[must_use]
    pub fn new(essence: impl Into<Box<str>>) -> Self {
        Self(essence.into())
    }

    /// The full `type/subtype` string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the slash, e.g. `image`.
    #[must_use]
    pub fn type_(&self) -> &str {
        self.0.split_once('/').map_or(&*self.0, |(t, _)| t)
    }

    /// The part after the slash, e.g. `png`; empty when there is no slash.
    #[must_use]
    pub fn subtype(&self) -> &str {
        self.0.split_once('/').map_or("", |(_, s)| s)
    }
}

impl fmt::Display for Mime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Mime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mime({:?})", &*self.0)
    }
}

impl From<&str> for Mime {
    fn from(essence: &str) -> Self {
        Self::new(essence)
    }
}

/// What kind of thing an entry is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link.
    Symlink {
        /// Where the link points, as the provider reports it.
        target: VPath,
        /// `true` when the target does not exist.
        broken: bool,
    },
    /// Anything else: device, socket, FIFO, reparse point.
    Other,
}

/// One entry of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Stable identity, independent of the path.
    pub id: EntryId,
    /// Display name; the raw bytes live in `path`.
    pub name: String,
    /// Full path inside the provider's tree.
    pub path: VPath,
    /// File, directory, link or other.
    pub kind: EntryKind,
    /// Size in bytes, when the provider knows it.
    pub size: Option<u64>,
    /// Creation time, when the provider knows it.
    pub created: Option<SystemTime>,
    /// Last modification time, when the provider knows it.
    pub modified: Option<SystemTime>,
    /// Last access time, when the provider knows it.
    pub accessed: Option<SystemTime>,
    /// Attribute flags.
    pub attrs: Attrs,
    /// Media type, filled lazily by sniffing.
    pub mime: Option<Mime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_combine_as_bitflags() {
        let attrs = Attrs::HIDDEN | Attrs::READONLY;
        assert!(attrs.contains(Attrs::HIDDEN));
        assert!(!attrs.contains(Attrs::EXECUTABLE));
        assert_eq!(attrs.bits(), 5);
        assert_eq!(Attrs::default(), Attrs::empty());
    }

    #[test]
    fn mime_splits_type_and_subtype() {
        let mime = Mime::from("image/png");
        assert_eq!(mime.type_(), "image");
        assert_eq!(mime.subtype(), "png");
        assert_eq!(mime.to_string(), "image/png");
        assert_eq!(format!("{mime:?}"), "Mime(\"image/png\")");
        let bare = Mime::new("text");
        assert_eq!(bare.type_(), "text");
        assert_eq!(bare.subtype(), "");
    }
}
