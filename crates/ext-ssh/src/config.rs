//! The user's OpenSSH client configuration, `~/.ssh/config` and the files
//! its `Include` lines name: every `Host` entry with a concrete name (no
//! wildcard) is a host Rick lists, with the directives it honours
//! (`HostName`, `User`, `Port`, `IdentityFile`, `IdentityAgent`,
//! `ProxyJump`) resolved the way `ssh` resolves them: every block whose
//! patterns match the alias contributes, in file order, and the first
//! value of a directive wins, except `IdentityFile`, which accumulates.
//! Other directives, and `Match` blocks, are ignored without complaint.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One host of the configuration as Rick reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigHost {
    /// The `Host` name: what `ssh <alias>` takes and what the row shows.
    pub alias: String,
    /// `HostName`, when set; the alias stands in otherwise.
    pub host_name: Option<String>,
    /// `User`, when set.
    pub user: Option<String>,
    /// `Port`, when set.
    pub port: Option<u16>,
    /// Every `IdentityFile`, `~` and the `%d`, `%h`, `%r`, `%u` tokens
    /// expanded, in order.
    pub identity_files: Vec<PathBuf>,
    /// `IdentityAgent`, when set: the agent socket to use instead of
    /// `SSH_AUTH_SOCK`; `none` turns the agent off.
    pub identity_agent: Option<IdentityAgent>,
    /// `ProxyJump`: the hops, in order, each `[user@]host[:port]`; empty
    /// when none or `none`.
    pub proxy_jump: Vec<String>,
}

/// What `IdentityAgent` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityAgent {
    /// No agent at all.
    None,
    /// The agent at this socket.
    Socket(PathBuf),
}

impl ConfigHost {
    /// The host name connected to: `HostName`, or the alias.
    #[must_use]
    pub fn host(&self) -> &str {
        self.host_name.as_deref().unwrap_or(&self.alias)
    }
}

/// What expands the tokens of the configuration: the home directory for
/// `~` and `%d`, the local user for `%u`.
#[derive(Debug, Clone)]
pub struct Env {
    /// The user's home directory.
    pub home: PathBuf,
    /// The local user name.
    pub user: String,
}

/// One `Host` block: its patterns and its directives in file order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Block {
    patterns: Vec<String>,
    directives: Vec<(String, String)>,
}

/// Whether `pattern` matches `text`, with `*` for any run and `?` for one
/// character, as OpenSSH matches host names (ASCII case-insensitively).
#[must_use]
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            // The pattern ended: a match when the text did too.
            (None, rest) => rest.is_none(),
            (Some(b'*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some(b'?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && rec(&p[1..], &t[1..]),
            (Some(_), None) => false,
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

/// Whether a block's pattern list applies to `alias`: one positive pattern
/// matches and no negated one does.
fn patterns_match(patterns: &[String], alias: &str) -> bool {
    let mut positive = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob_matches(negated, alias) {
                return false;
            }
        } else if glob_matches(pattern, alias) {
            positive = true;
        }
    }
    positive
}

/// Whether a pattern names one host and no family of them.
fn is_concrete(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.starts_with('!') && !pattern.contains(['*', '?'])
}

/// Splits a configuration line into its keyword and its value: `Key value`,
/// `Key=value`, `Key = value`; the keyword lower-cased, the value as
/// written after it. `None` for a blank line or a comment.
fn split_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_ascii_lowercase();
    let mut value = line[end..].trim_start();
    if let Some(rest) = value.strip_prefix('=') {
        value = rest.trim_start();
    }
    Some((keyword, value.trim_end().to_owned()))
}

