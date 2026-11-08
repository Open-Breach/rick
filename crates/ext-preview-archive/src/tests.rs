use std::path::Path;

use super::*;
use crate::fixtures::{
    bzip2_bytes, gzip, lz4, sample_7z, sample_iso, sample_rar, sample_tar, sample_zip, xz,
    zstd_bytes,
};
use ext_vfs_archive::Resolver;
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

fn limit() -> Limit {
    Limit::standard(CancelToken::new())
}

fn raw(path: &str, kind: EntryKind, size: u64) -> RawEntry {
    RawEntry {
        path: path.to_owned(),
        kind,
        size: Some(size),
        compressed: Some(size / 2),
        modified: None,
        encrypted: false,
    }
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
    path
}

fn names(listing: &Listing) -> Vec<String> {
    listing
        .entries
        .iter()
        .map(|e| format!("{}{}", "  ".repeat(e.depth), e.name))
        .collect()
}

#[test]
fn detects_formats_by_magic_then_by_name() {
    assert_eq!(detect("x.zip", b"PK\x03\x04rest"), Some((Format::Zip, 150)));
    assert_eq!(detect("x.bin", b"PK\x03\x04rest"), Some((Format::Zip, 150)));
    assert_eq!(detect("x.jar", b"garbage"), Some((Format::Zip, 60)));
    assert_eq!(detect("x.zip", b""), Some((Format::Zip, 120)));
    assert_eq!(
        detect("x.7z", b"7z\xBC\xAF\x27\x1C"),
        Some((Format::SevenZ, 150))
    );
    assert_eq!(
        detect("x.rar", b"Rar!\x1A\x07\x01\x00"),
        Some((Format::Rar, 150))
    );
    assert_eq!(detect("x.iso", b"\0\0\0"), Some((Format::Iso, 120)));
    assert_eq!(detect("notes.txt", b"hello"), None);
    assert_eq!(detect("photo.jpg", b"\xFF\xD8\xFF"), None);
    let tar = sample_tar(100);
    assert_eq!(
        detect("x.bin", &tar[..tar.len().min(4096)]),
        Some((Format::Tar(Compression::None), 150))
    );
    assert_eq!(
        detect("x.tar", b"not really"),
        Some((Format::Tar(Compression::None), 120))
    );
    let gz = gzip(&tar);
    assert_eq!(
        detect("x.bin", &gz),
        Some((Format::Tar(Compression::Gzip), 150))
    );
    assert_eq!(
        detect("plain.gz", &gzip(b"just text")),
        None,
        "a gzipped file that is no tar"
    );
    assert_eq!(
        detect("x.tgz", &gzip(b"text")),
        Some((Format::Tar(Compression::Gzip), 150))
    );
    let zst = zstd_bytes(&tar);
    assert_eq!(
        detect("x.bin", &zst),
        Some((Format::Tar(Compression::Zstd), 150))
    );
    let xz = xz(&tar);
    assert_eq!(
        detect("x.bin", &xz),
        Some((Format::Tar(Compression::Xz), 150))
    );
    let bz = bzip2_bytes(&tar);
    assert_eq!(
        detect("x.tar.bz2", &bz),
        Some((Format::Tar(Compression::Bzip2), 120))
    );
    assert_eq!(detect("x.bz2", &bz), None);
    let lz = lz4(&tar);
    assert_eq!(
        detect("x.tar.lz4", &lz),
        Some((Format::Tar(Compression::Lz4), 120))
    );
    let p = ArchivePreviewer::default();
    assert_eq!(p.score(&entry("x.zip"), b"PK\x03\x04"), 150);
    let mut dir = entry("x.zip");
    dir.kind = EntryKind::Dir;
    assert_eq!(p.score(&dir, b"PK\x03\x04"), 0);
    assert_eq!(Format::Tar(Compression::Xz).name(), "tar.xz");
}

