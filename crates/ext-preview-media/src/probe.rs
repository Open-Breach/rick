//! What a media file says about itself without decoding it: the container
//! and stream headers of MP3, WAV, MP4 and AVI, and the tags they carry
//! (ID3 through the `id3` crate, RIFF `INFO` lists, MP4 `ilst` items).
//! This is what the preview shows when GStreamer is not there, and the
//! File info rows in every case.

use std::time::Duration;

/// The four formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// MPEG audio layer III.
    Mp3,
    /// RIFF WAVE.
    Wav,
    /// ISO base media: MP4, M4V, M4A, MOV.
    Mp4,
    /// RIFF AVI.
    Avi,
}

impl Kind {
    /// All four.
    pub const ALL: [Self; 4] = [Self::Mp3, Self::Wav, Self::Mp4, Self::Avi];

    /// The name shown in the status bar and File info.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mp3 => "MP3 audio",
            Self::Wav => "WAV audio",
            Self::Mp4 => "MP4 video",
            Self::Avi => "AVI video",
        }
    }

    /// Whether the format is audio only.
    #[must_use]
    pub fn is_audio(self) -> bool {
        matches!(self, Self::Mp3 | Self::Wav)
    }

    /// The extensions the kind is known by, lower case.
    #[must_use]
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Mp3 => &["mp3"],
            Self::Wav => &["wav", "wave"],
            Self::Mp4 => &["mp4", "m4v", "m4a", "mov"],
            Self::Avi => &["avi"],
        }
    }
}

/// The lower-case extension of a name, if any.
fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// What the file is, with the score to claim it: 200 for a signature, 150
/// for MPEG audio frames without a tag, 120 for a known extension on an
/// empty sniff, 60 for a known extension the content contradicts.
#[must_use]
pub fn detect(name: &str, sniff: &[u8]) -> Option<(Kind, u8)> {
    let by_magic = if sniff.starts_with(b"ID3") {
        Some((Kind::Mp3, 200))
    } else if sniff.len() >= 12 && &sniff[..4] == b"RIFF" && &sniff[8..12] == b"WAVE" {
        Some((Kind::Wav, 200))
    } else if sniff.len() >= 12 && &sniff[..4] == b"RIFF" && &sniff[8..12] == b"AVI " {
        Some((Kind::Avi, 200))
    } else if sniff.len() >= 12 && &sniff[4..8] == b"ftyp" {
        Some((Kind::Mp4, 200))
    } else if mp3::frames_at(sniff, 0).is_some() {
        Some((Kind::Mp3, 150))
    } else {
        None
    };
    if by_magic.is_some() {
        return by_magic;
    }
    let ext = extension(name)?;
    let kind = Kind::ALL
        .into_iter()
        .find(|k| k.extensions().contains(&ext.as_str()))?;
    Some((kind, if sniff.is_empty() { 120 } else { 60 }))
}

/// What the headers say.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Probe {
    /// Playing time, when the headers give it.
    pub duration: Option<Duration>,
    /// Frame size of the video.
    pub dimensions: Option<(u32, u32)>,
    /// Frames per second of the video.
    pub frame_rate: Option<f64>,
    /// The video codec's name.
    pub video_codec: Option<String>,
    /// The audio codec's name.
    pub audio_codec: Option<String>,
    /// Audio sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Audio channels.
    pub channels: Option<u16>,
    /// Bit rate in bits per second.
    pub bit_rate: Option<u32>,
    /// Tags: title, artist, album, …, as key and value rows.
    pub tags: Vec<(String, String)>,
    /// Embedded cover art, encoded (JPEG or PNG).
    pub cover: Option<Vec<u8>>,
}

impl Probe {
    /// The codec line for the status bar: video codec and audio codec.
    #[must_use]
    pub fn codec_line(&self) -> Option<String> {
        match (&self.video_codec, &self.audio_codec) {
            (Some(v), Some(a)) => Some(format!("{v} + {a}")),
            (Some(c), None) | (None, Some(c)) => Some(c.clone()),
            (None, None) => None,
        }
    }
}

