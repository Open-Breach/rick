//! What is particular to Windows Search: the probe through the service
//! controller, the Windows Search SQL run through OLE DB (the
//! `Search.CollatorDSO` provider, reached from a PowerShell script over
//! ADODB, since a plugin may not use the COM interfaces directly:
//! QUESTIONS.md 120), and the paths the script prints.

use std::path::PathBuf;

use ext::{Query, QueryKind};
use types::Error;

use crate::MAX_HITS;
use crate::service::{Command, Service};

/// The Windows Search service.
pub const SERVICE: &str = "WSearch";

/// What the probe learnt about the tools: nothing to learn here, the
/// script runs through `powershell.exe`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools;

/// Whether the Windows Search service is running, as `sc.exe query` says.
#[must_use]
pub fn probe(service: &dyn Service) -> Option<Tools> {
    let output = service
        .output(&Command::new("sc.exe", &["query", SERVICE]))
        .ok()?;
    output.contains("RUNNING").then_some(Tools)
}

/// PowerShell reading the query script from its standard input: an ADODB
/// connection to the `Search.CollatorDSO` OLE DB provider runs the SQL for
/// `q` and prints one `System.ItemPathDisplay` per line.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn query_command(q: &Query, _tools: &Tools) -> Result<Command, Error> {
    Ok(Command {
        program: "powershell.exe".to_owned(),
        args: ["-NoProfile", "-NonInteractive", "-Command", "-"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect(),
        stdin: Some(script(&sql(q)?)),
    })
}

/// The Windows Search SQL for a query: the display paths of the items
/// whose file name contains the text, whose content contains it, or
/// either, within the root's scope when there is one, the first
/// [`MAX_HITS`] of them.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn sql(q: &Query) -> Result<String, Error> {
    let by_name = format!("System.FileName LIKE '%{}%'", like_escape(&q.text));
    let by_content = format!("CONTAINS(*, '\"{}*\"')", contains_escape(&q.text));
    let what = match q.kind {
        QueryKind::Name => by_name,
        QueryKind::Content => by_content,
        QueryKind::Both => format!("({by_name} OR {by_content})"),
    };
    let scope = match &q.root {
        Some(root) => {
            let path = ext::platform::paths::to_path(&root.path)?;
            format!(
                "SCOPE='file:{}' AND ",
                path.to_string_lossy()
                    .replace('\\', "/")
                    .replace('\'', "''")
            )
        }
        None => String::new(),
    };
    Ok(format!(
        "SELECT TOP {MAX_HITS} System.ItemPathDisplay FROM SYSTEMINDEX WHERE {scope}{what}"
    ))
}

/// `text` inside a SQL `LIKE '%…%'`: quotes doubled, the wildcards and
/// the bracket that opens a class bracketed.
#[must_use]
pub fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '%' | '_' | '[' => {
                out.push('[');
                out.push(c);
                out.push(']');
            }
            other => out.push(other),
        }
    }
    out
}

/// `text` inside a `CONTAINS('"…*"')` phrase: quotes doubled for SQL,
/// double quotes dropped since they would end the phrase.
#[must_use]
pub fn contains_escape(text: &str) -> String {
    text.replace('\'', "''").replace('"', "")
}

