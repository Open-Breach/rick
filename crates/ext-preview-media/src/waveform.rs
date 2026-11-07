//! The waveform of an audio file as peaks per bucket, and its picture: the
//! waveform band with the playback position marked, the cover art above
//! it when there is any.

use std::sync::Arc;
use std::time::Duration;

use ext::Frame;

/// The bucket count a waveform is computed with; the picture resamples it
/// to the pixels available.
pub const BUCKETS: usize = 2000;

/// Peaks per bucket over the whole file.
#[derive(Debug, Clone, PartialEq)]
pub struct Waveform {
    /// `(min, max)` per bucket, each in -1.0 ..= 1.0.
    pub peaks: Vec<(f32, f32)>,
}

impl Waveform {
    /// Buckets the mono samples `samples` into [`BUCKETS`] peaks.
    #[must_use]
    pub fn from_samples(samples: &[f32]) -> Self {
        Self::from_samples_in(samples, BUCKETS)
    }

    /// Buckets `samples` into `buckets` peaks.
    #[must_use]
    pub fn from_samples_in(samples: &[f32], buckets: usize) -> Self {
        let buckets = buckets.max(1);
        if samples.is_empty() {
            return Self {
                peaks: vec![(0.0, 0.0); buckets],
            };
        }
        let per_bucket = samples.len().div_ceil(buckets);
        let mut peaks: Vec<(f32, f32)> = samples
            .chunks(per_bucket)
            .map(|chunk| {
                chunk
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)))
            })
            .map(|(lo, hi)| (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0)))
            .collect();
        peaks.resize(buckets, (0.0, 0.0));
        Self { peaks }
    }

    /// An accumulator that takes samples as they are decoded and buckets
    /// them once the total count is known.
    #[must_use]
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Draws `width` × `height` pixels: the peaks as vertical bars around
    /// the middle line, the part before `position` (a fraction, 0 to 1) in
    /// the played colour and a marker there, colours for a dark or light
    /// pane. The background is transparent.
    #[must_use]
    pub fn draw(&self, width: u32, height: u32, position: Option<f32>, dark: bool) -> Vec<u8> {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        let mut pixels = vec![0u8; w * h * 4];
        let (ahead, played, marker, axis) = if dark {
            (
                [0x8A, 0x9B, 0xB3, 0xFF],
                [0x5B, 0xA8, 0xF5, 0xFF],
                [0xFF, 0x8C, 0x42, 0xFF],
                [0x5A, 0x62, 0x70, 0xFF],
            )
        } else {
            (
                [0x8E, 0x9A, 0xAB, 0xFF],
                [0x2F, 0x6F, 0xC9, 0xFF],
                [0xE0, 0x5A, 0x1C, 0xFF],
                [0xB8, 0xBE, 0xC8, 0xFF],
            )
        };
        let mid = h / 2;
        let half = (h / 2).max(1);
        let played_until = position.map_or(0, |p| {
            // A fraction of the width; truncation is the point.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let x = (p.clamp(0.0, 1.0) * w as f32).round() as usize;
            x
        });
        let put = |pixels: &mut [u8], x: usize, y: usize, rgba: [u8; 4]| {
            if x < w && y < h {
                let at = (y * w + x) * 4;
                pixels[at..at + 4].copy_from_slice(&rgba);
            }
        };
        for x in 0..w {
            // The buckets this column covers.
            let from = x * self.peaks.len() / w;
            let to = ((x + 1) * self.peaks.len() / w).max(from + 1);
            let (lo, hi) = self.peaks[from..to.min(self.peaks.len())]
                .iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)));
            // Amplitude to pixels; truncation is the point.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let (top, bottom) = (
                mid.saturating_sub((hi.max(0.0) * half as f32) as usize),
                (mid + (-lo.min(0.0) * half as f32) as usize).min(h - 1),
            );
            let colour = if x < played_until { played } else { ahead };
            for y in top..=bottom {
                put(&mut pixels, x, y, colour);
            }
            if top == bottom || (top..=bottom).contains(&mid) {
                put(
                    &mut pixels,
                    x,
                    mid,
                    if x < played_until { played } else { axis },
                );
            }
        }
        if position.is_some() {
            for y in 0..h {
                put(&mut pixels, played_until.min(w - 1), y, marker);
                if played_until + 1 < w {
                    put(&mut pixels, played_until + 1, y, marker);
                }
            }
        }
        pixels
    }
}