/// Reads `bytes` (the start of the file, or all of it) as `kind`.
#[must_use]
pub fn read(kind: Kind, bytes: &[u8], file_size: u64) -> Probe {
    match kind {
        Kind::Mp3 => mp3::probe(bytes, file_size),
        Kind::Wav => riff::probe_wav(bytes),
        Kind::Mp4 => mp4::probe(bytes),
        Kind::Avi => riff::probe_avi(bytes),
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// `m:ss` or `h:mm:ss`.
#[must_use]
pub fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// MPEG audio: ID3 tags and the frame headers.
pub mod mp3 {
    use std::io::Cursor;
    use std::time::Duration;

    use super::{Probe, be32};

    const BITRATES_V1_L3: [u32; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const BITRATES_V2_L3: [u32; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const RATES_V1: [u32; 4] = [44_100, 48_000, 32_000, 0];

    /// One frame header.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Header {
        /// MPEG version: 1, 2 or 25 (for 2.5).
        pub version: u8,
        /// Bit rate in kb/s.
        pub bitrate: u32,
        /// Sample rate in Hz.
        pub sample_rate: u32,
        /// Channels: 1 or 2.
        pub channels: u16,
        /// Frame length in bytes.
        pub length: usize,
        /// Samples per frame.
        pub samples: u32,
    }

    /// Parses a layer III frame header at `at`.
    #[must_use]
    pub fn header(bytes: &[u8], at: usize) -> Option<Header> {
        let b = bytes.get(at..at + 4)?;
        if b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
            return None;
        }
        let version = match (b[1] >> 3) & 0x3 {
            3 => 1,
            2 => 2,
            0 => 25,
            _ => return None,
        };
        let layer = (b[1] >> 1) & 0x3;
        if layer != 1 {
            // Layer III only.
            return None;
        }
        let bitrate_index = usize::from(b[2] >> 4);
        let rate_index = usize::from((b[2] >> 2) & 0x3);
        let padding = u32::from((b[2] >> 1) & 1);
        let bitrate = if version == 1 {
            BITRATES_V1_L3[bitrate_index]
        } else {
            BITRATES_V2_L3[bitrate_index]
        };
        let sample_rate = RATES_V1[rate_index]
            / match version {
                1 => 1,
                2 => 2,
                _ => 4,
            };
        if bitrate == 0 || sample_rate == 0 {
            return None;
        }
        let channels = if (b[3] >> 6) == 3 { 1 } else { 2 };
        let samples = if version == 1 { 1152 } else { 576 };
        let length = usize::try_from(samples / 8 * bitrate * 1000 / sample_rate + padding).ok()?;
        Some(Header {
            version,
            bitrate,
            sample_rate,
            channels,
            length,
            samples,
        })
    }

    /// Two frame headers in a row starting at `at`, which is what tells an
    /// MP3 from noise.
    #[must_use]
    pub fn frames_at(bytes: &[u8], at: usize) -> Option<Header> {
        let first = header(bytes, at)?;
        let next = at + first.length;
        if next + 4 <= bytes.len() {
            header(bytes, next)?;
        }
        Some(first)
    }

    /// The size of an `ID3v2` tag at the start, 0 when there is none.
    #[must_use]
    pub fn id3v2_size(bytes: &[u8]) -> usize {
        if bytes.len() < 10 || &bytes[..3] != b"ID3" {
            return 0;
        }
        let size = bytes[6..10]
            .iter()
            .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7F));
        let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
        10 + size + footer
    }

    /// The first frame header after the tag, found within a window.
    fn first_frame(bytes: &[u8]) -> Option<(usize, Header)> {
        let start = id3v2_size(bytes);
        let end = bytes.len().min(start + 64 * 1024);
        (start..end).find_map(|at| frames_at(bytes, at).map(|h| (at, h)))
    }

    /// The frame count from a Xing or Info header in the first frame.
    fn xing_frames(bytes: &[u8], at: usize, h: Header) -> Option<u32> {
        let side_info = match (h.version, h.channels) {
            (1, 1) => 17,
            (1, _) => 32,
            (_, 1) => 9,
            _ => 17,
        };
        let tag = bytes.get(at + 4 + side_info..at + 4 + side_info + 4)?;
        if tag != b"Xing" && tag != b"Info" {
            return None;
        }
        let flags = be32(bytes, at + 4 + side_info + 4)?;
        if flags & 1 == 0 {
            return None;
        }
        be32(bytes, at + 4 + side_info + 8)
    }

    /// Reads the tags and the first frame.
    #[must_use]
    pub fn probe(bytes: &[u8], file_size: u64) -> Probe {
        let mut probe = Probe::default();
        if let Ok(tag) = id3::v1v2::read_from(Cursor::new(bytes)) {
            use id3::TagLike as _;
            let mut push = |key: &str, value: Option<String>| {
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    probe.tags.push((key.to_owned(), value));
                }
            };
            push("Title", tag.title().map(str::to_owned));
            push("Artist", tag.artist().map(str::to_owned));
            push("Album", tag.album().map(str::to_owned));
            push("Album artist", tag.album_artist().map(str::to_owned));
            push(
                "Year",
                tag.year()
                    .map(|y| y.to_string())
                    .or_else(|| tag.date_recorded().map(|d| d.year.to_string())),
            );
            push(
                "Track",
                tag.track().map(|t| match tag.total_tracks() {
                    Some(total) => format!("{t} of {total}"),
                    None => t.to_string(),
                }),
            );
            push("Genre", tag.genre().map(str::to_owned));
            push("Comment", tag.comments().next().map(|c| c.text.clone()));
            if let Some(ms) = tag.duration() {
                probe.duration = Some(Duration::from_millis(u64::from(ms)));
            }
            if let Some(picture) = tag.pictures().next() {
                probe.cover = Some(picture.data.clone());
            }
        }
        if let Some((at, h)) = first_frame(bytes) {
            probe.audio_codec = Some(format!(
                "MPEG-{} Layer III",
                match h.version {
                    1 => "1",
                    2 => "2",
                    _ => "2.5",
                }
            ));
            probe.sample_rate = Some(h.sample_rate);
            probe.channels = Some(h.channels);
            probe.bit_rate = Some(h.bitrate * 1000);
            if probe.duration.is_none() {
                let seconds = if let Some(frames) = xing_frames(bytes, at, h) {
                    f64::from(frames) * f64::from(h.samples) / f64::from(h.sample_rate)
                } else {
                    let audio_bytes = file_size.saturating_sub(id3v2_size(bytes) as u64);
                    // A constant bit rate estimate; the precision loss on
                    // a file size is immaterial for a duration.
                    #[allow(clippy::cast_precision_loss)]
                    let seconds = audio_bytes as f64 * 8.0 / f64::from(h.bitrate * 1000);
                    seconds
                };
                probe.duration = Some(Duration::from_secs_f64(seconds.max(0.0)));
            }
        }
        probe
    }
}

