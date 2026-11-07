//! Playback and decoding through GStreamer, behind the `gstreamer` feature.
//!
//! [`Runtime::get`] initialises GStreamer once per process and adds the
//! plugin directories a bundle carries (`<binary dir>/gstreamer/lib/
//! gstreamer-1.0` on macOS, `<binary dir>/gstreamer-1.0` on Windows,
//! QUESTIONS.md 97). [`Player`] is a `playbin3` whose video sink is an
//! `appsink` asking for RGBA frames, kept as the latest frame for `render`;
//! it is the [`Transport`]. [`discover`] reads duration, streams and tags
//! with the Discoverer, and [`waveform`] decodes the audio to mono samples
//! for the waveform. Hardware decoders are GStreamer's choice: `playbin3`
//! picks the highest-ranked decoder it finds.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use ext::Transport;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_pbutils as gst_pbutils;
use gstreamer_pbutils::prelude::*;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use types::CancelToken;

use crate::waveform::Waveform;

/// How long a pipeline may take to preroll before the preview gives up.
pub const PREROLL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the Discoverer may take per file.
pub const DISCOVER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long decoding the audio for the waveform may take.
pub const WAVEFORM_TIMEOUT: Duration = Duration::from_secs(30);

/// The mono sample rate the waveform is computed at.
const WAVEFORM_RATE: u32 = 11_025;

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

/// The initialised GStreamer.
#[derive(Debug, Clone)]
pub struct Runtime {
    /// `gst_version_string()`.
    pub version: String,
}