#[test]
fn the_listing_is_a_folders_first_tree_with_implicit_parents() {
    let entries = vec![
        raw("b.txt", EntryKind::File, 10),
        raw("deep/er/file.bin", EntryKind::File, 100),
        raw("Alpha/z.txt", EntryKind::File, 5),
        raw("Alpha/", EntryKind::Dir, 0),
        raw("alpha/A.txt", EntryKind::File, 1),
        raw("./a.txt", EntryKind::File, 2),
    ];
    let (listing, totals) = build_listing(entries, false, &Facts::default());
    // Archives are case-sensitive: `Alpha` and `alpha` are two folders,
    // ordered without regard to case, folders before files at each level.
    assert_eq!(
        names(&listing),
        [
            "Alpha",
            "  z.txt",
            "alpha",
            "  A.txt",
            "deep",
            "  er",
            "    file.bin",
            "a.txt",
            "b.txt"
        ]
    );
    assert_eq!(totals.files, 5);
    assert_eq!(totals.dirs, 4);
    assert_eq!(totals.size, 118);
    assert_eq!(totals.compressed, Some(58), "5 + 50 + 2 + 0 + 1");
    assert_eq!(listing.stats, "5 files, 4 folders, 118 B (58 B compressed)");
    let deep = listing
        .entries
        .iter()
        .find(|e| e.name == "deep")
        .unwrap_or_else(|| panic!("deep"));
    assert_eq!(deep.kind, EntryKind::Dir);
    assert_eq!(deep.size, None);
    assert_eq!(deep.path, "deep");
    let file = listing
        .entries
        .iter()
        .find(|e| e.name == "file.bin")
        .unwrap_or_else(|| panic!("file.bin"));
    assert_eq!(file.path, "deep/er/file.bin");
    assert_eq!(file.depth, 2);
    let (listing, _) = build_listing(
        vec![raw("x/", EntryKind::Dir, 0), raw("x", EntryKind::File, 1)],
        true,
        &Facts::default(),
    );
    assert!(listing.truncated);
    assert!(
        listing.stats.ends_with("first 2 entries shown"),
        "{}",
        listing.stats
    );
}

#[test]
fn dates_convert() {
    assert_eq!(
        civil_to_system_time(1970, 1, 1, 0, 0, 0),
        Some(SystemTime::UNIX_EPOCH)
    );
    assert_eq!(
        civil_to_system_time(2024, 5, 6, 7, 8, 9),
        unix_to_system_time(1_714_979_289)
    );
    assert_eq!(civil_to_system_time(2024, 13, 1, 0, 0, 0), None);
    // 2024-05-06 07:08:08 in MS-DOS fields (seconds in units of two).
    let date = ((2024 - 1980) << 9) | (5 << 5) | 6;
    let time = (7 << 11) | (8 << 5) | 4;
    assert_eq!(
        dos_to_system_time(date, time),
        unix_to_system_time(1_714_979_288)
    );
    assert_eq!(format_size(1536), "1.5 KiB");
}

#[test]
fn zip_lists_entries_sizes_encryption_and_comment() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = write(dir.path(), "a.zip", &sample_zip());
    let listed =
        list(&Resolver::shared(), &path, Format::Zip, &limit()).unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::Zip, listed, Some(1000));
    assert_eq!(
        names(doc.listing()),
        ["docs", "  readme.txt", "a.txt", "secret.bin"]
    );
    let secret = doc
        .listing()
        .entries
        .iter()
        .find(|e| e.name == "secret.bin")
        .unwrap_or_else(|| panic!("secret"));
    assert!(secret.encrypted);
    assert_eq!(secret.size, Some(3));
    assert!(secret.compressed.is_some());
    let readme = doc
        .listing()
        .entries
        .iter()
        .find(|e| e.name == "readme.txt")
        .unwrap_or_else(|| panic!("readme"));
    assert_eq!(readme.size, Some(23));
    assert!(readme.modified.is_some());
    assert!(!readme.encrypted);
    assert_eq!(doc.totals().encrypted, 1);
    assert!(
        doc.listing().stats.contains("1 encrypted"),
        "{}",
        doc.listing().stats
    );
    let status = doc.status();
    assert_eq!(status[0], "4 entries");
    assert!(status[1].contains('→'), "{status:?}");
    assert_eq!(status[2], "zip");
    let rows = doc.sections().remove(0).rows;
    assert!(
        rows.iter().any(|(k, v)| k == "Comment" && v == "fixture"),
        "{rows:?}"
    );
    assert!(
        rows.iter().any(|(k, v)| k == "Encrypted" && v == "1 entry"),
        "{rows:?}"
    );
    assert!(matches!(doc.content(), Some(Content::Listing(_))));
}

