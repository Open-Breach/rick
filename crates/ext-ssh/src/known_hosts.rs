//! `~/.ssh/known_hosts` with OpenSSH semantics: plain and hashed host
//! names, port-qualified names, wildcard patterns and negations,
//! `@revoked` markers (`@cert-authority` lines are no host keys and are
//! skipped). Nothing here decides anything: [`check`] says whether a key is
//! known, unknown, changed or revoked, and the connection asks the user
//! what to do with the two latter through the two dialogues of the spec.

use std::io::Write as _;
use std::path::Path;

use base64::Engine as _;
use hmac::{Hmac, Mac as _};
use russh::keys::{HashAlg, PublicKey};
use sha1::Sha1;

use crate::config::glob_matches;

/// What `known_hosts` says about a server's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The key is on file for this host and port.
    Known,
    /// The host is not on file, or has no key of this type on file.
    Unknown,
    /// The host is on file with another key of the same type: the warning
    /// case.
    Changed {
        /// The line of the entry that disagrees, counted from one.
        line: usize,
        /// The SHA-256 fingerprint of the key on file.
        old_fingerprint: String,
    },
    /// The key is marked `@revoked` for this host.
    Revoked {
        /// The line of the marker, counted from one.
        line: usize,
    },
}

/// The name a host and port are looked up and recorded under: the bare
/// name on port 22, `[name]:port` otherwise, as OpenSSH writes them.
#[must_use]
pub fn host_pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Whether one host pattern of an entry (`host`, `*.example.com`,
/// `|1|salt|hash`) names `wanted`.
fn pattern_matches(pattern: &str, wanted: &str) -> bool {
    if let Some(hashed) = pattern.strip_prefix("|1|") {
        let Some((salt, hash)) = hashed.split_once('|') else {
            return false;
        };
        let engine = base64::engine::general_purpose::STANDARD;
        let (Ok(salt), Ok(hash)) = (engine.decode(salt), engine.decode(hash)) else {
            return false;
        };
        let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
            return false;
        };
        mac.update(wanted.as_bytes());
        return mac.verify_slice(&hash).is_ok();
    }
    glob_matches(pattern, wanted)
}

/// Whether an entry's comma-separated patterns name `wanted`: one positive
/// match and no negated one.
fn entry_matches(patterns: &str, wanted: &str) -> bool {
    let mut positive = false;
    for pattern in patterns.split(',') {
        if let Some(negated) = pattern.strip_prefix('!') {
            if pattern_matches(negated, wanted) {
                return false;
            }
        } else if pattern_matches(pattern, wanted) {
            positive = true;
        }
    }
    positive
}

/// One line of the file taken apart.
struct Entry<'a> {
    revoked: bool,
    patterns: &'a str,
    key_type: &'a str,
    key: &'a str,
}

/// Parses a line; `None` for a blank, a comment, a `@cert-authority` line
/// or anything that is not an entry.
fn parse_line(line: &str) -> Option<Entry<'_>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut words = line.split_whitespace();
    let mut first = words.next()?;
    let mut revoked = false;
    if let Some(marker) = first.strip_prefix('@') {
        match marker {
            "revoked" => revoked = true,
            _ => return None,
        }
        first = words.next()?;
    }
    let key_type = words.next()?;
    let key = words.next()?;
    Some(Entry {
        revoked,
        patterns: first,
        key_type,
        key,
    })
}

/// A public key from its type and base64 as the file holds them.
fn parse_key(key_type: &str, key: &str) -> Option<PublicKey> {
    PublicKey::from_openssh(&format!("{key_type} {key}")).ok()
}

/// The SHA-256 fingerprint of a key, as `ssh` prints it.
#[must_use]
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// What the file at `path` says about `key` for `host` and `port`. A
/// missing file knows nothing.
///
/// # Errors
///
/// The I/O error of reading the file; a missing file is no error.
pub fn check(path: &Path, host: &str, port: u16, key: &PublicKey) -> std::io::Result<Verdict> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Verdict::Unknown),
        Err(error) => return Err(error),
    };
    Ok(check_text(&text, host, port, key))
}