impl Runtime {
    /// Initialises GStreamer once and scans the bundled plugin directories;
    /// the reason when it cannot be initialised.
    ///
    /// # Errors
    ///
    /// GStreamer's own message when `gst_init` fails.
    pub fn get() -> Result<&'static Runtime, String> {
        RUNTIME
            .get_or_init(|| {
                gst::init().map_err(|e| e.to_string())?;
                let registry = gst::Registry::get();
                if let Ok(exe) = std::env::current_exe()
                    && let Some(dir) = exe.parent()
                {
                    for relative in [
                        Path::new("gstreamer").join("lib").join("gstreamer-1.0"),
                        Path::new("gstreamer-1.0").to_path_buf(),
                    ] {
                        let dir = dir.join(relative);
                        if dir.is_dir() {
                            registry.scan_path(&dir);
                            tracing::info!(dir = %dir.display(), "GStreamer plugins scanned");
                        }
                    }
                }
                Ok(Runtime {
                    version: gst::version_string().to_string(),
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// The `file://` URI GStreamer takes for a path.
fn uri_for(path: &Path) -> Result<String, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    gst::glib::filename_to_uri(&absolute, None)
        .map(|uri| uri.to_string())
        .map_err(|e| e.to_string())
}

/// Why GStreamer could not play or read a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GstError {
    /// A decoder or demuxer is not installed; the description GStreamer
    /// gives for it.
    MissingPlugin(String),
    /// The pipeline reported an error.
    Pipeline(String),
    /// Nothing happened within the timeout.
    Timeout,
    /// The caller cancelled.
    Cancelled,
}

impl std::fmt::Display for GstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPlugin(what) => write!(
                f,
                "GStreamer is missing a plug-in for this file: {what}. Install the GStreamer \
                 package that provides it (on Linux gstreamer1.0-plugins-good, -bad, -ugly or \
                 gstreamer1.0-libav)."
            ),
            Self::Pipeline(message) => write!(f, "GStreamer could not play this file: {message}"),
            Self::Timeout => write!(f, "GStreamer did not answer in time"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Turns an error message on the bus into a [`GstError`].
fn bus_error(message: &gst::MessageRef) -> Option<GstError> {
    match message.view() {
        gst::MessageView::Error(error) => {
            let text = error.error().to_string();
            let debug = error.debug().map(|d| d.to_string()).unwrap_or_default();
            if text.contains("missing a plug-in") || debug.contains("no suitable plugins") {
                Some(GstError::MissingPlugin(
                    debug
                        .lines()
                        .find(|l| {
                            l.contains("decoder")
                                || l.contains("demuxer")
                                || l.contains("No decoder")
                        })
                        .map_or_else(|| text.clone(), |l| l.trim().to_owned()),
                ))
            } else {
                Some(GstError::Pipeline(text))
            }
        }
        gst::MessageView::Element(element) => gst_pbutils::MissingPluginMessage::parse(message)
            .ok()
            .map(|missing| GstError::MissingPlugin(missing.description().to_string()))
            .or_else(|| {
                let _ = element;
                None
            }),
        _ => None,
    }
}

/// One RGBA frame from the video sink.
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA.
    pub data: Arc<[u8]>,
}

/// Copies a sample's pixels into a tightly packed frame.
fn frame_of(sample: &gst::Sample) -> Option<VideoFrame> {
    let caps = sample.caps()?;
    let info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let buffer = sample.buffer()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).ok()?;
    let (width, height) = (info.width(), info.height());
    let stride = usize::try_from(frame.plane_stride()[0]).ok()?;
    let plane = frame.plane_data(0).ok()?;
    let row = usize::try_from(width).ok()? * 4;
    let mut data = Vec::with_capacity(row * usize::try_from(height).ok()?);
    for y in 0..usize::try_from(height).ok()? {
        let start = y * stride;
        data.extend_from_slice(plane.get(start..start + row)?);
    }
    Some(VideoFrame {
        width,
        height,
        data: data.into(),
    })
}

/// The shared state the sink's callback and the transport read and write.
#[derive(Debug, Default)]
struct Shared {
    latest: Mutex<Option<VideoFrame>>,
    looping: AtomicBool,
}

/// A `playbin3` with an RGBA video sink: the transport of a media
/// document.
pub struct Player {
    pipeline: gst::Element,
    shared: Arc<Shared>,
    duration: Option<Duration>,
    speed: f64,
    ended: bool,
    error: Option<GstError>,
    /// Audio goes to a `fakesink` because no audio device could be opened.
    muted: bool,
    video: bool,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("duration", &self.duration)
            .field("speed", &self.speed)
            .field("video", &self.video)
            .field("muted", &self.muted)
            .finish_non_exhaustive()
    }
}

impl Player {
    /// Opens `path` and prerolls it, so the first frame, the duration and
    /// the stream size are known; playback waits for [`Transport::play`].
    /// Without an audio device (a headless machine) the audio goes to a
    /// `fakesink` and [`Player::is_muted`] says so.
    ///
    /// # Errors
    ///
    /// [`GstError`] when GStreamer cannot read the file.
    pub fn open(path: &Path, video: bool, cancel: &CancelToken) -> Result<Self, GstError> {
        Runtime::get().map_err(GstError::Pipeline)?;
        let uri = uri_for(path).map_err(GstError::Pipeline)?;
        match Self::build(&uri, video, false, cancel) {
            Ok(player) => Ok(player),
            // No audio device: play silently rather than not at all.
            Err(GstError::Pipeline(message))
                if message.to_ascii_lowercase().contains("audio")
                    || message.contains("Could not open")
                    || message.contains("device") =>
            {
                tracing::info!(%message, "no audio output; playing muted");
                Self::build(&uri, video, true, cancel)
            }
            Err(error) => Err(error),
        }
    }

    fn build(uri: &str, video: bool, muted: bool, cancel: &CancelToken) -> Result<Self, GstError> {
        let shared = Arc::new(Shared::default());
        let factory = if gst::ElementFactory::find("playbin3").is_some() {
            "playbin3"
        } else {
            "playbin"
        };
        let mut builder = gst::ElementFactory::make(factory).property("uri", uri);
        if video {
            let on_sample = Arc::clone(&shared);
            let on_preroll = Arc::clone(&shared);
            let keep = |shared: &Shared, sample: &gst::Sample| {
                if let Some(frame) = frame_of(sample) {
                    *shared.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
                }
            };
            let appsink = gst_app::AppSink::builder()
                .caps(
                    &gst_video::VideoCapsBuilder::new()
                        .format(gst_video::VideoFormat::Rgba)
                        .build(),
                )
                .max_buffers(2)
                .drop(true)
                .sync(true)
                .callbacks(
                    gst_app::AppSinkCallbacks::builder()
                        .new_sample(move |sink| {
                            let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                            keep(&on_sample, &sample);
                            Ok(gst::FlowSuccess::Ok)
                        })
                        .new_preroll(move |sink| {
                            let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                            keep(&on_preroll, &sample);
                            Ok(gst::FlowSuccess::Ok)
                        })
                        .build(),
                )
                .build();
            builder = builder.property("video-sink", appsink.to_value());
        }
        if muted {
            let fake = gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
                .map_err(|e| GstError::Pipeline(e.to_string()))?;
            builder = builder.property("audio-sink", fake.to_value());
        }
        let pipeline = builder
            .build()
            .map_err(|e| GstError::Pipeline(e.to_string()))?;
        let mut player = Self {
            pipeline,
            shared,
            duration: None,
            speed: 1.0,
            ended: false,
            error: None,
            muted,
            video,
        };
        player.preroll(cancel)?;
        Ok(player)
    }

