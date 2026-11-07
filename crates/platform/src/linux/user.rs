//! The user's standard folders from XDG user-dirs, and the system's
//! recently-used list from `recently-used.xbel`.
//!
//! `$XDG_CONFIG_HOME/user-dirs.dirs` names each folder as a shell-style
//! assignment (`XDG_DOCUMENTS_DIR="$HOME/Documents"`); a folder set to the
//! home itself is disabled. The recently-used list is the XBEL file the
//! desktop's file dialogues and editors append to, under
//! `$XDG_DATA_HOME`; every `<bookmark>` carries a `file://` href and
//! `modified` and `visited` times.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use types::Error;

use super::percent_decode;
use crate::time::parse_iso8601;
use crate::traits::{RecentItem, StandardFolder};

/// How many recently-used items are kept, newest first.
const RECENT_LIMIT: usize = 500;

/// The key of a folder in `user-dirs.dirs`.
fn xdg_key(folder: StandardFolder) -> Option<&'static str> {
    Some(match folder {
        StandardFolder::Home => return None,
        StandardFolder::Documents => "XDG_DOCUMENTS_DIR",
        StandardFolder::Pictures => "XDG_PICTURES_DIR",
        StandardFolder::Music => "XDG_MUSIC_DIR",
        StandardFolder::Video => "XDG_VIDEOS_DIR",
        StandardFolder::Downloads => "XDG_DOWNLOAD_DIR",
        StandardFolder::Desktop => "XDG_DESKTOP_DIR",
    })
}

/// The folder's name under the home when `user-dirs.dirs` says nothing.
fn default_name(folder: StandardFolder) -> &'static str {
    match folder {
        StandardFolder::Home => "",
        StandardFolder::Documents => "Documents",
        StandardFolder::Pictures => "Pictures",
        StandardFolder::Music => "Music",
        StandardFolder::Video => "Videos",
        StandardFolder::Downloads => "Downloads",
        StandardFolder::Desktop => "Desktop",
    }
}

/// The value of `key` in the text of `user-dirs.dirs`, with `$HOME`
/// expanded; `None` when the key is absent or the folder is disabled (set
/// to the home itself).
fn from_user_dirs(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            home.join(rest.trim_start_matches('/'))
        } else {
            PathBuf::from(value)
        };
        if path == home {
            return None;
        }
        return Some(path);
    }
    None
}

/// Where `folder` is: the XDG setting, else the conventional name under the
/// home; `None` when the folder does not exist.
pub(super) fn standard_folder(folder: StandardFolder, home: &Path) -> Option<PathBuf> {
    let path = match xdg_key(folder) {
        None => home.to_path_buf(),
        Some(key) => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map_or_else(|| home.join(".config"), PathBuf::from);
            let text = fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
            from_user_dirs(&text, key, home).unwrap_or_else(|| home.join(default_name(folder)))
        }
    };
    path.is_dir().then_some(path)
}

/// `$XDG_DATA_HOME/recently-used.xbel`.
fn xbel_path(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map_or_else(|| home.join(".local/share"), PathBuf::from)
        .join("recently-used.xbel")
}

/// The value of `name="…"` inside one tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let before_ok = at == 0
            || rest[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if before_ok && let Some(value) = after.trim_start().strip_prefix('=') {
            let value = value.trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let inner = &value[1..];
                let end = inner.find(quote)?;
                return Some(&inner[..end]);
            }
        }
        rest = after;
    }
    None
}

/// The XML entities an href or a date can carry.
fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// `2026-11-07T10:00:00.123456Z` as the XBEL writes it, fractional seconds
/// dropped.
fn parse_time(text: &str) -> Option<SystemTime> {
    let text = text.trim_end_matches('Z');
    let whole = text.split_once('.').map_or(text, |(whole, _)| whole);
    parse_iso8601(whole)
}

