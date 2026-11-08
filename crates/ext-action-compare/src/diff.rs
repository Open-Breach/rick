//! The comparisons: two texts line by line, two files byte for byte, two
//! folders entry by entry.

use std::io::Read;
use std::path::Path;

use ext::{Diff, FolderDiff, FolderRow, Hunk, HunkKind, RowStatus, Side, TextDiff};
use types::{CancelToken, Error, Location, ProviderId};

/// Files larger than this are compared byte for byte, not as text.
pub const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

/// Whether the first bytes of a file look like text: no NUL byte and, for
/// bytes that are not UTF-8, mostly printable ones.
fn looks_textual(sniff: &[u8]) -> bool {
    if sniff.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sniff) {
        Ok(_) => true,
        Err(error) if error.error_len().is_none() => true,
        Err(_) => {
            let printable = sniff
                .iter()
                .filter(|b| b.is_ascii_graphic() || b.is_ascii_whitespace() || **b >= 0xA0)
                .count();
            printable * 100 / sniff.len().max(1) >= 95
        }
    }
}

/// The lines of a text, without their line endings.
fn lines_of(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// The line diff of two texts: equal stretches and the differing ones,
/// covering every line of both sides.
#[must_use]
pub fn text_diff(left: &str, right: &str) -> TextDiff {
    let left = lines_of(left);
    let right = lines_of(right);
    let left_refs: Vec<&str> = left.iter().map(String::as_str).collect();
    let right_refs: Vec<&str> = right.iter().map(String::as_str).collect();
    let diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Patience)
        .diff_slices(&left_refs, &right_refs);
    let hunks = diff
        .ops()
        .iter()
        .map(|op| match *op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => Hunk {
                kind: HunkKind::Equal,
                left: old_index..old_index + len,
                right: new_index..new_index + len,
            },
            similar::DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => Hunk {
                kind: HunkKind::Delete,
                left: old_index..old_index + old_len,
                right: new_index..new_index,
            },
            similar::DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => Hunk {
                kind: HunkKind::Insert,
                left: old_index..old_index,
                right: new_index..new_index + new_len,
            },
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => Hunk {
                kind: HunkKind::Replace,
                left: old_index..old_index + old_len,
                right: new_index..new_index + new_len,
            },
        })
        .collect();
    TextDiff { left, right, hunks }
}

/// Whether two files hold the same bytes.
fn same_bytes(a: &Path, b: &Path, cancel: &CancelToken) -> Result<bool, Error> {
    let (ma, mb) = (
        std::fs::metadata(a).map_err(Error::from_io)?,
        std::fs::metadata(b).map_err(Error::from_io)?,
    );
    if ma.len() != mb.len() {
        return Ok(false);
    }
    let mut fa = std::io::BufReader::new(std::fs::File::open(a).map_err(Error::from_io)?);
    let mut fb = std::io::BufReader::new(std::fs::File::open(b).map_err(Error::from_io)?);
    let mut ba = vec![0u8; 1 << 16];
    let mut bb = vec![0u8; 1 << 16];
    loop {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let na = fa.read(&mut ba).map_err(Error::from_io)?;
        let mut filled = 0;
        while filled < na {
            let n = fb.read(&mut bb[filled..na]).map_err(Error::from_io)?;
            if n == 0 {
                return Ok(false);
            }
            filled += n;
        }
        if ba[..na] != bb[..na] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
    }
}

/// Reads a file as text, lossily where it is not UTF-8.
fn read_text(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path).map_err(Error::from_io)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Compares two files: a text diff when both look like text and are not
/// too large, a byte comparison otherwise.
///
/// # Errors
///
/// The I/O error from reading either file; `Error::Cancelled`.
pub fn compare_files(a: &Path, b: &Path, cancel: &CancelToken) -> Result<Diff, Error> {
    let textual = |path: &Path| -> Result<bool, Error> {
        let meta = std::fs::metadata(path).map_err(Error::from_io)?;
        if meta.len() > MAX_TEXT_BYTES {
            return Ok(false);
        }
        let mut file = std::fs::File::open(path).map_err(Error::from_io)?;
        let mut sniff = vec![0u8; 8192];
        let mut read = 0;
        while read < sniff.len() {
            let n = file.read(&mut sniff[read..]).map_err(Error::from_io)?;
            if n == 0 {
                break;
            }
            read += n;
        }
        Ok(looks_textual(&sniff[..read]))
    };
    if textual(a)? && textual(b)? {
        let left = read_text(a)?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let right = read_text(b)?;
        Ok(Diff::Text(text_diff(&left, &right)))
    } else {
        Ok(Diff::Binary {
            identical: same_bytes(a, b, cancel)?,
        })
    }
}

/// The entries of a folder, by name.
fn list(folder: &Location) -> Result<Vec<(String, Side)>, Error> {
    let path = ext::platform::paths::to_path(&folder.path)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(Error::from_io)? {
        let entry = entry.map_err(Error::from_io)?;
        let meta = entry.metadata().map_err(Error::from_io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        out.push((
            name.clone(),
            Side {
                location: Location::new(
                    ProviderId("local"),
                    ext::platform::paths::from_path(&entry.path()),
                ),
                is_dir,
                size: (!is_dir).then_some(meta.len()),
                modified: meta.modified().ok(),
            },
        ));
    }
    Ok(out)
}

/// Two times within a second of each other count as the same, since
/// file systems record them with different precision.
fn same_time(a: Option<std::time::SystemTime>, b: Option<std::time::SystemTime>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let diff = a
                .duration_since(b)
                .or_else(|e| Ok::<_, ()>(e.duration()))
                .unwrap_or_default();
            diff <= std::time::Duration::from_secs(1)
        }
        (None, None) => true,
        _ => false,
    }
}

/// Compares two folders by name, size and modification time, one level
/// deep; a folder on both sides counts as the same.
///
/// # Errors
///
/// The I/O error from listing either folder; `Error::Cancelled`.
pub fn compare_folders(a: &Location, b: &Location, cancel: &CancelToken) -> Result<Diff, Error> {
    let left = list(a)?;
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let right = list(b)?;
    let mut names: Vec<String> = left
        .iter()
        .chain(right.iter())
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names.dedup();
    let rows = names
        .into_iter()
        .map(|name| {
            let l = left
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s.clone());
            let r = right
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s.clone());
            let status = match (&l, &r) {
                (Some(_), None) => RowStatus::Removed,
                (None, Some(_)) => RowStatus::Added,
                (Some(l), Some(r)) => {
                    if l.is_dir && r.is_dir {
                        RowStatus::Same
                    } else if l.is_dir != r.is_dir
                        || l.size != r.size
                        || !same_time(l.modified, r.modified)
                    {
                        RowStatus::Changed
                    } else {
                        RowStatus::Same
                    }
                }
                (None, None) => RowStatus::Same,
            };
            FolderRow {
                name,
                status,
                left: l,
                right: r,
            }
        })
        .collect();
    Ok(Diff::Folders(FolderDiff {
        left: a.clone(),
        right: b.clone(),
        rows,
    }))
}
