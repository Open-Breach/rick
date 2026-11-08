//! The `text/uri-list` the drag-and-drop protocols carry: writing the
//! dragged paths as `file://` URIs, and reading the URIs of a drop back into
//! paths. Shared by the three platforms' drag sources and the Wayland drop
//! target.

use std::path::{Path, PathBuf};

/// The `text/uri-list` of `paths`: one `file://` URI per line, CRLF-ended
/// as RFC 2483 wants and every receiver accepts.
pub(crate) fn uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&crate::paths::file_uri(path));
        out.push_str("\r\n");
    }
    out
}

/// The local paths of a `text/uri-list`: its `file://` lines, percent
/// decoded, with an empty or `localhost` host; comment lines (`#`) and URIs
/// of other schemes or hosts are skipped.
pub(crate) fn parse_uri_list(text: &[u8]) -> Vec<PathBuf> {
    text.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(file_uri_path)
        .collect()
}

/// The path of one `file://` URI, or `None` for anything else.
fn file_uri_path(uri: &[u8]) -> Option<PathBuf> {
    let rest = uri.strip_prefix(b"file://")?;
    // `file:///path` has an empty host; `file://localhost/path` the local one.
    let (host, path) = match rest.iter().position(|&b| b == b'/') {
        Some(0) => (&b""[..], rest),
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => return None,
    };
    if !host.is_empty() && host != b"localhost" {
        return None;
    }
    let bytes = percent_decode(path);
    Some(decoded_path(&bytes))
}

/// Decodes `%XX` escapes byte by byte; a malformed escape is kept as is.
fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                // Two hex digits fit a byte.
                #[allow(clippy::cast_possible_truncation)]
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(unix)]
fn decoded_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt as _;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(windows)]
fn decoded_path(bytes: &[u8]) -> PathBuf {
    // `file:///C:/Users/me/x` → `C:\Users\me\x`; `file://server/share` is
    // skipped above since its host is not local.
    let text = String::from_utf8_lossy(bytes);
    let text = text
        .strip_prefix('/')
        .filter(|t| t.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&text);
    PathBuf::from(text.replace('/', "\\"))
}

/// Whether `path` is one the URI list can name: absolute, so the receiver
/// finds it from wherever it runs.
pub(crate) fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_list_round_trips_through_the_parser() {
        let paths = if cfg!(windows) {
            vec![
                PathBuf::from("C:\\Users\\me\\a b.txt"),
                PathBuf::from("D:\\x%y"),
            ]
        } else {
            vec![PathBuf::from("/home/me/a b.txt"), PathBuf::from("/x%y")]
        };
        let list = uri_list(&paths);
        assert!(list.ends_with("\r\n"), "{list:?}");
        assert_eq!(list.lines().count(), 2);
        assert!(list.contains("a%20b.txt"));
        assert_eq!(parse_uri_list(list.as_bytes()), paths);
    }

    #[test]
    fn foreign_lines_are_skipped() {
        let text = b"# a comment\r\nhttp://example.com/x\r\nfile://otherhost/share/x\r\nfile://localhost/tmp/y\nnot a uri\n";
        let parsed = parse_uri_list(text);
        if cfg!(windows) {
            assert_eq!(parsed, [PathBuf::from("\\tmp\\y")]);
        } else {
            assert_eq!(parsed, [PathBuf::from("/tmp/y")]);
        }
    }

    #[test]
    fn malformed_escapes_are_kept() {
        assert_eq!(percent_decode(b"a%2"), b"a%2");
        assert_eq!(percent_decode(b"a%zz"), b"a%zz");
        assert_eq!(percent_decode(b"%41%"), b"A%");
    }
}
