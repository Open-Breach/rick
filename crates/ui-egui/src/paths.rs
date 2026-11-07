//! Path arithmetic for the path bar: segments, parent and root of a `VPath`,
//! and the breadcrumbs of a [`Location`] on any provider.

use types::{Location, ProviderId, VPath};
use vfs_rick::{SCHEME, Virtual};

use crate::icons;

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

/// One segment of the path bar: its label and the location it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    /// The text on the segment.
    pub label: String,
    /// Where clicking it goes.
    pub location: Location,
}

/// The breadcrumbs of a location, provider by provider.
///
/// A local path is its segments. A `rick://` location starts with its view,
/// titled (`Trash` for `trash`), then the path below it; the root is
/// `rick://`. An archive location is an OS path with the path inside the
/// archive appended, so the segments above the archive file are local
/// folders and open as such, the archive and everything below it stay on
/// the archive provider: which segment is the archive is found by asking
/// the filesystem, once per location (the caller caches). Any other
/// provider's path is its segments on that provider.
#[must_use]
pub fn crumbs(location: &Location) -> Vec<Crumb> {
    let provider = location.provider;
    if provider == ProviderId::RICK {
        let Some(view) = vfs_rick::view_name(location) else {
            return vec![Crumb {
                label: SCHEME.to_owned(),
                location: location.clone(),
            }];
        };
        let title = Virtual::of(&Location::new(provider, view)).map_or_else(
            || {
                let mut title = view.to_owned();
                if let Some(first) = title.get(..1) {
                    title.replace_range(..1, &first.to_uppercase());
                }
                title
            },
            |which| which.title().to_owned(),
        );
        let mut out: Vec<Crumb> = Vec::new();
        for (label, prefix) in segments(&location.path) {
            if label == "/" {
                // A leading slash a hand-edited file carries; the view is next.
                continue;
            }
            // The crumb's location in the provider's own form, `rick:trash`.
            let bytes = prefix.as_bytes();
            let start = bytes.iter().take_while(|b| is_sep(**b)).count();
            out.push(Crumb {
                label: if out.is_empty() { title.clone() } else { label },
                location: Location::new(provider, VPath::from_bytes(bytes[start..].to_vec())),
            });
        }
        return out;
    }
    let mut inside_archive = false;
    segments(&location.path)
        .into_iter()
        .map(|(label, prefix)| {
            let crumb_provider = if provider == ProviderId::ARCHIVE {
                if !inside_archive
                    && platform::paths::to_path(&prefix)
                        .is_ok_and(|p| std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir()))
                {
                    inside_archive = true;
                }
                if inside_archive {
                    ProviderId::ARCHIVE
                } else {
                    ProviderId::LOCAL
                }
            } else {
                provider
            };
            Crumb {
                label,
                location: Location::new(crumb_provider, prefix),
            }
        })
        .collect()
}

/// The location one step up from `crumbs`'s last: the segment before it.
#[must_use]
pub fn parent_of(crumbs: &[Crumb]) -> Option<Location> {
    (crumbs.len() >= 2).then(|| crumbs[crumbs.len() - 2].location.clone())
}

/// The top of the location's own tree: the first segment on the same
/// provider as the last one (the archive file for a location inside an
/// archive, the view for a `rick://` location, `/` or the drive locally).
#[must_use]
pub fn root_of(crumbs: &[Crumb]) -> Option<Location> {
    let last = crumbs.last()?;
    crumbs
        .iter()
        .find(|c| c.location.provider == last.location.provider)
        .map(|c| c.location.clone())
}

/// The text a location shows in a tab's title: its last crumb's label.
#[must_use]
pub fn title_of(location: &Location) -> String {
    if location.provider == ProviderId::RICK {
        return crumbs(location)
            .pop()
            .map_or_else(|| SCHEME.to_owned(), |c| c.label);
    }
    title(&location.path)
}

/// The glyph in front of a location's title: a folder locally, the
/// archive glyph inside an archive, the trash's in the Trash view and
/// below it, the provider's otherwise (a view's own glyph is the shell's
/// `virtual_icon`).
#[must_use]
pub fn location_icon(location: &Location) -> &'static str {
    match location.provider {
        ProviderId::LOCAL => icons::FOLDER,
        ProviderId::ARCHIVE => icons::ARCHIVE,
        ProviderId::RICK => match vfs_rick::view_name(location) {
            Some(view) if view == Virtual::Trash.name() => icons::TRASH,
            _ => icons::PROVIDER,
        },
        _ => icons::PROVIDER,
    }
}

