//! What is particular to Tracker: the probe over the session bus, the
//! SPARQL the `tracker3` command line sends the miner over D-Bus, and the
//! `file://` URIs it prints.

use std::path::PathBuf;

use ext::{Query, QueryKind};
use types::{Error, VPath};

use crate::MAX_HITS;
use crate::service::{Command, Service};

/// The miner's D-Bus name: the SPARQL endpoint the queries go to.
pub const SERVICE: &str = "org.freedesktop.Tracker3.Miner.Files";

/// What the probe learnt about the tools: nothing to learn here, the
/// command line is `tracker3`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools;

/// Whether the miner owns its name on the session bus: asked through
/// `gdbus`, or `busctl` where there is no `gdbus`. A name without an
/// owner means Tracker is not running, even when it is installed; the
/// first query would start it by activation, so the box waits for the
/// desktop to start it.
#[must_use]
pub fn probe(service: &dyn Service) -> Option<Tools> {
    for command in [gdbus_has_owner(), busctl_has_owner()] {
        if let Ok(output) = service.output(&command) {
            return output.contains("true").then_some(Tools);
        }
    }
    None
}

fn gdbus_has_owner() -> Command {
    Command::new(
        "gdbus",
        &[
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            SERVICE,
        ],
    )
}

fn busctl_has_owner() -> Command {
    Command::new(
        "busctl",
        &[
            "--user",
            "call",
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            "s",
            SERVICE,
        ],
    )
}

/// `tracker3 sparql` against the miner, with the SPARQL for `q`.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn query_command(q: &Query, _tools: &Tools) -> Result<Command, Error> {
    Ok(Command::new(
        "tracker3",
        &[
            "sparql",
            &format!("--dbus-service={SERVICE}"),
            "-q",
            &sparql(q)?,
        ],
    ))
}

/// The SPARQL for a query: the URLs of the files whose name contains the
/// text (lower-cased on both sides), whose content matches it through
/// the full-text index, or either, under the root's `file://` URI when
/// there is one, at most [`MAX_HITS`] of them.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn sparql(q: &Query) -> Result<String, Error> {
    let by_name = format!(
        "?f a nfo:FileDataObject ; nie:url ?u ; nfo:fileName ?n . FILTER (CONTAINS(LCASE(?n), {}))",
        literal(&q.text.to_lowercase())
    );
    let by_content = format!(
        "?ie fts:match {} ; nie:isStoredAs ?f . ?f nie:url ?u",
        literal(&q.text)
    );
    let what = match q.kind {
        QueryKind::Name => by_name,
        QueryKind::Content => by_content,
        QueryKind::Both => format!("{{ {by_name} }} UNION {{ {by_content} }}"),
    };
    let scope = match &q.root {
        Some(root) => {
            let path = ext::platform::paths::to_path(&root.path)?;
            let uri = ext::platform::paths::file_uri(&path);
            format!(
                " FILTER (STRSTARTS(STR(?u), {}))",
                literal(&format!("{}/", uri.trim_end_matches('/')))
            )
        }
        None => String::new(),
    };
    Ok(format!(
        "SELECT DISTINCT ?u WHERE {{ {what}{scope} }} LIMIT {MAX_HITS}"
    ))
}

/// `text` as a SPARQL string literal.
#[must_use]
pub fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The path a line of `tracker3 sparql` names: an indented `file://` URI,
/// percent-decoded; the `Results:` header, `None` and blank lines name
/// none.
#[must_use]
pub fn path_of_line(line: &str) -> Option<PathBuf> {
    let uri = line.trim().strip_prefix("file://")?;
    // `file:///home/x`: no host; `file://host/x` is not a local path.
    let path = uri.strip_prefix('/')?;
    let bytes = percent_decode(&format!("/{path}"));
    ext::platform::paths::to_path(&VPath::from_bytes(bytes)).ok()
}