#[test]
fn every_tar_flavour_lists_the_same_tree() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let tar = sample_tar(100_000);
    let flavours: [(&str, Vec<u8>, Compression); 6] = [
        ("a.tar", tar.clone(), Compression::None),
        ("a.tar.gz", gzip(&tar), Compression::Gzip),
        ("a.tar.zst", zstd_bytes(&tar), Compression::Zstd),
        ("a.tar.xz", xz(&tar), Compression::Xz),
        ("a.tar.bz2", bzip2_bytes(&tar), Compression::Bzip2),
        ("a.tar.lz4", lz4(&tar), Compression::Lz4),
    ];
    for (name, bytes, compression) in flavours {
        let path = write(dir.path(), name, &bytes);
        let listed = list(
            &Resolver::shared(),
            &path,
            Format::Tar(compression),
            &limit(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let doc = ArchiveDoc::new(Format::Tar(compression), listed, Some(bytes.len() as u64));
        assert_eq!(
            names(doc.listing()),
            ["src", "  main.rs", "latest"],
            "{name}"
        );
        let link = &doc.listing().entries[2];
        assert!(
            matches!(&link.kind, EntryKind::Symlink { target, .. } if target.to_string_lossy() == "src/main.rs"),
            "{name}: {:?}",
            link.kind
        );
        assert_eq!(doc.listing().entries[1].size, Some(100_000), "{name}");
        assert_eq!(
            doc.listing().entries[1].modified,
            unix_to_system_time(1_700_000_000),
            "{name}"
        );
        assert_eq!(doc.status()[2], Format::Tar(compression).name());
        assert!(
            doc.status()[1].contains("97.7 KiB"),
            "{name}: {:?}",
            doc.status()
        );
    }
}

#[test]
fn a_tar_walk_stops_at_the_limit_and_says_so() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut builder = ::tar::Builder::new(Vec::new());
    for i in 0..300 {
        let mut header = ::tar::Header::new_ustar();
        header.set_entry_type(::tar::EntryType::Regular);
        header.set_size(1);
        header.set_mode(0o644);
        header
            .set_path(format!("f{i:04}.txt"))
            .unwrap_or_else(|e| panic!("{e}"));
        header.set_cksum();
        builder
            .append(&header, &b"x"[..])
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let bytes = builder.into_inner().unwrap_or_else(|e| panic!("{e}"));
    let path = write(dir.path(), "many.tar", &bytes);
    let limit = Limit {
        max_entries: 256,
        ..limit()
    };
    let listed = list(
        &Resolver::shared(),
        &path,
        Format::Tar(Compression::None),
        &limit,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(listed.truncated);
    assert_eq!(listed.entries.len(), 256);
    let doc = ArchiveDoc::new(Format::Tar(Compression::None), listed, None);
    assert_eq!(doc.status()[0], "256+ entries");
    assert!(doc.sections()[0].rows.iter().any(|(k, _)| k == "Shown"));
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let limit = Limit::standard(cancelled);
    assert!(matches!(
        list(
            &Resolver::shared(),
            &path,
            Format::Tar(Compression::None),
            &limit
        ),
        Err(Error::Cancelled)
    ));
}

#[test]
fn sevenz_lists_entries_and_marks_encryption() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plain = write(dir.path(), "p.7z", &sample_7z(false, false));
    let listed = list(&Resolver::shared(), &plain, Format::SevenZ, &limit())
        .unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::SevenZ, listed, None);
    assert_eq!(names(doc.listing()), ["pictures", "  one.txt", "two.txt"]);
    assert_eq!(doc.totals().encrypted, 0);
    assert_eq!(doc.listing().entries[1].size, Some(27));
    assert!(doc.listing().entries[1].modified.is_some());

    let locked = write(dir.path(), "e.7z", &sample_7z(true, false));
    let listed = list(&Resolver::shared(), &locked, Format::SevenZ, &limit())
        .unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::SevenZ, listed, None);
    assert_eq!(names(doc.listing()), ["pictures", "  one.txt", "two.txt"]);
    assert_eq!(doc.totals().encrypted, 2, "{:?}", doc.listing());
    assert!(
        doc.listing()
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .all(|e| e.encrypted)
    );

    let hidden = write(dir.path(), "h.7z", &sample_7z(true, true));
    let listed = list(&Resolver::shared(), &hidden, Format::SevenZ, &limit())
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(listed.facts.encrypted_headers);
    assert_eq!(listed.entries, []);
    let doc = ArchiveDoc::new(Format::SevenZ, listed, Some(500));
    assert_eq!(
        doc.listing().stats,
        "Encrypted archive: the list of files needs a password"
    );
    assert_eq!(doc.status(), ["0 entries", "encrypted", "7z"]);
    assert!(
        doc.sections()[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Encrypted" && v == "the file list itself")
    );
}