/// The words of a value, split on whitespace with double quotes keeping a
/// word together: `Host "my host" other` names two patterns.
fn words(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// One value: the whole text, its surrounding quotes removed.
fn unquoted(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

/// Reads the configuration at `path` and the files it includes, at most
/// `depth` levels deep, into `blocks`. A file that cannot be read is
/// skipped, as `ssh` skips a missing include.
fn read_blocks(path: &Path, env: &Env, depth: u8, blocks: &mut Vec<Block>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    parse_into(&text, env, depth, blocks);
}

/// Parses `text` into `blocks`, following `Include` lines through the
/// filesystem (relative to `~/.ssh/`, with `~` and a trailing glob
/// expanded). An include inside a `Host` block adds to that block; one
/// at the top adds to a block matching every host.
fn parse_into(text: &str, env: &Env, depth: u8, blocks: &mut Vec<Block>) {
    // The current block: directives before the first `Host` apply to every
    // host, as if under `Host *`.
    let mut current = Block {
        patterns: vec!["*".to_owned()],
        directives: Vec::new(),
    };
    // Inside a `Match` block nothing is read until the next `Host`.
    let mut skipping = false;
    for line in text.lines() {
        let Some((keyword, value)) = split_line(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => {
                blocks.push(std::mem::take(&mut current));
                current.patterns = words(&value);
                skipping = false;
            }
            "match" => {
                blocks.push(std::mem::take(&mut current));
                current.patterns = Vec::new();
                skipping = true;
            }
            "include" if depth > 0 => {
                for included in words(&value) {
                    for file in include_files(&included, env) {
                        // The included file's blocks apply inside the
                        // current one: its own `Host` lines start new
                        // blocks, its loose directives join this block.
                        let mut inner = Vec::new();
                        read_blocks(&file, env, depth - 1, &mut inner);
                        for (index, mut block) in inner.into_iter().enumerate() {
                            if index == 0 {
                                current.directives.append(&mut block.directives);
                            } else {
                                blocks.push(block);
                            }
                        }
                    }
                }
            }
            _ if skipping => {}
            _ => current.directives.push((keyword, unquoted(&value))),
        }
    }
    blocks.push(current);
}

/// The files an `Include` value names: `~` expanded, a relative path taken
/// under `~/.ssh/`, a `*` or `?` in the last component matched against the
/// directory's entries, sorted.
fn include_files(value: &str, env: &Env) -> Vec<PathBuf> {
    let expanded = expand_tilde(value, env);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        env.home.join(".ssh").join(expanded)
    };
    let name = path
        .file_name()
        .map_or_default(|n| n.to_string_lossy().into_owned());
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| glob_matches(&name, &n.to_string_lossy()))
        })
        .collect();
    files.sort();
    files
}

/// `~` and `~/x` as the home directory; anything else as is.
fn expand_tilde(value: &str, env: &Env) -> PathBuf {
    if value == "~" {
        env.home.clone()
    } else if let Some(rest) = value.strip_prefix("~/") {
        env.home.join(rest)
    } else {
        PathBuf::from(value)
    }
}

/// Expands the tokens `ssh` allows in `IdentityFile`: `%d` the home, `%h`
/// the host name, `%r` the remote user, `%u` the local user, `%%` a percent
/// sign, and a leading `~`.
fn expand_identity(value: &str, env: &Env, host: &str, user: &str) -> PathBuf {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('d') => out.push_str(&env.home.to_string_lossy()),
            Some('h') => out.push_str(host),
            Some('r') => out.push_str(user),
            Some('u') => out.push_str(&env.user),
            Some('%') | None => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
        }
    }
    expand_tilde(&out, env)
}

/// The hosts `blocks` define: every concrete `Host` pattern, resolved
/// against every block in order with the first value winning.
fn resolve(blocks: &[Block], env: &Env) -> Vec<ConfigHost> {
    let mut seen = BTreeSet::new();
    let mut hosts = Vec::new();
    for block in blocks {
        for alias in block.patterns.iter().filter(|p| is_concrete(p)) {
            if !seen.insert(alias.to_ascii_lowercase()) {
                continue;
            }
            hosts.push(resolve_one(blocks, env, alias));
        }
    }
    hosts
}