    /// Goes to `Paused` and waits for the preroll, polling the bus for
    /// errors and the token.
    fn preroll(&mut self, cancel: &CancelToken) -> Result<(), GstError> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| GstError::Pipeline(e.to_string()))?;
        let deadline = Instant::now() + PREROLL_TIMEOUT;
        loop {
            if cancel.is_cancelled() {
                let _ = self.pipeline.set_state(gst::State::Null);
                return Err(GstError::Cancelled);
            }
            if let Some(error) = self.poll_bus_for_error() {
                let _ = self.pipeline.set_state(gst::State::Null);
                return Err(error);
            }
            let (result, current, _) = self.pipeline.state(gst::ClockTime::from_mseconds(50));
            match result {
                Ok(gst::StateChangeSuccess::Success | gst::StateChangeSuccess::NoPreroll)
                    if current == gst::State::Paused =>
                {
                    break;
                }
                Err(_) => {
                    let error = self.poll_bus_for_error().unwrap_or_else(|| {
                        GstError::Pipeline("the pipeline failed to preroll".to_owned())
                    });
                    let _ = self.pipeline.set_state(gst::State::Null);
                    return Err(error);
                }
                _ => {}
            }
            if Instant::now() > deadline {
                let _ = self.pipeline.set_state(gst::State::Null);
                return Err(GstError::Timeout);
            }
        }
        self.duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(Duration::from);
        Ok(())
    }

    /// Takes the messages waiting on the bus: an error ends playback, an
    /// end of stream loops or stops.
    fn poll_bus(&mut self) {
        let Some(bus) = self.pipeline.bus() else {
            return;
        };
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Eos(_) => {
                    if self.shared.looping.load(Ordering::Relaxed) {
                        let _ = self.pipeline.seek_simple(
                            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                            gst::ClockTime::ZERO,
                        );
                    } else {
                        self.ended = true;
                        let _ = self.pipeline.set_state(gst::State::Paused);
                    }
                }
                _ => {
                    if let Some(error) = bus_error(&message) {
                        tracing::warn!(%error, "playback stopped");
                        self.error = Some(error);
                        let _ = self.pipeline.set_state(gst::State::Null);
                    }
                }
            }
        }
    }

    /// Takes only error messages, for the preroll.
    fn poll_bus_for_error(&self) -> Option<GstError> {
        let bus = self.pipeline.bus()?;
        while let Some(message) =
            bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Element])
        {
            if let Some(error) = bus_error(&message) {
                return Some(error);
            }
        }
        None
    }

    /// The latest video frame, if the stream has video and one arrived.
    #[must_use]
    pub fn frame(&mut self) -> Option<VideoFrame> {
        self.poll_bus();
        self.shared
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Why playback stopped, if it did.
    #[must_use]
    pub fn error(&self) -> Option<&GstError> {
        self.error.as_ref()
    }

    /// Whether the audio goes nowhere because no device could be opened.
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Whether playback reached the end and stopped there.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended
    }

    fn seek_to(&mut self, position: Duration, rate: f64) {
        let Ok(position) = gst::ClockTime::try_from(position) else {
            return;
        };
        if self
            .pipeline
            .seek(
                rate,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                position,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .is_ok()
        {
            self.speed = rate;
            self.ended = false;
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Transport for Player {
    fn play(&mut self) {
        self.poll_bus();
        if self.error.is_some() {
            return;
        }
        if self.ended {
            self.seek_to(Duration::ZERO, self.speed);
        }
        let _ = self.pipeline.set_state(gst::State::Playing);
    }

    fn pause(&mut self) {
        self.poll_bus();
        if self.error.is_none() {
            let _ = self.pipeline.set_state(gst::State::Paused);
        }
    }

    fn seek(&mut self, to: Duration) {
        self.poll_bus();
        if self.error.is_none() {
            let to = self.duration.map_or(to, |d| to.min(d));
            self.seek_to(to, self.speed);
        }
    }

    fn position(&self) -> Duration {
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map(Duration::from)
            .unwrap_or_default()
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn playing(&self) -> bool {
        let (_, current, pending) = self.pipeline.state(gst::ClockTime::ZERO);
        !self.ended
            && self.error.is_none()
            && (current == gst::State::Playing || pending == gst::State::Playing)
    }

    fn volume(&self) -> f32 {
        // A volume is a small number; the narrowing is exact enough.
        #[allow(clippy::cast_possible_truncation)]
        let volume = self.pipeline.property::<f64>("volume") as f32;
        volume.clamp(0.0, 1.0)
    }

    fn set_volume(&mut self, volume: f32) {
        self.pipeline
            .set_property("volume", f64::from(volume.clamp(0.0, 1.0)));
    }

    fn speed(&self) -> f32 {
        // A rate is a small number; the narrowing is exact enough.
        #[allow(clippy::cast_possible_truncation)]
        let speed = self.speed as f32;
        speed
    }

    fn set_speed(&mut self, speed: f32) {
        let rate = f64::from(speed.clamp(0.25, 4.0));
        let at = self.position();
        self.seek_to(at, rate);
    }

    fn step(&mut self) {
        self.poll_bus();
        if self.error.is_some() || !self.video {
            return;
        }
        let _ = self.pipeline.set_state(gst::State::Paused);
        let _ = self.pipeline.send_event(gst::event::Step::new(
            gst::format::Buffers::from_u64(1),
            1.0,
            true,
            false,
        ));
    }

    fn looping(&self) -> bool {
        self.shared.looping.load(Ordering::Relaxed)
    }

    fn set_looping(&mut self, on: bool) {
        self.shared.looping.store(on, Ordering::Relaxed);
    }
}

/// What the Discoverer found: the rows for the Stream details section and
/// the tags for the Tags section, plus the basics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Discovered {
    /// Playing time.
    pub duration: Option<Duration>,
    /// The video frame size.
    pub dimensions: Option<(u32, u32)>,
    /// Frames per second.
    pub frame_rate: Option<f64>,
    /// The container's and streams' descriptions: `("Container", "MP4")`,
    /// `("Video", "H.264 320×240 30 fps")`, `("Audio", "MPEG-1 Layer 3 …")`.
    pub streams: Vec<(String, String)>,
    /// Tags as key and value rows.
    pub tags: Vec<(String, String)>,
    /// Embedded cover art.
    pub cover: Option<Vec<u8>>,
    /// Plug-ins GStreamer says are missing to play the file.
    pub missing: Vec<String>,
}

/// A tag's value as text, or `None` for binary values.
fn tag_text(value: &gst::glib::SendValue) -> Option<String> {
    if let Ok(text) = value.get::<&str>() {
        return Some(text.to_owned());
    }
    if let Ok(date) = value.get::<gst::DateTime>() {
        return date.to_iso8601_string().ok().map(|s| s.to_string());
    }
    if value.type_() == gst::Sample::static_type() {
        return None;
    }
    value
        .transform::<String>()
        .ok()
        .and_then(|v| v.get::<String>().ok())
}

/// The display name of a tag.
fn tag_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "title" => "Title",
        "artist" => "Artist",
        "album" => "Album",
        "album-artist" => "Album artist",
        "date" | "datetime" => "Date",
        "genre" => "Genre",
        "track-number" => "Track",
        "track-count" => "Tracks",
        "comment" => "Comment",
        "composer" => "Composer",
        "copyright" => "Copyright",
        "encoder" => "Encoder",
        "description" => "Description",
        "language-code" => "Language",
        _ => return None,
    })
}

