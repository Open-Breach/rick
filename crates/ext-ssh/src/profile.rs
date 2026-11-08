//! The command profiles: what the remote userland offers, probed once per
//! host, and the templates and parsers of the listing and `stat` commands
//! for each.
//!
//! - `gnu`: GNU coreutils and findutils, so a directory lists through
//!   `find -printf` in one process with NUL between the fields.
//! - `bsd`: the BSD and macOS userland, `stat -f`.
//! - `busybox`: the reduced set, `stat -c` but no `find -printf`.
//!
//! The two latter list through a shell loop over the directory's names,
//! one `stat` per entry, with the name and the link target NUL-terminated
//! by `printf`, so every byte of a name survives there too. The `stat` of
//! one path uses the same record for every profile.

use std::fmt;

use types::Error;

use crate::shell::{Command, quote};

/// The remote userland, as the probe found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// GNU coreutils and findutils.
    Gnu,
    /// BSD and macOS.
    Bsd,
    /// `BusyBox` and other reduced sets with `stat -c`.
    Busybox,
}

impl Profile {
    /// The profile's name in logs and the spec: `gnu`, `bsd`, `busybox`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Gnu => "gnu",
            Self::Bsd => "bsd",
            Self::Busybox => "busybox",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The probe: the kernel name, then one line per tool that answers. Every
/// test runs against `/`, which every host has, and prints nothing but
/// its tag.
pub const PROBE: &str = "uname -s; echo '--'; \
(find / -maxdepth 0 -printf 'x' >/dev/null 2>&1 && echo find-printf); \
(stat -c '%F' / >/dev/null 2>&1 && echo stat-c); \
(stat -f '%HT' / >/dev/null 2>&1 && echo stat-f); \
(dd --version >/dev/null 2>&1 && echo dd-gnu); \
(head -c 1 /dev/null >/dev/null 2>&1 && echo head-c); \
(tail -c +1 /dev/null >/dev/null 2>&1 && echo tail-c); \
(busybox true >/dev/null 2>&1 && echo busybox); \
(readlink -n / >/dev/null 2>&1 && echo readlink-n)";

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Probe {
    /// `uname -s`: `Linux`, `Darwin`, `FreeBSD`.
    pub uname: String,
    /// The tags of the tools that answered.
    pub tags: Vec<String>,
}

impl Probe {
    /// Reads the probe's output.
    #[must_use]
    pub fn parse(output: &[u8]) -> Self {
        let text = String::from_utf8_lossy(output);
        let mut lines = text.lines().map(str::trim);
        let mut probe = Self::default();
        for line in lines.by_ref() {
            if line == "--" {
                break;
            }
            if probe.uname.is_empty() && !line.is_empty() {
                line.clone_into(&mut probe.uname);
            }
        }
        probe.tags = lines.filter(|l| !l.is_empty()).map(str::to_owned).collect();
        probe
    }

    fn has(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// The profile for this userland: GNU where `find -printf` and
    /// `stat -c` answer, BSD where `stat -f` does, `BusyBox` where only
    /// `stat -c` does.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when no `stat` the plugin knows answers (a
    /// Windows host, a bare shell).
    pub fn profile(&self) -> Result<Profile, Error> {
        if self.has("find-printf") && self.has("stat-c") {
            Ok(Profile::Gnu)
        } else if self.has("stat-f") {
            Ok(Profile::Bsd)
        } else if self.has("stat-c") {
            Ok(Profile::Busybox)
        } else {
            Err(Error::Unsupported(
                "the host has no stat the SSH plugin knows; Unix-like hosts only",
            ))
        }
    }
}

/// What an entry is, as the listing says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link; `link` holds the target.
    Link,
    /// A device, a fifo, a socket.
    Other,
}

/// One entry as the remote commands report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// The name, raw bytes.
    pub name: Vec<u8>,
    /// What it is.
    pub kind: RawKind,
    /// Its size in bytes.
    pub size: u64,
    /// Modification time as seconds and nanoseconds since the epoch.
    pub mtime: Option<(i64, u32)>,
    /// The permission bits, with setuid, setgid and sticky.
    pub mode: u32,
    /// The owner's name.
    pub user: String,
    /// The group's name.
    pub group: String,
    /// The link target, for a link.
    pub link: Option<Vec<u8>>,
}

