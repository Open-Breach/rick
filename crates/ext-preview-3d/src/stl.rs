//! STL, ASCII and binary, through `stl_io`.
//!
//! Which of the two a file is follows from its size first: a binary STL is
//! an 80-byte header, a triangle count and 50 bytes per triangle, exactly.
//! A binary file whose header happens to start with `solid`, which many
//! exporters write, is read as binary all the same.

use std::io::Cursor;

use crate::mesh::{self, Mesh};

/// The two encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `solid … facet normal … endsolid`.
    Ascii,
    /// A header, a count and fixed-size records.
    Binary,
}

/// The bytes of a binary STL before the triangles.
pub const BINARY_HEADER: usize = 84;
/// The bytes of one binary triangle.
pub const BINARY_RECORD: u64 = 50;

/// The triangle count a binary STL of `size` bytes declares in `head`, its
/// first bytes, when the two agree.
#[must_use]
pub fn binary_count(head: &[u8], size: u64) -> Option<u64> {
    let bytes: [u8; 4] = head.get(80..84)?.try_into().ok()?;
    let count = u64::from(u32::from_le_bytes(bytes));
    (count
        .checked_mul(BINARY_RECORD)
        .and_then(|body| body.checked_add(BINARY_HEADER as u64))
        == Some(size))
    .then_some(count)
}

/// Whether `head` reads like the start of an ASCII STL: `solid`, then a
/// facet within the sniff unless the sniff is the whole file.
#[must_use]
pub fn looks_ascii(head: &[u8]) -> bool {
    let trimmed = head.trim_ascii_start();
    trimmed.starts_with(b"solid")
        && !head.contains(&0)
        && (head.windows(5).any(|w| w == b"facet") || head.len() < 4096)
}

/// Which encoding `head` and `size` describe, if either.
#[must_use]
pub fn detect(head: &[u8], size: u64) -> Option<Kind> {
    if binary_count(head, size).is_some() {
        Some(Kind::Binary)
    } else if looks_ascii(head) {
        Some(Kind::Ascii)
    } else {
        None
    }
}

/// Parses `bytes`, the whole file, in the neutral colour.
///
/// # Errors
///
/// `stl_io`'s error when the bytes are not an STL of either kind.
pub fn parse(mut bytes: Vec<u8>) -> Result<(Mesh, Kind), std::io::Error> {
    let size = bytes.len() as u64;
    let kind = match detect(&bytes[..bytes.len().min(4096)], size) {
        Some(kind) => kind,
        None if bytes.trim_ascii_start().starts_with(b"solid") => Kind::Ascii,
        None => Kind::Binary,
    };
    if kind == Kind::Binary && bytes.starts_with(b"solid") {
        // `stl_io` takes a first line starting with `solid ` for ASCII; the
        // header of a binary file carries no meaning, so make it say
        // otherwise.
        bytes[0] = b'B';
    }
    let mut cursor = Cursor::new(bytes);
    let reader = stl_io::create_stl_reader(&mut cursor)?;
    let mut mesh = Mesh::with_capacity(match kind {
        Kind::Binary => {
            usize::try_from(size.saturating_sub(BINARY_HEADER as u64) / BINARY_RECORD).unwrap_or(0)
        }
        Kind::Ascii => 0,
    });
    let color = mesh::pack(mesh::NEUTRAL);
    for triangle in reader {
        let triangle = triangle?;
        let corner = |i: usize| -> [f32; 3] { triangle.vertices[i].0 };
        mesh.push([corner(0), corner(1), corner(2)], color);
    }
    Ok((mesh, kind))
}
