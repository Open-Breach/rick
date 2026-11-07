//! ISO 9660 listing: the primary volume descriptor, the Joliet
//! supplementary descriptor when there is one (for the Unicode names), Rock
//! Ridge `NM` names otherwise, and the directory tree walked extent by
//! extent.

use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

use types::{EntryKind, Error};

use crate::{Facts, Limit, Listed, RawEntry, civil_to_system_time, invalid};

/// The logical sector size every ISO 9660 image uses for its descriptors.
pub const SECTOR: u64 = 2048;

/// The first volume descriptor's sector.
const FIRST_DESCRIPTOR: u64 = 16;

/// A directory extent larger than this is refused as damaged.
const MAX_DIRECTORY: u32 = 16 * 1024 * 1024;

/// Reads `len` bytes at `offset`.
fn read_at(file: &mut File, offset: u64, len: usize) -> Result<Vec<u8>, Error> {
    file.seek(SeekFrom::Start(offset)).map_err(Error::from_io)?;
    let mut buffer = vec![0u8; len];
    file.read_exact(&mut buffer)
        .map_err(|e| invalid(format!("ISO 9660: short read at {offset}: {e}")))?;
    Ok(buffer)
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// A directory record's fields.
#[derive(Debug, Clone)]
struct Record {
    extent: u32,
    size: u32,
    is_dir: bool,
    name: String,
    /// `\0` or `\1`: the directory itself or its parent.
    is_dot: bool,
    modified: Option<SystemTime>,
}

/// Which name table the walk reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Names {
    /// 8.3 names from the primary descriptor, Rock Ridge `NM` when present.
    Primary,
    /// UCS-2 names from a Joliet supplementary descriptor.
    Joliet,
}

/// The 7-byte directory record date, taken to UTC through its offset.
fn record_date(bytes: &[u8]) -> Option<SystemTime> {
    let year = i64::from(bytes[0]) + 1900;
    let (month, day, hour, minute, second) = (
        u32::from(bytes[1]),
        u32::from(bytes[2]),
        u32::from(bytes[3]),
        u32::from(bytes[4]),
        u32::from(bytes[5]),
    );
    if month == 0 {
        return None;
    }
    // The offset from GMT in 15-minute units, -48 to +52.
    #[allow(clippy::cast_possible_wrap)]
    let offset = i64::from(bytes[6] as i8) * 15 * 60;
    let local = civil_to_system_time(year, month, day, hour, minute, second)?;
    if offset >= 0 {
        local.checked_sub(std::time::Duration::from_secs(u64::try_from(offset).ok()?))
    } else {
        local.checked_add(std::time::Duration::from_secs(u64::try_from(-offset).ok()?))
    }
}

/// Parses the records of one directory extent.
fn parse_directory(bytes: &[u8], names: Names, skip: usize) -> Vec<Record> {
    let mut records = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let len = usize::from(bytes[at]);
        if len == 0 {
            // Records never cross a sector; skip to the next one.
            at = (at / 2048 + 1) * 2048;
            continue;
        }
        if at + len > bytes.len() || len < 34 {
            break;
        }
        let record = &bytes[at..at + len];
        let name_len = usize::from(record[32]);
        if 33 + name_len > len {
            break;
        }
        let raw_name = &record[33..33 + name_len];
        let flags = record[25];
        let is_dot = name_len == 1 && (raw_name[0] == 0 || raw_name[0] == 1);
        let mut name = match names {
            Names::Joliet if !is_dot => {
                let units: Vec<u16> = raw_name
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_be_bytes(*c))
                    .collect();
                String::from_utf16_lossy(&units)
            }
            _ => String::from_utf8_lossy(raw_name).into_owned(),
        };
        if names == Names::Primary && !is_dot {
            // The system use area follows the name and its padding byte.
            let mut su = 33 + name_len + usize::from(name_len.is_multiple_of(2));
            su += skip;
            if let Some(alternate) = rock_ridge_name(&record[su.min(len)..]) {
                name = alternate;
            } else {
                // `NAME.EXT;1` to `NAME.EXT`, a trailing dot dropped.
                if let Some(cut) = name.rfind(';') {
                    name.truncate(cut);
                }
                if name.ends_with('.') {
                    name.pop();
                }
            }
        }
        records.push(Record {
            extent: le32(record, 2),
            size: le32(record, 10),
            is_dir: flags & 0x02 != 0,
            name,
            is_dot,
            modified: record_date(&record[18..25]),
        });
        at += len;
    }
    records
}

/// The Rock Ridge `NM` name in a system use area, pieces joined.
fn rock_ridge_name(su: &[u8]) -> Option<String> {
    let mut at = 0;
    let mut name: Option<String> = None;
    while at + 4 <= su.len() {
        let len = usize::from(su[at + 2]);
        if len < 4 || at + len > su.len() {
            break;
        }
        if &su[at..at + 2] == b"NM" && len >= 5 {
            let piece = String::from_utf8_lossy(&su[at + 5..at + len]);
            name.get_or_insert_with(String::new).push_str(&piece);
            if su[at + 4] & 0x01 == 0 {
                break;
            }
        }
        at += len;
    }
    name.filter(|n| !n.is_empty())
}

