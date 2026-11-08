//! Audio and video previews: MP3, WAV, MP4 and AVI through GStreamer, one
//! code path on every platform (system GStreamer on Linux, the runtime a
//! bundle carries on macOS and Windows, QUESTIONS.md 106).
//!
//! `open` reads what the file says about itself without decoding (the
//! headers and tags, [`probe`]), then, when the plugin was built with the
//! `gstreamer` feature and GStreamer initialises, asks the Discoverer for
//! the streams and tags, opens a `gst::Player` that prerolls the first
//! frame and becomes the document's [`Transport`] (play, pause, seek,
//! volume, speed, frame step, loop), and for audio decodes the file to a
//! [`Waveform`] inside the preview job. `render` returns the latest video
//! frame, or the waveform with the position marked and the cover art above
//! it. Playback starts when the transport is asked to and stops when the
//! document is dropped, which the selection change does. Without GStreamer
//! (not built in, missing, or missing the plug-in a file needs) the
//! document shows the metadata and says what is missing.

#![deny(unsafe_code)]

pub mod fixtures;
#[cfg(feature = "gstreamer")]
pub mod gst;
pub mod probe;
pub mod waveform;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection,
    Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider, Registry,
    RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, EntryKind, Error, ExtId, Location};

pub use probe::{Kind, Probe, detect, format_duration};
pub use waveform::{Cover, Waveform};

/// The extension id.
pub const ID: ExtId = ExtId("preview-media");

/// How much of the file the header probe reads from the start.
pub const PROBE_BYTES: usize = 4 * 1024 * 1024;

/// A file on another provider than the local one is copied to a temporary
/// file for GStreamer; above this size only its headers are read.
pub const REMOTE_LIMIT: usize = 256 * 1024 * 1024;

/// The extension: registers [`MediaPreviewer`].
#[derive(Debug)]
pub struct MediaPreview {
    manifest: Manifest,
}

impl Default for MediaPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Media preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for MediaPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(MediaPreviewer));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(MediaPreview::new()) as Arc<dyn Extension>);

/// Previews audio and video files.
#[derive(Debug, Default)]
pub struct MediaPreviewer;

/// Whether playback is possible, and why not when it is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Playback {
    /// GStreamer plays the file; the version, and whether the audio goes
    /// nowhere for want of a device.
    Ready {
        /// `gst_version_string()`.
        version: String,
        /// No audio device could be opened.
        muted: bool,
    },
    /// No playback, for the reason given: not built with GStreamer, the
    /// runtime missing, or the plug-in this file needs missing.
    Unavailable(String),
}

/// The reason shown when the plugin was built without GStreamer.
pub const NOT_BUILT: &str = "Playback is not available: this build of the media previewer has no \
                             GStreamer support.";

/// The decoded first frame of a video, kept for `render`.
#[derive(Debug, Clone)]
pub struct Poster {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA.
    pub data: Arc<[u8]>,
}

/// An opened media file.
pub struct MediaDoc {
    kind: Kind,
    probe: Probe,
    size: Option<u64>,
    playback: Playback,
    streams: Vec<(String, String)>,
    tags: Vec<(String, String)>,
    waveform: Option<Waveform>,
    /// The playing time measured by decoding the whole file for the
    /// waveform, exact where the headers only estimate.
    measured: Option<Duration>,
    cover: Option<Cover>,
    #[cfg(feature = "gstreamer")]
    player: Option<gst::Player>,
    poster: Option<Poster>,
}

impl std::fmt::Debug for MediaDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaDoc")
            .field("kind", &self.kind)
            .field("duration", &self.probe.duration)
            .field("playback", &self.playback)
            .field("waveform", &self.waveform.is_some())
            .finish_non_exhaustive()
    }
}

impl MediaDoc {
    /// A document without playback: the headers, and the reason.
    #[must_use]
    pub fn without_playback(kind: Kind, probe: Probe, size: Option<u64>, reason: String) -> Self {
        let cover = probe.cover.as_deref().and_then(Cover::decode);
        let tags = probe.tags.clone();
        Self {
            kind,
            probe,
            size,
            playback: Playback::Unavailable(reason),
            streams: Vec::new(),
            tags,
            waveform: None,
            measured: None,
            cover,
            #[cfg(feature = "gstreamer")]
            player: None,
            poster: None,
        }
    }

    /// The kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// What the headers said.
    #[must_use]
    pub fn probe(&self) -> &Probe {
        &self.probe
    }

    /// Whether playback is possible.
    #[must_use]
    pub fn playback(&self) -> &Playback {
        &self.playback
    }

    /// The waveform, when it was computed.
    #[must_use]
    pub fn waveform(&self) -> Option<&Waveform> {
        self.waveform.as_ref()
    }