/// [`check`] over the file's text.
#[must_use]
pub fn check_text(text: &str, host: &str, port: u16, key: &PublicKey) -> Verdict {
    let wanted = host_pattern(host, port);
    let algorithm = key.algorithm();
    let mut changed: Option<(usize, String)> = None;
    for (index, line) in text.lines().enumerate() {
        let Some(entry) = parse_line(line) else {
            continue;
        };
        if !entry_matches(entry.patterns, &wanted) {
            continue;
        }
        let Some(recorded) = parse_key(entry.key_type, entry.key) else {
            continue;
        };
        if recorded.algorithm() != algorithm {
            continue;
        }
        let same = recorded.key_data() == key.key_data();
        if entry.revoked {
            if same {
                return Verdict::Revoked { line: index + 1 };
            }
            continue;
        }
        if same {
            return Verdict::Known;
        }
        if changed.is_none() {
            changed = Some((index + 1, fingerprint(&recorded)));
        }
    }
    match changed {
        Some((line, old_fingerprint)) => Verdict::Changed {
            line,
            old_fingerprint,
        },
        None => Verdict::Unknown,
    }
}

/// Appends an entry for `key` under `host` and `port`, creating the file
/// and its directory.
///
/// # Errors
///
/// The I/O error.
pub fn append(path: &Path, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    let openssh = key
        .to_openssh()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writeln!(file, "{} {openssh}", host_pattern(host, port))?;
    Ok(())
}