#[test]
fn rar_lists_entries_and_marks_encrypted_content() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = write(dir.path(), "a.rar", &sample_rar());
    assert_eq!(
        detect("a.rar", &sample_rar()[..64]),
        Some((Format::Rar, 150))
    );
    let listed =
        list(&Resolver::shared(), &path, Format::Rar, &limit()).unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::Rar, listed, Some(300));
    assert_eq!(names(doc.listing()), ["photos", "  note.txt", "secret.txt"]);
    let note = &doc.listing().entries[1];
    assert_eq!(note.size, Some(11));
    assert!(!note.encrypted);
    assert!(note.modified.is_some());
    let secret = &doc.listing().entries[2];
    assert!(secret.encrypted, "{secret:?}");
    assert_eq!(secret.size, Some(10));
    assert_eq!(doc.status()[2], "RAR");
    assert!(doc.listing().stats.contains("1 encrypted"));
}

#[test]
fn iso_lists_the_tree_with_rock_ridge_or_joliet_names() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plain = write(dir.path(), "p.iso", &sample_iso(false));
    let listed =
        list(&Resolver::shared(), &plain, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        listed
            .facts
            .rows
            .iter()
            .any(|(k, v)| k == "Volume label" && v == "FIXTURE"),
        "{:?}",
        listed.facts
    );
    assert!(
        listed
            .facts
            .rows
            .iter()
            .any(|(k, v)| k == "Publisher" && v == "RICK TESTS")
    );
    assert!(
        listed
            .facts
            .rows
            .iter()
            .any(|(k, v)| k == "Created" && v == "2024-05-06 07:08")
    );
    let doc = ArchiveDoc::new(Format::Iso, listed, None);
    assert_eq!(
        names(doc.listing()),
        ["DIR1", "  NESTED.DAT", "hello-world.txt"]
    );
    let file = &doc.listing().entries[2];
    assert_eq!(file.size, Some(11));
    // 2024-05-06 07:08:09 at UTC+1 is 06:08:09 UTC.
    assert_eq!(file.modified, civil_to_system_time(2024, 5, 6, 6, 8, 9));
    assert_eq!(doc.status()[2], "ISO 9660");

    let joliet = write(dir.path(), "j.iso", &sample_iso(true));
    let listed =
        list(&Resolver::shared(), &joliet, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        listed
            .facts
            .rows
            .iter()
            .any(|(k, v)| k == "Names" && v == "Joliet")
    );
    let doc = ArchiveDoc::new(Format::Iso, listed, None);
    assert_eq!(
        names(doc.listing()),
        ["Dir One", "  nested.dat", "héllo wörld.txt"]
    );

    let not_iso = write(dir.path(), "n.iso", &vec![0u8; 40_000]);
    assert!(list(&Resolver::shared(), &not_iso, Format::Iso, &limit()).is_err());
}

/// A real image when `genisoimage` is around (it is not on the CI
/// runners): Joliet and Rock Ridge as the tool writes them.
#[test]
fn a_genisoimage_image_lists_when_the_tool_is_present() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(tree.join("Sub Folder")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(tree.join("Long File Name.txt"), b"abc").unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(tree.join("Sub Folder").join("inner.dat"), b"defg")
        .unwrap_or_else(|e| panic!("{e}"));
    let image = dir.path().join("real.iso");
    let status = std::process::Command::new("genisoimage")
        .args(["-quiet", "-R", "-J", "-V", "REAL", "-o"])
        .arg(&image)
        .arg(&tree)
        .status();
    let Ok(status) = status else {
        eprintln!("genisoimage not available; skipping");
        return;
    };
    assert!(status.success());
    let listed =
        list(&Resolver::shared(), &image, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::Iso, listed, None);
    assert_eq!(
        names(doc.listing()),
        ["Sub Folder", "  inner.dat", "Long File Name.txt"],
        "{:?}",
        doc.listing()
    );
    assert_eq!(doc.listing().entries[1].size, Some(4));
}

