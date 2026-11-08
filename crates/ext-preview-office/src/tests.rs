use super::*;
use crate::fixtures::{sample, sample_docx, sample_ods, sample_xlsx};
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

fn req() -> RenderRequest {
    RenderRequest {
        width: 300,
        height: 400,
        scale: 1.0,
        dark: false,
    }
}

#[test]
fn detects_the_six_kinds_by_name_and_opendocument_by_content() {
    for kind in Kind::ALL {
        let bytes = sample(kind, "T", "A", 2);
        let name = format!("file.{}", kind.extension());
        let (found, score) =
            detect(&name, &bytes[..4096.min(bytes.len())]).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(found, kind, "{name}");
        assert_eq!(
            score,
            if kind.is_opendocument() { 190 } else { 180 },
            "{name}"
        );
        // OpenDocument says what it is whatever the name.
        let anonymous = detect("blob.bin", &bytes[..4096.min(bytes.len())]);
        if kind.is_opendocument() {
            assert_eq!(anonymous, Some((kind, 190)), "{name}");
        } else {
            assert_eq!(anonymous, None, "{name}");
        }
        assert_eq!(
            Kind::from_extension(&kind.extension().to_ascii_uppercase()),
            Some(kind)
        );
    }
    assert_eq!(detect("notes.docx", b"not a zip"), Some((Kind::Docx, 60)));
    assert_eq!(detect("notes.docx", b""), Some((Kind::Docx, 180)));
    assert_eq!(detect("notes.doc", b"PK\x03\x04"), None);
    assert_eq!(detect("photo.zip", b"PK\x03\x04"), None);
    let p = OfficePreviewer::default();
    assert_eq!(p.score(&entry("a.xlsx"), b"PK\x03\x04"), 180);
    let mut dir = entry("a.xlsx");
    dir.kind = EntryKind::Dir;
    assert_eq!(p.score(&dir, b"PK\x03\x04"), 0);
}

#[test]
fn metadata_comes_from_the_zip() {
    let docx = meta::read(&sample_docx("Quarterly report", "Ada", 3), Kind::Docx);
    let row = |m: &Meta, key: &str| {
        m.rows
            .iter()
            .find(|(k, _)| k == key)
            .map_or_else(|| panic!("no row {key}: {:?}", m.rows), |(_, v)| v.clone())
    };
    assert_eq!(row(&docx, "Title"), "Quarterly report");
    assert_eq!(row(&docx, "Author"), "Ada");
    assert_eq!(row(&docx, "Created"), "2026-11-07 10:00");
    assert_eq!(row(&docx, "Modified"), "2026-11-07 11:30");
    assert_eq!(row(&docx, "Words"), "18");
    assert_eq!(row(&docx, "Application"), "Rick fixture");
    assert_eq!(docx.count, Some(3));

    let xlsx = meta::read(
        &sample_xlsx("Budget", "Alan", &["Income", "Costs"]),
        Kind::Xlsx,
    );
    assert_eq!(xlsx.count, Some(2));
    assert_eq!(row(&xlsx, "Sheets"), "Income, Costs");

    let pptx = meta::read(&sample(Kind::Pptx, "Deck", "Grace", 4), Kind::Pptx);
    assert_eq!(pptx.count, Some(4));
    assert_eq!(row(&pptx, "Title"), "Deck");

    let odt = meta::read(&sample(Kind::Odt, "Letter", "Linus", 2), Kind::Odt);
    assert_eq!(odt.count, Some(2));
    assert_eq!(row(&odt, "Title"), "Letter");
    assert_eq!(row(&odt, "Author"), "Linus");
    assert_eq!(row(&odt, "Words"), "12");
    assert_eq!(row(&odt, "Application"), "Rick fixture");
    assert_eq!(row(&odt, "Created"), "2026-11-07 10:00");

    let ods = meta::read(&sample_ods("Table", "Ken", &["A", "B", "C"]), Kind::Ods);
    assert_eq!(ods.count, Some(3));
    let odp = meta::read(&sample(Kind::Odp, "Talk", "Dennis", 5), Kind::Odp);
    assert_eq!(odp.count, Some(5));

    assert_eq!(meta::read(b"not a zip", Kind::Docx), Meta::default());
}

