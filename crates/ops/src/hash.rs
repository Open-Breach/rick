//! Compute hash…: MD5, SHA-1, SHA-256 and BLAKE3 of the selected files in
//! one pass each, for a dialogue with a copy button per value, *Copy all*,
//! a format toggle remembered in `core.toml` and a verify field.
//!
//! The action builds a [`Hashes`] slot with the files' names, spawns one
//! job for the whole selection and returns at once with
//! [`ActionOutcome::Hashes`]; the dialogue opens empty and fills in. The
//! job reads each file once through its provider's `open_read`, in 1 MiB
//! chunks, feeds every chunk to the four hashers on the blocking pool so
//! the runtime's workers and the UI thread stay free, publishes byte
//! progress over the selection's total and checks the cancel token between
//! chunks. The pure functions — [`render`], [`parse_verify`], [`detect`],
//! [`verify`], [`copy_all`] — are what the dialogue draws with; the format
//! is read and written through [`remembered`] and [`remember`].

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use ext::t;
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Algorithm, Availability, Digests,
    Hashes, Provider, Shortcut, UiHost,
};
use md5::Md5;
use platform::Platform;
use runtime::{JobCtx, JobRunner};
use sha1::Sha1;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;
use types::{EntryKind, Error, Location, Progress};

use crate::{State, describe, location_of, needs_entries};

/// The table of `core.toml` that remembers the format.
const TABLE: &str = "hash";
/// Its one key.
const KEY: &str = "format";
/// How much of a file is read, and fed to the hashers, at a time.
pub const CHUNK: usize = 1024 * 1024;

/// Base64 as a download page writes it, read with or without padding.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// How the dialogue writes a digest; `[hash] format` in `core.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Lower-case hexadecimal, the default.
    #[default]
    Hex,
    /// Upper-case hexadecimal.
    HexUpper,
    /// Standard Base64 with padding.
    Base64,
}

impl Format {
    /// The three, in the toggle's order.
    pub const ALL: [Self; 3] = [Self::Hex, Self::HexUpper, Self::Base64];

    /// The value written to `core.toml`: `hex`, `hex-upper` or `base64`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::HexUpper => "hex-upper",
            Self::Base64 => "base64",
        }
    }

    /// The format a `core.toml` value names, if any.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// A digest written in `format`.
#[must_use]
pub fn render(digest: &[u8], format: Format) -> String {
    match format {
        Format::Hex => hex::encode(digest),
        Format::HexUpper => hex::encode_upper(digest),
        Format::Base64 => BASE64.encode(digest),
    }
}

/// The bytes a pasted value stands for: surrounding whitespace ignored,
/// hex in either case, Base64 with or without padding; `None` when the
/// text does not decode in `format`.
#[must_use]
pub fn parse_verify(text: &str, format: Format) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    match format {
        Format::Hex | Format::HexUpper => hex::decode(text.to_ascii_lowercase()).ok(),
        Format::Base64 => BASE64.decode(text).ok(),
    }
}

/// The algorithms whose digest has `len` bytes: 16 is MD5, 20 is SHA-1, 32
/// is SHA-256 and BLAKE3; anything else fits none.
#[must_use]
pub fn detect(len: usize) -> &'static [Algorithm] {
    match len {
        16 => &[Algorithm::Md5],
        20 => &[Algorithm::Sha1],
        32 => &[Algorithm::Sha256, Algorithm::Blake3],
        _ => &[],
    }
}

/// What the Verify field says about `digests`: for every algorithm whose
/// length the pasted value has, whether it matches; `None` when the text
/// does not decode in `format` or its length fits no algorithm
/// (*Unrecognised length*).
#[must_use]
pub fn verify(text: &str, format: Format, digests: &Digests) -> Option<Vec<(Algorithm, bool)>> {
    let expected = parse_verify(text, format)?;
    let algorithms = detect(expected.len());
    if algorithms.is_empty() {
        return None;
    }
    Some(
        algorithms
            .iter()
            .map(|&algorithm| (algorithm, digests.get(algorithm) == expected.as_slice()))
            .collect(),
    )
}