/// The container and stream rows of a Discoverer result.
fn describe_streams(info: &gst_pbutils::DiscovererInfo, out: &mut Discovered) {
    if let Some(container) = info.stream_info()
        && let Some(caps) = container.caps()
    {
        let description = gst_pbutils::pb_utils_get_codec_description(&caps).to_string();
        if !description.is_empty()
            && !caps.to_string().starts_with("video/x-raw")
            && !caps.to_string().starts_with("audio/x-raw")
        {
            out.streams.push(("Container".to_owned(), description));
        }
    }
    for video in info.video_streams() {
        let codec = video
            .caps()
            .map(|c| gst_pbutils::pb_utils_get_codec_description(&c).to_string())
            .unwrap_or_default();
        let fps = video.framerate();
        let fps_value = if fps.denom() > 0 {
            f64::from(fps.numer()) / f64::from(fps.denom())
        } else {
            0.0
        };
        if out.dimensions.is_none() && video.width() > 0 {
            out.dimensions = Some((video.width(), video.height()));
            if fps_value > 0.0 {
                out.frame_rate = Some(fps_value);
            }
        }
        let mut text = format!("{codec} {}×{}", video.width(), video.height());
        if fps_value > 0.0 {
            let precision = if fps_value.fract() == 0.0 { 0 } else { 2 };
            let _ = write!(text, " {fps_value:.precision$} fps");
        }
        if video.bitrate() > 0 {
            let _ = write!(text, " {} kb/s", video.bitrate() / 1000);
        }
        if video.is_interlaced() {
            text.push_str(" interlaced");
        }
        out.streams
            .push(("Video".to_owned(), text.trim().to_owned()));
    }
    for audio in info.audio_streams() {
        let codec = audio
            .caps()
            .map(|c| gst_pbutils::pb_utils_get_codec_description(&c).to_string())
            .unwrap_or_default();
        let mut text = format!(
            "{codec} {} Hz {}",
            audio.sample_rate(),
            match audio.channels() {
                1 => "mono".to_owned(),
                2 => "stereo".to_owned(),
                n => format!("{n} channels"),
            }
        );
        if audio.bitrate() > 0 {
            let _ = write!(text, " {} kb/s", audio.bitrate() / 1000);
        }
        if let Some(language) = audio.language() {
            let _ = write!(text, " ({language})");
        }
        out.streams
            .push(("Audio".to_owned(), text.trim().to_owned()));
    }
}