/// The bookmarks of an XBEL text that point at existing local paths,
/// newest first.
fn parse_xbel(text: &str, exists: &dyn Fn(&Path) -> bool) -> Vec<RecentItem> {
    let mut items: Vec<RecentItem> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<bookmark") {
        let tag_start = &rest[start..];
        let Some(end) = tag_start.find('>') else {
            break;
        };
        let tag = &tag_start[..end];
        rest = &tag_start[end + 1..];
        if !tag.starts_with("<bookmark ") && !tag.starts_with("<bookmark\n") {
            continue;
        }
        let Some(href) = attribute(tag, "href") else {
            continue;
        };
        let href = unescape(href);
        let Some(encoded) = href.strip_prefix("file://") else {
            continue;
        };
        // `file://host/path` names a host; only the local form is a path.
        let encoded = encoded.strip_prefix("localhost").unwrap_or(encoded);
        if !encoded.starts_with('/') {
            continue;
        }
        let path = PathBuf::from(OsString::from_vec(percent_decode(encoded)));
        let used_at = ["visited", "modified", "added"]
            .iter()
            .filter_map(|name| attribute(tag, name))
            .filter_map(parse_time)
            .max()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if !exists(&path) {
            continue;
        }
        items.retain(|item| item.path != path);
        items.push(RecentItem { path, used_at });
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.used_at));
    items.truncate(RECENT_LIMIT);
    items
}

/// The recently-used list, newest first; an absent file is an empty list.
pub(super) fn recently_used(home: &Path) -> Result<Vec<RecentItem>, Error> {
    let text = match fs::read_to_string(xbel_path(home)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::from_io(e)),
    };
    Ok(parse_xbel(&text, &|path| {
        fs::symlink_metadata(path).is_ok()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_expand_home_and_a_disabled_folder_is_absent() {
        let home = Path::new("/home/me");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME\"\nXDG_VIDEOS_DIR=\"/data/video\"\n";
        assert_eq!(
            from_user_dirs(text, "XDG_DESKTOP_DIR", home),
            Some(PathBuf::from("/home/me/Desktop"))
        );
        assert_eq!(from_user_dirs(text, "XDG_DOCUMENTS_DIR", home), None);
        assert_eq!(
            from_user_dirs(text, "XDG_VIDEOS_DIR", home),
            Some(PathBuf::from("/data/video"))
        );
        assert_eq!(from_user_dirs(text, "XDG_MUSIC_DIR", home), None);
    }

    #[test]
    fn standard_folders_need_an_existing_directory() {
        let home = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        fs::create_dir(home.path().join("Downloads")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            standard_folder(StandardFolder::Home, home.path()),
            Some(home.path().to_path_buf())
        );
        assert_eq!(
            standard_folder(StandardFolder::Downloads, home.path())
                .and_then(|p| p.file_name().map(std::ffi::OsStr::to_os_string)),
            Some("Downloads".into())
        );
        assert_eq!(standard_folder(StandardFolder::Music, home.path()), None);
    }

    #[test]
    fn xbel_bookmarks_give_existing_paths_newest_first() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0">
  <bookmark href="file:///home/me/a%20b.txt" added="2026-11-01T10:00:00.000000Z" modified="2026-11-02T10:00:00.500000Z" visited="2026-11-02T10:00:00.500000Z">
    <info><metadata owner="http://freedesktop.org"><mime:mime-type type="text/plain"/></metadata></info>
  </bookmark>
  <bookmark href="file:///home/me/gone.txt" added="2026-11-03T10:00:00Z" modified="2026-11-03T10:00:00Z" visited="2026-11-03T10:00:00Z"/>
  <bookmark href="file:///home/me/new.txt" added="2026-11-05T10:00:00Z" modified="2026-11-05T10:00:00Z" visited="2026-11-05T10:00:00Z"/>
  <bookmark href="https://example.com/x" added="2026-11-06T10:00:00Z" modified="2026-11-06T10:00:00Z" visited="2026-11-06T10:00:00Z"/>
  <bookmarks/>
</xbel>"#;
        let items = parse_xbel(text, &|p| p != Path::new("/home/me/gone.txt"));
        let paths: Vec<&Path> = items.iter().map(|i| i.path.as_path()).collect();
        assert_eq!(
            paths,
            [Path::new("/home/me/new.txt"), Path::new("/home/me/a b.txt")]
        );
        assert!(items[0].used_at > items[1].used_at);
        assert_eq!(
            parse_time("2026-11-02T10:00:00.500000Z"),
            parse_iso8601("2026-11-02T10:00:00")
        );
        assert_eq!(
            attribute(r#"<bookmark xhref="x" href='y'"#, "href"),
            Some("y")
        );
    }
}
