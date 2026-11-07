//! Free space of a mounted volume on Unix, through `statvfs`.

use std::path::Path;

/// `(total, free)` bytes of the volume holding `path`; `None` when the call
/// fails (a network mount that is gone, a path the user may not stat).
pub(crate) fn space(path: &Path) -> Option<(u64, u64)> {
    let vfs = rustix::fs::statvfs(path).ok()?;
    let frsize = if vfs.f_frsize == 0 {
        vfs.f_bsize
    } else {
        vfs.f_frsize
    };
    Some((
        vfs.f_blocks.saturating_mul(frsize),
        vfs.f_bavail.saturating_mul(frsize),
    ))
}