/// One host's directives from every block that matches it.
fn resolve_one(blocks: &[Block], env: &Env, alias: &str) -> ConfigHost {
    let mut host = ConfigHost {
        alias: alias.to_owned(),
        ..ConfigHost::default()
    };
    let mut identity_files: Vec<String> = Vec::new();
    let mut proxy_jump: Option<String> = None;
    let mut agent: Option<String> = None;
    for block in blocks.iter().filter(|b| patterns_match(&b.patterns, alias)) {
        for (keyword, value) in &block.directives {
            match keyword.as_str() {
                "hostname" if host.host_name.is_none() => host.host_name = Some(value.clone()),
                "user" if host.user.is_none() => host.user = Some(value.clone()),
                "port" if host.port.is_none() => host.port = value.parse().ok(),
                "identityfile" => identity_files.push(value.clone()),
                "identityagent" if agent.is_none() => agent = Some(value.clone()),
                "proxyjump" if proxy_jump.is_none() => proxy_jump = Some(value.clone()),
                _ => {}
            }
        }
    }
    // `HostName` may itself use `%h` for the alias.
    if let Some(name) = host.host_name.take() {
        host.host_name = Some(name.replace("%h", alias));
    }
    let remote_user = host.user.clone().unwrap_or_else(|| env.user.clone());
    let host_name = host.host().to_owned();
    host.identity_files = identity_files
        .iter()
        .map(|f| expand_identity(f, env, &host_name, &remote_user))
        .collect();
    host.identity_agent = agent.map(|a| {
        if a.eq_ignore_ascii_case("none") {
            IdentityAgent::None
        } else if a.eq_ignore_ascii_case("SSH_AUTH_SOCK") {
            IdentityAgent::Socket(PathBuf::from(
                std::env::var("SSH_AUTH_SOCK").unwrap_or_default(),
            ))
        } else {
            IdentityAgent::Socket(expand_tilde(&a, env))
        }
    });
    host.proxy_jump = proxy_jump
        .filter(|j| !j.eq_ignore_ascii_case("none"))
        .map_or_default(|j| {
            j.split(',')
                .map(str::trim)
                .filter(|h| !h.is_empty())
                .map(str::to_owned)
                .collect()
        });
    host
}

/// The hosts of the configuration text, with includes resolved through
/// the filesystem as `ssh` would.
#[must_use]
pub fn parse(text: &str, env: &Env) -> Vec<ConfigHost> {
    let mut blocks = Vec::new();
    parse_into(text, env, MAX_INCLUDE_DEPTH, &mut blocks);
    resolve(&blocks, env)
}

/// How deep `Include` may nest, as `ssh` bounds it.
const MAX_INCLUDE_DEPTH: u8 = 16;

/// The hosts of the configuration file at `path`, none when it is missing
/// or unreadable.
#[must_use]
pub fn load(path: &Path, env: &Env) -> Vec<ConfigHost> {
    let mut blocks = Vec::new();
    read_blocks(path, env, MAX_INCLUDE_DEPTH, &mut blocks);
    resolve(&blocks, env)
}

/// A `ProxyJump` hop, `[user@]host[:port]`, taken apart; an IPv6 address
/// in brackets keeps its colons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    /// The user, when the hop names one.
    pub user: Option<String>,
    /// The host name, or an alias of the configuration.
    pub host: String,
    /// The port, when the hop names one.
    pub port: Option<u16>,
}