/// Collects decoded mono samples for a [`Waveform`].
#[derive(Debug, Default)]
pub struct Builder {
    samples: Vec<f32>,
}

impl Builder {
    /// Adds decoded samples; interleaved channels are averaged into one
    /// when `channels` is more than one.
    pub fn push(&mut self, samples: &[f32], channels: usize) {
        let channels = channels.max(1);
        if channels == 1 {
            self.samples.extend_from_slice(samples);
        } else {
            self.samples
                .extend(samples.chunks_exact(channels).map(|frame| {
                    // Channels to a mean; the count is tiny.
                    #[allow(clippy::cast_precision_loss)]
                    let mean = frame.iter().sum::<f32>() / channels as f32;
                    mean
                }));
        }
    }

    /// How many mono samples were collected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether nothing was collected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The waveform.
    #[must_use]
    pub fn finish(self) -> Waveform {
        Waveform::from_samples(&self.samples)
    }
}

/// Cover art decoded once, scaled to fit when drawn.
#[derive(Debug, Clone)]
pub struct Cover {
    image: Arc<image::RgbaImage>,
}

impl Cover {
    /// Decodes the embedded picture; `None` when it is not an image the
    /// `image` crate reads.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let image = image::load_from_memory(bytes).ok()?.into_rgba8();
        (image.width() > 0 && image.height() > 0).then(|| Self {
            image: Arc::new(image),
        })
    }

    /// The decoded size.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.image.width(), self.image.height())
    }

    /// The cover scaled to fit `width` × `height` (never enlarged) and
    /// centred on a transparent canvas of that size.
    #[must_use]
    pub fn draw(&self, width: u32, height: u32) -> Vec<u8> {
        let (w, h) = (width.max(1), height.max(1));
        let (iw, ih) = self.size();
        let scale = (f64::from(w) / f64::from(iw))
            .min(f64::from(h) / f64::from(ih))
            .min(1.0);
        // Scaled sizes; truncation is the point.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (sw, sh) = (
            ((f64::from(iw) * scale).round() as u32).max(1),
            ((f64::from(ih) * scale).round() as u32).max(1),
        );
        let scaled = if (sw, sh) == (iw, ih) {
            (*self.image).clone()
        } else {
            image::imageops::resize(&*self.image, sw, sh, image::imageops::FilterType::Triangle)
        };
        let mut canvas = image::RgbaImage::new(w, h);
        image::imageops::overlay(
            &mut canvas,
            &scaled,
            i64::from((w - sw) / 2),
            i64::from((h - sh) / 2),
        );
        canvas.into_raw()
    }
}

/// The picture of an audio file at `width` × `height`: the cover above
/// the waveform band when there is one, the waveform alone otherwise; the
/// position marker at `position` out of `duration`.
#[must_use]
pub fn picture(
    waveform: Option<&Waveform>,
    cover: Option<&Cover>,
    width: u32,
    height: u32,
    position: Option<(Duration, Duration)>,
    dark: bool,
) -> Frame {
    let (w, h) = (width.max(1), height.max(1));
    let fraction = position.and_then(|(at, total)| {
        (total > Duration::ZERO).then(|| {
            // A ratio of durations; the loss is immaterial.
            #[allow(clippy::cast_possible_truncation)]
            let f = (at.as_secs_f64() / total.as_secs_f64()) as f32;
            f
        })
    });
    let data: Vec<u8> = match (waveform, cover) {
        (Some(waveform), Some(cover)) if h >= 96 => {
            let band = (h / 4).max(48);
            let mut pixels = cover.draw(w, h - band);
            pixels.extend(waveform.draw(w, band, fraction, dark));
            pixels
        }
        (Some(waveform), _) => waveform.draw(w, h, fraction, dark),
        (None, Some(cover)) => cover.draw(w, h),
        (None, None) => vec![0u8; (w * h * 4) as usize],
    };
    Frame::Rgba {
        width: w,
        height: h,
        data: data.into(),
    }
}