/// GNU `find`'s `-printf` format: type, size, mtime, mode, user, group,
/// link target and name, NUL after each.
const FIND_FORMAT: &str = "%y\\0%s\\0%T@\\0%m\\0%u\\0%g\\0%l\\0%f\\0";

/// The `stat` of one name under the loop, per profile: six fields with
/// `|` between them and a newline after.
fn stat_fields(profile: Profile) -> &'static str {
    match profile {
        Profile::Gnu | Profile::Busybox => "stat -c '%F|%s|%Y|%a|%U|%G' --",
        Profile::Bsd => "stat -f '%HT|%z|%m|%Mp%Lp|%Su|%Sg' --",
    }
}

/// The shell that prints one record for `$f`: the stat line, the name
/// NUL-terminated, the link target (empty when none) NUL-terminated.
fn record_of(profile: Profile) -> String {
    format!(
        "{stat} \"$f\"; printf '%s\\0' \"$f\"; if [ -L \"$f\" ]; then readlink -n -- \"$f\"; fi; printf '\\0'",
        stat = stat_fields(profile)
    )
}

/// The command that lists the directory `dir` (absolute) in the profile's
/// record format.
#[must_use]
pub fn list_command(profile: Profile, dir: &[u8]) -> Vec<u8> {
    match profile {
        Profile::Gnu => Command::new("find")
            .arg(dir)
            .raw(" -mindepth 1 -maxdepth 1 -printf '")
            .raw(FIND_FORMAT)
            .raw("'")
            .into_bytes(),
        Profile::Bsd | Profile::Busybox => {
            let mut command = b"cd ".to_vec();
            command.extend_from_slice(&quote(dir));
            command.extend_from_slice(
                format!(
                    " && for f in * .[!.]* ..?*; do [ -e \"$f\" ] || [ -L \"$f\" ] || continue; {}; done",
                    record_of(profile)
                )
                .as_bytes(),
            );
            command
        }
    }
}

/// The command that prints the record of one path, exiting 2 when there
/// is nothing there.
#[must_use]
pub fn stat_command(profile: Profile, path: &[u8]) -> Vec<u8> {
    let mut command = b"f=".to_vec();
    command.extend_from_slice(&quote(path));
    command.extend_from_slice(
        format!(
            "; if [ -e \"$f\" ] || [ -L \"$f\" ]; then {}; else exit 2; fi",
            record_of(profile)
        )
        .as_bytes(),
    );
    command
}

/// The command that writes the file from byte `offset` on.
#[must_use]
pub fn read_from_command(path: &[u8], offset: u64) -> Vec<u8> {
    if offset == 0 {
        return Command::new("cat").raw(" --").arg(path).into_bytes();
    }
    Command::new("tail")
        .raw(&format!(" -c +{} --", offset.saturating_add(1)))
        .arg(path)
        .into_bytes()
}

/// The shape of the records a command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// GNU `find -printf`: eight NUL-terminated fields.
    Find,
    /// The shell loop and the `stat` command: a `|`-separated line, then
    /// the name and the link target NUL-terminated.
    Loop,
}

/// Reads the output of the listing and stat commands record by record, as
/// the bytes arrive.
#[derive(Debug)]
pub struct Parser {
    format: Format,
    buffer: Vec<u8>,
}

impl Parser {
    /// A parser for the listing records of `profile`.
    #[must_use]
    pub fn new(profile: Profile) -> Self {
        Self {
            format: match profile {
                Profile::Gnu => Format::Find,
                Profile::Bsd | Profile::Busybox => Format::Loop,
            },
            buffer: Vec::new(),
        }
    }

