//! Shell quoting and the fixed command templates every remote operation is
//! built from. User input (a path, a name) is never interpolated raw: it
//! goes through [`quote`], which wraps it in single quotes the way a POSIX
//! shell reads them, so a name with spaces, newlines, quotes or bytes that
//! are no UTF-8 survives the trip through `sh -c` unchanged.

/// `bytes` as one word for a POSIX shell: single-quoted, with every single
/// quote inside written as `'\''` (end the quotes, an escaped quote, start
/// them again). The empty string is `''`.
#[must_use]
pub fn quote(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'\'');
    for &b in bytes {
        if b == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(b);
        }
    }
    out.push(b'\'');
    out
}

/// A command line for the remote shell, built from a template and quoted
/// arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Command(Vec<u8>);

impl Command {
    /// A command starting with `program`, a fixed word of the template.
    #[must_use]
    pub fn new(program: &str) -> Self {
        Self(program.as_bytes().to_vec())
    }

    /// Appends a fixed part of the template: an option, an operator, a
    /// pipe. Never user input.
    #[must_use]
    pub fn raw(mut self, text: &str) -> Self {
        self.0.extend_from_slice(text.as_bytes());
        self
    }

    /// Appends a space and a quoted argument.
    #[must_use]
    pub fn arg(mut self, bytes: &[u8]) -> Self {
        self.0.push(b' ');
        self.0.extend_from_slice(&quote(bytes));
        self
    }

    /// Appends a space and a quoted text argument.
    #[must_use]
    pub fn arg_str(self, text: &str) -> Self {
        self.arg(text.as_bytes())
    }

    /// The command line as the shell gets it.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// The command line as bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// `dir/name` with one separator, for a remote path and a name from a
/// listing.
#[must_use]
pub fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    let mut out = dir.to_vec();
    if !out.ends_with(b"/") {
        out.push(b'/');
    }
    out.extend_from_slice(name);
    out
}

/// The parent of a remote path: `/a/b` is `/a`, `/a` and `/` are `/`.
#[must_use]
pub fn parent(path: &[u8]) -> &[u8] {
    let trimmed = trim_trailing_slashes(path);
    match trimmed.iter().rposition(|b| *b == b'/') {
        Some(0) | None => b"/",
        Some(i) => &trimmed[..i],
    }
}

/// The last component of a remote path, empty for the root.
#[must_use]
pub fn file_name(path: &[u8]) -> &[u8] {
    let trimmed = trim_trailing_slashes(path);
    match trimmed.iter().rposition(|b| *b == b'/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

fn trim_trailing_slashes(path: &[u8]) -> &[u8] {
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }
    &path[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_every_byte_a_shell_reads() {
        assert_eq!(quote(b""), b"''");
        assert_eq!(quote(b"plain"), b"'plain'");
        assert_eq!(quote(b"with space"), b"'with space'");
        assert_eq!(quote(b"it's"), b"'it'\\''s'");
        assert_eq!(quote(b"new\nline"), b"'new\nline'");
        assert_eq!(quote(b"$HOME `id` \\"), b"'$HOME `id` \\'");
        assert_eq!(quote(&[0xff, 0xfe]), [b'\'', 0xff, 0xfe, b'\'']);
    }

    #[test]
    fn commands_are_templates_with_quoted_arguments() {
        let command = Command::new("cat")
            .raw(" --")
            .arg(b"/srv/it's here/a b.txt")
            .raw(" | head -c 10");
        assert_eq!(
            command.as_bytes(),
            b"cat -- '/srv/it'\\''s here/a b.txt' | head -c 10"
        );
        assert_eq!(
            Command::new("mkdir").raw(" --").arg_str("/x").into_bytes(),
            b"mkdir -- '/x'"
        );
    }

    #[test]
    fn remote_paths_join_and_split() {
        assert_eq!(join(b"/", b"a"), b"/a");
        assert_eq!(join(b"/srv", b"a b"), b"/srv/a b");
        assert_eq!(join(b"/srv/", b"a"), b"/srv/a");
        assert_eq!(parent(b"/srv/a"), b"/srv");
        assert_eq!(parent(b"/srv/a/"), b"/srv");
        assert_eq!(parent(b"/a"), b"/");
        assert_eq!(parent(b"/"), b"/");
        assert_eq!(file_name(b"/srv/a b"), b"a b");
        assert_eq!(file_name(b"/srv/a/"), b"a");
        assert_eq!(file_name(b"/"), b"");
    }
}