/// RIFF containers: WAV and AVI.
pub mod riff {
    use std::time::Duration;

    use super::{Probe, le16, le32};

    /// Walks the chunks of a RIFF `LIST` or file body, calling `f` with
    /// each chunk's id and body.
    fn chunks<'a>(body: &'a [u8], mut f: impl FnMut(&'a [u8], &'a [u8])) {
        let mut at = 0;
        while at + 8 <= body.len() {
            let id = &body[at..at + 4];
            let Some(size) = le32(body, at + 4) else {
                break;
            };
            let size = usize::try_from(size).unwrap_or(usize::MAX);
            let start = at + 8;
            let end = start.saturating_add(size).min(body.len());
            f(id, &body[start..end]);
            at = start.saturating_add(size + (size & 1));
        }
    }

    /// The `INFO` list's tags as rows.
    fn info_tags(body: &[u8], tags: &mut Vec<(String, String)>) {
        chunks(body, |id, value| {
            let key = match id {
                b"INAM" => "Title",
                b"IART" => "Artist",
                b"IPRD" => "Album",
                b"ICRD" => "Date",
                b"IGNR" => "Genre",
                b"ICMT" => "Comment",
                b"ISFT" => "Software",
                b"ICOP" => "Copyright",
                _ => return,
            };
            let text = String::from_utf8_lossy(value)
                .trim_end_matches('\0')
                .trim()
                .to_owned();
            if !text.is_empty() {
                tags.push((key.to_owned(), text));
            }
        });
    }

    /// A `WAVEFORMATEX` as the codec name, rate, channels and byte rate.
    fn wave_format(fmt: &[u8]) -> Option<(String, u32, u16, u32)> {
        let tag = le16(fmt, 0)?;
        let channels = le16(fmt, 2)?;
        let rate = le32(fmt, 4)?;
        let byte_rate = le32(fmt, 8)?;
        let bits = le16(fmt, 14).unwrap_or(0);
        let tag = if tag == 0xFFFE {
            le16(fmt, 24).unwrap_or(tag)
        } else {
            tag
        };
        let codec = match tag {
            1 => format!("PCM {bits}-bit"),
            3 => format!("PCM {bits}-bit float"),
            6 => "A-law".to_owned(),
            7 => "µ-law".to_owned(),
            0x55 => "MPEG Layer III".to_owned(),
            0x2000 => "AC-3".to_owned(),
            other => format!("WAVE format 0x{other:X}"),
        };
        Some((codec, rate, channels, byte_rate))
    }

    /// Reads a WAV's `fmt `, `data` and `INFO` chunks.
    #[must_use]
    pub fn probe_wav(bytes: &[u8]) -> Probe {
        let mut probe = Probe::default();
        if bytes.len() < 12 {
            return probe;
        }
        let mut byte_rate = 0;
        let mut data_size = None;
        chunks(&bytes[12..], |id, body| match id {
            b"fmt " => {
                if let Some((codec, rate, channels, rate_bytes)) = wave_format(body) {
                    probe.audio_codec = Some(codec);
                    probe.sample_rate = Some(rate);
                    probe.channels = Some(channels);
                    probe.bit_rate = Some(rate_bytes.saturating_mul(8));
                    byte_rate = rate_bytes;
                }
            }
            b"data" => {
                // The body may be cut by the sniff; the declared size is
                // in the header before it.
                data_size = Some(body.len());
            }
            b"LIST" if body.starts_with(b"INFO") => info_tags(&body[4..], &mut probe.tags),
            _ => {}
        });
        // The declared data size, from the chunk header, not the cut body.
        let mut at = 12;
        while at + 8 <= bytes.len() {
            if &bytes[at..at + 4] == b"data"
                && let Some(size) = le32(bytes, at + 4)
            {
                data_size = Some(usize::try_from(size).unwrap_or(usize::MAX));
                break;
            }
            let size = le32(bytes, at + 4)
                .map_or(usize::MAX, |s| usize::try_from(s).unwrap_or(usize::MAX));
            at = (at + 8).saturating_add(size + (size & 1));
        }
        if let Some(size) = data_size
            && byte_rate > 0
        {
            // A byte count to a duration; precision loss is immaterial.
            #[allow(clippy::cast_precision_loss)]
            let seconds = size as f64 / f64::from(byte_rate);
            probe.duration = Some(Duration::from_secs_f64(seconds));
        }
        probe
    }

    fn fourcc(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .trim()
            .to_owned()
    }

    /// A video codec name for a `FourCC`.
    fn video_codec_name(cc: &str) -> String {
        match cc.to_ascii_uppercase().as_str() {
            "MJPG" => "Motion JPEG".to_owned(),
            "H264" | "AVC1" | "X264" => "H.264".to_owned(),
            "HEVC" | "H265" | "HVC1" => "H.265".to_owned(),
            "XVID" | "DIVX" | "DX50" | "FMP4" | "MP4V" => "MPEG-4 Part 2".to_owned(),
            "DIV3" | "MP43" => "MPEG-4 v3".to_owned(),
            "VP80" => "VP8".to_owned(),
            "VP90" => "VP9".to_owned(),
            "AV01" => "AV1".to_owned(),
            "" | "DIB" | "RGB" => "Uncompressed".to_owned(),
            other => other.to_owned(),
        }
    }

    /// Reads an AVI's `hdrl` headers and `INFO` list.
    #[must_use]
    pub fn probe_avi(bytes: &[u8]) -> Probe {
        let mut probe = Probe::default();
        if bytes.len() < 12 {
            return probe;
        }
        let mut micros_per_frame = 0u64;
        let mut total_frames = 0u64;
        chunks(&bytes[12..], |id, body| {
            if id != b"LIST" || body.len() < 4 {
                return;
            }
            match &body[..4] {
                b"hdrl" => chunks(&body[4..], |id, body| match id {
                    b"avih" => {
                        micros_per_frame = u64::from(le32(body, 0).unwrap_or(0));
                        total_frames = u64::from(le32(body, 16).unwrap_or(0));
                        if let (Some(w), Some(h)) = (le32(body, 32), le32(body, 36))
                            && w > 0
                            && h > 0
                        {
                            probe.dimensions = Some((w, h));
                        }
                    }
                    b"LIST" if body.starts_with(b"strl") => {
                        let mut stream_type = [0u8; 4];
                        let mut handler = String::new();
                        let mut scale = 0u32;
                        let mut rate = 0u32;
                        chunks(&body[4..], |id, body| match id {
                            b"strh" => {
                                stream_type.copy_from_slice(body.get(..4).unwrap_or(&[0; 4]));
                                handler = fourcc(body.get(4..8).unwrap_or(&[]));
                                scale = le32(body, 20).unwrap_or(0);
                                rate = le32(body, 24).unwrap_or(0);
                            }
                            b"strf" => match &stream_type {
                                b"vids" => {
                                    let cc = fourcc(body.get(16..20).unwrap_or(&[]));
                                    let name = if cc.is_empty() { handler.clone() } else { cc };
                                    probe.video_codec = Some(video_codec_name(&name));
                                    if scale > 0 && rate > 0 {
                                        probe.frame_rate = Some(f64::from(rate) / f64::from(scale));
                                    }
                                }
                                b"auds" => {
                                    if let Some((codec, rate, channels, byte_rate)) =
                                        wave_format(body)
                                    {
                                        probe.audio_codec = Some(codec);
                                        probe.sample_rate = Some(rate);
                                        probe.channels = Some(channels);
                                        if probe.bit_rate.is_none() {
                                            probe.bit_rate = Some(byte_rate.saturating_mul(8));
                                        }
                                    }
                                }
                                _ => {}
                            },
                            _ => {}
                        });
                    }
                    _ => {}
                }),
                b"INFO" => info_tags(&body[4..], &mut probe.tags),
                _ => {}
            }
        });
        if micros_per_frame > 0 && total_frames > 0 {
            probe.duration = Some(Duration::from_micros(micros_per_frame * total_frames));
            if probe.frame_rate.is_none() {
                // Microseconds to a rate; the loss is immaterial.
                #[allow(clippy::cast_precision_loss)]
                let fps = 1_000_000.0 / micros_per_frame as f64;
                probe.frame_rate = Some(fps);
            }
        }
        probe
    }
}

