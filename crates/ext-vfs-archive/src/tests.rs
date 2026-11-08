use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ext::{Depth, ListOptions, Provider, Registry, RemoveMode, WriteOptions};
use futures::StreamExt;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use types::{CancelToken, Entry, EntryKind, Error, Location, ProviderId};

use super::*;

fn loc(archive: &Path, inner: &str) -> Location {
    let mut path = ext::platform::paths::from_path(archive);
    if !inner.is_empty() {
        path = path.join(inner.as_bytes());
    }
    Location::new(PROVIDER_ID, path)
}

fn list(provider: &ArchiveProvider, at: &Location, depth: Depth) -> Vec<Entry> {
    runtime::block_on(async {
        let mut stream = provider.list(
            at,
            ListOptions {
                hidden: false,
                system: false,
                depth,
            },
            CancelToken::new(),
        );
        let mut out = Vec::new();
        while let Some(batch) = stream.next().await {
            out.extend(batch.unwrap_or_else(|e| panic!("{at}: {e}")));
        }
        out
    })
}

fn names(entries: &[Entry]) -> Vec<String> {
    let mut names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    names.sort();
    names
}

fn read(provider: &ArchiveProvider, at: &Location) -> Vec<u8> {
    runtime::block_on(async {
        let mut reader = provider
            .open_read(at)
            .await
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        let mut out = Vec::new();
        reader
            .read_to_end(&mut out)
            .await
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        out
    })
}

fn write(provider: &ArchiveProvider, at: &Location, bytes: &[u8]) {
    runtime::block_on(async {
        let mut writer = provider
            .open_write(at, WriteOptions { overwrite: false })
            .await
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        writer
            .write_all(bytes)
            .await
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        writer
            .shutdown()
            .await
            .unwrap_or_else(|e| panic!("{at}: {e}"));
    });
}

fn stat(provider: &ArchiveProvider, at: &Location) -> Result<Entry, Error> {
    runtime::block_on(provider.stat(at))
}

const MTIME: u64 = 1_700_000_000;

fn tar_header(kind: tar::EntryType, size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(if kind.is_dir() { 0o755 } else { 0o644 });
    header.set_mtime(MTIME);
    header
}

