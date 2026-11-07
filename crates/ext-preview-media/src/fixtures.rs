//! Media files for tests and fixtures: a WAV written here, and MP3, MP4
//! and AVI encoded through GStreamer when its encoders are around.

use std::f32::consts::TAU;

/// A 16-bit PCM WAV of `seconds` of a sine at `frequency` Hz, `rate`
/// samples per second, mono, with an `INFO` list carrying a title and an
/// artist.
#[must_use]
pub fn sample_wav(seconds: f32, rate: u32, frequency: f32) -> Vec<u8> {
    // Sample count from a duration; truncation is the point.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let count = (seconds * rate as f32) as usize;
    let mut data = Vec::with_capacity(count * 2);
    for i in 0..count {
        // Sample index to time; the precision is ample.
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / rate as f32;
        // A sine with a slow envelope, so the waveform has a shape.
        let envelope = 0.2 + 0.8 * (0.5 + 0.5 * (TAU * t / seconds.max(0.001)).cos());
        let value = (TAU * frequency * t).sin() * envelope;
        // Amplitude to a 16-bit sample; truncation is the point.
        #[allow(clippy::cast_possible_truncation)]
        let sample = (value * 32_000.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
    }
    let mut info = Vec::new();
    for (id, text) in [(b"INAM", "Test tone"), (b"IART", "Rick fixtures")] {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        if !value.len().is_multiple_of(2) {
            value.push(0);
        }
        info.extend_from_slice(id);
        info.extend_from_slice(&u32::try_from(value.len()).unwrap_or(0).to_le_bytes());
        info.extend_from_slice(&value);
    }
    let mut list = b"INFO".to_vec();
    list.extend_from_slice(&info);
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    let riff_size = 4 + (8 + 16) + (8 + list.len()) + (8 + data.len());
    out.extend_from_slice(&u32::try_from(riff_size).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&u32::try_from(list.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&list);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// The encoders the GStreamer fixtures need, in the order they are tried.
#[cfg(feature = "gstreamer")]
pub mod encode {
    use std::path::Path;
    use std::time::Duration;

    use crate::gst::{GstError, has_element, run_to_eos};

    /// A GStreamer-quoted file path for a `location=` property.
    fn location(path: &Path) -> String {
        format!(
            "\"{}\"",
            path.to_string_lossy()
                .replace('\\', "/")
                .replace('"', "\\\"")
        )
    }

    /// Writes an MP3 of `seconds` of a 440 Hz tone with `ID3v2` tags.
    ///
    /// # Errors
    ///
    /// [`GstError`] when the encoder or muxer is missing or fails.
    pub fn mp3(path: &Path, seconds: u32) -> Result<(), GstError> {
        for element in ["lamemp3enc", "id3v2mux", "taginject"] {
            if !has_element(element) {
                return Err(GstError::MissingPlugin(element.to_owned()));
            }
        }
        // audiotestsrc buffers hold 1024 samples at 44 100 Hz.
        let buffers = u64::from(seconds) * 44_100 / 1024 + 1;
        run_to_eos(
            &format!(
                "audiotestsrc num-buffers={buffers} wave=sine freq=440 ! audioconvert ! \
                 lamemp3enc target=bitrate cbr=true bitrate=128 ! \
                 taginject tags=\"title=\\\"Test tone\\\",artist=\\\"Rick fixtures\\\",album=\\\"Fixtures\\\"\" ! \
                 id3v2mux ! filesink location={}",
                location(path)
            ),
            Duration::from_mins(1),
        )
    }

    /// Writes an MP4 of `frames` frames of the test pattern at `width` ×
    /// `height` and 30 fps, H.264 when an encoder is there, MPEG-4 part 2
    /// otherwise.
    ///
    /// # Errors
    ///
    /// [`GstError`] when no encoder or the muxer is missing or fails.
    pub fn mp4(path: &Path, frames: u32, width: u32, height: u32) -> Result<(), GstError> {
        if !has_element("mp4mux") {
            return Err(GstError::MissingPlugin("mp4mux".to_owned()));
        }
        let encoder = if has_element("x264enc") {
            "x264enc speed-preset=ultrafast tune=zerolatency key-int-max=15"
        } else if has_element("openh264enc") {
            "openh264enc"
        } else if has_element("avenc_mpeg4") {
            "avenc_mpeg4"
        } else {
            return Err(GstError::MissingPlugin("a video encoder".to_owned()));
        };
        run_to_eos(
            &format!(
                "videotestsrc num-buffers={frames} pattern=smpte ! \
                 video/x-raw,width={width},height={height},framerate=30/1 ! videoconvert ! \
                 {encoder} ! mp4mux ! filesink location={}",
                location(path)
            ),
            Duration::from_mins(1),
        )
    }

    /// Writes a Motion JPEG AVI of `frames` frames at 30 fps.
    ///
    /// # Errors
    ///
    /// [`GstError`] when the encoder or muxer is missing or fails.
    pub fn avi(path: &Path, frames: u32, width: u32, height: u32) -> Result<(), GstError> {
        for element in ["jpegenc", "avimux"] {
            if !has_element(element) {
                return Err(GstError::MissingPlugin(element.to_owned()));
            }
        }
        run_to_eos(
            &format!(
                "videotestsrc num-buffers={frames} pattern=ball ! \
                 video/x-raw,width={width},height={height},framerate=30/1 ! videoconvert ! \
                 jpegenc ! avimux ! filesink location={}",
                location(path)
            ),
            Duration::from_mins(1),
        )
    }
}