/// ISO base media files: the `moov` box and what it holds.
pub mod mp4 {
    use std::time::Duration;

    use super::{Probe, be32};

    /// Walks the boxes in `body`, calling `f` with each box's type and body.
    fn boxes<'a>(body: &'a [u8], mut f: impl FnMut(&'a [u8], &'a [u8])) {
        let mut at = 0;
        while at + 8 <= body.len() {
            let Some(size32) = be32(body, at) else {
                break;
            };
            let kind = &body[at + 4..at + 8];
            let (header, size) = match size32 {
                0 => (8, body.len() - at),
                1 => {
                    let Some(hi) = be32(body, at + 8) else { break };
                    let Some(lo) = be32(body, at + 12) else { break };
                    let size = (u64::from(hi) << 32) | u64::from(lo);
                    (16, usize::try_from(size).unwrap_or(usize::MAX))
                }
                n => (8, usize::try_from(n).unwrap_or(usize::MAX)),
            };
            if size < header {
                break;
            }
            let start = at + header;
            let end = at.saturating_add(size).min(body.len());
            f(kind, &body[start.min(end)..end]);
            if at.saturating_add(size) <= at {
                break;
            }
            at = at.saturating_add(size);
        }
    }

    /// A codec name for a sample entry's type.
    fn codec_name(cc: &[u8]) -> String {
        match cc {
            b"avc1" | b"avc3" => "H.264".to_owned(),
            b"hvc1" | b"hev1" => "H.265".to_owned(),
            b"vp09" => "VP9".to_owned(),
            b"vp08" => "VP8".to_owned(),
            b"av01" => "AV1".to_owned(),
            b"mp4v" => "MPEG-4 Part 2".to_owned(),
            b"mp4a" => "AAC".to_owned(),
            b"ac-3" => "AC-3".to_owned(),
            b"ec-3" => "E-AC-3".to_owned(),
            b"Opus" => "Opus".to_owned(),
            b"fLaC" => "FLAC".to_owned(),
            b"alac" => "Apple Lossless".to_owned(),
            b".mp3" => "MP3".to_owned(),
            b"jpeg" => "Motion JPEG".to_owned(),
            other => String::from_utf8_lossy(other).trim().to_owned(),
        }
    }

    /// Reads `moov`: `mvhd` for the duration, each `trak` for its
    /// handler, size and codec, `udta/meta/ilst` for the tags.
    #[must_use]
    pub fn probe(bytes: &[u8]) -> Probe {
        let mut probe = Probe::default();
        boxes(bytes, |kind, body| {
            if kind != b"moov" {
                return;
            }
            boxes(body, |kind, body| match kind {
                b"mvhd" => {
                    let version = body.first().copied().unwrap_or(0);
                    let (timescale, duration) = if version == 1 {
                        (
                            be32(body, 20),
                            be32(body, 24)
                                .zip(be32(body, 28))
                                .map(|(hi, lo)| (u64::from(hi) << 32) | u64::from(lo)),
                        )
                    } else {
                        (be32(body, 12), be32(body, 16).map(u64::from))
                    };
                    if let (Some(timescale), Some(duration)) = (timescale, duration)
                        && timescale > 0
                        && duration != u64::MAX
                        && duration != u64::from(u32::MAX)
                    {
                        // Ticks to seconds; the loss is immaterial.
                        #[allow(clippy::cast_precision_loss)]
                        let seconds = duration as f64 / f64::from(timescale);
                        probe.duration = Some(Duration::from_secs_f64(seconds));
                    }
                }
                b"trak" => track(body, &mut probe),
                b"udta" => boxes(body, |kind, body| {
                    if kind == b"meta" && body.len() > 4 {
                        boxes(&body[4..], |kind, body| {
                            if kind == b"ilst" {
                                items(body, &mut probe);
                            }
                        });
                    }
                }),
                _ => {}
            });
        });
        probe
    }

    fn track(body: &[u8], probe: &mut Probe) {
        let mut handler = [0u8; 4];
        let mut size = None;
        let mut codec = None;
        let mut duration = None;
        let mut timescale = None;
        boxes(body, |kind, body| match kind {
            b"tkhd" => {
                let version = body.first().copied().unwrap_or(0);
                let at = if version == 1 { 88 } else { 76 };
                if let (Some(w), Some(h)) = (be32(body, at), be32(body, at + 4))
                    && w > 0
                    && h > 0
                {
                    size = Some((w >> 16, h >> 16));
                }
            }
            b"mdia" => boxes(body, |kind, body| match kind {
                b"hdlr" => handler.copy_from_slice(body.get(8..12).unwrap_or(&[0; 4])),
                b"mdhd" => {
                    let version = body.first().copied().unwrap_or(0);
                    if version == 1 {
                        timescale = be32(body, 20);
                        duration = be32(body, 24)
                            .zip(be32(body, 28))
                            .map(|(hi, lo)| (u64::from(hi) << 32) | u64::from(lo));
                    } else {
                        timescale = be32(body, 12);
                        duration = be32(body, 16).map(u64::from);
                    }
                }
                b"minf" => boxes(body, |kind, body| {
                    if kind == b"stbl" {
                        boxes(body, |kind, body| {
                            if kind == b"stsd" && body.len() >= 16 {
                                // Version and flags, entry count, then the
                                // first sample entry: size and type.
                                codec = Some(codec_name(&body[12..16]));
                                let entry = &body[8..];
                                if handler == *b"soun" && entry.len() >= 36 {
                                    probe.channels =
                                        Some(u16::from_be_bytes([entry[24], entry[25]]));
                                    probe.sample_rate = be32(entry, 32).map(|r| r >> 16);
                                }
                            }
                        });
                    }
                }),
                _ => {}
            }),
            _ => {}
        });
        match &handler {
            b"vide" => {
                probe.video_codec = codec;
                if size.is_some() {
                    probe.dimensions = size;
                }
                if let (Some(ts), Some(d)) = (timescale, duration)
                    && d > 0
                    && ts > 0
                    && probe.frame_rate.is_none()
                {
                    // The sample count lives in `stts`; the track duration
                    // over the file's duration is not the frame rate, so
                    // leave it to GStreamer.
                }
            }
            b"soun" => probe.audio_codec = codec,
            _ => {}
        }
    }

    fn items(body: &[u8], probe: &mut Probe) {
        boxes(body, |kind, body| {
            let key = match kind {
                b"\xA9nam" => "Title",
                b"\xA9ART" => "Artist",
                b"\xA9alb" => "Album",
                b"aART" => "Album artist",
                b"\xA9day" => "Year",
                b"\xA9gen" | b"gnre" => "Genre",
                b"\xA9cmt" => "Comment",
                b"\xA9too" => "Encoder",
                b"covr" => "",
                _ => return,
            };
            boxes(body, |kind, data| {
                if kind != b"data" || data.len() < 8 {
                    return;
                }
                let value = &data[8..];
                if key.is_empty() {
                    if probe.cover.is_none() {
                        probe.cover = Some(value.to_vec());
                    }
                } else {
                    let text = String::from_utf8_lossy(value).trim().to_owned();
                    if !text.is_empty() {
                        probe.tags.push((key.to_owned(), text));
                    }
                }
            });
        });
    }
}