/// The PowerShell that runs `sql` and prints the paths. The SQL travels
/// in a here-string, so nothing in it is interpreted.
#[must_use]
pub fn script(sql: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         try {{\n\
         \x20 $conn = New-Object -ComObject ADODB.Connection\n\
         \x20 $conn.Open(\"Provider=Search.CollatorDSO;Extended Properties='Application=Windows';\")\n\
         \x20 $rs = $conn.Execute(@'\n{sql}\n'@)\n\
         \x20 while (-not $rs.EOF) {{\n\
         \x20   [Console]::Out.WriteLine([string]$rs.Fields.Item('System.ItemPathDisplay').Value)\n\
         \x20   $rs.MoveNext()\n\
         \x20 }}\n\
         \x20 $rs.Close()\n\
         \x20 $conn.Close()\n\
         }} catch {{\n\
         \x20 [Console]::Error.WriteLine($_.Exception.Message)\n\
         \x20 exit 1\n\
         }}\n"
    )
}

/// The path a line of the script names: a drive path (`C:\…`), a UNC path
/// (`\\server\…`) or, so the connector's tests run on every platform, a
/// slash-rooted one; anything else names none.
#[must_use]
pub fn path_of_line(line: &str) -> Option<PathBuf> {
    let line = line.trim_end();
    let bytes = line.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = line.starts_with("\\\\");
    let rooted = line.starts_with('/');
    (drive || unc || rooted).then(|| PathBuf::from(line))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::service::Fake;

    pub(crate) fn make_present(fake: &Fake) {
        fake.answers(
            "sc.exe",
            Ok("SERVICE_NAME: WSearch\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 4  RUNNING\n"),
        );
    }

    pub(crate) fn make_absent(fake: &Fake) {
        fake.answers(
            "sc.exe",
            Ok("SERVICE_NAME: WSearch\n        STATE              : 1  STOPPED\n"),
        );
    }

    pub(crate) fn is_probe(command: &Command) -> bool {
        command.program == "sc.exe"
    }

    /// What the script prints for these files.
    pub(crate) fn output_for(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    pub(crate) fn noise() -> Vec<String> {
        vec![String::new(), "WARNING: something".to_owned()]
    }

    fn sql_of(command: &Command) -> String {
        assert_eq!(command.program, "powershell.exe");
        assert_eq!(
            command.args,
            ["-NoProfile", "-NonInteractive", "-Command", "-"]
        );
        let script = command
            .stdin
            .as_deref()
            .unwrap_or_else(|| panic!("no script"));
        assert!(script.contains("Provider=Search.CollatorDSO"), "{script}");
        assert!(script.contains("System.ItemPathDisplay"), "{script}");
        let start = script.find("@'\n").unwrap_or_else(|| panic!("{script}")) + 3;
        let end = script[start..]
            .find("\n'@")
            .unwrap_or_else(|| panic!("{script}"));
        script[start..start + end].to_owned()
    }

    pub(crate) fn check_name_command(command: &Command, text: &str, root: Option<&Path>) {
        let sql = sql_of(command);
        assert!(
            sql.starts_with(&format!(
                "SELECT TOP {MAX_HITS} System.ItemPathDisplay FROM SYSTEMINDEX WHERE "
            )),
            "{sql}"
        );
        assert!(
            sql.ends_with(&format!("System.FileName LIKE '%{text}%'")),
            "{sql}"
        );
        match root {
            Some(root) => {
                let scope = root.to_string_lossy().replace('\\', "/");
                assert!(sql.contains(&format!("SCOPE='file:{scope}' AND ")), "{sql}");
            }
            None => assert!(!sql.contains("SCOPE="), "{sql}"),
        }
    }

    pub(crate) fn check_kind_command(command: &Command, text: &str, kind: QueryKind) {
        let sql = sql_of(command);
        let by_content = format!("CONTAINS(*, '\"{text}*\"')");
        match kind {
            QueryKind::Content => {
                assert!(sql.contains(&by_content), "{sql}");
                assert!(!sql.contains("LIKE"), "{sql}");
            }
            QueryKind::Both => {
                assert!(sql.contains(&by_content), "{sql}");
                assert!(sql.contains(" OR "), "{sql}");
                assert!(sql.contains("LIKE"), "{sql}");
            }
            QueryKind::Name => unreachable!(),
        }
    }

    #[test]
    fn the_sql_is_escaped_and_only_path_lines_are_hits() {
        assert_eq!(like_escape("100%_[x]'y"), "100[%][_][[]x]''y");
        assert_eq!(contains_escape("it's \"x\""), "it''s x");
        assert_eq!(
            path_of_line("C:\\Users\\me\\a.txt"),
            Some(PathBuf::from("C:\\Users\\me\\a.txt"))
        );
        assert_eq!(
            path_of_line("\\\\server\\share\\a.txt"),
            Some(PathBuf::from("\\\\server\\share\\a.txt"))
        );
        assert_eq!(path_of_line("WARNING: x"), None);
        assert_eq!(path_of_line(""), None);
        let fake = Fake::new();
        assert_eq!(probe(&*fake), None, "no service controller");
        make_absent(&fake);
        assert_eq!(probe(&*fake), None);
        make_present(&fake);
        assert_eq!(probe(&*fake), Some(Tools));
    }
}
