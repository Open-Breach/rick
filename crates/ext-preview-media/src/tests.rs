use super::*;
use crate::fixtures::sample_wav;
use types::{Attrs, EntryId, ProviderId};

fn entry(name: &str) -> Entry {
    Entry {
        id: EntryId::new(ProviderId("t"), 1),
        name: name.to_owned(),
        path: format!("/{name}").into(),
        kind: EntryKind::File,
        size: Some(1),
        created: None,
        modified: None,
        accessed: None,
        attrs: Attrs::empty(),
        mime: None,
    }
}

fn req(width: u32, height: u32) -> RenderRequest {
    RenderRequest {
        width,
        height,
        scale: 1.0,
        dark: false,
    }
}

/// One MPEG-1 layer III frame header: 128 kb/s, 44.1 kHz, joint stereo.
const MP3_FRAME: [u8; 4] = [0xFF, 0xFB, 0x90, 0x64];

fn fake_mp3() -> Vec<u8> {
    let header = probe::mp3::header(&MP3_FRAME, 0).unwrap_or_else(|| panic!("header"));
    let mut out = Vec::new();
    for _ in 0..3 {
        out.extend_from_slice(&MP3_FRAME);
        out.extend(std::iter::repeat_n(0u8, header.length - 4));
    }
    out
}

