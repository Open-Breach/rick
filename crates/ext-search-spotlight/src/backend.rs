//! What is particular to Spotlight: the probe through `mdutil`, the
//! `mdfind` command line and the paths it prints.

use std::path::PathBuf;

use ext::{Query, QueryKind};
use types::Error;

use crate::service::{Command, Service};

/// What the probe learnt about the tools: nothing to learn here, the
/// command line is `mdfind`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools;

/// Whether indexing is enabled on the root volume, as `mdutil -s /` says.
#[must_use]
pub fn probe(service: &dyn Service) -> Option<Tools> {
    let output = service.output(&Command::new("mdutil", &["-s", "/"])).ok()?;
    output.contains("Indexing enabled").then_some(Tools)
}

/// `mdfind` for `q`: `-name text` for a name query, the text as a plain
/// query for both (Spotlight matches every attribute, names included), a
/// `kMDItemTextContent` query for contents, under the root with
/// `-onlyin`. `mdfind` has no result limit; the index stops reading at
/// its own cap.
///
/// # Errors
///
/// `Error::Unsupported` when the query's root is not a local location.
pub fn query_command(q: &Query, _tools: &Tools) -> Result<Command, Error> {
    let mut args = Vec::new();
    if let Some(root) = &q.root {
        let path = ext::platform::paths::to_path(&root.path)?;
        args.push("-onlyin".to_owned());
        args.push(path.to_string_lossy().into_owned());
    }
    match q.kind {
        QueryKind::Name => {
            args.push("-name".to_owned());
            args.push(q.text.clone());
        }
        QueryKind::Both => args.push(q.text.clone()),
        QueryKind::Content => args.push(format!(
            "kMDItemTextContent == \"*{}*\"cd",
            q.text.replace('\\', "\\\\").replace('"', "\\\"")
        )),
    }
    Ok(Command {
        program: "mdfind".to_owned(),
        args,
        stdin: None,
    })
}

/// The path a line of `mdfind` names: an absolute path; anything else
/// names none.
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
        fake.answers("mdutil", Ok("/:\n\tIndexing enabled.\n"));
    }

    pub(crate) fn make_absent(fake: &Fake) {
        fake.answers("mdutil", Ok("/:\n\tIndexing disabled.\n"));
    }

    pub(crate) fn is_probe(command: &Command) -> bool {
        command.program == "mdutil"
    }

    /// What `mdfind` prints for these files.
    pub(crate) fn output_for(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    pub(crate) fn noise() -> Vec<String> {
        vec![String::new(), "mdfind: nothing".to_owned()]
    }

    pub(crate) fn check_name_command(command: &Command, text: &str, root: Option<&Path>) {
        assert_eq!(command.program, "mdfind");
        assert!(command.stdin.is_none());
        let mut expected = Vec::new();
        let root_text = root.map(|r| r.to_string_lossy().into_owned());
        if let Some(root) = &root_text {
            expected.push("-onlyin");
            expected.push(root);
        }
        expected.push("-name");
        expected.push(text);
        assert_eq!(command.args, expected, "{command}");
    }

    pub(crate) fn check_kind_command(command: &Command, text: &str, kind: QueryKind) {
        match kind {
            QueryKind::Both => assert_eq!(command.args, [text], "{command}"),
            QueryKind::Content => assert_eq!(
                command.args,
                [format!("kMDItemTextContent == \"*{text}*\"cd")],
                "{command}"
            ),
            QueryKind::Name => unreachable!(),
        }
    }

    #[test]
    fn a_content_query_escapes_quotes_and_the_probe_needs_mdutil() {
        let q = Query {
            text: "say \"hi\"".into(),
            root: None,
            kind: QueryKind::Content,
            filters: Vec::new(),
        };
        let command = query_command(&q, &Tools).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            command.args,
            ["kMDItemTextContent == \"*say \\\"hi\\\"*\"cd"]
        );
        let fake = Fake::new();
        assert_eq!(probe(&*fake), None);
        fake.answers("mdutil", Err("mdutil: not permitted"));
        assert_eq!(probe(&*fake), None);
        assert_eq!(
            path_of_line("/Users/me/a.txt"),
            Some(PathBuf::from("/Users/me/a.txt"))
        );
        assert_eq!(path_of_line("nothing"), None);
    }
}