    /// The playing time: measured by the waveform decode when there was
    /// one, else the player's, else what the headers say.
    fn duration(&self) -> Option<Duration> {
        if let Some(measured) = self.measured {
            return Some(measured);
        }
        #[cfg(feature = "gstreamer")]
        if let Some(player) = &self.player
            && let Some(duration) = player.duration()
        {
            return Some(duration);
        }
        self.probe.duration
    }

    /// The current position and the duration, while a player exists.
    // Without the feature there is no player and nothing to read.
    #[cfg_attr(not(feature = "gstreamer"), allow(clippy::unused_self))]
    fn position(&self) -> Option<(Duration, Duration)> {
        #[cfg(feature = "gstreamer")]
        if let Some(player) = &self.player {
            return player.duration().map(|d| (player.position(), d));
        }
        None
    }

    /// The text shown when nothing can be drawn: the reason and the facts.
    fn fallback_text(&self) -> String {
        let mut text = format!("{}\n\n", self.kind.name());
        if let Playback::Unavailable(reason) = &self.playback {
            text.push_str(reason);
            text.push_str("\n\n");
        }
        for (key, value) in self.facts() {
            text.push_str(&key);
            text.push_str(": ");
            text.push_str(&value);
            text.push('\n');
        }
        for (key, value) in &self.tags {
            text.push_str(key);
            text.push_str(": ");
            text.push_str(value);
            text.push('\n');
        }
        text
    }

    /// The File info rows after the kind.
    fn facts(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        if let Some(duration) = self.duration() {
            rows.push(("Duration".to_owned(), format_duration(duration)));
        }
        if let Some((w, h)) = self.probe.dimensions {
            rows.push(("Dimensions".to_owned(), format!("{w} × {h}")));
        }
        if let Some(fps) = self.probe.frame_rate {
            let precision = if fps.fract() == 0.0 { 0 } else { 2 };
            rows.push(("Frame rate".to_owned(), format!("{fps:.precision$} fps")));
        }
        if let Some(codec) = &self.probe.video_codec {
            rows.push(("Video codec".to_owned(), codec.clone()));
        }
        if let Some(codec) = &self.probe.audio_codec {
            rows.push(("Audio codec".to_owned(), codec.clone()));
        }
        if let Some(rate) = self.probe.sample_rate {
            rows.push(("Sample rate".to_owned(), format!("{rate} Hz")));
        }
        if let Some(channels) = self.probe.channels {
            rows.push((
                "Channels".to_owned(),
                match channels {
                    1 => "mono".to_owned(),
                    2 => "stereo".to_owned(),
                    n => n.to_string(),
                },
            ));
        }
        if let Some(bits) = self.probe.bit_rate {
            rows.push(("Bit rate".to_owned(), format!("{} kb/s", bits / 1000)));
        }
        if let Some(size) = self.size {
            rows.push(("Size".to_owned(), format_size(size)));
        }
        rows
    }
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    // Sizes shown to one decimal; the precision loss is immaterial.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    }
}