#[test]
fn detects_the_four_kinds_by_signature_then_by_name() {
    assert_eq!(
        detect("x.bin", b"ID3\x04\x00\x00\x00\x00\x00\x00"),
        Some((Kind::Mp3, 200))
    );
    assert_eq!(detect("x.bin", &fake_mp3()), Some((Kind::Mp3, 150)));
    let wav = sample_wav(0.1, 8000, 440.0);
    assert_eq!(detect("x.bin", &wav[..64]), Some((Kind::Wav, 200)));
    assert_eq!(
        detect("x.bin", b"RIFF\x00\x00\x00\x00AVI LIST"),
        Some((Kind::Avi, 200))
    );
    assert_eq!(
        detect("x.bin", b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00"),
        Some((Kind::Mp4, 200))
    );
    assert_eq!(detect("song.mp3", b""), Some((Kind::Mp3, 120)));
    assert_eq!(detect("song.mp3", b"plain text"), Some((Kind::Mp3, 60)));
    assert_eq!(detect("clip.MOV", b"garbage"), Some((Kind::Mp4, 60)));
    assert_eq!(detect("notes.txt", b"hello"), None);
    assert_eq!(
        detect("photo.jpg", b"\xFF\xD8\xFF\xE0"),
        None,
        "a JPEG is not MPEG audio"
    );
    let p = MediaPreviewer;
    assert_eq!(p.score(&entry("a.wav"), &wav[..64]), 200);
    let mut dir = entry("a.wav");
    dir.kind = EntryKind::Dir;
    assert_eq!(p.score(&dir, &wav[..64]), 0);
}

#[test]
fn mp3_headers_parse() {
    let h = probe::mp3::header(&MP3_FRAME, 0).unwrap_or_else(|| panic!("header"));
    assert_eq!(h.version, 1);
    assert_eq!(h.bitrate, 128);
    assert_eq!(h.sample_rate, 44_100);
    assert_eq!(h.channels, 2);
    assert_eq!(h.length, 417);
    assert_eq!(h.samples, 1152);
    assert!(
        probe::mp3::header(b"\xFF\xFB\xF0\x00", 0).is_none(),
        "bad bitrate index"
    );
    assert!(probe::mp3::header(b"\x00\x00\x00\x00", 0).is_none());
    assert_eq!(
        probe::mp3::id3v2_size(b"ID3\x04\x00\x00\x00\x00\x02\x01rest"),
        10 + 257
    );
    assert_eq!(probe::mp3::id3v2_size(b"RIFF"), 0);
    // Three frames of a constant 128 kb/s: the duration comes from the size.
    let bytes = fake_mp3();
    let p = probe::read(Kind::Mp3, &bytes, bytes.len() as u64);
    assert_eq!(p.audio_codec.as_deref(), Some("MPEG-1 Layer III"));
    assert_eq!(p.bit_rate, Some(128_000));
    assert_eq!(p.channels, Some(2));
    let seconds = p.duration.unwrap_or_default().as_secs_f64();
    assert!(
        (seconds - 3.0 * 1152.0 / 44_100.0).abs() < 0.01,
        "{seconds}"
    );
}

#[test]
fn wav_headers_and_info_tags_parse() {
    let wav = sample_wav(2.0, 8000, 440.0);
    let p = probe::read(Kind::Wav, &wav[..1024], wav.len() as u64);
    assert_eq!(p.audio_codec.as_deref(), Some("PCM 16-bit"));
    assert_eq!(p.sample_rate, Some(8000));
    assert_eq!(p.channels, Some(1));
    assert_eq!(p.bit_rate, Some(128_000));
    assert_eq!(p.duration, Some(Duration::from_secs(2)));
    assert_eq!(
        p.tags,
        vec![
            ("Title".to_owned(), "Test tone".to_owned()),
            ("Artist".to_owned(), "Rick fixtures".to_owned())
        ]
    );
    assert_eq!(p.codec_line().as_deref(), Some("PCM 16-bit"));
    assert_eq!(format_duration(Duration::from_secs(125)), "2:05");
    assert_eq!(format_duration(Duration::from_secs(3725)), "1:02:05");
}

/// A minimal `moov`: `mvhd` with a 10 s duration, a video track of
/// 320 × 240 with an `avc1` sample entry, an `ilst` title.
fn fake_mp4() -> Vec<u8> {
    fn atom(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(body.len() + 8)
            .unwrap_or(0)
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(&kind);
        out.extend_from_slice(body);
        out
    }
    let mut mvhd = vec![0u8; 100];
    mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
    mvhd[16..20].copy_from_slice(&10_000u32.to_be_bytes());
    let mut tkhd = vec![0u8; 84];
    tkhd[76..80].copy_from_slice(&(320u32 << 16).to_be_bytes());
    tkhd[80..84].copy_from_slice(&(240u32 << 16).to_be_bytes());
    let mut hdlr = vec![0u8; 24];
    hdlr[8..12].copy_from_slice(b"vide");
    let mut stsd = vec![0u8; 8];
    stsd[4..8].copy_from_slice(&1u32.to_be_bytes());
    stsd.extend(atom(*b"avc1", &[0u8; 78]));
    let stbl = atom(*b"stbl", &atom(*b"stsd", &stsd));
    let minf = atom(*b"minf", &stbl);
    let mut mdia = atom(*b"hdlr", &hdlr);
    mdia.extend(minf);
    let mut trak = atom(*b"tkhd", &tkhd);
    trak.extend(atom(*b"mdia", &mdia));
    let mut data = vec![0u8; 8];
    data.extend_from_slice(b"A title");
    let ilst = atom(*b"\xA9nam", &atom(*b"data", &data));
    let mut meta = vec![0u8; 4];
    meta.extend(atom(*b"ilst", &ilst));
    let udta = atom(*b"udta", &atom(*b"meta", &meta));
    let mut moov = atom(*b"mvhd", &mvhd);
    moov.extend(atom(*b"trak", &trak));
    moov.extend(udta);
    let mut file = atom(*b"ftyp", b"isom\x00\x00\x02\x00isomiso2");
    file.extend(atom(*b"moov", &moov));
    file.extend(atom(*b"mdat", &[0u8; 16]));
    file
}

#[test]
fn mp4_boxes_parse() {
    let bytes = fake_mp4();
    assert_eq!(detect("x.bin", &bytes), Some((Kind::Mp4, 200)));
    let p = probe::read(Kind::Mp4, &bytes, bytes.len() as u64);
    assert_eq!(p.duration, Some(Duration::from_secs(10)));
    assert_eq!(p.dimensions, Some((320, 240)));
    assert_eq!(p.video_codec.as_deref(), Some("H.264"));
    assert_eq!(p.tags, vec![("Title".to_owned(), "A title".to_owned())]);
}

#[test]
fn the_waveform_buckets_and_draws() {
    let samples: Vec<f32> = (0..10_000)
        .map(|i| if i < 5000 { 0.5 } else { -0.25 })
        .collect();
    let waveform = Waveform::from_samples_in(&samples, 10);
    assert_eq!(waveform.peaks.len(), 10);
    assert_eq!(waveform.peaks[0], (0.5, 0.5));
    assert_eq!(waveform.peaks[9], (-0.25, -0.25));
    let empty = Waveform::from_samples(&[]);
    assert_eq!(empty.peaks.len(), waveform::BUCKETS);
    let pixels = waveform.draw(100, 40, Some(0.5), false);
    assert_eq!(pixels.len(), 100 * 40 * 4);
    // The first half is drawn in the played colour, the marker at x = 50,
    // and the top-left corner stays transparent.
    assert_eq!(pixels[3], 0);
    let at = |x: usize, y: usize| &pixels[(y * 100 + x) * 4..(y * 100 + x) * 4 + 4];
    assert_eq!(at(10, 15), &[0x2F, 0x6F, 0xC9, 0xFF], "played bar");
    assert_eq!(at(50, 0), &[0xE0, 0x5A, 0x1C, 0xFF], "marker");
    assert_eq!(
        at(80, 20),
        &[0xB8, 0xBE, 0xC8, 0xFF],
        "axis in the quiet part"
    );
    let mut builder = Waveform::builder();
    builder.push(&[0.2, 0.4, -0.2, -0.4], 2);
    assert_eq!(builder.len(), 2);
    assert!(!builder.is_empty());
    let built = builder.finish();
    assert!((built.peaks[0].1 - 0.3).abs() < 1e-6);
    let Frame::Rgba {
        width,
        height,
        data,
    } = waveform::picture(
        Some(&waveform),
        None,
        64,
        32,
        Some((Duration::from_secs(1), Duration::from_secs(2))),
        true,
    )
    else {
        panic!("not pixels");
    };
    assert_eq!((width, height, data.len()), (64, 32, 64 * 32 * 4));
}

#[test]
fn cover_art_decodes_and_fits() {
    let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 10, 10, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap_or_else(|e| panic!("{e}"));
    let cover = Cover::decode(&png.into_inner()).unwrap_or_else(|| panic!("cover"));
    assert_eq!(cover.size(), (40, 20));
    let pixels = cover.draw(20, 20);
    assert_eq!(pixels.len(), 20 * 20 * 4);
    // Fitted to 20 × 10, centred: rows 0..5 transparent, row 10 red.
    assert_eq!(pixels[3], 0);
    assert_eq!(
        &pixels[(10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 3],
        &[200, 10, 10]
    );
    assert!(Cover::decode(b"not an image").is_none());
    let waveform = Waveform::from_samples(&[0.5, -0.5]);
    let Frame::Rgba { height, .. } =
        waveform::picture(Some(&waveform), Some(&cover), 100, 200, None, false)
    else {
        panic!("not pixels");
    };
    assert_eq!(height, 200);
}

#[test]
fn without_playback_the_document_shows_the_facts_and_the_reason() {
    let wav = sample_wav(1.5, 8000, 440.0);
    let probe = probe::read(Kind::Wav, &wav, wav.len() as u64);
    let mut doc = MediaDoc::without_playback(
        Kind::Wav,
        probe,
        Some(wav.len() as u64),
        NOT_BUILT.to_owned(),
    );
    assert_eq!(doc.pages(), None);
    assert!(doc.transport().is_none());
    assert!(doc.text_layer().is_none());
    assert!(doc.controls().is_empty());
    assert!(
        doc.set_control(ControlId("x"), ControlValue::Pressed)
            .is_err()
    );
    let Frame::Text(block) = doc
        .render(0, req(100, 100))
        .unwrap_or_else(|e| panic!("{e}"))
    else {
        panic!("not text");
    };
    assert!(
        block
            .text
            .starts_with("WAV audio\n\nPlayback is not available"),
        "{}",
        block.text
    );
    assert!(block.text.contains("Duration: 0:01"));
    assert!(block.text.contains("Title: Test tone"));
    assert!(doc.render(1, req(1, 1)).is_err());
    let status = doc.status();
    assert_eq!(status[0], "WAV audio");
    assert_eq!(status[1], "0:01");
    assert_eq!(status[2], "PCM 16-bit");
    assert!(
        status
            .last()
            .is_some_and(|s| s.starts_with("Playback is not available"))
    );
    let sections = doc.sections();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].title, "File info");
    assert!(
        sections[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Sample rate" && v == "8000 Hz")
    );
    assert!(
        sections[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Channels" && v == "mono")
    );
    assert!(
        sections[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Playback" && v.starts_with("Playback is not available"))
    );
    assert_eq!(sections[1].title, "Tags");
    assert_eq!(sections[1].icon, Icon::Tag);
    assert_eq!(doc.kind(), Kind::Wav);
    assert_eq!(doc.probe().channels, Some(1));
    assert!(matches!(doc.playback(), Playback::Unavailable(_)));
    assert!(doc.waveform().is_none());
}

#[cfg(feature = "gstreamer")]
mod with_gstreamer {
    use std::path::Path;
    use std::time::Instant;

    use super::*;
    use crate::fixtures::encode;
    use crate::gst::{GstError, Player, Runtime};

    fn gstreamer_or_skip() -> bool {
        match Runtime::get() {
            Ok(runtime) => {
                eprintln!("GStreamer {}", runtime.version);
                true
            }
            Err(error) => {
                eprintln!("GStreamer not available ({error}); skipping");
                assert!(
                    std::env::var_os("RICK_REQUIRE_GSTREAMER").is_none(),
                    "GStreamer required but not initialised: {error}"
                );
                false
            }
        }
    }

    fn encoder_or_skip(outcome: Result<(), GstError>, what: &str) -> bool {
        match outcome {
            Ok(()) => true,
            Err(GstError::MissingPlugin(element)) => {
                eprintln!("{what}: no {element}; skipping");
                assert!(
                    std::env::var_os("RICK_REQUIRE_GSTREAMER").is_none(),
                    "{what}: GStreamer required but {element} is missing"
                );
                false
            }
            Err(error) => panic!("{what}: {error}"),
        }
    }

    fn open(path: &Path) -> Box<dyn PreviewDoc> {
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path));
        runtime::block_on(MediaPreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn wait_until(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check() {
            assert!(Instant::now() < deadline, "{what} did not happen in 10 s");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_wav_opens_with_a_waveform_and_a_transport() {
        if !gstreamer_or_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("tone.wav");
        std::fs::write(&path, sample_wav(3.0, 22_050, 440.0)).unwrap_or_else(|e| panic!("{e}"));
        let mut doc = open(&path);
        let sections = doc.sections();
        assert!(
            sections[0]
                .rows
                .iter()
                .any(|(k, v)| k == "Playback" && v.starts_with("GStreamer")),
            "{sections:?}"
        );
        assert!(
            sections.iter().any(|s| s.title == "Stream details"),
            "{sections:?}"
        );
        let Frame::Rgba {
            width,
            height,
            data,
        } = doc
            .render(0, req(300, 100))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not the waveform");
        };
        assert_eq!((width, height, data.len()), (300, 100, 300 * 100 * 4));
        assert!(
            data.iter().skip(3).step_by(4).any(|&a| a > 0),
            "some bars drawn"
        );
        let transport = doc.transport().unwrap_or_else(|| panic!("transport"));
        assert!(!transport.playing());
        let duration = transport.duration().unwrap_or_else(|| panic!("duration"));
        assert!((duration.as_secs_f64() - 3.0).abs() < 0.1, "{duration:?}");
        transport.set_volume(0.5);
        assert!((transport.volume() - 0.5).abs() < 0.01);
        transport.set_looping(true);
        assert!(transport.looping());
        transport.seek(Duration::from_secs(1));
        transport.play();
        let started = Instant::now();
        wait_until("position past 1 s", || {
            doc.transport()
                .is_some_and(|t| t.position() > Duration::from_millis(1050))
        });
        let position = doc.transport().map(|t| t.position()).unwrap_or_default();
        assert!(
            position < Duration::from_secs(1) + started.elapsed() + Duration::from_millis(500),
            "{position:?}"
        );
        doc.transport()
            .unwrap_or_else(|| panic!("transport"))
            .pause();
        let paused_at = doc.transport().map(|t| t.position()).unwrap_or_default();
        std::thread::sleep(Duration::from_millis(200));
        let later = doc.transport().map(|t| t.position()).unwrap_or_default();
        assert!(
            later.abs_diff(paused_at) < Duration::from_millis(100),
            "paused: {paused_at:?} vs {later:?}"
        );
        assert_eq!(doc.status()[0], "WAV audio");
        assert_eq!(doc.status()[1], "0:03");
    }

    #[test]
    fn an_mp4_plays_seeks_and_steps() {
        if !gstreamer_or_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("clip.mp4");
        if !encoder_or_skip(encode::mp4(&path, 90, 320, 240), "mp4 fixture") {
            return;
        }
        let mut doc = open(&path);
        let status = doc.status();
        assert_eq!(status[0], "MP4 video", "{status:?}");
        assert_eq!(status[1], "0:03", "{status:?}");
        assert!(status.iter().any(|s| s == "320 × 240"), "{status:?}");
        let Frame::Rgba {
            width,
            height,
            data,
        } = doc
            .render(0, req(100, 100))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not a frame");
        };
        assert_eq!(
            (width, height),
            (320, 240),
            "the first frame at native size"
        );
        assert_eq!(data.len(), 320 * 240 * 4);
        assert!(
            data.chunks_exact(4)
                .any(|p| p[0] > 100 || p[1] > 100 || p[2] > 100),
            "a test pattern has colour"
        );
        let transport = doc.transport().unwrap_or_else(|| panic!("transport"));
        assert!(!transport.playing());
        transport.play();
        wait_until("playing", || doc.transport().is_some_and(|t| t.playing()));
        wait_until("position advances", || {
            doc.transport()
                .is_some_and(|t| t.position() > Duration::from_millis(300))
        });
        let transport = doc.transport().unwrap_or_else(|| panic!("transport"));
        transport.pause();
        transport.seek(Duration::from_secs(2));
        wait_until("seek lands", || {
            doc.transport().is_some_and(|t| {
                t.position().abs_diff(Duration::from_secs(2)) < Duration::from_millis(150)
            })
        });
        let before = doc.transport().map(|t| t.position()).unwrap_or_default();
        doc.transport()
            .unwrap_or_else(|| panic!("transport"))
            .step();
        wait_until("a frame step", || {
            doc.transport().is_some_and(|t| t.position() > before)
        });
        let after = doc.transport().map(|t| t.position()).unwrap_or_default();
        assert!(
            after.saturating_sub(before) < Duration::from_millis(100),
            "one frame: {before:?} -> {after:?}"
        );
        let transport = doc.transport().unwrap_or_else(|| panic!("transport"));
        transport.set_speed(2.0);
        assert!((transport.speed() - 2.0).abs() < 0.01);
        assert!(!transport.playing());
        let sections = doc.sections();
        let streams = sections
            .iter()
            .find(|s| s.title == "Stream details")
            .unwrap_or_else(|| panic!("{sections:?}"));
        assert!(
            streams
                .rows
                .iter()
                .any(|(k, v)| k == "Video" && v.contains("320×240")),
            "{streams:?}"
        );
    }

    #[test]
    fn an_avi_and_an_mp3_open_with_their_facts() {
        if !gstreamer_or_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let avi = dir.path().join("clip.avi");
        if encoder_or_skip(encode::avi(&avi, 30, 160, 120), "avi fixture") {
            let mut doc = open(&avi);
            assert_eq!(doc.status()[0], "AVI video");
            assert!(
                doc.status().iter().any(|s| s == "160 × 120"),
                "{:?}",
                doc.status()
            );
            assert_eq!(probe_dims(&*doc), Some((160, 120)));
            let Frame::Rgba { width, .. } =
                doc.render(0, req(50, 50)).unwrap_or_else(|e| panic!("{e}"))
            else {
                panic!("not a frame");
            };
            assert_eq!(width, 160);
        }
        let mp3 = dir.path().join("tone.mp3");
        if encoder_or_skip(encode::mp3(&mp3, 4), "mp3 fixture") {
            let started = Instant::now();
            let mut doc = open(&mp3);
            eprintln!(
                "4 s MP3 opened with its waveform in {:?}",
                started.elapsed()
            );
            assert_eq!(doc.status()[0], "MP3 audio");
            assert_eq!(doc.status()[1], "0:04", "{:?}", doc.status());
            let sections = doc.sections();
            let tags = sections
                .iter()
                .find(|s| s.title == "Tags")
                .unwrap_or_else(|| panic!("{sections:?}"));
            assert!(
                tags.rows
                    .iter()
                    .any(|(k, v)| k == "Title" && v == "Test tone"),
                "{tags:?}"
            );
            assert!(
                tags.rows
                    .iter()
                    .any(|(k, v)| k == "Artist" && v == "Rick fixtures"),
                "{tags:?}"
            );
            let Frame::Rgba { .. } = doc
                .render(0, req(200, 60))
                .unwrap_or_else(|e| panic!("{e}"))
            else {
                panic!("not the waveform");
            };
            assert!(doc.transport().is_some());
        }
    }

    #[test]
    fn a_three_minute_mp3_gets_its_waveform_quickly() {
        if !gstreamer_or_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mp3 = dir.path().join("long.mp3");
        if !encoder_or_skip(encode::mp3(&mp3, 180), "mp3 fixture") {
            return;
        }
        let started = Instant::now();
        let (waveform, duration) =
            crate::gst::waveform(&mp3, &CancelToken::new()).unwrap_or_else(|e| panic!("{e}"));
        let took = started.elapsed();
        eprintln!("3-minute MP3 waveform in {took:?}");
        assert!((duration.as_secs_f64() - 180.0).abs() < 1.0, "{duration:?}");
        assert_eq!(waveform.peaks.len(), waveform::BUCKETS);
        assert!(waveform.peaks.iter().any(|&(lo, hi)| hi > 0.1 && lo < -0.1));
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(matches!(
            crate::gst::waveform(&mp3, &cancelled),
            Err(GstError::Cancelled)
        ));
        assert!(matches!(
            Player::open(&mp3, false, &cancelled),
            Err(GstError::Cancelled)
        ));
    }

    #[test]
    fn a_file_that_is_no_media_reports_an_error() {
        if !gstreamer_or_skip() {
            return;
        }
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("bogus.mp4");
        std::fs::write(
            &path,
            b"\x00\x00\x00\x18ftypisom and then nothing useful at all",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let mut doc = open(&path);
        let playback = doc.sections()[0]
            .rows
            .iter()
            .find(|(k, _)| k == "Playback")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        assert!(playback.contains("GStreamer"), "{playback}");
        assert!(
            !playback.starts_with("GStreamer 1"),
            "not ready: {playback}"
        );
        assert!(doc.transport().is_none());
        let Frame::Text(block) = doc.render(0, req(10, 10)).unwrap_or_else(|e| panic!("{e}"))
        else {
            panic!("not text");
        };
        assert!(block.text.contains("GStreamer"), "{}", block.text);
    }
}

/// The probed dimensions of a document, from its File info rows.
#[cfg(feature = "gstreamer")]
fn probe_dims(doc: &dyn PreviewDoc) -> Option<(u32, u32)> {
    doc.sections()
        .first()?
        .rows
        .iter()
        .find(|(k, _)| k == "Dimensions")
        .and_then(|(_, v)| {
            let (w, h) = v.split_once(" × ")?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
}