/// The bytes to skip at the start of every system use area, from a Rock
/// Ridge `SP` entry in the root's `\0` record.
fn susp_skip(root_dot: &[u8]) -> usize {
    let name_len = usize::from(root_dot.get(32).copied().unwrap_or(0));
    let su_start = 33 + name_len + usize::from(name_len.is_multiple_of(2));
    let su = root_dot.get(su_start..).unwrap_or(&[]);
    if su.len() >= 7 && &su[..2] == b"SP" && su[4] == 0xBE && su[5] == 0xEF {
        usize::from(su[6])
    } else {
        0
    }
}

/// A fixed-width `a`-characters field, its padding (spaces, or NULs from
/// a careless writer) trimmed.
fn str_a(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(|c: char| c == ' ' || c == '\0')
        .to_owned()
}

/// `YYYYMMDDHHMMSS…` as `YYYY-MM-DD HH:MM`, or nothing for a blank field.
fn descriptor_date(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.get(..14)?).ok()?;
    if text.starts_with("0000") || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{} {}:{}",
        &text[0..4],
        &text[4..6],
        &text[6..8],
        &text[8..10],
        &text[10..12]
    ))
}

/// The descriptors that matter: the primary one and a Joliet
/// supplementary one when there is any.
fn descriptors(file: &mut File) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let mut primary: Option<Vec<u8>> = None;
    let mut joliet: Option<Vec<u8>> = None;
    for index in 0..64 {
        let descriptor = read_at(file, (FIRST_DESCRIPTOR + index) * SECTOR, 2048)?;
        if &descriptor[1..6] != b"CD001" {
            return Err(invalid("ISO 9660: no volume descriptor"));
        }
        match descriptor[0] {
            1 if primary.is_none() => primary = Some(descriptor),
            2 => {
                let escapes = &descriptor[88..120];
                let is_joliet = escapes
                    .windows(3)
                    .any(|w| w[0] == 0x25 && w[1] == 0x2F && matches!(w[2], 0x40 | 0x43 | 0x45));
                if is_joliet && joliet.is_none() {
                    joliet = Some(descriptor);
                }
            }
            255 => break,
            _ => {}
        }
    }
    let primary = primary.ok_or_else(|| invalid("ISO 9660: no primary volume descriptor"))?;
    Ok((primary, joliet))
}

/// The File info rows a primary descriptor gives.
fn facts_of(primary: &[u8], joliet: bool) -> Facts {
    let mut facts = Facts::default();
    let label = str_a(&primary[40..72]);
    if !label.is_empty() {
        facts.rows.push(("Volume label".to_owned(), label));
    }
    let publisher = str_a(&primary[318..446]);
    if !publisher.is_empty() {
        facts.rows.push(("Publisher".to_owned(), publisher));
    }
    if let Some(created) = descriptor_date(&primary[813..830]) {
        facts.rows.push(("Created".to_owned(), created));
    }
    facts.rows.push((
        "Names".to_owned(),
        if joliet {
            "Joliet"
        } else {
            "ISO 9660, Rock Ridge where present"
        }
        .to_owned(),
    ));
    facts
}

/// Lists the ISO 9660 image at `path`.
pub(crate) fn list(path: &Path, limit: &Limit) -> Result<Listed, Error> {
    let mut file = File::open(path).map_err(Error::from_io)?;
    let (primary, joliet) = descriptors(&mut file)?;
    let facts = facts_of(&primary, joliet.is_some());
    let (descriptor, names) = match &joliet {
        Some(joliet) => (joliet, Names::Joliet),
        None => (&primary, Names::Primary),
    };
    let root = &descriptor[156..190];
    // The SUSP skip comes from the primary tree's root record.
    let skip = {
        let primary_root = &primary[156..190];
        let bytes = read_at(
            &mut file,
            u64::from(le32(primary_root, 2)) * SECTOR,
            usize::try_from(le32(primary_root, 10).min(2048)).unwrap_or(2048),
        )
        .unwrap_or_default();
        susp_skip(&bytes)
    };

    let mut entries = Vec::new();
    let mut truncated = false;
    let mut visited: HashSet<u32> = HashSet::new();
    let mut queue: VecDeque<(u32, u32, String)> = VecDeque::new();
    queue.push_back((le32(root, 2), le32(root, 10), String::new()));
    while let Some((extent, size, prefix)) = queue.pop_front() {
        if !visited.insert(extent) || size == 0 || size > MAX_DIRECTORY {
            continue;
        }
        let bytes = read_at(
            &mut file,
            u64::from(extent) * SECTOR,
            usize::try_from(size).unwrap_or(0),
        )?;
        for record in parse_directory(&bytes, names, skip) {
            if record.is_dot {
                continue;
            }
            if limit.reached(entries.len()) {
                truncated = true;
                queue.clear();
                break;
            }
            let path = if prefix.is_empty() {
                record.name.clone()
            } else {
                format!("{prefix}/{}", record.name)
            };
            if record.is_dir {
                queue.push_back((record.extent, record.size, path.clone()));
            }
            entries.push(RawEntry {
                path,
                kind: if record.is_dir {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                },
                size: Some(u64::from(record.size)),
                compressed: None,
                modified: record.modified,
                encrypted: false,
            });
        }
    }
    Ok(Listed {
        entries,
        truncated,
        facts,
    })
}
