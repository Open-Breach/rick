//! The download of a plugin package.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::Error;
use crate::fetch::{client, failed};

/// What a package must come out as: the size and the SHA-256 the index
/// names for its build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected<'a> {
    /// The package's size in bytes.
    pub size: u64,
    /// Its SHA-256 in hexadecimal, in either case.
    pub sha256: &'a str,
}

/// A package as it arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256 in lower-case hexadecimal.
    pub sha256: String,
}

/// The file a download writes, deleted again unless the download is kept:
/// on an error, on a package that is another one, and when the download
/// is dropped half way, as a cancelled job's is.
struct Partial {
    path: PathBuf,
    keep: bool,
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Downloads `url` to the file `to`, as a Rick of version `rick`, calling
/// `progress` with the bytes written so far. With `expected` the package
/// is held against it: the download stops as soon as more bytes arrive
/// than the index names, and a package of another size or another hash
/// is deleted. Without it nothing is checked, and what arrived is
/// reported. No file is left behind unless the download succeeds.
///
/// # Errors
///
/// [`Error::Fetch`] when the package does not arrive, [`Error::Mismatch`]
/// when it is not what `expected` describes, and the I/O error of the
/// file.
pub async fn download(
    url: &str,
    to: &Path,
    rick: &str,
    expected: Option<Expected<'_>>,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<Downloaded, Error> {
    let io = |source: std::io::Error| Error::Io {
        path: to.to_owned(),
        source,
    };
    let client = client(rick, None)?;
    let mut response = client.get(url).send().await.map_err(failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Fetch(format!("{url}: HTTP {}", status.as_u16())));
    }
    let mut file = tokio::fs::File::create(to).await.map_err(io)?;
    let mut partial = Partial {
        path: to.to_owned(),
        keep: false,
    };
    let mut hash = Sha256::new();
    let mut size: u64 = 0;
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        size += chunk.len() as u64;
        if expected.is_some_and(|expected| size > expected.size) {
            return Err(Error::Mismatch);
        }
        hash.update(&chunk);
        file.write_all(&chunk).await.map_err(io)?;
        progress(size);
    }
    file.flush().await.map_err(io)?;
    drop(file);
    let sha256 = hex::encode(hash.finalize());
    if let Some(expected) = expected
        && (size != expected.size || !sha256.eq_ignore_ascii_case(expected.sha256))
    {
        return Err(Error::Mismatch);
    }
    partial.keep = true;
    Ok(Downloaded { size, sha256 })
}
