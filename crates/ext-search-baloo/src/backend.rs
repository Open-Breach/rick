//! What is particular to Baloo: the probe through `balooctl`, which also
//! tells the KDE Frameworks generation (`balooctl6` and `baloosearch6`, or
//! the unsuffixed pair), the `baloosearch` command line and the paths it
//! prints.

use std::path::PathBuf;

use ext::{Query, QueryKind};
use types::Error;

use crate::MAX_HITS;
use crate::service::{Command, Service};

/// What the probe learnt about the tools: the suffix of the Baloo command
/// line, `6` for KDE Frameworks 6 and empty for 5.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    /// `"6"` or `""`.
    pub suffix: String,
}

/// Whether the file indexer is running, as `balooctl status` says; the
/// `6`-suffixed tool is asked first.
#[must_use]
pub fn probe(service: &dyn Service) -> Option<Tools> {
    for suffix in ["6", ""] {
        let command = Command::new(&format!("balooctl{suffix}"), &["status"]);
        if let Ok(output) = service.output(&command) {
            return output.contains("Indexer is running").then(|| Tools {
                suffix: suffix.to_owned(),
            });
        }
    }
    None
}

/// `baloosearch` with the terms for `q`: `filename:` on each word for a
/// name query, the words as they are for content or both (Baloo matches
/// names and contents by default), under the root with `-d`, at most
/// [`MAX_HITS`] results.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn query_command(q: &Query, tools: &Tools) -> Result<Command, Error> {
    let mut args = vec!["-l".to_owned(), MAX_HITS.to_string()];
    if let Some(root) = &q.root {
        let path = ext::platform::paths::to_path(&root.path)?;
        args.push("-d".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    args.push(terms(q));
    Ok(Command {
        program: format!("baloosearch{}", tools.suffix),
        args,
        stdin: None,
    })
}

/// The search terms: `filename:word` per word for names, the text itself
/// otherwise.
#[must_use]
pub fn terms(q: &Query) -> String {
    match q.kind {
        QueryKind::Name => q
            .text
            .split_whitespace()
            .map(|word| format!("filename:{word}"))
            .collect::<Vec<_>>()
            .join(" "),
        QueryKind::Content | QueryKind::Both => q.text.trim().to_owned(),
    }
}

/// The path a line of `baloosearch` names: an absolute path; the
/// `Elapsed:` footer and blank lines name none.
#[must_use]
pub fn path_of_line(line: &str) -> Option<PathBuf> {
    let line = line.trim_end();
    line.starts_with('/').then(|| PathBuf::from(line))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::service::Fake;

    pub(crate) fn make_present(fake: &Fake) {
        fake.answers(
            "balooctl6",
            Ok("Baloo File Indexer is running\nIndexer state: Idle\nTotal files indexed: 4\n"),
        );
    }

    pub(crate) fn make_absent(fake: &Fake) {
        fake.answers("balooctl6", Ok("Baloo File Indexer is not running\n"));
    }

    pub(crate) fn is_probe(command: &Command) -> bool {
        command.program.starts_with("balooctl")
    }

    /// What `baloosearch` prints for these files.
    pub(crate) fn output_for(paths: &[PathBuf]) -> Vec<String> {
        let mut lines: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        lines.push("Elapsed: 0.155212 msecs".to_owned());
        lines
    }

    pub(crate) fn noise() -> Vec<String> {
        vec!["Elapsed: 0.07 msecs".to_owned(), String::new()]
    }

    pub(crate) fn check_name_command(command: &Command, text: &str, root: Option<&Path>) {
        assert_eq!(command.program, "baloosearch6");
        assert!(command.stdin.is_none());
        let limit = MAX_HITS.to_string();
        let mut expected = vec!["-l", &limit];
        let root_text = root.map(|r| r.to_string_lossy().into_owned());
        if let Some(root) = &root_text {
            expected.push("-d");
            expected.push(root);
        }
        let term = format!("filename:{text}");
        expected.push(&term);
        assert_eq!(command.args, expected, "{command}");
    }

    pub(crate) fn check_kind_command(command: &Command, text: &str, kind: QueryKind) {
        assert!(matches!(kind, QueryKind::Content | QueryKind::Both));
        assert_eq!(
            command.args.last().map(String::as_str),
            Some(text),
            "{command}"
        );
    }

    #[test]
    fn the_probe_tells_frameworks_6_from_5_and_the_search_tool_follows() {
        let fake = Fake::new();
        assert_eq!(probe(&*fake), None, "no balooctl at all");
        fake.answers("balooctl", Ok("Baloo File Indexer is running\n"));
        let tools = probe(&*fake).unwrap_or_else(|| panic!("present"));
        assert_eq!(tools.suffix, "");
        let q = Query {
            text: "foo bar".into(),
            root: None,
            kind: QueryKind::Name,
            filters: Vec::new(),
        };
        let command = query_command(&q, &tools).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(command.program, "baloosearch");
        assert_eq!(
            command.args.last().map(String::as_str),
            Some("filename:foo filename:bar")
        );
        fake.answers("balooctl", Ok("Baloo is currently disabled\n"));
        assert_eq!(probe(&*fake), None);
        make_present(&fake);
        assert_eq!(probe(&*fake).map(|t| t.suffix), Some("6".to_owned()));
    }

    #[test]
    fn only_absolute_paths_are_hits() {
        assert_eq!(
            path_of_line("/a/b c.txt"),
            Some(PathBuf::from("/a/b c.txt"))
        );
        assert_eq!(path_of_line("Elapsed: 1 msecs"), None);
        assert_eq!(path_of_line(""), None);
    }
}