/// The text of a location in the path bar's edit field: `rick://trash` for
/// a view, the path otherwise.
#[must_use]
pub fn text_of(location: &Location) -> String {
    if location.provider == ProviderId::RICK {
        let path = location.path.to_string_lossy();
        return format!("{SCHEME}{}", path.trim_start_matches(['/', '\\']));
    }
    location.path.to_string_lossy().into_owned()
}

/// The location a path typed into the path bar means while the pane is at
/// `current`: `rick://…` is a view; otherwise the text is a path on the
/// current provider, except that a path typed in a `rick://` view or in an
/// archive is a local one when it names an existing folder.
#[must_use]
pub fn typed(current: &Location, text: &str) -> Location {
    if let Some(rest) = text.trim().strip_prefix(SCHEME) {
        return vfs_rick::parse_uri(text)
            .unwrap_or_else(|| Location::new(ProviderId::RICK, rest.trim_matches(['/', '\\'])));
    }
    let provider = match current.provider {
        ProviderId::RICK => ProviderId::LOCAL,
        ProviderId::ARCHIVE if std::path::Path::new(text).is_dir() => ProviderId::LOCAL,
        other => other,
    };
    Location::new(provider, text)
}

#[cfg(test)]
mod crumb_tests {
    use super::*;

    #[test]
    fn rick_locations_start_with_their_view() {
        let trash = Location::new(ProviderId::RICK, "trash");
        let c = crumbs(&trash);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].label, "Trash");
        assert_eq!(c[0].location, trash);
        assert_eq!(crumbs(&Location::new(ProviderId::RICK, "/trash")), c);
        assert_eq!(location_icon(&trash), icons::TRASH);
        assert_eq!(
            location_icon(&Location::new(ProviderId::RICK, "favorites")),
            icons::PROVIDER
        );
        let deeper = Location::new(ProviderId::RICK, "trash/folder");
        let c = crumbs(&deeper);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].label, "folder");
        assert_eq!(parent_of(&c), Some(trash.clone()));
        assert_eq!(root_of(&c), Some(trash.clone()));
        assert_eq!(title_of(&deeper), "folder");
        assert_eq!(title_of(&trash), "Trash");
        assert_eq!(text_of(&trash), "rick://trash");
        assert_eq!(text_of(&deeper), "rick://trash/folder");
        assert_eq!(location_icon(&deeper), icons::TRASH);
        let root = Location::new(ProviderId::RICK, "");
        assert_eq!(crumbs(&root)[0].label, "rick://");
        assert_eq!(
            typed(&trash, "rick://trash/x"),
            Location::new(ProviderId::RICK, "trash/x")
        );
        assert_eq!(typed(&trash, "rick://machine"), Virtual::Machine.location());
        assert_eq!(typed(&trash, "/home").provider, ProviderId::LOCAL);
    }

    #[test]
    fn archive_locations_split_at_the_archive_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let archive = dir.path().join("a.zip");
        std::fs::write(&archive, b"PK").unwrap_or_else(|e| panic!("{e}"));
        let inside = Location::new(
            ProviderId::ARCHIVE,
            platform::paths::from_path(&archive).join("dir"),
        );
        let c = crumbs(&inside);
        let n = c.len();
        assert_eq!(c[n - 1].label, "dir");
        assert_eq!(c[n - 1].location.provider, ProviderId::ARCHIVE);
        assert_eq!(c[n - 2].label, "a.zip");
        assert_eq!(c[n - 2].location.provider, ProviderId::ARCHIVE);
        assert_eq!(c[n - 3].location.provider, ProviderId::LOCAL);
        assert_eq!(
            root_of(&c),
            Some(Location::new(
                ProviderId::ARCHIVE,
                platform::paths::from_path(&archive)
            ))
        );
        let up_from_archive = parent_of(&c[..n - 1]);
        assert_eq!(
            up_from_archive,
            Some(Location::new(
                ProviderId::LOCAL,
                platform::paths::from_path(dir.path())
            ))
        );
        assert_eq!(
            typed(&inside, &dir.path().display().to_string()).provider,
            ProviderId::LOCAL
        );
        assert_eq!(
            typed(&inside, &archive.display().to_string()).provider,
            ProviderId::ARCHIVE
        );
    }
}