    /// A parser for the records of [`stat_command`], the same for every
    /// profile.
    #[must_use]
    pub fn for_stat() -> Self {
        Self {
            format: Format::Loop,
            buffer: Vec::new(),
        }
    }

    /// Feeds bytes and returns the complete records among them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<RawEntry> {
        self.buffer.extend_from_slice(bytes);
        let mut entries = Vec::new();
        let mut consumed = 0;
        loop {
            let rest = &self.buffer[consumed..];
            let Some((entry, used)) = (match self.format {
                Format::Find => parse_find_record(rest),
                Format::Loop => parse_loop_record(rest),
            }) else {
                break;
            };
            consumed += used;
            if let Some(entry) = entry {
                entries.push(entry);
            }
        }
        self.buffer.drain(..consumed);
        entries
    }

    /// What is left unparsed, which should be nothing at the end.
    #[must_use]
    pub fn remainder(&self) -> &[u8] {
        &self.buffer
    }
}

/// Splits the next NUL-terminated field off `bytes`.
fn field(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let end = bytes.iter().position(|b| *b == 0)?;
    Some((&bytes[..end], &bytes[end + 1..]))
}

/// One record of `find -printf`: eight NUL-terminated fields. `None`
/// while the record is incomplete; `Some((None, used))` for a record that
/// is `.` or `..`.
fn parse_find_record(bytes: &[u8]) -> Option<(Option<RawEntry>, usize)> {
    let mut rest = bytes;
    let mut fields: Vec<&[u8]> = Vec::with_capacity(8);
    for _ in 0..8 {
        let (f, r) = field(rest)?;
        fields.push(f);
        rest = r;
    }
    let used = bytes.len() - rest.len();
    let kind = match fields[0].first() {
        Some(b'f') => RawKind::File,
        Some(b'd') => RawKind::Dir,
        Some(b'l') => RawKind::Link,
        _ => RawKind::Other,
    };
    let entry = RawEntry {
        name: fields[7].to_vec(),
        kind,
        size: parse_u64(fields[1]),
        mtime: parse_time(fields[2]),
        mode: parse_mode(fields[3]),
        user: String::from_utf8_lossy(fields[4]).into_owned(),
        group: String::from_utf8_lossy(fields[5]).into_owned(),
        link: (kind == RawKind::Link).then(|| fields[6].to_vec()),
    };
    Some((Some(entry), used))
}

/// One record of the shell loop and the stat command: the six-field stat
/// line, the name to a NUL, the link target to a NUL.
fn parse_loop_record(bytes: &[u8]) -> Option<(Option<RawEntry>, usize)> {
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    let header = &bytes[..newline];
    let (name, rest) = field(&bytes[newline + 1..])?;
    let (link, rest) = field(rest)?;
    let used = bytes.len() - rest.len();
    let parts: Vec<&[u8]> = header.splitn(6, |b| *b == b'|').collect();
    if parts.len() != 6 {
        // A stat that failed printed something else; skip the record.
        return Some((None, used));
    }
    let kind = kind_of(&String::from_utf8_lossy(parts[0]));
    let entry = RawEntry {
        name: name.to_vec(),
        kind,
        size: parse_u64(parts[1]),
        mtime: parse_time(parts[2]),
        mode: parse_mode(parts[3]),
        user: String::from_utf8_lossy(parts[4]).into_owned(),
        group: String::from_utf8_lossy(parts[5]).into_owned(),
        link: (kind == RawKind::Link).then(|| link.to_vec()),
    };
    Some((Some(entry), used))
}

/// The kind a `%F` (GNU, `BusyBox`) or `%HT` (BSD) type string names.
fn kind_of(text: &str) -> RawKind {
    let lower = text.to_ascii_lowercase();
    if lower.contains("regular") {
        RawKind::File
    } else if lower.contains("directory") {
        RawKind::Dir
    } else if lower.contains("symbolic") {
        RawKind::Link
    } else {
        RawKind::Other
    }
}