/// Decodes `%XX` escapes byte by byte; a malformed escape stays as it is.
fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::service::Fake;

    pub(crate) fn make_present(fake: &Fake) {
        fake.answers("gdbus", Ok("(true,)\n"));
    }

    pub(crate) fn make_absent(fake: &Fake) {
        fake.answers("gdbus", Ok("(false,)\n"));
    }

    pub(crate) fn is_probe(command: &Command) -> bool {
        command.program == "gdbus" || command.program == "busctl"
    }

    /// What `tracker3 sparql` prints for these files.
    pub(crate) fn output_for(paths: &[PathBuf]) -> Vec<String> {
        let mut lines = vec!["Results:".to_owned()];
        lines.extend(
            paths
                .iter()
                .map(|p| format!("  {}", ext::platform::paths::file_uri(p))),
        );
        lines.push(String::new());
        lines
    }

    pub(crate) fn noise() -> Vec<String> {
        vec!["Results:".to_owned(), "  None".to_owned(), String::new()]
    }

    fn sparql_of(command: &Command) -> &str {
        assert_eq!(command.program, "tracker3");
        assert_eq!(command.args[0], "sparql");
        assert_eq!(command.args[1], format!("--dbus-service={SERVICE}"));
        assert_eq!(command.args[2], "-q");
        assert!(command.stdin.is_none());
        &command.args[3]
    }

    pub(crate) fn check_name_command(command: &Command, text: &str, root: Option<&Path>) {
        let sparql = sparql_of(command);
        assert!(
            sparql.contains(&format!("CONTAINS(LCASE(?n), \"{text}\")")),
            "{sparql}"
        );
        assert!(sparql.ends_with(&format!(" LIMIT {MAX_HITS}")), "{sparql}");
        match root {
            Some(root) => {
                let uri = ext::platform::paths::file_uri(root);
                assert!(
                    sparql.contains(&format!("STRSTARTS(STR(?u), \"{uri}/\")")),
                    "{sparql}"
                );
            }
            None => assert!(!sparql.contains("STRSTARTS"), "{sparql}"),
        }
    }

    pub(crate) fn check_kind_command(command: &Command, text: &str, kind: QueryKind) {
        let sparql = sparql_of(command);
        let by_content = format!("fts:match \"{text}\"");
        match kind {
            QueryKind::Content => {
                assert!(sparql.contains(&by_content), "{sparql}");
                assert!(!sparql.contains("nfo:fileName"), "{sparql}");
            }
            QueryKind::Both => {
                assert!(sparql.contains(&by_content), "{sparql}");
                assert!(sparql.contains("nfo:fileName"), "{sparql}");
                assert!(sparql.contains("UNION"), "{sparql}");
            }
            QueryKind::Name => unreachable!(),
        }
    }

    #[test]
    fn the_probe_falls_back_to_busctl_and_reads_either_answer() {
        let fake = Fake::new();
        assert_eq!(probe(&*fake), None, "no bus tool at all");
        fake.answers("busctl", Ok("b true\n"));
        assert_eq!(probe(&*fake), Some(Tools));
        fake.answers("busctl", Ok("b false\n"));
        assert_eq!(probe(&*fake), None);
        fake.answers("gdbus", Ok("(true,)\n"));
        assert_eq!(probe(&*fake), Some(Tools), "gdbus wins when both are there");
        let programs: Vec<String> = fake.commands().iter().map(|c| c.program.clone()).collect();
        assert_eq!(
            programs,
            [
                "gdbus", "busctl", "gdbus", "busctl", "gdbus", "busctl", "gdbus"
            ]
        );
    }

    #[test]
    fn literals_are_escaped_and_uris_decoded() {
        assert_eq!(literal(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(literal("x\ny"), "\"x\\ny\"");
        assert_eq!(
            path_of_line("  file:///home/me/a%20b%2Fc.txt"),
            Some(PathBuf::from("/home/me/a b/c.txt"))
        );
        assert_eq!(path_of_line("file:///x%zz"), Some(PathBuf::from("/x%zz")));
        assert_eq!(
            path_of_line("file://server/share"),
            None,
            "not a local path"
        );
        assert_eq!(path_of_line("Results:"), None);
        assert_eq!(path_of_line("  None"), None);
        assert_eq!(path_of_line(""), None);
    }
}