impl Hop {
    /// Parses one hop.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        let (user, rest) = match text.rsplit_once('@') {
            Some((user, rest)) => (Some(user.to_owned()), rest),
            None => (None, text),
        };
        let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
            match inner.split_once(']') {
                Some((host, tail)) => (
                    host.to_owned(),
                    tail.strip_prefix(':').and_then(|p| p.parse().ok()),
                ),
                None => (inner.to_owned(), None),
            }
        } else {
            match rest.rsplit_once(':') {
                // One colon: host and port; more: a bare IPv6 address.
                Some((host, port)) if !host.contains(':') => (host.to_owned(), port.parse().ok()),
                _ => (rest.to_owned(), None),
            }
        };
        Self { user, host, port }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            home: PathBuf::from("/home/me"),
            user: "me".into(),
        }
    }

    #[test]
    fn concrete_hosts_resolve_with_first_value_winning() {
        let text = "\
# a comment
IdentityFile ~/.ssh/everywhere
Host nas
    HostName 192.168.1.20
    User serge
    Port = 2222
    IdentityFile %d/.ssh/id_%r
Host *.example.com
    User ops
    Port 22
Host bastion
    ProxyJump none
    IdentityAgent none
Host prod-db bastion
    ProxyJump bastion, ops@jump.example.com:2200
Host prod-db
    User dba
    HostName db.%h.internal
Host \"quoted one\"
Match host nope
    User never
Host !nothing
";
        let hosts = parse(text, &env());
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["nas", "bastion", "prod-db", "quoted one"]);
        let nas = &hosts[0];
        assert_eq!(nas.host(), "192.168.1.20");
        assert_eq!(nas.user.as_deref(), Some("serge"));
        assert_eq!(nas.port, Some(2222));
        assert_eq!(
            nas.identity_files,
            [
                PathBuf::from("/home/me/.ssh/everywhere"),
                PathBuf::from("/home/me/.ssh/id_serge"),
            ]
        );
        assert!(nas.proxy_jump.is_empty(), "{:?}", nas.proxy_jump);
        let db = &hosts[2];
        assert_eq!(db.user.as_deref(), Some("dba"), "the first block wins");
        assert_eq!(db.host(), "db.prod-db.internal");
        assert_eq!(db.proxy_jump, ["bastion", "ops@jump.example.com:2200"]);
        assert_eq!(db.port, None);
        let bastion = &hosts[1];
        assert!(bastion.proxy_jump.is_empty(), "the first block says none");
        assert_eq!(bastion.identity_agent, Some(IdentityAgent::None));
        assert_eq!(bastion.user, None, "a Match block contributes nothing");
    }

    #[test]
    fn globs_match_like_ssh() {
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("*.example.com", "web.example.com"));
        assert!(!glob_matches("*.example.com", "example.com"));
        assert!(glob_matches("web?", "web1"));
        assert!(!glob_matches("web?", "web12"));
        assert!(glob_matches("NAS", "nas"));
        assert!(patterns_match(
            &["*".to_owned(), "!secret".to_owned()],
            "public"
        ));
        assert!(!patterns_match(
            &["*".to_owned(), "!secret".to_owned()],
            "secret"
        ));
        assert!(!patterns_match(&["!x".to_owned()], "y"));
    }

    #[test]
    fn includes_are_read_relative_to_the_ssh_directory() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let home = dir.path();
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(ssh.join("conf.d")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            ssh.join("config"),
            "Include conf.d/*.conf\nHost local\n  IdentityFile ~/.ssh/id_local\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            ssh.join("conf.d").join("work.conf"),
            "Host work\n  HostName work.example.com\n  Include extra\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(ssh.join("conf.d").join("ignored.txt"), "Host no\n")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(ssh.join("extra"), "User fromextra\n").unwrap_or_else(|e| panic!("{e}"));
        let env = Env {
            home: home.to_path_buf(),
            user: "me".into(),
        };
        let hosts = load(&ssh.join("config"), &env);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["work", "local"]);
        assert_eq!(hosts[0].host(), "work.example.com");
        assert_eq!(
            hosts[0].user.as_deref(),
            Some("fromextra"),
            "an include inside a block joins the block"
        );
        assert_eq!(hosts[1].identity_files, [ssh.join("id_local")]);
        let hosts = load(&home.join("missing"), &env);
        assert!(hosts.is_empty(), "{hosts:?}");
    }

    #[test]
    fn hops_take_user_host_and_port_apart() {
        assert_eq!(
            Hop::parse("bastion"),
            Hop {
                user: None,
                host: "bastion".into(),
                port: None
            }
        );
        assert_eq!(
            Hop::parse("ops@jump.example.com:2200"),
            Hop {
                user: Some("ops".into()),
                host: "jump.example.com".into(),
                port: Some(2200)
            }
        );
        assert_eq!(
            Hop::parse("[fe80::1]:22"),
            Hop {
                user: None,
                host: "fe80::1".into(),
                port: Some(22)
            }
        );
        assert_eq!(
            Hop::parse("fe80::1"),
            Hop {
                user: None,
                host: "fe80::1".into(),
                port: None
            }
        );
        assert_eq!(
            split_line("  Port = 2222  "),
            Some(("port".to_owned(), "2222".to_owned()))
        );
        assert_eq!(split_line("# comment"), None);
        assert_eq!(words("a \"b c\" d"), ["a", "b c", "d"]);
        assert_eq!(unquoted("\"x y\""), "x y");
    }
}
