//! The cache: the last index that verified, and its signature.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::{Error, Fetched, Index, Key, Source, fetch};

/// The folder in Rick's cache directory that holds the two files.
pub const CACHE_DIR: &str = "registry";
/// The cached index.
pub const INDEX_FILE: &str = "index.json";
/// The cached signature of [`INDEX_FILE`].
pub const SIGNATURE_FILE: &str = "index.json.minisig";

/// How long a cached index is good for before the check in the
/// background asks again: a day.
pub const CHECK_EVERY: Duration = Duration::from_hours(24);

/// The cache of one Rick: `registry/` in its cache directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    dir: PathBuf,
}

/// An index read from the cache, or just written to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    /// The index.
    pub index: Index,
    /// When it was fetched: the time the cached file was written.
    pub fetched: SystemTime,
}

impl Cache {
    /// The cache in `cache_dir`, Rick's cache directory. Nothing is
    /// created before the first write.
    #[must_use]
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            dir: cache_dir.join(CACHE_DIR),
        }
    }

    /// The folder of the two files.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// When the cached index was fetched; `None` with nothing cached.
    #[must_use]
    pub fn fetched(&self) -> Option<SystemTime> {
        fs::metadata(self.dir.join(INDEX_FILE))
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// Reads the cached index and verifies it with `key`, as every read
    /// does: what is on the disk is trusted no further than what arrives
    /// from the network. `None` with nothing cached.
    ///
    /// # Errors
    ///
    /// [`Error::Unverified`] when the signature is missing or is not the
    /// key's signature of the cached bytes; [`Error::NewerSchema`] and
    /// [`Error::Unreadable`] as [`Index::parse`] has them; the I/O error
    /// of a file that is there and cannot be read.
    pub fn read(&self, key: &Key) -> Result<Option<Cached>, Error> {
        let path = self.dir.join(INDEX_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(Error::Io { path, source }),
        };
        let fetched = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .map_err(|source| Error::Io { path, source })?;
        let signature =
            fs::read_to_string(self.dir.join(SIGNATURE_FILE)).map_err(|_| Error::Unverified)?;
        key.verify(&bytes, &signature)?;
        let index = Index::parse(&bytes)?;
        Ok(Some(Cached { index, fetched }))
    }

    /// Keeps a fetched index and its signature in place of what was
    /// cached, each through a temporary file renamed over the old one, so
    /// that a reader never meets half a file.
    ///
    /// # Errors
    ///
    /// The I/O error of the first write that failed.
    pub fn write(&self, fetched: &Fetched) -> Result<(), Error> {
        fs::create_dir_all(&self.dir).map_err(|source| Error::Io {
            path: self.dir.clone(),
            source,
        })?;
        // The signature first: between the two renames the index on the
        // disk is the old one beside the new signature, which does not
        // verify and is not used.
        self.replace(SIGNATURE_FILE, fetched.signature.as_bytes())?;
        self.replace(INDEX_FILE, &fetched.bytes)
    }

    /// Writes `bytes` to the file `name` of the cache through a temporary
    /// file beside it.
    fn replace(&self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        let path = self.dir.join(name);
        let part = self.dir.join(format!("{name}.part"));
        fs::write(&part, bytes).map_err(|source| Error::Io {
            path: part.clone(),
            source,
        })?;
        fs::rename(&part, &path).map_err(|source| {
            let _ = fs::remove_file(&part);
            Error::Io { path, source }
        })
    }
}

/// Whether the check in the background is due at `now`: nothing is
/// cached, or what is cached was fetched a day ago or longer. `fetched`
/// is [`Cache::fetched`].
#[must_use]
pub fn due(fetched: Option<SystemTime>, now: SystemTime) -> bool {
    fetched.is_none_or(|fetched| {
        now.duration_since(fetched)
            // A file from the future says nothing about its age.
            .map_or(true, |age| age >= CHECK_EVERY)
    })
}

/// Fetches the index of `source` as a Rick of version `rick` and keeps it
/// in `cache`. An index that does not arrive, does not verify or cannot
/// be read changes nothing: the last good one stays cached.
///
/// # Errors
///
/// What [`fetch`] returns. An index that verified and could not be
/// written to the cache is returned all the same, and the failure logged:
/// it is good for this session.
pub async fn refresh(source: &Source, cache: &Cache, rick: &str) -> Result<Cached, Error> {
    let fetched = fetch(source, rick).await?;
    let written = match cache.write(&fetched) {
        Ok(()) => cache.fetched(),
        Err(error) => {
            tracing::warn!(%error, "the plugin list was not cached");
            None
        }
    };
    Ok(Cached {
        fetched: written.unwrap_or_else(SystemTime::now),
        index: fetched.index,
    })
}