fn parse_u64(bytes: &[u8]) -> u64 {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Seconds since the epoch, with a fraction after a `.` where the tool
/// gives one.
fn parse_time(bytes: &[u8]) -> Option<(i64, u32)> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let (secs, frac) = text.split_once('.').unwrap_or((text, ""));
    let secs: i64 = secs.parse().ok()?;
    let mut nanos: u32 = 0;
    if !frac.is_empty() {
        let digits: String = frac.chars().filter(char::is_ascii_digit).take(9).collect();
        let padded = format!("{digits:0<9}");
        nanos = padded.parse().unwrap_or(0);
    }
    Some((secs, nanos))
}

/// Octal permission bits.
fn parse_mode(bytes: &[u8]) -> u32 {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| u32::from_str_radix(s.trim(), 8).ok())
        .unwrap_or(0)
}

/// `df -P -k` of a path: the total and free bytes of its filesystem, when
/// the output has the POSIX shape.
#[must_use]
pub fn df_command(path: &[u8]) -> Vec<u8> {
    Command::new("df").raw(" -P -k --").arg(path).into_bytes()
}

/// The total and free bytes from `df -P -k` output.
#[must_use]
pub fn parse_df(output: &[u8]) -> Option<(u64, u64)> {
    let text = String::from_utf8_lossy(output);
    let mut lines = text.lines();
    let header = lines.next()?;
    let block = header
        .split_whitespace()
        .find_map(|w| w.strip_suffix("-blocks"))
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(1024);
    // The device name may hold spaces; the four numbers and the capacity
    // come before the mount point, so read from the right.
    let line = lines.next()?;
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() < 6 {
        return None;
    }
    let total: u64 = words[words.len() - 5].parse().ok()?;
    let free: u64 = words[words.len() - 3].parse().ok()?;
    Some((total.saturating_mul(block), free.saturating_mul(block)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_picks_the_profile() {
        let debian =
            Probe::parse(b"Linux\n--\nfind-printf\nstat-c\ndd-gnu\nhead-c\ntail-c\nreadlink-n\n");
        assert_eq!(debian.uname, "Linux");
        assert_eq!(debian.profile().ok(), Some(Profile::Gnu));
        let alpine = Probe::parse(b"Linux\n--\nstat-c\nhead-c\ntail-c\nbusybox\nreadlink-n\n");
        assert_eq!(alpine.profile().ok(), Some(Profile::Busybox));
        let mac = Probe::parse(b"Darwin\n--\nstat-f\nhead-c\ntail-c\nreadlink-n\n");
        assert_eq!(mac.profile().ok(), Some(Profile::Bsd));
        let bare = Probe::parse(b"Linux\n--\n");
        assert!(matches!(bare.profile(), Err(Error::Unsupported(_))));
        let windows = Probe::parse(b"'uname' is not recognized\n");
        assert!(windows.profile().is_err());
        assert_eq!(Profile::Busybox.to_string(), "busybox");
    }

    #[test]
    fn find_records_parse_as_they_arrive() {
        let mut parser = Parser::new(Profile::Gnu);
        let record =
            b"f\x001234\x001700000000.5000000000\x00644\x00serge\x00users\x00\x00a b.txt\x00";
        let (head, tail) = record.split_at(20);
        assert!(parser.push(head).is_empty());
        let entries = parser.push(tail);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.name, b"a b.txt");
        assert_eq!(e.kind, RawKind::File);
        assert_eq!(e.size, 1234);
        assert_eq!(e.mtime, Some((1_700_000_000, 500_000_000)));
        assert_eq!(e.mode, 0o644);
        assert_eq!((e.user.as_str(), e.group.as_str()), ("serge", "users"));
        assert_eq!(e.link, None);
        let more = parser.push(
            b"l\x0011\x001600000000\x00777\x00root\x00root\x00/srv/target\x00new\nline\x00d\x004096\x00-5.0\x001755\x00u\x00g\x00\x00\xff\xfe\x00",
        );
        assert_eq!(more.len(), 2);
        assert_eq!(more[0].kind, RawKind::Link);
        assert_eq!(more[0].link.as_deref(), Some(&b"/srv/target"[..]));
        assert_eq!(more[0].name, b"new\nline");
        assert_eq!(more[1].kind, RawKind::Dir);
        assert_eq!(more[1].mode, 0o1755);
        assert_eq!(more[1].mtime, Some((-5, 0)));
        assert_eq!(more[1].name, [0xff, 0xfe]);
        assert!(parser.remainder().is_empty());
    }

    #[test]
    fn loop_records_parse_for_busybox_and_bsd() {
        let mut parser = Parser::new(Profile::Busybox);
        let entries = parser.push(
            b"regular file|10|1700000000|644|serge|users\na b\x00\x00symbolic link|3|1|777|r|r\nlink\x00/tmp/x\x00directory|4096|2|755|u|g\n.\xffdot\x00\x00",
        );
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, b"a b");
        assert_eq!(entries[0].kind, RawKind::File);
        assert_eq!(entries[1].kind, RawKind::Link);
        assert_eq!(entries[1].link.as_deref(), Some(&b"/tmp/x"[..]));
        assert_eq!(entries[2].kind, RawKind::Dir);
        assert_eq!(entries[2].name, b".\xffdot");
        // A stat line that is not a record is skipped, its name and link
        // consumed with it.
        let skipped = parser.push(b"stat: cannot stat\nname\x00\x00");
        assert!(skipped.is_empty());
        assert!(parser.remainder().is_empty());
        let mut bsd = Parser::new(Profile::Bsd);
        let entries = bsd.push(b"Regular File|5|1700000000|0644|me|staff\nnotes.txt\x00\x00");
        assert_eq!(entries[0].kind, RawKind::File);
        assert_eq!(entries[0].mode, 0o644);
        assert_eq!(kind_of("Character Device"), RawKind::Other);
    }

    #[test]
    fn commands_quote_their_paths() {
        let gnu = list_command(Profile::Gnu, b"/srv/it's");
        assert!(gnu.starts_with(b"find '/srv/it'\\''s' -mindepth 1 -maxdepth 1 -printf '"));
        let busy = String::from_utf8_lossy(&list_command(Profile::Busybox, b"/a b")).into_owned();
        assert!(
            busy.starts_with("cd '/a b' && for f in * .[!.]* ..?*; do "),
            "{busy}"
        );
        assert!(
            busy.contains("stat -c '%F|%s|%Y|%a|%U|%G' -- \"$f\""),
            "{busy}"
        );
        assert!(busy.contains("readlink -n -- \"$f\""), "{busy}");
        let bsd = String::from_utf8_lossy(&stat_command(Profile::Bsd, b"/x")).into_owned();
        assert!(bsd.starts_with("f='/x'; if [ -e \"$f\" ] || [ -L \"$f\" ]; then stat -f '%HT|%z|%m|%Mp%Lp|%Su|%Sg' -- \"$f\""), "{bsd}");
        assert!(bsd.ends_with("else exit 2; fi"), "{bsd}");
        assert_eq!(read_from_command(b"/f", 0), b"cat -- '/f'");
        assert_eq!(read_from_command(b"/f", 1023), b"tail -c +1024 -- '/f'");
        assert_eq!(df_command(b"/srv"), b"df -P -k -- '/srv'");
    }

    #[test]
    fn df_output_gives_total_and_free() {
        let gnu = b"Filesystem     1024-blocks     Used Available Capacity Mounted on\n/dev/sda1         10000000  4000000   5500000      42% /\n";
        assert_eq!(parse_df(gnu), Some((10_000_000 * 1024, 5_500_000 * 1024)));
        let posix = b"Filesystem 512-blocks Used Available Capacity Mounted on\nmy device 200 100 50 50% /mnt\n";
        assert_eq!(parse_df(posix), Some((200 * 512, 50 * 512)));
        assert_eq!(parse_df(b"nothing"), None);
    }
}
