//! The renaming rules and the preview they produce.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use types::Entry;

/// How the names change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// The new name, with `{name}`, `{ext}`, `{n}`, `{n:4}` and `{date}`.
    pub pattern: String,
    /// Text, or a regular expression, to find in the original name.
    pub find: String,
    /// What the found text becomes; `$1` and friends for a regular
    /// expression's groups.
    pub replace: String,
    /// Whether `find` is a regular expression.
    pub regex: bool,
    /// The case change applied to the whole result.
    pub case: Case,
    /// Where `{n}` starts.
    pub start: u64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            pattern: "{name}{ext}".to_owned(),
            find: String::new(),
            replace: String::new(),
            regex: false,
            case: Case::Keep,
            start: 1,
        }
    }
}

/// A case change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Case {
    /// As it is.
    #[default]
    Keep,
    /// All lower case.
    Lower,
    /// All upper case.
    Upper,
    /// The first letter of every word upper case, the rest lower.
    Title,
}

impl Case {
    /// The options, in the order the form lists them.
    pub const OPTIONS: [&'static str; 4] = ["Keep", "lower case", "UPPER CASE", "Title Case"];

    /// The case for the form's choice index.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Lower,
            2 => Self::Upper,
            3 => Self::Title,
            _ => Self::Keep,
        }
    }

    fn apply(self, text: &str) -> String {
        match self {
            Self::Keep => text.to_owned(),
            Self::Lower => text.to_lowercase(),
            Self::Upper => text.to_uppercase(),
            Self::Title => {
                let mut out = String::with_capacity(text.len());
                let mut start = true;
                for c in text.chars() {
                    if c.is_alphanumeric() {
                        if start {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        start = false;
                    } else {
                        out.push(c);
                        start = true;
                    }
                }
                out
            }
        }
    }
}

/// One entry's planned rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The entry.
    pub entry: Entry,
    /// Its new name.
    pub to: String,
    /// Why it cannot be applied, if it cannot.
    pub problem: Option<String>,
}

impl Planned {
    /// Whether the name stays.
    #[must_use]
    pub fn unchanged(&self) -> bool {
        self.entry.name == self.to
    }
}

/// The name without its extension and the extension with its dot: a
/// folder has no extension, nor has a name that starts with its only dot.
fn split(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    }
}

/// `{date}` for an entry: its modification date, local time.
fn date_of(time: Option<SystemTime>) -> String {
    time.map(|t| {
        let local: DateTime<Local> = t.into();
        local.format("%Y-%m-%d").to_string()
    })
    .unwrap_or_default()
}