/// The tag rows and the cover art of a Discoverer result.
fn collect_tags(tags: &gst::TagList, out: &mut Discovered) {
    for (name, mut values) in tags.iter_generic() {
        let Some(value) = values.next() else {
            continue;
        };
        if name == "image" || name == "preview-image" {
            if out.cover.is_none()
                && let Ok(sample) = value.get::<gst::Sample>()
                && let Some(buffer) = sample.buffer()
                && let Ok(map) = buffer.map_readable()
            {
                out.cover = Some(map.as_slice().to_vec());
            }
            continue;
        }
        let Some(label) = tag_label(name.as_str()) else {
            continue;
        };
        if let Some(text) = tag_text(value)
            && !text.trim().is_empty()
            && !out.tags.iter().any(|(k, _)| k == label)
        {
            out.tags.push((label.to_owned(), text));
        }
    }
}

/// Reads a file's streams and tags with the Discoverer.
///
/// # Errors
///
/// [`GstError`] when GStreamer cannot read the file at all.
pub fn discover(path: &Path) -> Result<Discovered, GstError> {
    Runtime::get().map_err(GstError::Pipeline)?;
    let uri = uri_for(path).map_err(GstError::Pipeline)?;
    let timeout = gst::ClockTime::try_from(DISCOVER_TIMEOUT)
        .map_err(|e| GstError::Pipeline(e.to_string()))?;
    let discoverer =
        gst_pbutils::Discoverer::new(timeout).map_err(|e| GstError::Pipeline(e.to_string()))?;
    let info = discoverer
        .discover_uri(&uri)
        .map_err(|e| GstError::Pipeline(e.to_string()))?;
    let mut out = Discovered {
        duration: info.duration().map(Duration::from),
        ..Discovered::default()
    };
    match info.result() {
        gst_pbutils::DiscovererResult::Ok | gst_pbutils::DiscovererResult::MissingPlugins => {}
        gst_pbutils::DiscovererResult::Timeout => return Err(GstError::Timeout),
        other => {
            return Err(GstError::Pipeline(format!("{other:?}")));
        }
    }
    out.missing = info
        .missing_elements_installer_details()
        .iter()
        .map(ToString::to_string)
        .collect();
    describe_streams(&info, &mut out);
    if let Some(tags) = info.tags() {
        collect_tags(&tags, &mut out);
    }
    Ok(out)
}

