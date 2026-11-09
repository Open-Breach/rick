//! The applications registered for a file type, from the freedesktop
//! databases: the MIME type from `globs2`, the defaults and associations
//! from `mimeapps.list`, the rest from `mimeinfo.cache`, each one a
//! desktop file whose `Exec` line launches it.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use types::Error;

use crate::traits::Application;

/// The data directories, in precedence order: `$XDG_DATA_HOME`, then
/// `$XDG_DATA_DIRS`.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    if let Some(home) = home {
        dirs.push(home);
    }
    let system = std::env::var_os("XDG_DATA_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
    dirs.extend(std::env::split_paths(&system));
    dirs
}

/// The configuration directories, in precedence order.
fn config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    if let Some(home) = home {
        dirs.push(home);
    }
    let system = std::env::var_os("XDG_CONFIG_DIRS")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| OsString::from("/etc/xdg"));
    dirs.extend(std::env::split_paths(&system));
    dirs
}

/// Whether `name` matches the shell-style `pattern` (`*`, `?`, `[…]`),
/// ignoring case.
pub(crate) fn glob_matches(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some(('[', rest)) => {
                let Some(close) = rest.iter().position(|c| *c == ']') else {
                    return !n.is_empty() && n[0] == '[' && go(rest, &n[1..]);
                };
                let (set, after) = (&rest[..close], &rest[close + 1..]);
                let Some((first, tail)) = n.split_first() else {
                    return false;
                };
                let (negate, set) = match set.split_first() {
                    Some(('!' | '^', s)) => (true, s),
                    _ => (false, set),
                };
                let mut found = false;
                let mut i = 0;
                while i < set.len() {
                    if i + 2 < set.len() && set[i + 1] == '-' {
                        if (set[i]..=set[i + 2]).contains(first) {
                            found = true;
                        }
                        i += 3;
                    } else {
                        if set[i] == *first {
                            found = true;
                        }
                        i += 1;
                    }
                }
                found != negate && go(after, tail)
            }
            Some((c, rest)) => !n.is_empty() && n[0] == *c && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    go(&p, &n)
}

/// The MIME type of a file name from the `globs2` databases: the heaviest
/// matching pattern wins, the longest on a tie.
pub(crate) fn mime_by_name(name: &str) -> Option<String> {
    let mut best: Option<(u32, usize, String)> = None;
    for dir in data_dirs() {
        let Ok(text) = std::fs::read_to_string(dir.join("mime/globs2")) else {
            continue;
        };
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut parts = line.split(':');
            let (Some(weight), Some(mime), Some(pattern)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<u32>() else {
                continue;
            };
            if !glob_matches(pattern, name) {
                continue;
            }
            let better = best
                .as_ref()
                .is_none_or(|(w, len, _)| weight > *w || (weight == *w && pattern.len() > *len));
            if better {
                best = Some((weight, pattern.len(), mime.to_owned()));
            }
        }
    }
    best.map(|(_, _, mime)| mime)
}

/// The MIME type of an entry: a folder is `inode/directory`, a file is
/// what its name says, `application/octet-stream` when nothing does.
pub(crate) fn mime_of(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".to_owned();
    }
    path.file_name()
        .and_then(|n| mime_by_name(&n.to_string_lossy()))
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

/// The desktop ids listed for `mime` under `section` of an INI-like file:
/// `mime=a.desktop;b.desktop;`.
fn ids_in(text: &str, section: &str, mime: &str) -> Vec<String> {
    let mut inside = false;
    let mut ids = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == mime {
            ids.extend(
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    ids
}

/// The MIME types to look up for `mime`: itself, then the generic one of
/// its family where the databases usually register editors.
fn candidates(mime: &str) -> Vec<String> {
    let mut out = vec![mime.to_owned()];
    if mime.starts_with("text/") && mime != "text/plain" {
        out.push("text/plain".to_owned());
    }
    if mime != "application/octet-stream" && !mime.starts_with("inode/") {
        // `subclasses` makes most structured text a text/plain subclass.
        let text_like = [
            "xml",
            "json",
            "yaml",
            "toml",
            "javascript",
            "x-sh",
            "x-shellscript",
        ];
        if text_like.iter().any(|t| mime.ends_with(t)) && !out.contains(&"text/plain".to_owned()) {
            out.push("text/plain".to_owned());
        }
    }
    out
}

/// One desktop file, as far as launching it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub terminal: bool,
    pub hidden: bool,
}

/// Reads the `[Desktop Entry]` group of a desktop file.
pub(crate) fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let mut inside = false;
    let mut name = None;
    let mut exec = None;
    let mut terminal = false;
    let mut hidden = false;
    let mut try_exec: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "TryExec" => try_exec = Some(value.trim().to_owned()),
            "Terminal" => terminal = value.trim() == "true",
            "NoDisplay" | "Hidden" if value.trim() == "true" => hidden = true,
            _ => {}
        }
    }
    if let Some(program) = try_exec
        && !program.is_empty()
        && on_path(&program).is_none()
        && !Path::new(&program).is_absolute()
    {
        hidden = true;
    }
    Some(DesktopEntry {
        name: name?,
        exec: exec?,
        terminal,
        hidden,
    })
}