#[test]
fn the_fallback_text_lists_the_tree() {
    let (listing, _) = build_listing(
        vec![
            raw("dir/", EntryKind::Dir, 0),
            raw("dir/file.txt", EntryKind::File, 2048),
            RawEntry {
                encrypted: true,
                ..raw("locked.bin", EntryKind::File, 10)
            },
        ],
        false,
        &Facts::default(),
    );
    let text = fallback_text(&listing);
    assert_eq!(
        text,
        "2 files, 1 folder, 2.0 KiB (1.0 KiB compressed), 1 encrypted\n\ndir/\n  file.txt  2.0 KiB\nlocked.bin  10 B  (encrypted)"
    );
}

#[test]
fn open_lists_through_the_provider_and_renders_text() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = write(dir.path(), "fixture.zip", &sample_zip());
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
    let mut doc =
        runtime::block_on(ArchivePreviewer::default().open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(doc.status()[0], "4 entries");
    let Frame::Text(block) = doc
        .render(
            0,
            RenderRequest {
                width: 10,
                height: 10,
                scale: 1.0,
                dark: false,
            },
        )
        .unwrap_or_else(|e| panic!("{e}"))
    else {
        panic!("not text");
    };
    assert!(block.text.contains("  readme.txt  23 B"), "{}", block.text);
    assert!(
        block.text.contains("secret.bin  3 B  (encrypted)"),
        "{}",
        block.text
    );
    assert!(doc.text_layer().is_some());
    assert!(doc.transport().is_none());
    assert!(
        doc.set_control(ControlId("x"), ControlValue::Pressed)
            .is_err()
    );
    assert_eq!(doc.controls().len(), 0);
    assert_eq!(doc.pages(), None);
}

/// The 7-Zip library from the usual locations, for the disk-image tests;
/// `None` with a note otherwise, a failure where CI requires it.
fn sevenzip_library() -> Option<std::path::PathBuf> {
    let found = ext::platform::sevenzip::usual_locations()
        .into_iter()
        .find(|p| p.is_file());
    if found.is_none() {
        eprintln!("no 7-Zip library on this machine; the disk image tests are skipped");
        assert!(
            std::env::var_os("RICK_REQUIRE_SEVENZIP").is_none(),
            "the 7-Zip library is required here but was not found"
        );
    }
    found
}

#[test]
fn disk_images_are_detected_by_signature_or_name() {
    use ext::platform::ImageFormat;
    assert_eq!(
        detect("disk.vhdx", b"vhdxfile"),
        Some((Format::Disk(ImageFormat::Vhdx), 150))
    );
    assert_eq!(
        detect("anything.bin", b"QFI\xfb\0\0\0\x03"),
        Some((Format::Disk(ImageFormat::Qcow2), 150))
    );
    assert_eq!(
        detect("disk.vmdk", b"KDMV"),
        Some((Format::Disk(ImageFormat::Vmdk), 150))
    );
    assert_eq!(
        detect("dyn.vhd", b"conectix"),
        Some((Format::Disk(ImageFormat::Vhd), 150))
    );
    // A fixed VHD, a DMG and a raw image carry nothing at the start.
    assert_eq!(
        detect("fixed.vhd", b"\xEB\x3C\x90MSWIN4.1"),
        Some((Format::Disk(ImageFormat::Vhd), 120))
    );
    assert_eq!(
        detect("app.dmg", b"\0\0\0\0"),
        Some((Format::Disk(ImageFormat::Dmg), 120))
    );
    assert_eq!(
        detect("floppy.img", b"\xEB\x3C\x90"),
        Some((Format::Disk(ImageFormat::Img), 120))
    );
    assert_eq!(
        detect("disk.vdi", b""),
        Some((Format::Disk(ImageFormat::Vdi), 120))
    );
    assert_eq!(
        detect("x.iso", b"\0\0\0"),
        Some((Format::Iso, 120)),
        "ISO stays native"
    );
    assert_eq!(Format::Disk(ImageFormat::Qcow2).name(), "QCOW2");
    assert_eq!(Format::Disk(ImageFormat::Img).name(), "raw disk image");
    let p = ArchivePreviewer::default();
    assert_eq!(p.score(&entry("disk.vhdx"), b"vhdxfile"), 150);
    assert_eq!(p.score(&entry("disk.qcow2"), b"garbage"), 120);
}