impl PreviewDoc for MediaDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        #[cfg(feature = "gstreamer")]
        if let Some(player) = &mut self.player {
            if let Some(error) = player.error().cloned() {
                self.playback = Playback::Unavailable(error.to_string());
                self.player = None;
            } else if !self.kind.is_audio()
                && let Some(frame) = player.frame()
            {
                self.poster = Some(Poster {
                    width: frame.width,
                    height: frame.height,
                    data: Arc::clone(&frame.data),
                });
                return Ok(Frame::Rgba {
                    width: frame.width,
                    height: frame.height,
                    data: frame.data,
                });
            }
        }
        if self.kind.is_audio() && (self.waveform.is_some() || self.cover.is_some()) {
            return Ok(waveform::picture(
                self.waveform.as_ref(),
                self.cover.as_ref(),
                req.width,
                req.height,
                self.position(),
                req.dark,
            ));
        }
        if let Some(poster) = &self.poster {
            return Ok(Frame::Rgba {
                width: poster.width,
                height: poster.height,
                data: Arc::clone(&poster.data),
            });
        }
        if let Some(cover) = &self.cover {
            return Ok(waveform::picture(
                None,
                Some(cover),
                req.width,
                req.height,
                None,
                req.dark,
            ));
        }
        Ok(Frame::Text(TextBlock {
            text: self.fallback_text(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut rows = vec![("Kind".to_owned(), self.kind.name().to_owned())];
        rows.extend(self.facts());
        rows.push((
            "Playback".to_owned(),
            match &self.playback {
                Playback::Ready {
                    version,
                    muted: false,
                } => version.clone(),
                Playback::Ready {
                    version,
                    muted: true,
                } => format!("{version}, no audio device"),
                Playback::Unavailable(reason) => reason.clone(),
            },
        ));
        let mut sections = vec![InfoSection {
            id: "file",
            title: Text("preview-media-file-info"),
            icon: Icon::Info,
            rows,
        }];
        if !self.tags.is_empty() {
            sections.push(InfoSection {
                id: "tags",
                title: Text("preview-media-tags"),
                icon: Icon::Tag,
                rows: self.tags.clone(),
            });
        }
        if !self.streams.is_empty() {
            sections.push(InfoSection {
                id: "streams",
                title: Text("preview-media-stream-details"),
                icon: Icon::Film,
                rows: self.streams.clone(),
            });
        }
        sections
    }

    fn status(&self) -> Vec<String> {
        let mut status = vec![self.kind.name().to_owned()];
        if let Some(duration) = self.duration() {
            status.push(format_duration(duration));
        }
        if let Some(codec) = self.probe.codec_line() {
            status.push(codec);
        }
        if let Some((w, h)) = self.probe.dimensions {
            status.push(format!("{w} × {h}"));
        }
        if let Some(size) = self.size {
            status.push(format_size(size));
        }
        if let Playback::Unavailable(reason) = &self.playback {
            status.push(reason.clone());
        }
        status
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported(
            "media files have no controls of their own",
        ))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        #[cfg(feature = "gstreamer")]
        {
            self.player.as_mut().map(|p| p as &mut dyn Transport)
        }
        #[cfg(not(feature = "gstreamer"))]
        {
            None
        }
    }
}

/// The OS path of a local location; `None` for other providers.
fn local_path(loc: &Location) -> Option<PathBuf> {
    (loc.provider.0 == "local").then(|| PathBuf::from(loc.path.to_string_lossy().into_owned()))
}

/// The headers of a local file: the start, and for an MP4 whose `moov`
/// box sits at the end, the end too.
fn probe_local(kind: Kind, path: &std::path::Path, head: &[u8], size: u64) -> Probe {
    let mut probe = probe::read(kind, head, size);
    if kind == Kind::Mp4 && probe.duration.is_none() && size > head.len() as u64 {
        use std::io::{Read, Seek, SeekFrom};
        let tail_len = (PROBE_BYTES as u64).min(size);
        if let Ok(mut file) = std::fs::File::open(path)
            && file.seek(SeekFrom::Start(size - tail_len)).is_ok()
        {
            let mut tail = Vec::new();
            if file.read_to_end(&mut tail).is_ok() {
                // The tail starts mid-box; scan for `moov` from every
                // plausible offset.
                if let Some(at) = tail
                    .windows(4)
                    .position(|w| w == b"moov")
                    .filter(|&at| at >= 4)
                {
                    let from_moov = probe::read(kind, &tail[at - 4..], size);
                    if from_moov.duration.is_some() {
                        probe.duration = from_moov.duration;
                        probe.dimensions = probe.dimensions.or(from_moov.dimensions);
                        probe.video_codec = probe.video_codec.or(from_moov.video_codec);
                        probe.audio_codec = probe.audio_codec.or(from_moov.audio_codec);
                        probe.sample_rate = probe.sample_rate.or(from_moov.sample_rate);
                        probe.channels = probe.channels.or(from_moov.channels);
                        if probe.tags.is_empty() {
                            probe.tags = from_moov.tags;
                        }
                        if probe.cover.is_none() {
                            probe.cover = from_moov.cover;
                        }
                    }
                }
            }
        }
    }
    probe
}

/// Folds what the Discoverer found into the probe, the tags and the
/// stream rows; returns the first missing plug-in it names.
#[cfg(feature = "gstreamer")]
fn merge_discovery(
    probe: &mut Probe,
    tags: &mut Vec<(String, String)>,
    streams: &mut Vec<(String, String)>,
    found: gst::Discovered,
) -> Option<String> {
    // The headers estimate (a VBR MP3 without a Xing header); the
    // Discoverer looked at the stream.
    if found.duration.is_some() {
        probe.duration = found.duration;
    }
    if probe.dimensions.is_none() {
        probe.dimensions = found.dimensions;
    }
    if probe.frame_rate.is_none() {
        probe.frame_rate = found.frame_rate;
    }
    for (video_or_audio, text) in &found.streams {
        let name = text.split(' ').next().unwrap_or("").to_owned();
        if video_or_audio == "Video" && probe.video_codec.is_none() && !name.is_empty() {
            probe.video_codec = Some(name);
        } else if video_or_audio == "Audio" && probe.audio_codec.is_none() && !name.is_empty() {
            probe.audio_codec = Some(name);
        }
    }
    *streams = found.streams;
    for (key, value) in found.tags {
        if !tags.iter().any(|(k, _)| *k == key) {
            tags.push((key, value));
        }
    }
    if probe.cover.is_none() {
        probe.cover = found.cover;
    }
    found.missing.into_iter().next()
}

/// Builds the document with GStreamer: streams and tags, the player, the
/// waveform for audio. Blocking.
#[cfg(feature = "gstreamer")]
fn with_gstreamer(
    kind: Kind,
    mut probe: Probe,
    size: Option<u64>,
    path: &std::path::Path,
    cancel: &CancelToken,
) -> Result<MediaDoc, Error> {
    let version = match gst::Runtime::get() {
        Ok(runtime) => runtime.version.clone(),
        Err(reason) => {
            return Ok(MediaDoc::without_playback(
                kind,
                probe,
                size,
                format!("GStreamer could not be initialised: {reason}"),
            ));
        }
    };
    let mut streams = Vec::new();
    let mut tags = probe.tags.clone();
    let missing = match gst::discover(path) {
        Ok(found) => merge_discovery(&mut probe, &mut tags, &mut streams, found),
        Err(gst::GstError::Cancelled) => return Err(Error::Cancelled),
        Err(error) => {
            tracing::debug!(%error, "discoverer failed");
            match error {
                gst::GstError::MissingPlugin(what) => Some(what),
                _ => None,
            }
        }
    };
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let cover = probe.cover.as_deref().and_then(Cover::decode);
    let (player, playback) = match gst::Player::open(path, !kind.is_audio(), cancel) {
        Ok(player) => {
            let muted = player.is_muted();
            (Some(player), Playback::Ready { version, muted })
        }
        Err(gst::GstError::Cancelled) => return Err(Error::Cancelled),
        Err(error) => {
            tracing::warn!(%error, "no playback");
            let reason = match (&error, missing) {
                (gst::GstError::MissingPlugin(_), _) => error.to_string(),
                (_, Some(detail)) => gst::GstError::MissingPlugin(detail).to_string(),
                _ => error.to_string(),
            };
            (None, Playback::Unavailable(reason))
        }
    };
    let (waveform, measured) = if kind.is_audio() && player.is_some() {
        match gst::waveform(path, cancel) {
            Ok((waveform, duration)) => (Some(waveform), Some(duration)),
            Err(gst::GstError::Cancelled) => return Err(Error::Cancelled),
            Err(error) => {
                tracing::warn!(%error, "no waveform");
                (None, None)
            }
        }
    } else {
        (None, None)
    };
    Ok(MediaDoc {
        kind,
        probe,
        size,
        playback,
        streams,
        tags,
        waveform,
        measured,
        cover,
        player,
        poster: None,
    })
}

#[async_trait]
impl Previewer for MediaPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != EntryKind::File {
            return 0;
        }
        detect(&entry.name, sniff).map_or(0, |(_, score)| score)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let entry = provider.stat(loc).await?;
        let head = ext::read_bytes(&*provider, loc, Some(PROBE_BYTES), &cancel).await?;
        let (kind, _) = detect(&entry.name, &head[..head.len().min(4096)])
            .ok_or(Error::Unsupported("not a media file this previewer plays"))?;
        let size = entry.size;
        let file_size = size.unwrap_or(head.len() as u64);
        // GStreamer takes a path: the local file, or a temporary copy of a
        // file on another provider when it is not too big.
        let local = local_path(loc);
        let temp = if local.is_none() && file_size <= REMOTE_LIMIT as u64 {
            let bytes = ext::read_bytes(&*provider, loc, Some(REMOTE_LIMIT + 1), &cancel).await?;
            let suffix = format!(".{}", kind.extensions()[0]);
            Some(
                ext::blocking(move || {
                    let mut file = tempfile::Builder::new()
                        .prefix("rick-media-")
                        .suffix(&suffix)
                        .tempfile()
                        .map_err(Error::from_io)?;
                    std::io::Write::write_all(&mut file, &bytes).map_err(Error::from_io)?;
                    Ok(file)
                })
                .await?,
            )
        } else {
            None
        };
        let token = cancel.clone();
        let doc = ext::blocking(move || {
            let path = local
                .clone()
                .or_else(|| temp.as_ref().map(|t| t.path().to_path_buf()));
            let probe = match &path {
                Some(path) => probe_local(kind, path, &head, file_size),
                None => probe::read(kind, &head, file_size),
            };
            if token.is_cancelled() {
                return Err(Error::Cancelled);
            }
            #[cfg(feature = "gstreamer")]
            {
                match path {
                    Some(path) => with_gstreamer(kind, probe, size, &path, &token),
                    None => Ok(MediaDoc::without_playback(
                        kind,
                        probe,
                        size,
                        "the file is on another provider and larger than 256 MiB".to_owned(),
                    )),
                }
            }
            #[cfg(not(feature = "gstreamer"))]
            {
                let _ = path;
                Ok(MediaDoc::without_playback(
                    kind,
                    probe,
                    size,
                    NOT_BUILT.to_owned(),
                ))
            }
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(Box::new(doc))
    }
}

#[cfg(test)]
mod tests;
