//! Desktop entries: reading a vendor's `.desktop` file, rewriting its
//! `Exec` and `Icon` to go through `current`, generating a minimal one, and
//! the `update-desktop-database` call after a write.

use std::path::Path;
use std::process::{Command, Stdio};

/// The group every desktop entry starts with.
const GROUP: &str = "[Desktop Entry]";

/// A desktop entry kept as its lines, so a vendor's comments, locale keys
/// and other groups survive a rewrite of `Exec` and `Icon`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    lines: Vec<String>,
}

impl DesktopEntry {
    /// Parses a file's text; nothing is validated here.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self {
            lines: text.lines().map(str::to_owned).collect(),
        }
    }

    /// A minimal entry of the spec's shape: `Name`, `Exec`, `Icon`,
    /// `Type=Application`, `Terminal=false`, `Categories` and, when known,
    /// `StartupWMClass`.
    #[must_use]
    pub fn generate(
        name: &str,
        exec: &str,
        icon: &str,
        categories: &str,
        wm_class: Option<&str>,
    ) -> Self {
        let mut lines = vec![
            GROUP.to_owned(),
            "Type=Application".to_owned(),
            format!("Name={}", clean(name)),
            format!("Exec={exec}"),
            "Terminal=false".to_owned(),
        ];
        if !icon.is_empty() {
            lines.push(format!("Icon={}", clean(icon)));
        }
        let categories = clean(categories);
        if !categories.is_empty() {
            let mut categories = categories;
            if !categories.ends_with(';') {
                categories.push(';');
            }
            lines.push(format!("Categories={categories}"));
        }
        if let Some(class) = wm_class.map(clean).filter(|c| !c.is_empty()) {
            lines.push(format!("StartupWMClass={class}"));
        }
        Self { lines }
    }

    /// The lines of the `[Desktop Entry]` group: its index range.
    fn group_range(&self) -> Option<std::ops::Range<usize>> {
        let start = self.lines.iter().position(|l| l.trim() == GROUP)?;
        let end = self.lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map_or(self.lines.len(), |i| start + 1 + i);
        Some(start + 1..end)
    }

    /// The value of `key` in the main group (the unlocalised key only).
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        let range = self.group_range()?;
        self.lines[range].iter().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then(|| v.trim())
        })
    }

    /// Sets `key` in the main group, replacing its first occurrence or
    /// adding it at the group's end; a missing group is created.
    pub fn set(&mut self, key: &str, value: &str) {
        let value = clean(value);
        let Some(range) = self.group_range() else {
            self.lines.insert(0, GROUP.to_owned());
            self.lines.insert(1, format!("{key}={value}"));
            return;
        };
        let found = self.lines[range.clone()]
            .iter()
            .position(|line| line.split_once('=').is_some_and(|(k, _)| k.trim() == key));
        if let Some(offset) = found {
            self.lines[range.start + offset] = format!("{key}={value}");
        } else {
            // Before the trailing blank lines of the group, if any.
            let mut at = range.end;
            while at > range.start && self.lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            self.lines.insert(at, format!("{key}={value}"));
        }
    }

    /// Removes every `key` line of the main group.
    pub fn remove(&mut self, key: &str) {
        if let Some(range) = self.group_range() {
            let mut i = range.start;
            let mut end = range.end;
            while i < end {
                if self.lines[i]
                    .split_once('=')
                    .is_some_and(|(k, _)| k.trim() == key)
                {
                    self.lines.remove(i);
                    end -= 1;
                } else {
                    i += 1;
                }
            }
        }
    }

    /// The file's text, with a final newline.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }
}

/// A value without the characters a desktop entry's line cannot hold.
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\n' | '\r' | '\0'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The program an `Exec` line runs: its first argument without quotes and
/// without a directory, `postman` for `"/opt/Postman/app/Postman" %U`.
#[must_use]
pub fn exec_program(exec: &str) -> Option<String> {
    let first = exec_split(exec).into_iter().next()?;
    let name = first.rsplit('/').next()?.to_owned();
    (!name.is_empty()).then_some(name)
}

/// The arguments of an `Exec` line after the program, as written.
#[must_use]
pub fn exec_arguments(exec: &str) -> Vec<String> {
    exec_split(exec).into_iter().skip(1).collect()
}

/// Splits an `Exec` line into arguments: whitespace-separated, with
/// double-quoted arguments and their backslash escapes.
fn exec_split(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = exec.trim().chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            c => {
                current.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(current);
    }
    out
}

/// A path as one quoted `Exec` argument: double quotes, with `"`, `` ` ``,
/// `$` and `\` escaped as the desktop entry specification asks, and `%`
/// doubled so it is not a field code.
#[must_use]
pub fn quote(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An `Exec` line that runs `program` with the arguments of `original`
/// (its field codes such as `%U` included), or with none.
#[must_use]
pub fn exec_through(program: &Path, original: Option<&str>) -> String {
    let mut out = quote(program);
    for arg in original.map_or_default(exec_arguments) {
        out.push(' ');
        if arg.starts_with('%') && arg.len() == 2 {
            out.push_str(&arg);
        } else {
            out.push_str(&quote(Path::new(&arg)));
        }
    }
    out
}

/// Runs `update-desktop-database` on `dir` when it is installed; the
/// launcher picks a new entry up without it on most desktops, so a missing
/// tool is only logged.
pub fn update_database(dir: &Path) {
    let status = Command::new("update-desktop-database")
        .arg("-q")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::debug!(%status, dir = %dir.display(), "update-desktop-database"),
        Err(error) => tracing::debug!(%error, "update-desktop-database is not installed"),
    }
}

/// Runs `desktop-file-validate` on `path` when it is installed: `None`
/// without the tool, otherwise its verdict with its output. For the tests.
#[must_use]
pub fn validate(path: &Path) -> Option<Result<(), String>> {
    let output = Command::new("desktop-file-validate")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    Some(if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr))
    })
}
