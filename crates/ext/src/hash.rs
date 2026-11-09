//! The result slot of Compute hash: the digests of the selected files,
//! filled one file at a time by the job that reads them.
//!
//! The action creates a [`Hashes`] with the files' names and no results,
//! spawns the job that fills it and returns
//! [`ActionOutcome::Hashes`](crate::ActionOutcome::Hashes); the application
//! opens the dialogue at once and the UI polls [`Hashes::result`] each
//! frame, following the job's progress by [`Hashes::job`]. Digests are
//! kept as bytes: the display format (hex, upper-case hex, Base64) is
//! applied when a value is drawn, so switching it never recomputes.
//!
//! # Worked example
//!
//! ```
//! use ext::{Algorithm, Digests, Hashes};
//! use types::JobId;
//!
//! let hashes = Hashes::pending(vec!["a.txt".to_owned(), "b.txt".to_owned()]);
//! hashes.set_job(JobId(7));
//! assert_eq!(hashes.job(), Some(JobId(7)));
//! assert!(hashes.result(0).is_none());
//! hashes.finish(0, Ok(Digests::default()));
//! let digests = hashes.result(0).and_then(Result::ok).unwrap();
//! assert_eq!(digests.get(Algorithm::Sha256).len(), 32);
//! assert!(!hashes.is_complete(), "b.txt has no result yet");
//! ```

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use types::{Error, JobId};

/// One of the four digests Compute hash computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Algorithm {
    /// MD5, 16 bytes.
    Md5,
    /// SHA-1, 20 bytes.
    Sha1,
    /// SHA-256, 32 bytes.
    Sha256,
    /// BLAKE3, 32 bytes.
    Blake3,
}

impl Algorithm {
    /// The four, in the order the dialogue lists them.
    pub const ALL: [Self; 4] = [Self::Md5, Self::Sha1, Self::Sha256, Self::Blake3];

    /// The algorithm's name as written: `MD5`, `SHA-1`, `SHA-256`, `BLAKE3`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Blake3 => "BLAKE3",
        }
    }

    /// The digest's length in bytes.
    #[must_use]
    pub const fn digest_len(self) -> usize {
        match self {
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha256 | Self::Blake3 => 32,
        }
    }
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The four digests of one file, as bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Digests {
    /// MD5.
    pub md5: [u8; 16],
    /// SHA-1.
    pub sha1: [u8; 20],
    /// SHA-256.
    pub sha256: [u8; 32],
    /// BLAKE3.
    pub blake3: [u8; 32],
}

impl Digests {
    /// One digest's bytes.
    #[must_use]
    pub fn get(&self, algorithm: Algorithm) -> &[u8] {
        match algorithm {
            Algorithm::Md5 => &self.md5,
            Algorithm::Sha1 => &self.sha1,
            Algorithm::Sha256 => &self.sha256,
            Algorithm::Blake3 => &self.blake3,
        }
    }
}

/// The digests of a selection: opened empty, filled file by file by the
/// job that reads them.
pub struct Hashes {
    names: Vec<String>,
    sizes: Vec<Option<u64>>,
    job: OnceLock<JobId>,
    results: Vec<Mutex<Option<Result<Digests, String>>>>,
}

impl fmt::Debug for Hashes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hashes")
            .field("names", &self.names)
            .field("job", &self.job.get())
            .field("complete", &self.is_complete())
            .finish_non_exhaustive()
    }
}

impl Hashes {
    /// A slot per file name, none with a result yet.
    #[must_use]
    pub fn pending(names: Vec<String>) -> Arc<Self> {
        Self::pending_sized(names.into_iter().map(|name| (name, None)).collect())
    }

    /// A slot per file name with the file's size where the listing knows
    /// it, for the dialogue's header.
    #[must_use]
    pub fn pending_sized(files: Vec<(String, Option<u64>)>) -> Arc<Self> {
        let results = files.iter().map(|_| Mutex::new(None)).collect();
        let (names, sizes) = files.into_iter().unzip();
        Arc::new(Self {
            names,
            sizes,
            job: OnceLock::new(),
            results,
        })
    }

    /// File `index`'s size, when the listing knew it.
    #[must_use]
    pub fn size(&self, index: usize) -> Option<u64> {
        self.sizes.get(index).copied().flatten()
    }

    /// The files' sizes summed, when every one is known.
    #[must_use]
    pub fn total_size(&self) -> Option<u64> {
        self.sizes.iter().copied().sum()
    }

    /// The files' names, in the selection's order; one block each.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Records the job that fills the slot, once; a second call changes
    /// nothing.
    pub fn set_job(&self, id: JobId) {
        let _ = self.job.set(id);
    }

    /// The job that fills the slot, once the action spawned it; the UI
    /// follows its progress by this id and cancels it through it.
    #[must_use]
    pub fn job(&self) -> Option<JobId> {
        self.job.get().copied()
    }

    /// Stores file `index`'s digests, or why there are none; the UI shows
    /// it on its next frame. An index past the names changes nothing.
    pub fn finish(&self, index: usize, result: Result<Digests, Error>) {
        if let Some(slot) = self.results.get(index) {
            *slot.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(result.map_err(|e| e.to_string()));
        }
    }

    /// File `index`'s digests, or the error text, once the job got there.
    #[must_use]
    pub fn result(&self, index: usize) -> Option<Result<Digests, String>> {
        self.results
            .get(index)?
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether every file has its digests or its error.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.results.iter().all(|slot| {
            slot.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_some()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_fill_independently_and_keep_the_first_job() {
        let hashes = Hashes::pending(vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(hashes.names(), ["a", "b"]);
        assert_eq!(hashes.job(), None);
        hashes.set_job(JobId(1));
        hashes.set_job(JobId(2));
        assert_eq!(hashes.job(), Some(JobId(1)));
        assert!(!hashes.is_complete());
        hashes.finish(1, Err(Error::NotFound));
        assert_eq!(hashes.result(1), Some(Err("not found".to_owned())));
        assert!(hashes.result(0).is_none());
        assert!(!hashes.is_complete());
        hashes.finish(0, Ok(Digests::default()));
        assert!(hashes.is_complete());
        // Out of range: ignored, not a panic.
        hashes.finish(5, Ok(Digests::default()));
        assert!(hashes.result(5).is_none());
    }

    #[test]
    fn algorithms_have_their_lengths_and_names() {
        for algorithm in Algorithm::ALL {
            assert_eq!(
                Digests::default().get(algorithm).len(),
                algorithm.digest_len()
            );
        }
        assert_eq!(Algorithm::Sha1.to_string(), "SHA-1");
    }
}