#[test]
fn without_libreoffice_the_document_shows_metadata_and_the_hint() {
    let meta = meta::read(&sample_docx("Plan", "Ada", 2), Kind::Docx);
    let lookup = Lookup::default().with_search(Search::SettingOnly);
    let Err(missing) = lookup.find() else {
        panic!("nothing to find with the setting alone");
    };
    assert_eq!(missing.name, "LibreOffice");
    assert_eq!(missing.setting, (ID, SOFFICE_SETTING));
    assert!(!missing.install.is_empty());
    let mut doc = OfficeDoc::new(
        Kind::Docx,
        meta,
        Rendering::Missing(Arc::new(missing.clone())),
        Some(4096),
        false,
    );
    assert_eq!(doc.pages(), None);
    assert!(!doc.has_pages());
    assert!(
        matches!(doc.content(), Some(Content::MissingLibrary(m)) if *m == missing),
        "the pane lays the message out"
    );
    let Frame::Text(block) = doc.render(0, req()).unwrap_or_else(|e| panic!("{e}")) else {
        panic!("not text");
    };
    assert!(
        block
            .text
            .starts_with("Word document\n\nLibreOffice is not installed"),
        "{}",
        block.text
    );
    assert!(block.text.contains(DOWNLOAD_URL));
    assert!(block.text.contains("Title: Plan"));
    assert!(block.text.contains("2 pages"));
    assert!(doc.render(1, req()).is_err());
    assert_eq!(
        doc.status(),
        [
            "Word document",
            "2 pages",
            "4.0 KiB",
            "LibreOffice is not installed"
        ]
    );
    let rows = doc.sections().remove(0).rows;
    assert_eq!(rows[0], ("Kind".to_owned(), "Word document".to_owned()));
    assert!(
        rows.iter().any(|(k, v)| k == "Pages" && v == "2"),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(k, v)| k == "Preview" && v.starts_with("LibreOffice is not installed"))
    );
    assert!(doc.text_layer().is_none());
    assert!(doc.transport().is_none());
    assert!(
        doc.set_control(ControlId("x"), ControlValue::Pressed)
            .is_err()
    );
    assert_eq!(doc.controls(), []);
    assert_eq!(
        ConvertError::TimedOut.to_string(),
        "LibreOffice took longer than 60 s"
    );
    assert_eq!(
        ConvertError::Failed(String::new()).to_string(),
        "LibreOffice did not produce a PDF"
    );
}

#[test]
fn cache_keys_follow_path_time_and_size() {
    let path = Path::new("/tmp/a.docx");
    let now = SystemTime::now();
    let key = cache_key(path, Some(now), Some(10));
    assert_eq!(key.len(), 16);
    assert_eq!(key, cache_key(path, Some(now), Some(10)));
    assert_ne!(key, cache_key(path, Some(now), Some(11)));
    assert_ne!(
        key,
        cache_key(Path::new("/tmp/b.docx"), Some(now), Some(10))
    );
    assert_ne!(
        key,
        cache_key(path, Some(now + Duration::from_secs(1)), Some(10))
    );
    assert!(cache_dir().ends_with(Path::new("rick").join("office")));
}