/// Fills the pattern's placeholders.
fn expand(pattern: &str, name: &str, ext: &str, n: u64, width: usize, date: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + name.len());
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let key = &after[..close];
        match key {
            "name" => out.push_str(name),
            "ext" => out.push_str(ext),
            "date" => out.push_str(date),
            // Writing into a String cannot fail.
            "n" => {
                let _ = write!(out, "{n:0width$}");
            }
            other => {
                if let Some(digits) = other.strip_prefix("n:")
                    && let Ok(w) = digits.parse::<usize>()
                {
                    let _ = write!(out, "{n:0w$}");
                } else {
                    // An unknown placeholder stays as typed.
                    out.push('{');
                    out.push_str(key);
                    out.push('}');
                }
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The planned renames of `entries` under `rules`, in the entries' order,
/// with the problems marked: an empty name, a separator, two entries
/// landing on one name, or a name taken by an entry of `folder` that is
/// not being renamed.
///
/// # Errors
///
/// The regular expression's parse error when `rules.regex` and `find`
/// does not parse.
pub fn plan(
    entries: &[Entry],
    rules: &Rules,
    folder: Option<&Path>,
) -> Result<Vec<Planned>, String> {
    let regex = if rules.regex && !rules.find.is_empty() {
        Some(regex::Regex::new(&rules.find).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let width = rules
        .start
        .saturating_add(entries.len() as u64)
        .saturating_sub(1)
        .to_string()
        .len();
    let mut planned: Vec<Planned> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let replaced = if rules.find.is_empty() {
            entry.name.clone()
        } else if let Some(regex) = &regex {
            regex
                .replace_all(&entry.name, rules.replace.as_str())
                .into_owned()
        } else {
            entry.name.replace(&rules.find, &rules.replace)
        };
        let is_dir = entry.kind == types::EntryKind::Dir;
        let (name, ext) = split(&replaced, is_dir);
        let n = rules.start.saturating_add(index as u64);
        let expanded = expand(
            &rules.pattern,
            name,
            ext,
            n,
            width,
            &date_of(entry.modified),
        );
        let to = rules.case.apply(&expanded);
        let problem = if to.trim().is_empty() {
            Some("the name is empty".to_owned())
        } else if to.contains(['/', '\\']) {
            Some("a name cannot hold a separator".to_owned())
        } else if to == "." || to == ".." {
            Some("not a name".to_owned())
        } else {
            None
        };
        planned.push(Planned {
            entry: entry.clone(),
            to,
            problem,
        });
    }
    // Clashes: two entries on one name, or a name another entry has.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in &planned {
        *counts.entry(p.to.clone()).or_default() += 1;
    }
    let renamed: BTreeSet<&str> = planned.iter().map(|p| p.entry.name.as_str()).collect();
    let taken: BTreeSet<String> = folder
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|dir| {
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| !renamed.contains(name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    for p in &mut planned {
        if p.problem.is_some() {
            continue;
        }
        if counts.get(&p.to).copied().unwrap_or(0) > 1 {
            p.problem = Some("two entries would get this name".to_owned());
        } else if !p.unchanged() && taken.contains(&p.to) {
            p.problem = Some("another entry has this name".to_owned());
        }
    }
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Attrs, EntryId, EntryKind, ProviderId, VPath};

    fn entry(name: &str, dir: bool) -> Entry {
        Entry {
            id: EntryId {
                provider: ProviderId("local"),
                key: u128::from(name.len() as u64) + 7,
            },
            name: name.to_owned(),
            path: VPath::from(format!("/x/{name}")),
            kind: if dir { EntryKind::Dir } else { EntryKind::File },
            size: Some(1),
            created: None,
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_hours(8760)),
            accessed: None,
            attrs: Attrs::empty(),
            mime: None,
        }
    }

    fn names(planned: &[Planned]) -> Vec<String> {
        planned.iter().map(|p| p.to.clone()).collect()
    }

    #[test]
    fn patterns_expand_name_extension_counter_and_date() {
        let entries = [
            entry("IMG_001.JPG", false),
            entry("notes", false),
            entry("Sub.dir", true),
        ];
        let rules = Rules {
            pattern: "photo_{n}{ext}".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&entries, &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&planned), ["photo_1.JPG", "photo_2", "photo_3"]);
        let rules = Rules {
            pattern: "{date} {name}{ext}".to_owned(),
            case: Case::Lower,
            ..Rules::default()
        };
        let planned = plan(&entries[..1], &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert!(planned[0].to.ends_with(" img_001.jpg"), "{}", planned[0].to);
        assert_eq!(planned[0].to.len(), "1971-01-01 img_001.jpg".len());
        // Explicit width, the counter's start, and unknown placeholders.
        let rules = Rules {
            pattern: "{n:4}-{what}{ext}".to_owned(),
            start: 7,
            ..Rules::default()
        };
        let planned = plan(&entries[..2], &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&planned), ["0007-{what}.JPG", "0008-{what}"]);
        // Padding follows the count: 1 000 entries from 1 need four digits.
        let many: Vec<Entry> = (0..1000)
            .map(|i| entry(&format!("f{i}.txt"), false))
            .collect();
        let rules = Rules {
            pattern: "{n}".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&many, &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned[0].to, "0001");
        assert_eq!(planned[999].to, "1000");
    }

    #[test]
    fn find_replace_and_case_changes() {
        let entries = [
            entry("Draft Report v2.DOCX", false),
            entry("draft notes.txt", false),
        ];
        let rules = Rules {
            find: "draft".to_owned(),
            replace: "final".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&entries, &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&planned), ["Draft Report v2.DOCX", "final notes.txt"]);
        assert!(planned[0].unchanged());
        let rules = Rules {
            find: r"(?i)^draft (\w+)".to_owned(),
            replace: "$1-final".to_owned(),
            regex: true,
            case: Case::Title,
            ..Rules::default()
        };
        let planned = plan(&entries, &rules, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&planned), ["Report-Final V2.Docx", "Notes-Final.Txt"]);
        let bad = Rules {
            find: "(".to_owned(),
            regex: true,
            ..Rules::default()
        };
        assert!(plan(&entries, &bad, None).is_err());
        assert_eq!(Case::from_index(2), Case::Upper);
        assert_eq!(Case::Upper.apply("ab.c"), "AB.C");
    }

    #[test]
    fn clashes_are_marked() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("taken.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("a.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.path().join("b.txt"), "x").unwrap_or_else(|e| panic!("{e}"));
        let entries = [entry("a.txt", false), entry("b.txt", false)];
        let same = Rules {
            pattern: "same{ext}".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&entries, &same, Some(dir.path())).unwrap_or_else(|e| panic!("{e}"));
        assert!(planned.iter().all(|p| p.problem.is_some()));
        let taken = Rules {
            find: "a".to_owned(),
            replace: "taken".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&entries, &taken, Some(dir.path())).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned[0].to, "taken.txt");
        assert!(planned[0].problem.is_some());
        assert!(planned[1].problem.is_none());
        // Swapping two names is fine: both are being renamed.
        let swap = Rules {
            pattern: "{n}{ext}".to_owned(),
            start: 0,
            ..Rules::default()
        };
        let entries = [entry("1.txt", false), entry("0.txt", false)];
        let planned = plan(&entries, &swap, Some(dir.path())).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names(&planned), ["0.txt", "1.txt"]);
        assert!(planned.iter().all(|p| p.problem.is_none()));
        let empty = Rules {
            pattern: String::new(),
            ..Rules::default()
        };
        let planned = plan(&entries, &empty, None).unwrap_or_else(|e| panic!("{e}"));
        assert!(planned.iter().all(|p| p.problem.is_some()));
        let slash = Rules {
            pattern: "a/b".to_owned(),
            ..Rules::default()
        };
        let planned = plan(&entries[..1], &slash, None).unwrap_or_else(|e| panic!("{e}"));
        assert!(planned[0].problem.is_some());
    }
}