/// Where the desktop file `id` is, if anywhere: `applications/<id>` in a
/// data directory, with a `-` standing for a subdirectory as well.
fn find_desktop(id: &str) -> Option<PathBuf> {
    for dir in data_dirs() {
        let apps = dir.join("applications");
        let direct = apps.join(id);
        if direct.is_file() {
            return Some(direct);
        }
        let nested = apps.join(id.replace('-', "/"));
        if nested.is_file() {
            return Some(nested);
        }
    }
    None
}

/// The first directory of `PATH` holding an executable `program`.
pub(crate) fn on_path(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let path = PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// The applications for `path`, the default first.
pub(crate) fn applications(path: &Path) -> Result<Vec<Application>, Error> {
    std::fs::symlink_metadata(path).map_err(Error::from_io)?;
    let mime = mime_of(path);
    let mimes = candidates(&mime);
    let mut ordered: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    let push = |id: String, ordered: &mut Vec<String>| {
        if !ordered.contains(&id) {
            ordered.push(id);
        }
    };
    // mimeapps.list: the user's, then the system's; defaults first.
    let mut lists: Vec<PathBuf> = Vec::new();
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|d| d.split(':').map(str::to_lowercase).collect())
        .unwrap_or_default();
    for dir in config_dirs() {
        for desktop in &desktops {
            lists.push(dir.join(format!("{desktop}-mimeapps.list")));
        }
        lists.push(dir.join("mimeapps.list"));
    }
    for dir in data_dirs() {
        for desktop in &desktops {
            lists.push(dir.join(format!("applications/{desktop}-mimeapps.list")));
        }
        lists.push(dir.join("applications/mimeapps.list"));
    }
    let texts: Vec<String> = lists
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    for mime in &mimes {
        for text in &texts {
            for id in ids_in(text, "Default Applications", mime) {
                push(id, &mut ordered);
            }
        }
    }
    for mime in &mimes {
        for text in &texts {
            for id in ids_in(text, "Added Associations", mime) {
                push(id, &mut ordered);
            }
            removed.extend(ids_in(text, "Removed Associations", mime));
        }
    }
    // mimeinfo.cache: everything registered for the type.
    for dir in data_dirs() {
        let Ok(text) = std::fs::read_to_string(dir.join("applications/mimeinfo.cache")) else {
            continue;
        };
        for mime in &mimes {
            for id in ids_in(&text, "MIME Cache", mime) {
                push(id, &mut ordered);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for id in ordered {
        if removed.contains(&id) {
            continue;
        }
        let Some(file) = find_desktop(&id) else {
            continue;
        };
        if seen.contains(&file) {
            continue;
        }
        let Some(entry) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| parse_desktop(&text))
        else {
            continue;
        };
        if entry.hidden {
            continue;
        }
        seen.insert(file.clone());
        out.push(Application {
            name: entry.name,
            id: file.to_string_lossy().into_owned(),
        });
    }
    if out.is_empty() && on_path("xdg-open").is_some() {
        out.push(Application {
            name: "System default".to_owned(),
            id: "xdg-open".to_owned(),
        });
    }
    Ok(out)
}

/// Splits a desktop `Exec` line into its words: quoted with `"`, with the
/// backslash escapes the specification names.
pub(crate) fn split_exec(exec: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// The command line of a desktop `Exec` for `path`: the field codes
/// replaced (`%f`, `%F`, `%u`, `%U` by the path or its URI), the others
/// dropped, the path appended when the line names no file.
pub(crate) fn command_for(exec: &str, path: &Path) -> Vec<OsString> {
    let uri = crate::paths::file_uri(path);
    let mut out: Vec<OsString> = Vec::new();
    let mut placed = false;
    for word in split_exec(exec) {
        match word.as_str() {
            "%f" | "%F" => {
                out.push(path.as_os_str().to_owned());
                placed = true;
            }
            "%u" | "%U" => {
                out.push(OsString::from(&uri));
                placed = true;
            }
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            other => {
                let mut text = other.replace("%%", "\u{0}");
                for code in ["%f", "%F", "%u", "%U"] {
                    if text.contains(code) {
                        text = text.replace(code, &path.to_string_lossy());
                        placed = true;
                    }
                }
                for code in ["%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m"] {
                    text = text.replace(code, "");
                }
                out.push(OsString::from(text.replace('\u{0}', "%")));
            }
        }
    }
    if !placed {
        out.push(path.as_os_str().to_owned());
    }
    out
}

/// Starts `app` on `path`: a desktop file's `Exec` line, or an executable
/// with the path as its argument.
pub(crate) fn open_with(path: &Path, app: &Application) -> Result<(), Error> {
    let path = std::path::absolute(path).map_err(Error::from_io)?;
    let command: Vec<OsString> = if app.id.ends_with(".desktop") {
        let text = std::fs::read_to_string(&app.id).map_err(Error::from_io)?;
        let entry = parse_desktop(&text).ok_or(Error::Unsupported(
            "the desktop file has no name or Exec line",
        ))?;
        let mut command = command_for(&entry.exec, &path);
        if entry.terminal {
            // A terminal application runs in the user's terminal.
            let args: Vec<String> = command
                .iter()
                .map(|w| w.to_string_lossy().into_owned())
                .collect();
            return super::terminal::open(path.parent().unwrap_or(&path), Some(&args));
        }
        if command.is_empty() {
            return Err(Error::Unsupported("the Exec line is empty"));
        }
        std::mem::take(&mut command)
    } else {
        let mut words: Vec<OsString> = split_exec(&app.id)
            .into_iter()
            .map(OsString::from)
            .collect();
        if words.is_empty() {
            return Err(Error::Unsupported("no application named"));
        }
        words.push(path.as_os_str().to_owned());
        words
    };
    let (program, args) = command
        .split_first()
        .ok_or(Error::Unsupported("no application named"))?;
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(Error::from_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_like_the_shell() {
        assert!(glob_matches("*.txt", "notes.TXT"));
        assert!(glob_matches("*.tar.gz", "a.tar.gz"));
        assert!(!glob_matches("*.txt", "notes.txt.bak"));
        assert!(glob_matches("Makefile", "makefile"));
        assert!(glob_matches("*.[ch]", "x.c"));
        assert!(glob_matches("*.[ch]", "x.h"));
        assert!(!glob_matches("*.[ch]", "x.o"));
        assert!(glob_matches("*.p[a-c]", "x.pb"));
        assert!(glob_matches("[!a]*", "b"));
        assert!(glob_matches("?.txt", "a.txt"));
        assert!(!glob_matches("?.txt", "ab.txt"));
    }

    #[test]
    fn desktop_files_parse_and_exec_lines_split() {
        let entry = parse_desktop(
            "[Desktop Entry]\nName=Editor\nExec=edit --new-window %F\nTerminal=false\n\
             [Desktop Action new]\nName=New\nExec=edit %u\n",
        )
        .unwrap_or_else(|| panic!("parsed"));
        assert_eq!(entry.name, "Editor");
        assert_eq!(entry.exec, "edit --new-window %F");
        assert!(!entry.hidden);
        let hidden = parse_desktop("[Desktop Entry]\nName=X\nExec=x\nNoDisplay=true\n")
            .unwrap_or_else(|| panic!("parsed"));
        assert!(hidden.hidden);
        assert!(parse_desktop("[Desktop Entry]\nName=X\n").is_none());

        assert_eq!(
            split_exec(r#"edit "a b" c\ d %%"#),
            vec!["edit", "a b", "c\\", "d", "%%"]
        );
        let command = command_for("edit --new-window %F", Path::new("/tmp/x y.txt"));
        assert_eq!(
            command,
            vec![
                OsString::from("edit"),
                OsString::from("--new-window"),
                OsString::from("/tmp/x y.txt")
            ]
        );
        let command = command_for("view %i %c", Path::new("/tmp/x.png"));
        assert_eq!(
            command,
            vec![OsString::from("view"), OsString::from("/tmp/x.png")]
        );
        let command = command_for("browse %u", Path::new("/tmp/x.html"));
        assert_eq!(
            command,
            vec![
                OsString::from("browse"),
                OsString::from("file:///tmp/x.html")
            ]
        );
    }

    #[test]
    fn mime_types_come_from_the_databases_when_present() {
        let mime = mime_of(Path::new("/"));
        assert_eq!(mime, "inode/directory");
        if Path::new("/usr/share/mime/globs2").is_file() {
            assert_eq!(mime_by_name("notes.txt").as_deref(), Some("text/plain"));
        }
    }
}