/// A zip with a file at the top, one in a folder and an empty folder.
fn zip_fixture(path: &Path) {
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap_or_else(|e| panic!("{e}")));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(
            zip::DateTime::from_date_and_time(2023, 11, 14, 22, 13, 20).unwrap_or_default(),
        );
    writer
        .start_file("top.txt", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .write_all(b"top level")
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .start_file(
            "docs/readme.txt",
            options.compression_method(zip::CompressionMethod::Zstd),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .write_all(b"hello from the zip")
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .add_directory("empty/", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .start_file(".hidden", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer.write_all(b"h").unwrap_or_else(|e| panic!("{e}"));
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
}

/// A tar, through `codec` when given, with the same shape as the zip.
fn tar_fixture(path: &Path, codec: Option<Codec>) {
    let file: Box<dyn Write + Send> =
        Box::new(File::create(path).unwrap_or_else(|e| panic!("{e}")));
    let mut encoder = codec.map(|c| {
        c.encoder(Box::new(
            File::create(path).unwrap_or_else(|e| panic!("{e}")),
        ))
        .unwrap_or_else(|e| panic!("{e}"))
    });
    let mut builder: tar::Builder<Box<dyn Write + Send>> = match encoder.take() {
        Some(encoder) => tar::Builder::new(Box::new(FinishOnDrop(Some(encoder)))),
        None => tar::Builder::new(file),
    };
    builder
        .append_data(
            &mut tar_header(tar::EntryType::Regular, 9),
            "top.txt",
            &b"top level"[..],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    builder
        .append_data(
            &mut tar_header(tar::EntryType::Directory, 0),
            "docs/",
            std::io::empty(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    builder
        .append_data(
            &mut tar_header(tar::EntryType::Regular, 18),
            "docs/readme.txt",
            &b"hello from the tar"[..],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    builder
        .append_data(
            &mut tar_header(tar::EntryType::Directory, 0),
            "empty/",
            std::io::empty(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let inner = builder.into_inner().unwrap_or_else(|e| panic!("{e}"));
    drop(inner);
}

/// Ends a codec stream when the tar builder lets go of it.
struct FinishOnDrop(Option<Box<dyn format::Finish>>);

impl Write for FinishOnDrop {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.as_mut().map_or(Ok(0), |w| w.write(buf))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.as_mut().map_or(Ok(()), std::io::Write::flush)
    }
}

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        if let Some(encoder) = self.0.take() {
            encoder.finish().unwrap_or_else(|e| panic!("{e}"));
        }
    }
}

/// A 7z with the same shape.
fn sevenz_fixture(path: &Path) {
    let mut writer = sevenz_rust2::ArchiveWriter::create(path).unwrap_or_else(|e| panic!("{e}"));
    let mut top = sevenz_rust2::ArchiveEntry::new_file("top.txt");
    top.has_last_modified_date = true;
    top.last_modified_date =
        sevenz_rust2::NtTime::try_from(SystemTime::UNIX_EPOCH + Duration::from_secs(MTIME))
            .unwrap_or_default();
    writer
        .push_archive_entry(top, Some(Cursor::new(b"top level".to_vec())))
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_directory("docs"),
            None::<File>,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file("docs/readme.txt"),
            Some(Cursor::new(b"hello from the 7z".to_vec())),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_directory("empty"),
            None::<File>,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
}

/// An ISO 9660 image with Rock Ridge and Joliet names, same shape.
fn iso_fixture(path: &Path) {
    let tree = image_tree("hello from the iso");
    let mut image: Vec<u8> = Vec::new();
    hadris_iso::sync::write(
        &mut image,
        &tree,
        &hadris_iso::IsoOptions::new()
            .with_rock_ridge()
            .with_joliet(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path, image).unwrap_or_else(|e| panic!("{e}"));
}

/// A UDF image, same shape.
fn udf_fixture(path: &Path) {
    let tree = image_tree("hello from the udf");
    let mut image: Vec<u8> = Vec::new();
    hadris_udf::sync::write(&mut image, &tree, &hadris_udf::UdfOptions::new())
        .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path, image).unwrap_or_else(|e| panic!("{e}"));
}

fn image_tree(readme: &str) -> hadris_fs::Tree {
    use hadris_fs::{Content, Node, Tree};
    let mut tree = Tree::new();
    tree.insert("top.txt", Node::file(Content::bytes("top level")))
        .unwrap_or_else(|e| panic!("{e}"));
    tree.insert("docs", Node::dir())
        .unwrap_or_else(|e| panic!("{e}"));
    tree.insert("docs/readme.txt", Node::file(Content::bytes(readme)))
        .unwrap_or_else(|e| panic!("{e}"));
    tree.insert("empty", Node::dir())
        .unwrap_or_else(|e| panic!("{e}"));
    tree
}

/// Every fixture shares a shape: `top.txt`, `docs/readme.txt`, `empty/`.
fn check_shape(provider: &ArchiveProvider, archive: &Path, readme: &[u8]) {
    let root = loc(archive, "");
    let top = list(provider, &root, Depth::One);
    assert_eq!(
        names(&top),
        ["docs", "empty", "top.txt"],
        "{}",
        archive.display()
    );
    let docs = top
        .iter()
        .find(|e| e.name == "docs")
        .unwrap_or_else(|| panic!("docs"));
    assert_eq!(docs.kind, EntryKind::Dir);
    assert!(docs.path.is_inside(&root.path), "{}", docs.path);
    let top_txt = top
        .iter()
        .find(|e| e.name == "top.txt")
        .unwrap_or_else(|| panic!("top"));
    assert_eq!(top_txt.kind, EntryKind::File);
    assert_eq!(top_txt.size, Some(9), "{}", archive.display());
    assert!(top_txt.modified.is_some(), "{}", archive.display());
    let inside = list(provider, &loc(archive, "docs"), Depth::One);
    assert_eq!(names(&inside), ["readme.txt"]);
    assert_eq!(read(provider, &loc(archive, "docs/readme.txt")), readme);
    assert_eq!(read(provider, &loc(archive, "top.txt")), b"top level");
    let all = list(provider, &root, Depth::Unlimited);
    assert_eq!(names(&all), ["docs", "empty", "readme.txt", "top.txt"]);
    let root_entry = stat(provider, &root).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(root_entry.kind, EntryKind::Dir);
    assert_eq!(
        root_entry.name,
        archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    );
    assert_eq!(
        stat(provider, &loc(archive, "docs/readme.txt"))
            .map(|e| e.id)
            .ok(),
        Some(inside[0].id),
        "ids are stable"
    );
    assert!(matches!(
        stat(provider, &loc(archive, "nope")),
        Err(Error::NotFound)
    ));
    assert!(matches!(provider.watch(&root), Err(Error::Unsupported(_))));
}

#[test]
fn every_format_lists_stats_and_reads() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let provider = ArchiveProvider::new();
    let zip_path = dir.path().join("a.zip");
    zip_fixture(&zip_path);
    check_shape(&provider, &zip_path, b"hello from the zip");
    // The hidden entry needs the flag.
    let shown = runtime::block_on(async {
        let mut stream = provider.list(
            &loc(&zip_path, ""),
            ListOptions {
                hidden: true,
                system: false,
                depth: Depth::One,
            },
            CancelToken::new(),
        );
        let mut out = Vec::new();
        while let Some(batch) = stream.next().await {
            out.extend(batch.unwrap_or_else(|e| panic!("{e}")));
        }
        out
    });
    assert_eq!(names(&shown), [".hidden", "docs", "empty", "top.txt"]);
    assert_eq!(
        provider.capabilities(&loc(&zip_path, "")),
        Capabilities::WRITE | Capabilities::RENAME
    );

    for (name, codec) in [
        ("a.tar", None),
        ("a.tar.gz", Some(Codec::Gzip)),
        ("a.tar.zst", Some(Codec::Zstd)),
        ("a.tar.xz", Some(Codec::Xz)),
        ("a.tar.bz2", Some(Codec::Bzip2)),
        ("a.tar.lz4", Some(Codec::Lz4)),
        ("a.tgz", Some(Codec::Gzip)),
        // A tar behind a codec extension alone: the contents say tar.
        ("plain.gz", Some(Codec::Gzip)),
    ] {
        let path = dir.path().join(name);
        tar_fixture(&path, codec);
        check_shape(&provider, &path, b"hello from the tar");
    }
    let sevenz_path = dir.path().join("a.7z");
    sevenz_fixture(&sevenz_path);
    check_shape(&provider, &sevenz_path, b"hello from the 7z");

    let iso_path = dir.path().join("disc.iso");
    iso_fixture(&iso_path);
    check_shape(&provider, &iso_path, b"hello from the iso");
    assert_eq!(
        provider.capabilities(&loc(&iso_path, "")),
        Capabilities::empty()
    );
    let udf_path = dir.path().join("disc.udf");
    udf_fixture(&udf_path);
    check_shape(&provider, &udf_path, b"hello from the udf");
    // A UDF image under the usual extension is told by its contents.
    let udf_as_iso = dir.path().join("video.iso");
    fs::copy(&udf_path, &udf_as_iso).unwrap_or_else(|e| panic!("{e}"));
    check_shape(&provider, &udf_as_iso, b"hello from the udf");
}

#[test]
fn a_single_compressed_file_is_a_one_entry_archive() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let provider = ArchiveProvider::new();
    let path = dir.path().join("notes.txt.gz");
    let mut encoder = flate2::write::GzEncoder::new(
        File::create(&path).unwrap_or_else(|e| panic!("{e}")),
        flate2::Compression::default(),
    );
    encoder
        .write_all(b"just one file")
        .unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"));
    let entries = list(&provider, &loc(&path, ""), Depth::One);
    assert_eq!(names(&entries), ["notes.txt"]);
    assert_eq!(entries[0].size, Some(13), "gzip records the size");
    assert_eq!(read(&provider, &loc(&path, "notes.txt")), b"just one file");
    assert_eq!(
        provider.capabilities(&loc(&path, "")),
        Capabilities::empty()
    );
    assert!(matches!(
        runtime::block_on(provider.open_write(&loc(&path, "x"), WriteOptions::default())),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn rar_lists_and_extracts_but_takes_no_writes() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/version.rar");
    let provider = ArchiveProvider::new();
    let entries = list(&provider, &loc(&fixture, ""), Depth::One);
    assert_eq!(names(&entries), ["VERSION"], "as the archive spells it");
    assert_eq!(entries[0].kind, EntryKind::File);
    assert_eq!(read(&provider, &loc(&fixture, "VERSION")), b"unrar-0.4.0");
    assert_eq!(
        provider.capabilities(&loc(&fixture, "")),
        Capabilities::empty()
    );
    for outcome in [
        runtime::block_on(provider.create_dir(&loc(&fixture, "new"))).map(drop),
        runtime::block_on(provider.remove(&loc(&fixture, "VERSION"), RemoveMode::Permanent)),
        runtime::block_on(provider.rename(&loc(&fixture, "VERSION"), &loc(&fixture, "v2")))
            .map(drop),
    ] {
        assert!(matches!(outcome, Err(Error::Unsupported(_))), "{outcome:?}");
    }
}

fn check_writes(provider: &ArchiveProvider, archive: &Path, readme: &[u8]) {
    let what = archive.display().to_string();
    // Add a file: the archive is rewritten and the new entry lists.
    write(provider, &loc(archive, "docs/added.txt"), b"added bytes");
    let docs = list(provider, &loc(archive, "docs"), Depth::One);
    assert_eq!(names(&docs), ["added.txt", "readme.txt"], "{what}");
    assert_eq!(
        read(provider, &loc(archive, "docs/added.txt")),
        b"added bytes",
        "{what}"
    );
    assert_eq!(
        read(provider, &loc(archive, "docs/readme.txt")),
        readme,
        "{what}"
    );
    assert!(matches!(
        runtime::block_on(provider.open_write(&loc(archive, "docs/added.txt"), WriteOptions::default())),
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists
    ));

    // A folder, a rename, a copy within the archive, a removal.
    let made = runtime::block_on(provider.create_dir(&loc(archive, "made")))
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(made.kind, EntryKind::Dir);
    let renamed =
        runtime::block_on(provider.rename(&loc(archive, "docs"), &loc(archive, "made/papers")))
            .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(renamed.name, "papers");
    assert_eq!(
        names(&list(provider, &loc(archive, "made/papers"), Depth::One)),
        ["added.txt", "readme.txt"],
        "{what}"
    );
    assert_eq!(
        read(provider, &loc(archive, "made/papers/readme.txt")),
        readme,
        "{what}"
    );
    runtime::block_on(provider.copy(
        &loc(archive, "top.txt"),
        &loc(archive, "made/copy.txt"),
        ext::CopyOptions::default(),
        ext::ProgressSink::channel().0,
        CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(
        read(provider, &loc(archive, "made/copy.txt")),
        b"top level",
        "{what}"
    );
    runtime::block_on(provider.remove(&loc(archive, "top.txt"), RemoveMode::Permanent))
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    runtime::block_on(provider.remove(&loc(archive, "empty"), RemoveMode::Permanent))
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(
        names(&list(provider, &loc(archive, ""), Depth::One)),
        ["made"],
        "{what}"
    );
    assert!(matches!(
        runtime::block_on(provider.remove(&loc(archive, "made"), RemoveMode::Trash)),
        Err(Error::Unsupported(_))
    ));
    // No stray temporary file is left beside the archive.
    let strays: Vec<PathBuf> = fs::read_dir(archive.parent().unwrap_or_else(|| Path::new(".")))
        .unwrap_or_else(|e| panic!("{e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".rick-"))
        })
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
}

#[test]
fn zip_tar_and_7z_take_writes() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let provider = ArchiveProvider::new();
    let zip_path = dir.path().join("w.zip");
    zip_fixture(&zip_path);
    check_writes(&provider, &zip_path, b"hello from the zip");
    let tar_path = dir.path().join("w.tar.zst");
    tar_fixture(&tar_path, Some(Codec::Zstd));
    check_writes(&provider, &tar_path, b"hello from the tar");
    let plain_tar = dir.path().join("w.tar");
    tar_fixture(&plain_tar, None);
    check_writes(&provider, &plain_tar, b"hello from the tar");
    let sevenz_path = dir.path().join("w.7z");
    sevenz_fixture(&sevenz_path);
    check_writes(&provider, &sevenz_path, b"hello from the 7z");
}

#[test]
fn locations_split_at_the_archive_and_other_files_are_refused() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let zip_path = dir.path().join("a.zip");
    zip_fixture(&zip_path);
    let inside = loc(&zip_path, "docs/readme.txt");
    let split = split(&inside).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(split.archive, zip_path);
    assert_eq!(split.inner, b"docs/readme.txt");
    assert_eq!(split.location_of(b"docs"), loc(&zip_path, "docs"));
    assert_eq!(split.location_of(b""), loc(&zip_path, ""));
    assert!(
        matches!(
            split_fn(&Location::new(
                ProviderId::ARCHIVE,
                ext::platform::paths::from_path(dir.path())
            )),
            Err(Error::NotFound)
        ),
        "a folder is no archive location"
    );
    assert!(matches!(
        split_fn(&Location::new(ProviderId::LOCAL, "/x")),
        Err(Error::Unsupported(_))
    ));
    // A file that is no archive: stat says so, which is how Open falls back.
    let text = dir.path().join("notes.txt");
    fs::write(&text, "plain").unwrap_or_else(|e| panic!("{e}"));
    let provider = ArchiveProvider::new();
    assert!(matches!(
        stat(&provider, &loc(&text, "")),
        Err(Error::Unsupported(_))
    ));
    // The cache follows the file: a rewritten archive is read again.
    let before = list(&provider, &loc(&zip_path, ""), Depth::One).len();
    std::thread::sleep(Duration::from_millis(20));
    {
        let mut writer =
            zip::ZipWriter::new(File::create(&zip_path).unwrap_or_else(|e| panic!("{e}")));
        writer
            .start_file("only.txt", zip::write::SimpleFileOptions::default())
            .unwrap_or_else(|e| panic!("{e}"));
        writer.write_all(b"1").unwrap_or_else(|e| panic!("{e}"));
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
    }
    let after = list(&provider, &loc(&zip_path, ""), Depth::One);
    assert_ne!(after.len(), before);
    assert_eq!(names(&after), ["only.txt"]);
}

fn split_fn(location: &Location) -> Result<Split, Error> {
    split(location)
}

#[test]
fn the_extension_registers_one_provider() {
    let mut registry = Registry::new();
    registry
        .load(&ArchiveExtension::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.extensions(), &[ID]);
    assert_eq!(registry.providers().len(), 1);
    assert_eq!(registry.providers()[0].id(), ProviderId::ARCHIVE);
    assert_eq!(registry.points(), Points::PROVIDER);
    assert!(format!("{:?}", ArchiveProvider::new()).contains("cached"));
}