#[test]
fn every_disk_image_fixture_lists_its_files_through_the_library() {
    use ext::platform::ImageFormat;
    use ext_vfs_archive::fixtures as disks;
    if sevenzip_library().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fat = disks::fat12_image();
    let fixtures = [
        ("floppy.img", fat.clone(), ImageFormat::Img),
        ("disk.vhd", disks::vhd_fixed(&fat), ImageFormat::Vhd),
        ("disk.vhdx", disks::vhdx(&fat), ImageFormat::Vhdx),
        ("disk.qcow2", disks::qcow2(&fat), ImageFormat::Qcow2),
    ];
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake.clone()));
    let previewer = ArchivePreviewer::new(Arc::new(Resolver::new(Some(fake))));
    for (name, bytes, image) in fixtures {
        let path = write(dir.path(), name, &bytes);
        let listed = list(previewer.resolver(), &path, Format::Disk(image), &limit())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let (listing, totals) = build_listing(listed.entries, listed.truncated, &listed.facts);
        assert_eq!(
            names(&listing),
            ["DOCS", "  README.TXT", "HELLO.TXT"],
            "{name}"
        );
        assert_eq!(totals.files, 2, "{name}");
        assert_eq!(totals.dirs, 1, "{name}");
        assert_eq!(
            totals.size,
            (disks::HELLO.len() + disks::README.len()) as u64,
            "{name}"
        );
        let layout = listed
            .facts
            .rows
            .iter()
            .find(|(k, _)| k == "Layout")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        assert!(layout.ends_with("FAT"), "{name}: {layout}");
        assert!(
            listed
                .facts
                .rows
                .iter()
                .any(|(k, v)| k == "Volume label" && v == "RICKFIX"),
            "{name}: {:?}",
            listed.facts.rows
        );
        // Through the previewer, as the pane does: by sniff, then open.
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
        let sniff = &bytes[..bytes.len().min(4096)];
        assert!(previewer.score(&entry(name), sniff) >= 120, "{name}");
        let mut doc =
            runtime::block_on(previewer.open(&loc, Arc::clone(&provider), CancelToken::new()))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        let Some(Content::Listing(content)) = doc.content() else {
            panic!("{name}: no listing");
        };
        assert_eq!(content.entries.len(), 3, "{name}");
        assert_eq!(doc.status()[2], Format::Disk(image).name(), "{name}");
        let Frame::Text(block) = doc
            .render(
                0,
                RenderRequest {
                    width: 10,
                    height: 10,
                    scale: 1.0,
                    dark: false,
                },
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"))
        else {
            panic!("{name}: not text");
        };
        assert!(block.text.contains("HELLO.TXT"), "{name}: {}", block.text);
    }
}

#[test]
fn a_disk_image_without_the_library_says_what_to_install_and_browsing_to_it_lists() {
    use ext::platform::ImageFormat;
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = write(
        dir.path(),
        "disk.vhdx",
        &ext_vfs_archive::fixtures::vhdx(&ext_vfs_archive::fixtures::fat12_image()),
    );
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    // A machine without the library: the resolver looks in an empty folder.
    let resolver = Arc::new(Resolver::with_locations(
        Some(fake.clone()),
        vec![dir.path().join("none")],
    ));
    let previewer = ArchivePreviewer::new(Arc::clone(&resolver));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
    let error = runtime::block_on(previewer.open(&loc, Arc::clone(&provider), CancelToken::new()))
        .err()
        .unwrap_or_else(|| panic!("opened without the library"));
    let message = error.to_string();
    assert!(message.contains("7-Zip library"), "{message}");
    assert!(
        message.contains(ext::platform::sevenzip::install_hint()),
        "{message}"
    );
    let Error::Extension { source, .. } = &error else {
        panic!("{error}");
    };
    assert!(
        source
            .downcast_ref::<ext_vfs_archive::MissingLibrary>()
            .is_some()
    );
    assert!(list(&resolver, &path, Format::Disk(ImageFormat::Vhdx), &limit()).is_err());
    let Some(library) = sevenzip_library() else {
        return;
    };
    // Browse… names the library: stored, and the listing appears.
    resolver
        .set_library(&library)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resolver.setting().as_deref(), Some(library.as_path()));
    let doc = runtime::block_on(previewer.open(&loc, provider, CancelToken::new()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(doc.content(), Some(Content::Listing(l)) if l.entries.len() == 3));
}
