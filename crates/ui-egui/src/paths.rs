//! Path arithmetic for the path bar: segments, parent and root of a `VPath`.

use types::VPath;

/// Whether `b` separates path components.
fn is_sep(b: u8) -> bool {
    matches!(b, b'/' | b'\\')
}

/// The breadcrumbs of `path`: each segment's label and the path up to and
/// including it. A Unix root gives a first segment labelled `/`; a Windows
/// drive gives `C:\`.
#[must_use]
pub fn segments(path: &VPath) -> Vec<(String, VPath)> {
    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    if bytes.first().is_some_and(|&b| is_sep(b)) {
        out.push(("/".to_owned(), VPath::from_bytes(b"/".to_vec())));
        start = 1;
    }
    let mut i = start;
    while i <= bytes.len() {
        if i == bytes.len() || is_sep(bytes[i]) {
            if i > start {
                let label = String::from_utf8_lossy(&bytes[start..i]).into_owned();
                let mut prefix = bytes[..i].to_vec();
                // A bare drive letter needs its separator to be a location.
                if out.is_empty() && label.len() == 2 && label.ends_with(':') {
                    prefix.push(b'\\');
                }
                out.push((label, VPath::from_bytes(prefix)));
            }
            start = i + 1;
        }
        i += 1;
    }
    out
}

/// The path without its last component; `None` at a root.
#[must_use]
pub fn parent(path: &VPath) -> Option<VPath> {
    let mut segments = segments(path);
    if segments.len() < 2 {
        return None;
    }
    segments.pop();
    segments.pop().map(|(_, p)| p)
}

/// The top of the volume: `/`, or the drive root on Windows.
#[must_use]
pub fn root(path: &VPath) -> Option<VPath> {
    segments(path).into_iter().next().map(|(_, p)| p)
}

/// The text shown for a location: the last segment, or the whole path for a
/// root.
#[must_use]
pub fn title(path: &VPath) -> String {
    segments(path).last().map_or_else(
        || path.to_string_lossy().into_owned(),
        |(label, _)| label.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(path: &str) -> Vec<String> {
        segments(&VPath::from(path))
            .into_iter()
            .map(|(l, _)| l)
            .collect()
    }

    fn prefixes(path: &str) -> Vec<String> {
        segments(&VPath::from(path))
            .into_iter()
            .map(|(_, p)| p.to_string())
            .collect()
    }

    #[test]
    fn unix_paths_split_into_breadcrumbs() {
        assert_eq!(labels("/home/me/docs"), ["/", "home", "me", "docs"]);
        assert_eq!(
            prefixes("/home/me/docs"),
            ["/", "/home", "/home/me", "/home/me/docs"]
        );
        assert_eq!(labels("/"), ["/"]);
        assert_eq!(
            parent(&VPath::from("/home/me")).map(|p| p.to_string()),
            Some("/home".into())
        );
        assert_eq!(
            parent(&VPath::from("/home")).map(|p| p.to_string()),
            Some("/".into())
        );
        assert_eq!(parent(&VPath::from("/")), None);
        assert_eq!(
            root(&VPath::from("/home/me")).map(|p| p.to_string()),
            Some("/".into())
        );
        assert_eq!(title(&VPath::from("/home/me")), "me");
        assert_eq!(title(&VPath::from("/")), "/");
    }

    #[test]
    fn windows_paths_keep_the_drive() {
        assert_eq!(labels("C:\\Users\\me"), ["C:", "Users", "me"]);
        assert_eq!(
            prefixes("C:\\Users\\me"),
            ["C:\\", "C:\\Users", "C:\\Users\\me"]
        );
        assert_eq!(
            parent(&VPath::from("C:\\Users")).map(|p| p.to_string()),
            Some("C:\\".into())
        );
        assert_eq!(parent(&VPath::from("C:\\")), None);
        assert_eq!(
            root(&VPath::from("C:\\Users\\me")).map(|p| p.to_string()),
            Some("C:\\".into())
        );
    }
}