/// Decodes the audio of `path` to mono samples and buckets them into a
/// waveform, which also gives the exact playing time (the sample count
/// over the rate); stops with [`GstError::Cancelled`] when the token
/// fires.
///
/// # Errors
///
/// [`GstError`] when GStreamer cannot decode the file.
pub fn waveform(path: &Path, cancel: &CancelToken) -> Result<(Waveform, Duration), GstError> {
    Runtime::get().map_err(GstError::Pipeline)?;
    let uri = uri_for(path).map_err(GstError::Pipeline)?;
    let description = format!(
        "uridecodebin uri=\"{uri}\" ! audioconvert ! audioresample ! \
         audio/x-raw,format=F32LE,channels=1,rate={WAVEFORM_RATE} ! \
         appsink name=sink sync=false max-buffers=64"
    );
    let pipeline =
        gst::parse::launch(&description).map_err(|e| GstError::Pipeline(e.to_string()))?;
    let bin = pipeline
        .downcast_ref::<gst::Bin>()
        .ok_or_else(|| GstError::Pipeline("not a bin".to_owned()))?;
    let sink = bin
        .by_name("sink")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or_else(|| GstError::Pipeline("no appsink".to_owned()))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| GstError::Pipeline(e.to_string()))?;
    let bus = pipeline.bus();
    let mut builder = Waveform::builder();
    let deadline = Instant::now() + WAVEFORM_TIMEOUT;
    let outcome = loop {
        if cancel.is_cancelled() {
            break Err(GstError::Cancelled);
        }
        if Instant::now() > deadline {
            break Err(GstError::Timeout);
        }
        if let Some(bus) = &bus
            && let Some(message) =
                bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Element])
            && let Some(error) = bus_error(&message)
        {
            break Err(error);
        }
        match sink.try_pull_sample(gst::ClockTime::from_mseconds(100)) {
            Some(sample) => {
                if let Some(buffer) = sample.buffer()
                    && let Ok(map) = buffer.map_readable()
                {
                    let bytes = map.as_slice();
                    let samples: Vec<f32> = bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| f32::from_le_bytes(*c))
                        .collect();
                    builder.push(&samples, 1);
                }
            }
            None if sink.is_eos() => break Ok(()),
            None => {}
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    outcome?;
    if builder.is_empty() {
        return Err(GstError::Pipeline("no audio was decoded".to_owned()));
    }
    // Samples over the rate; the precision is ample for a duration.
    #[allow(clippy::cast_precision_loss)]
    let seconds = builder.len() as f64 / f64::from(WAVEFORM_RATE);
    Ok((builder.finish(), Duration::from_secs_f64(seconds)))
}

/// Whether an element exists, for the fixtures and the tests.
#[must_use]
pub fn has_element(name: &str) -> bool {
    Runtime::get().is_ok() && gst::ElementFactory::find(name).is_some()
}

/// Runs a `gst-launch` style description to completion, for the fixtures
/// the tests write (an encoder pipeline ending in a `filesink`).
///
/// # Errors
///
/// [`GstError`] when the pipeline cannot be built or fails.
pub fn run_to_eos(description: &str, timeout: Duration) -> Result<(), GstError> {
    Runtime::get().map_err(GstError::Pipeline)?;
    let pipeline =
        gst::parse::launch(description).map_err(|e| GstError::Pipeline(e.to_string()))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| GstError::Pipeline(e.to_string()))?;
    let bus = pipeline
        .bus()
        .ok_or_else(|| GstError::Pipeline("no bus".to_owned()))?;
    let deadline = Instant::now() + timeout;
    let outcome = loop {
        if Instant::now() > deadline {
            break Err(GstError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(100),
            &[
                gst::MessageType::Eos,
                gst::MessageType::Error,
                gst::MessageType::Element,
            ],
        ) else {
            continue;
        };
        if matches!(message.view(), gst::MessageView::Eos(_)) {
            break Ok(());
        }
        if let Some(error) = bus_error(&message) {
            break Err(error);
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    outcome
}