/// The text *Copy all* puts on the clipboard: one line per algorithm,
/// `<algorithm>  <hash>  <file name>` with two spaces between.
#[must_use]
pub fn copy_all(name: &str, digests: &Digests, format: Format) -> String {
    Algorithm::ALL
        .into_iter()
        .map(|algorithm| {
            format!(
                "{}  {}  {name}",
                algorithm.label(),
                render(digests.get(algorithm), format)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The four digests of `bytes` in one pass, for tests and small inputs; a
/// file goes through the job, chunk by chunk.
#[must_use]
pub fn digests_of(bytes: &[u8]) -> Digests {
    let mut hashers = Hashers::new();
    hashers.update(bytes);
    hashers.finish()
}

/// Where the format is remembered.
fn file(platform: &dyn Platform) -> PathBuf {
    platform.config_dir().join(crate::core_toml::FILE)
}

/// The remembered format; [`Format::Hex`] until one was chosen.
#[must_use]
pub fn remembered(platform: &dyn Platform) -> Format {
    crate::core_toml::table(&file(platform), TABLE)
        .get(KEY)
        .and_then(|v| v.as_str())
        .and_then(Format::from_key)
        .unwrap_or_default()
}

/// Remembers `format` in `core.toml`, keeping the rest of the file.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn remember(platform: &dyn Platform, format: Format) -> Result<(), Error> {
    crate::core_toml::set(
        &file(platform),
        TABLE,
        KEY,
        toml::Value::String(format.key().to_owned()),
    )
}

/// The four hashers, fed the same bytes.
struct Hashers {
    md5: Md5,
    sha1: Sha1,
    sha256: Sha256,
    blake3: blake3::Hasher,
}

impl Hashers {
    fn new() -> Self {
        Self {
            md5: Md5::new(),
            sha1: Sha1::new(),
            sha256: Sha256::new(),
            blake3: blake3::Hasher::new(),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        self.md5.update(bytes);
        self.sha1.update(bytes);
        self.sha256.update(bytes);
        self.blake3.update(bytes);
    }

    fn finish(self) -> Digests {
        Digests {
            md5: self.md5.finalize().into(),
            sha1: self.sha1.finalize().into(),
            sha256: self.sha256.finalize().into(),
            blake3: *self.blake3.finalize().as_bytes(),
        }
    }
}

/// One file of the selection, with the provider that serves it.
struct Target {
    provider: Arc<dyn Provider>,
    location: Location,
    size: Option<u64>,
}

/// Hashes one file: read once in [`CHUNK`]s, every chunk fed to the four
/// hashers on the blocking pool, `done` advanced and reported against
/// `total` after each, the cancel token checked before each.
async fn hash_file(
    target: &Target,
    job: &JobCtx,
    done: &mut u64,
    total: Option<u64>,
) -> Result<Digests, Error> {
    let span = tracing::info_span!("hash.file", location = %target.location);
    let _enter = span.enter();
    let mut reader = target.provider.open_read(&target.location).await?;
    let mut hashers = Hashers::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if job.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // Fill the chunk, or as much of it as the file has left.
        let mut filled = 0;
        while filled < CHUNK {
            let read = reader
                .read(&mut buffer[filled..])
                .await
                .map_err(Error::from_io)?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }
        (hashers, buffer) = ext::blocking(move || {
            hashers.update(&buffer[..filled]);
            Ok((hashers, buffer))
        })
        .await?;
        *done += filled as u64;
        job.progress.report(Progress::Bytes { done: *done, total });
        if filled < CHUNK {
            break;
        }
    }
    Ok(hashers.finish())
}

/// The job: the selection's total from the entries' sizes (asked of the
/// provider where the listing had none), then every file in order, each
/// result stored as it arrives. Cancelling ends the job with
/// `Error::Cancelled` and stores nothing more; a file that cannot be read
/// stores its error and the others go on.
async fn hash_all(targets: Vec<Target>, hashes: Arc<Hashes>, job: JobCtx) -> Result<(), Error> {
    let mut total = Some(0u64);
    for target in &targets {
        let size = match target.size {
            Some(size) => Some(size),
            None => target
                .provider
                .stat(&target.location)
                .await
                .ok()
                .and_then(|entry| entry.size),
        };
        total = match (total, size) {
            (Some(sum), Some(size)) => Some(sum.saturating_add(size)),
            _ => None,
        };
    }
    job.progress.report(Progress::Bytes { done: 0, total });
    let mut done = 0;
    let mut failed = false;
    for (index, target) in targets.iter().enumerate() {
        match hash_file(target, &job, &mut done, total).await {
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Ok(digests) => hashes.finish(index, Ok(digests)),
            Err(error) => {
                tracing::warn!(location = %target.location, %error, "hashing failed");
                failed = true;
                hashes.finish(index, Err(error));
            }
        }
    }
    if failed {
        Err(Error::Unsupported(
            "a file could not be hashed; see the dialogue",
        ))
    } else {
        Ok(())
    }
}

/// Compute hash…: the four digests of every selected file in one job,
/// shown in the Hash dialogue.
pub(crate) struct Hash(pub(crate) Arc<State>);

impl Action for Hash {
    fn id(&self) -> ActionId {
        ActionId("core.hash")
    }
    fn label(&self) -> String {
        t!("action-compute-hash")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    /// Hidden unless every selected entry is a file: the spec hides what
    /// does not apply, and a folder cannot be hashed.
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match needs_entries(ctx) {
            Availability::Enabled
                if !ctx
                    .selection
                    .iter()
                    .all(|entry| entry.kind == EntryKind::File) =>
            {
                Availability::Hidden
            }
            other => other,
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let targets = ctx
            .selection
            .iter()
            .map(|entry| {
                let location = location_of(entry);
                Ok(Target {
                    provider: self.0.mounts.resolve(&location)?,
                    location,
                    size: entry.size,
                })
            })
            .collect::<Result<Vec<Target>, Error>>()?;
        let names = ctx
            .selection
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let hashes = Hashes::pending(names);
        let filled = Arc::clone(&hashes);
        let handle = jobs.spawn_named(format!("Hash {}", describe(&ctx.selection)), move |job| {
            hash_all(targets, filled, job)
        });
        hashes.set_job(handle.id);
        Ok(ActionOutcome::Hashes(hashes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference digests of the empty input and of `abc` (the test
    /// vectors of the `RustCrypto` crates and of BLAKE3), as hex.
    const EMPTY: [(&str, &str); 4] = [
        ("MD5", "d41d8cd98f00b204e9800998ecf8427e"),
        ("SHA-1", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
        (
            "SHA-256",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "BLAKE3",
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        ),
    ];
    const ABC: [(&str, &str); 4] = [
        ("MD5", "900150983cd24fb0d6963f7d28e17f72"),
        ("SHA-1", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        (
            "SHA-256",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "BLAKE3",
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
        ),
    ];

    fn check(input: &[u8], expected: &[(&str, &str); 4]) {
        let digests = digests_of(input);
        for (algorithm, (label, hex)) in Algorithm::ALL.into_iter().zip(expected) {
            assert_eq!(algorithm.label(), *label);
            assert_eq!(render(digests.get(algorithm), Format::Hex), *hex, "{label}");
        }
    }

    #[test]
    fn the_known_vectors_come_out_in_every_format() {
        check(b"", &EMPTY);
        check(b"abc", &ABC);
        let digests = digests_of(b"abc");
        assert_eq!(
            render(&digests.md5, Format::HexUpper),
            "900150983CD24FB0D6963F7D28E17F72"
        );
        assert_eq!(
            render(&digests.md5, Format::Base64),
            "kAFQmDzST7DWlj99KOF/cg=="
        );
        assert_eq!(
            render(&digests.sha256, Format::Base64),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
    }

    #[test]
    fn parsing_tolerates_whitespace_case_and_padding() {
        let digests = digests_of(b"abc");
        let upper = render(&digests.sha1, Format::HexUpper);
        assert_eq!(
            parse_verify(&format!("  {upper}\n"), Format::Hex).as_deref(),
            Some(&digests.sha1[..])
        );
        assert_eq!(
            parse_verify(&upper.to_lowercase(), Format::HexUpper).as_deref(),
            Some(&digests.sha1[..])
        );
        let b64 = render(&digests.sha256, Format::Base64);
        assert_eq!(
            parse_verify(&format!(" {b64} "), Format::Base64).as_deref(),
            Some(&digests.sha256[..])
        );
        assert_eq!(
            parse_verify(b64.trim_end_matches('='), Format::Base64).as_deref(),
            Some(&digests.sha256[..])
        );
        assert_eq!(parse_verify("", Format::Hex), None);
        assert_eq!(parse_verify("   ", Format::Base64), None);
        assert_eq!(parse_verify("xyz", Format::Hex), None);
        assert_eq!(parse_verify("abc", Format::Hex), None, "an odd length");
        assert_eq!(parse_verify("!!!!", Format::Base64), None);
    }

    #[test]
    fn lengths_map_to_algorithms() {
        assert_eq!(detect(16), &[Algorithm::Md5]);
        assert_eq!(detect(20), &[Algorithm::Sha1]);
        assert_eq!(detect(32), &[Algorithm::Sha256, Algorithm::Blake3]);
        assert_eq!(detect(0), [] as [Algorithm; 0]);
        assert_eq!(detect(5), [] as [Algorithm; 0]);
        assert_eq!(detect(64), [] as [Algorithm; 0]);
    }

    #[test]
    fn verify_compares_every_row_of_the_pasted_length() {
        let digests = digests_of(b"abc");
        let sha256 = render(&digests.sha256, Format::Hex);
        assert_eq!(
            verify(&sha256, Format::Hex, &digests),
            Some(vec![(Algorithm::Sha256, true), (Algorithm::Blake3, false)])
        );
        let blake3 = render(&digests.blake3, Format::HexUpper);
        assert_eq!(
            verify(&blake3, Format::Hex, &digests),
            Some(vec![(Algorithm::Sha256, false), (Algorithm::Blake3, true)])
        );
        let sha1 = render(&digests.sha1, Format::Hex);
        assert_eq!(
            verify(&sha1, Format::Hex, &digests),
            Some(vec![(Algorithm::Sha1, true)])
        );
        assert_eq!(
            verify(&"0".repeat(40), Format::Hex, &digests),
            Some(vec![(Algorithm::Sha1, false)])
        );
        assert_eq!(verify(&"0".repeat(10), Format::Hex, &digests), None);
        assert_eq!(verify("not hex", Format::Hex, &digests), None);
        // The toggle decides how the text is read: a hex SHA-256 is not
        // 32 bytes of Base64.
        assert_eq!(verify(&sha256, Format::Base64, &digests), None);
        let b64 = render(&digests.md5, Format::Base64);
        assert_eq!(
            verify(&b64, Format::Base64, &digests),
            Some(vec![(Algorithm::Md5, true)])
        );
    }

    #[test]
    fn copy_all_lists_the_four_lines() {
        let digests = digests_of(b"abc");
        let text = copy_all("abc.txt", &digests, Format::Hex);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "MD5  900150983cd24fb0d6963f7d28e17f72  abc.txt");
        assert!(lines[3].starts_with("BLAKE3  6437b3ac"));
        assert!(lines[3].ends_with("  abc.txt"));
    }

    #[test]
    fn format_keys_round_trip() {
        for format in Format::ALL {
            assert_eq!(Format::from_key(format.key()), Some(format));
        }
        assert_eq!(Format::from_key("HEX"), None);
        assert_eq!(Format::default(), Format::Hex);
    }

    #[test]
    fn the_format_is_remembered_in_core_toml() {
        let fake = platform::Fake::new().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(remembered(&fake), Format::Hex);
        remember(&fake, Format::Base64).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(remembered(&fake), Format::Base64);
        let text = std::fs::read_to_string(file(&fake)).unwrap_or_default();
        assert!(text.contains("[hash]"), "{text}");
        assert!(text.contains("format = \"base64\""), "{text}");
        remember(&fake, Format::HexUpper).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(remembered(&fake), Format::HexUpper);
    }
}