/// Replaces the entries that disagree with `key` for `host` and `port`
/// (the lines of the same key type that name the host, hashed or not) with
/// one for `key`: what *Replace* does after the warning dialogue. Entries
/// for other hosts on the same line keep their line without this host.
///
/// # Errors
///
/// The I/O error.
pub fn replace(path: &Path, host: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let wanted = host_pattern(host, port);
    let mut kept = String::with_capacity(text.len());
    for line in text.lines() {
        let keep = match parse_line(line) {
            Some(entry)
                if !entry.revoked
                    && entry_matches(entry.patterns, &wanted)
                    && parse_key(entry.key_type, entry.key)
                        .is_some_and(|k| k.algorithm() == key.algorithm()) =>
            {
                // Drop this host from the line; keep the line for the rest.
                let rest: Vec<&str> = entry
                    .patterns
                    .split(',')
                    .filter(|p| !pattern_matches(p.trim_start_matches('!'), &wanted))
                    .collect();
                if rest.is_empty() || rest.iter().all(|p| p.starts_with('!')) {
                    None
                } else {
                    Some(format!(
                        "{} {} {}",
                        rest.join(","),
                        entry.key_type,
                        entry.key
                    ))
                }
            }
            _ => Some(line.to_owned()),
        };
        if let Some(line) = keep {
            kept.push_str(&line);
            kept.push('\n');
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, kept)?;
    append(path, host, port, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED25519_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ";
    const ED25519_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA6rWI3G1sz07DnfFlrouTcysQlj2P+jpNSOEWD9OJ3X";
    const ED25519_C: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF";

    fn key(text: &str) -> PublicKey {
        PublicKey::from_openssh(text).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn plain_hashed_ported_and_wildcard_entries_match() {
        let text = format!(
            "# comment\n\
             [localhost]:13265 {ED25519_A}\n\
             pijul.org,37.120.161.53 {ED25519_B}\n\
             |1|O33ESRMWPVkMYIwJ1Uw+n877jTo=|nuuC5vEqXlEZ/8BXQR7m619W6Ak= {ED25519_C}\n\
             *.example.net,!bad.example.net {ED25519_A}\n\
             @cert-authority *.corp {ED25519_B}\n"
        );
        assert_eq!(
            check_text(&text, "localhost", 13265, &key(ED25519_A)),
            Verdict::Known
        );
        assert_eq!(
            check_text(&text, "localhost", 22, &key(ED25519_A)),
            Verdict::Unknown,
            "a port-qualified entry says nothing about port 22"
        );
        assert_eq!(
            check_text(&text, "37.120.161.53", 22, &key(ED25519_B)),
            Verdict::Known
        );
        assert_eq!(
            check_text(&text, "example.com", 22, &key(ED25519_C)),
            Verdict::Known,
            "the hashed entry"
        );
        assert_eq!(
            check_text(&text, "web.example.net", 22, &key(ED25519_A)),
            Verdict::Known
        );
        assert_eq!(
            check_text(&text, "bad.example.net", 22, &key(ED25519_A)),
            Verdict::Unknown,
            "negated"
        );
        assert_eq!(
            check_text(&text, "ca.corp", 22, &key(ED25519_B)),
            Verdict::Unknown,
            "a certificate authority is no host key"
        );
        assert!(matches!(
            check_text(&text, "pijul.org", 22, &key(ED25519_A)),
            Verdict::Changed { line: 3, ref old_fingerprint } if old_fingerprint.starts_with("SHA256:")
        ));
    }

    #[test]
    fn a_revoked_key_is_refused_and_another_type_is_unknown() {
        let text = format!("@revoked host {ED25519_A}\nhost {ED25519_B}\n");
        assert_eq!(
            check_text(&text, "host", 22, &key(ED25519_A)),
            Verdict::Revoked { line: 1 }
        );
        assert_eq!(
            check_text(&text, "host", 22, &key(ED25519_B)),
            Verdict::Known
        );
        // A published RSA host key (GitHub's), another type than the entries.
        let rsa = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQCj7ndNxQowgcQnjshcLrqPEiiphnt+VTTvDP6mHBL9j1aNUkY4Ue1gvwnGLVlOhGeYrnZaMgRK6+PKCUXaDbC7qtbW8gIkhL7aGCsOr/C56SJMy/BCZfxd1nWzAOxSDPgVsmerOBYfNqltV9/hWCqBywINIR+5dIg6JTJ72pcEpEjcYgXkE2YEFXV1JHnsKgbLWNlhScqb2UmyRkQyytRLtL+38TGxkxCflmO+5Z8CSSNY7GidjMIZ7Q4zMjA2n1nGrlTDkzwDCsw+wqFPGQA179cnfGWOWRVruj16z6XyvxvjJwbz0wQZ75XK5tKSb7FNyeIEs4TT4jk+S4dhPeAUC5y+bDYirYgM4GC7uEnztnZyaVWQ7B381AK4Qdrwt51ZqExKbQpTUNn+EjqoTwvqNj4kqx5QUCI0ThS/YkOxJCXmPUWZbhjpCg56i+2aB6CmK2JGhn57K5mj0MNdBXA4/WnwH6XoPWJzK5Nyu2zB3nAZp+S5hpQs+p1vN1/wsjk=";
        let rsa = key(rsa);
        assert_eq!(
            check_text(&text, "host", 22, &rsa),
            Verdict::Unknown,
            "no RSA key on file"
        );
    }

    #[test]
    fn append_and_replace_write_entries_like_ssh() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join(".ssh").join("known_hosts");
        assert_eq!(
            check(&path, "nas", 22, &key(ED25519_A)).ok(),
            Some(Verdict::Unknown)
        );
        append(&path, "nas", 22, &key(ED25519_A)).unwrap_or_else(|e| panic!("{e}"));
        append(&path, "nas", 2222, &key(ED25519_B)).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            text.starts_with(&format!("nas {ED25519_A}\n[nas]:2222 ")),
            "{text}"
        );
        assert_eq!(
            check(&path, "nas", 22, &key(ED25519_A)).ok(),
            Some(Verdict::Known)
        );
        assert_eq!(
            check(&path, "nas", 2222, &key(ED25519_B)).ok(),
            Some(Verdict::Known)
        );
        assert!(matches!(
            check(&path, "nas", 22, &key(ED25519_C)).ok(),
            Some(Verdict::Changed { line: 1, .. })
        ));
        // The changed key replaces the entry of its host and type only.
        std::fs::write(
            &path,
            format!("nas,other {ED25519_A}\n[nas]:2222 {ED25519_B}\n"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        replace(&path, "nas", 22, &key(ED25519_C)).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            text,
            format!("other {ED25519_A}\n[nas]:2222 {ED25519_B}\nnas {ED25519_C}\n")
        );
        assert_eq!(
            check(&path, "nas", 22, &key(ED25519_C)).ok(),
            Some(Verdict::Known)
        );
        assert_eq!(
            check(&path, "other", 22, &key(ED25519_A)).ok(),
            Some(Verdict::Known)
        );
        assert_eq!(host_pattern("h", 22), "h");
        assert_eq!(host_pattern("h", 2200), "[h]:2200");
    }
}