/// Needs LibreOffice and pdfium; the Linux CI run has both. Converts a
/// three-page document, renders a page, hits the cache the second time
/// and stops at once when cancelled.
#[test]
fn converts_through_libreoffice_when_present() {
    let require = std::env::var_os("RICK_REQUIRE_LIBREOFFICE").is_some();
    if !available() {
        eprintln!("LibreOffice not available; skipping");
        assert!(!require, "LibreOffice required but not found");
        return;
    }
    if !pdf::available() {
        eprintln!("pdfium not available; skipping");
        assert!(
            std::env::var_os("RICK_REQUIRE_PDFIUM").is_none(),
            "pdfium required but not found"
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("report.docx");
    std::fs::write(&path, sample_docx("Report", "Ada", 3)).unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));

    let started = Instant::now();
    let mut doc = runtime::block_on(OfficePreviewer::default().open(
        &loc,
        Arc::clone(&provider),
        CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    let took = started.elapsed();
    eprintln!("conversion took {took:?}");
    assert_eq!(doc.pages(), Some(3), "{:?}", doc.sections());
    assert_eq!(doc.status()[0], "Word document");
    assert_eq!(doc.status()[1], "3 pages");
    let Frame::Rgba {
        width,
        height,
        data,
    } = doc.render(1, req()).unwrap_or_else(|e| panic!("{e}"))
    else {
        panic!("not pixels");
    };
    assert_eq!(data.len(), width as usize * height as usize * 4);
    assert!(width <= 300 && height <= 400, "{width}x{height}");
    let layer = doc.text_layer().unwrap_or_else(|| panic!("layer"));
    assert!(
        layer.pages[1].contains("page 2 of 3"),
        "{:?}",
        layer.pages[1]
    );
    let rows = doc.sections().remove(0).rows;
    assert!(
        rows.iter()
            .any(|(k, v)| k == "Preview" && v == "converted to PDF by LibreOffice"),
        "{rows:?}"
    );

    // The second look comes from the cache.
    let doc = runtime::block_on(OfficePreviewer::default().open(
        &loc,
        Arc::clone(&provider),
        CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    let rows = doc.sections().remove(0).rows;
    assert!(
        rows.iter()
            .any(|(k, v)| k == "Preview" && v.ends_with("(cached)")),
        "{rows:?}"
    );

    // A changed file is converted again, and a cancelled open stops.
    std::fs::write(&path, sample_docx("Report", "Ada", 5)).unwrap_or_else(|e| panic!("{e}"));
    let cancel = CancelToken::new();
    cancel.cancel();
    let outcome = runtime::block_on(OfficePreviewer::default().open(&loc, provider, cancel));
    assert!(
        matches!(outcome, Err(Error::Cancelled)),
        "{:?}",
        outcome.map(|_| "a document")
    );
}

/// Needs LibreOffice only: every kind's fixture converts to a PDF, so the
/// fixtures are documents LibreOffice accepts.
#[test]
fn every_fixture_converts_to_a_pdf_when_libreoffice_is_present() {
    let Some(soffice) = find_soffice() else {
        eprintln!("LibreOffice not available; skipping");
        assert!(
            std::env::var_os("RICK_REQUIRE_LIBREOFFICE").is_none(),
            "LibreOffice required but not found"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for kind in Kind::ALL {
        let input = dir.path().join(format!("sample.{}", kind.extension()));
        std::fs::write(&input, sample(kind, "Sample", "Ada", 2)).unwrap_or_else(|e| panic!("{e}"));
        let pdf = dir.path().join(format!("{}.pdf", kind.extension()));
        let started = Instant::now();
        convert(&soffice, &input, &pdf, &CancelToken::new())
            .unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        eprintln!("{kind:?} converted in {:?}", started.elapsed());
        let bytes = std::fs::read(&pdf).unwrap_or_else(|e| panic!("{e}"));
        assert!(bytes.starts_with(b"%PDF-"), "{kind:?}: not a PDF");
        // The page count is pdfium's to read (LibreOffice packs its
        // objects into compressed streams); a real document is a few KiB.
        assert!(bytes.len() > 1024, "{kind:?}: {} bytes", bytes.len());
    }
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let input = dir.path().join("sample.docx");
    let outcome = convert(&soffice, &input, &dir.path().join("x.pdf"), &cancelled);
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
}
