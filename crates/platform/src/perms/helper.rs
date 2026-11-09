//! The protocol between Rick and `rick-perms`, the privileged helper: a
//! JSON request file naming the entries and their changes, a JSON reply
//! on standard output with what each entry had before the change (for
//! undo) and what failed. The helper runs as the administrator through
//! [`Elevate`](crate::Elevate), so it validates everything it reads and
//! does nothing but read and set permissions.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use types::Error;

use super::{FilePermissions, PermissionChange, Permissions};

/// One entry to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// The entry.
    pub path: PathBuf,
    /// What to change on it.
    pub change: PermissionChange,
}

/// What Rick asks the helper to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// The entries, in order.
    pub items: Vec<Item>,
}

/// An entry the helper changed, with what it had before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    /// The entry.
    pub path: PathBuf,
    /// Its permissions before the change.
    pub previous: FilePermissions,
}

/// An entry the helper could not change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    /// The entry.
    pub path: PathBuf,
    /// Why.
    pub error: String,
}

/// What the helper answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// The entries changed.
    pub done: Vec<Done>,
    /// The entries that failed.
    pub failed: Vec<Failed>,
}

/// Writes `request` to a temporary file the helper can read even as another
/// user: world-readable, in the system's temporary directory. The file is
/// removed when the returned guard drops.
///
/// # Errors
///
/// The I/O error from writing the file.
pub fn write_request(request: &Request) -> Result<tempfile::NamedTempFile, Error> {
    let mut file = tempfile::Builder::new()
        .prefix("rick-perms-")
        .suffix(".json")
        .tempfile()
        .map_err(Error::from_io)?;
    let text = serde_json::to_vec(request).map_err(|e| Error::Io(std::io::Error::other(e)))?;
    file.write_all(&text).map_err(Error::from_io)?;
    file.flush().map_err(Error::from_io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o644))
            .map_err(Error::from_io)?;
    }
    Ok(file)
}

/// Reads a request file.
///
/// # Errors
///
/// The I/O error, or an error for a file that is not a request.
pub fn read_request(path: &Path) -> Result<Request, Error> {
    let text = std::fs::read(path).map_err(Error::from_io)?;
    serde_json::from_slice(&text).map_err(|e| Error::Io(std::io::Error::other(e)))
}

/// Parses the helper's reply.
///
/// # Errors
///
/// An error for output that is not a reply.
pub fn parse_reply(stdout: &[u8]) -> Result<Reply, Error> {
    serde_json::from_slice(stdout).map_err(|e| {
        Error::Io(std::io::Error::other(format!(
            "the privileged helper answered something else: {e}"
        )))
    })
}

/// The reply as the helper prints it.
///
/// # Errors
///
/// An error for a reply that does not serialise, which does not happen.
pub fn format_reply(reply: &Reply) -> Result<String, Error> {
    serde_json::to_string(reply).map_err(|e| Error::Io(std::io::Error::other(e)))
}

/// What the helper does with a request: records each entry's permissions,
/// applies the change, and goes on past an entry that fails.
#[must_use]
pub fn apply(platform: &dyn Permissions, request: &Request) -> Reply {
    let mut reply = Reply::default();
    for item in &request.items {
        let previous = match platform.permissions(&item.path) {
            Ok(previous) => previous,
            Err(error) => {
                reply.failed.push(Failed {
                    path: item.path.clone(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        match platform.set_permissions(&item.path, &item.change) {
            Ok(()) => reply.done.push(Done {
                path: item.path.clone(),
                previous,
            }),
            Err(error) => reply.failed.push(Failed {
                path: item.path.clone(),
                error: error.to_string(),
            }),
        }
    }
    reply
}

/// The helper's main: `rick-perms apply <request-file> [--output <file>]`
/// prints the reply (and writes it to the file, where an elevated process
/// has no standard output to speak of: Windows) and exits 0; anything else
/// is usage, exit 2; a request that does not read is exit 1.
#[must_use]
pub fn main(platform: &dyn Permissions, args: &[String]) -> i32 {
    let usage = || {
        eprintln!(
            "usage: {} apply <request.json> [--output <reply.json>]",
            super::HELPER_NAME
        );
        2
    };
    let (path, output) = match args {
        [verb, path] if verb == "apply" => (path, None),
        [verb, path, flag, output] if verb == "apply" && flag == "--output" => (path, Some(output)),
        _ => return usage(),
    };
    let request = match read_request(Path::new(path)) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("{}: {error}", super::HELPER_NAME);
            return 1;
        }
    };
    let reply = apply(platform, &request);
    let text = match format_reply(&reply) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{}: {error}", super::HELPER_NAME);
            return 1;
        }
    };
    if let Some(output) = output
        && let Err(error) = std::fs::write(output, &text)
    {
        eprintln!("{}: {error}", super::HELPER_NAME);
        return 1;
    }
    println!("{text}");
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perms::Principal;

    #[test]
    fn a_request_round_trips_through_the_file_and_the_reply_through_text() {
        let request = Request {
            items: vec![Item {
                path: PathBuf::from("/tmp/x"),
                change: PermissionChange {
                    owner: Some(Principal::named("0", "root")),
                    mode: Some(0o644),
                    ..PermissionChange::default()
                },
            }],
        };
        let file = write_request(&request).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            read_request(file.path()).unwrap_or_else(|e| panic!("{e}")),
            request
        );
        let reply = Reply {
            done: vec![Done {
                path: PathBuf::from("/tmp/x"),
                previous: FilePermissions {
                    owner: Principal::named("1000", "me"),
                    group: Principal::id("1000"),
                    mode: Some(0o600),
                    acl: None,
                    is_dir: false,
                },
            }],
            failed: vec![Failed {
                path: PathBuf::from("/tmp/y"),
                error: "not found".to_owned(),
            }],
        };
        let text = format_reply(&reply).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            parse_reply(text.as_bytes()).unwrap_or_else(|e| panic!("{e}")),
            reply
        );
        assert!(parse_reply(b"nonsense").is_err());
    }

    #[test]
    fn usage_errors_exit_two() {
        struct Nothing;
        impl Permissions for Nothing {
            fn permissions(&self, _: &Path) -> Result<FilePermissions, Error> {
                Err(Error::NotFound)
            }
            fn set_permissions(&self, _: &Path, _: &PermissionChange) -> Result<(), Error> {
                Err(Error::NotFound)
            }
            fn principals(&self) -> Result<super::super::Principals, Error> {
                Ok(super::super::Principals::default())
            }
            fn principal(
                &self,
                _: &str,
                _: super::super::PrincipalKind,
            ) -> Result<Principal, Error> {
                Err(Error::NotFound)
            }
        }
        assert_eq!(main(&Nothing, &[]), 2);
        assert_eq!(main(&Nothing, &["frob".to_owned(), "x".to_owned()]), 2);
        assert_eq!(
            main(
                &Nothing,
                &["apply".to_owned(), "/nonexistent/request".to_owned()]
            ),
            1
        );
    }
}
