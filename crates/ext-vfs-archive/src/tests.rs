use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ext::{Depth, ListOptions, Provider, Registry, RemoveMode, SettingsStore, WriteOptions};
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
    // A zip can be rewritten: it takes new folders and files (issue 149).
    assert_eq!(
        provider.capabilities(&loc(&zip_path, "")),
        Capabilities::WRITE
            | Capabilities::RENAME
            | Capabilities::CREATE_DIR
            | Capabilities::CREATE_FILE
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

/// A small tree to compress: a file at the top, a folder with a file, an
/// empty folder.
fn source_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    fs::create_dir_all(root.join("docs")).unwrap_or_else(|e| panic!("{e}"));
    fs::create_dir_all(root.join("empty")).unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("top.txt"), b"top level").unwrap_or_else(|e| panic!("{e}"));
    fs::write(root.join("docs/readme.txt"), b"hello from the tree")
        .unwrap_or_else(|e| panic!("{e}"));
    root
}

/// Every file under `dir`, as (relative path, contents), sorted.
fn tree_contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}")) {
            let entry = entry.unwrap_or_else(|e| panic!("{e}"));
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.push((format!("{rel}/"), Vec::new()));
                walk(root, &path, out);
            } else {
                out.push((rel, fs::read(&path).unwrap_or_else(|e| panic!("{e}"))));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn create_and_extract_round_trip_in_every_creatable_format() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = source_tree(dir.path());
    let sources = create::collect(std::slice::from_ref(&root)).unwrap_or_else(|e| panic!("{e}"));
    let names: Vec<String> = sources
        .iter()
        .map(|s| String::from_utf8_lossy(&s.path).into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "src",
            "src/docs",
            "src/docs/readme.txt",
            "src/empty",
            "src/top.txt"
        ]
    );
    let total: u64 = sources.iter().map(create::Source::size).sum();
    assert_eq!(total, 9 + 19);

    for (ext, format) in create::CREATABLE {
        if matches!(format, Format::Single(_)) {
            continue;
        }
        let archive = dir.path().join(format!("out.{ext}"));
        let mut seen = Vec::new();
        create::create(*format, &archive, &sources, &mut |source, done| {
            seen.push((String::from_utf8_lossy(&source.path).into_owned(), done));
            Ok(())
        })
        .unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(seen.len(), sources.len(), "{ext}");
        assert_eq!(
            seen.last().map(|(_, done)| *done),
            Some(19),
            "{ext}: bytes before the last"
        );
        assert_eq!(
            Format::detect(&archive).unwrap_or_else(|e| panic!("{ext}: {e}")),
            *format
        );

        let into = dir.path().join(format!("into-{ext}"));
        let mut extracted = Vec::new();
        extract::extract(&archive, *format, &mut |entry| {
            extracted.push(String::from_utf8_lossy(&entry.path).into_owned());
            let rel = String::from_utf8_lossy(&entry.path).into_owned();
            Ok(extract::Target::Write(into.join(rel)))
        })
        .unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert!(
            extracted.contains(&"src/docs/readme.txt".to_owned()),
            "{ext}: {extracted:?}"
        );
        assert_eq!(
            tree_contents(&into.join("src")),
            tree_contents(&root),
            "{ext}"
        );
    }

    // A single compressed file takes one file and gives it back.
    let one = create::collect(&[root.join("top.txt")]).unwrap_or_else(|e| panic!("{e}"));
    for (ext, format) in create::CREATABLE {
        let Format::Single(codec) = format else {
            continue;
        };
        let archive = dir.path().join(format!("top.txt.{ext}"));
        create::create(*format, &archive, &one, &mut |_, _| Ok(()))
            .unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(Codec::by_extension(ext), Some(*codec));
        let into = dir.path().join(format!("single-{ext}"));
        extract::extract(&archive, *format, &mut |entry| {
            Ok(extract::Target::Write(
                into.join(String::from_utf8_lossy(&entry.path).into_owned()),
            ))
        })
        .unwrap_or_else(|e| panic!("{ext}: {e}"));
        assert_eq!(
            fs::read(into.join("top.txt")).unwrap_or_default(),
            b"top level"
        );
    }
    assert!(matches!(
        create::create(
            Format::Single(Codec::Gzip),
            &dir.path().join("x.gz"),
            &sources,
            &mut |_, _| Ok(())
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        create::create(
            Format::Rar,
            &dir.path().join("x.rar"),
            &sources,
            &mut |_, _| Ok(())
        ),
        Err(Error::Unsupported(_))
    ));
    assert!(!dir.path().join("x.rar").exists());
}

#[test]
fn extraction_skips_what_the_caller_says_and_refuses_climbing_paths() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let archive = dir.path().join("a.zip");
    {
        let mut writer =
            zip::ZipWriter::new(File::create(&archive).unwrap_or_else(|e| panic!("{e}")));
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("keep.txt", options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer.write_all(b"keep").unwrap_or_else(|e| panic!("{e}"));
        writer
            .start_file("skip.txt", options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer.write_all(b"skip").unwrap_or_else(|e| panic!("{e}"));
        writer
            .start_file("../escape.txt", options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer.write_all(b"no").unwrap_or_else(|e| panic!("{e}"));
        writer.finish().unwrap_or_else(|e| panic!("{e}"));
    }
    let into = dir.path().join("into");
    let result = extract::extract(&archive, Format::Zip, &mut |entry| {
        let rel = String::from_utf8_lossy(&entry.path).into_owned();
        if rel == "skip.txt" {
            return Ok(extract::Target::Skip);
        }
        Ok(extract::Target::Write(into.join(rel)))
    });
    assert!(matches!(result, Err(Error::Unsupported(_))), "{result:?}");
    assert_eq!(fs::read(into.join("keep.txt")).unwrap_or_default(), b"keep");
    assert!(!into.join("skip.txt").exists());
    assert!(!dir.path().join("escape.txt").exists());

    // A cancel from the callback stops the write and leaves no archive.
    let sources = create::collect(&[into.join("keep.txt")]).unwrap_or_else(|e| panic!("{e}"));
    let out = dir.path().join("cancelled.zip");
    let result = create::create(Format::Zip, &out, &sources, &mut |_, _| {
        Err(Error::Cancelled)
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(!out.exists());
}

/// The 7-Zip library for the tests that need it: the one CI put in
/// `RICK_SEVENZIP_DIR` (test infrastructure; the plugin itself reads no
/// environment variable), else the platform's usual locations; `None`
/// with a note otherwise, a failure where CI requires it.
fn sevenzip_library() -> Option<PathBuf> {
    let found = std::env::var_os("RICK_SEVENZIP_DIR")
        .map(|dir| PathBuf::from(dir).join(ext::platform::sevenzip::LIBRARY_FILE))
        .into_iter()
        .chain(ext::platform::sevenzip::usual_locations())
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

/// A store of one extension's values in memory: the registry's
/// `SettingsStore` for a test without the application.
#[derive(Default)]
struct Memory(std::sync::Mutex<Vec<(String, ext::SettingValue)>>);

impl Memory {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn library(&self) -> Option<PathBuf> {
        match SettingsStore::get(self, ID, LIBRARY_SETTING)? {
            ext::SettingValue::Text(text) => Some(PathBuf::from(text)),
            _ => None,
        }
    }
}

impl ext::SettingsStore for Memory {
    fn get(&self, ext: types::ExtId, key: &str) -> Option<ext::SettingValue> {
        assert_eq!(ext, ID);
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    fn set(&self, ext: types::ExtId, key: &str, value: ext::SettingValue) -> Result<(), Error> {
        assert_eq!(ext, ID);
        let mut values = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        values.retain(|(k, _)| k != key);
        values.push((key.to_owned(), value));
        Ok(())
    }
}

/// Every disk-image fixture: the FAT file system raw and in its three
/// containers.
fn disk_fixtures(dir: &Path) -> Vec<(PathBuf, Format)> {
    use ext::platform::ImageFormat;
    let fat = crate::fixtures::fat12_image();
    let files = [
        ("floppy.img", fat.clone(), Format::Disk(ImageFormat::Img)),
        (
            "disk.vhd",
            crate::fixtures::vhd_fixed(&fat),
            Format::Disk(ImageFormat::Vhd),
        ),
        (
            "disk.vhdx",
            crate::fixtures::vhdx(&fat),
            Format::Disk(ImageFormat::Vhdx),
        ),
        (
            "disk.qcow2",
            crate::fixtures::qcow2(&fat),
            Format::Disk(ImageFormat::Qcow2),
        ),
    ];
    files
        .into_iter()
        .map(|(name, bytes, format)| {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap_or_else(|e| panic!("{e}"));
            (path, format)
        })
        .collect()
}

#[test]
fn disk_images_list_and_read_through_the_library_and_the_setting_remembers_it() {
    let Some(library) = sevenzip_library() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let store = Memory::new();
    let resolver = Arc::new(Resolver::with_locations(
        Some(Arc::clone(&store) as Arc<dyn SettingsStore>),
        vec![library.clone()],
    ));
    let provider = ArchiveProvider::with_resolver(Arc::clone(&resolver));
    assert!(!resolver.is_loaded(), "nothing is probed at start");
    assert_eq!(store.library(), None, "and the setting is untouched");
    for (path, format) in disk_fixtures(dir.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        assert_eq!(Format::detect(&path).ok(), Some(format), "{name}");
        let root = list(&provider, &loc(&path, ""), Depth::One);
        assert_eq!(names(&root), ["DOCS", "HELLO.TXT"], "{name}");
        let hello = root
            .iter()
            .find(|e| e.name == "HELLO.TXT")
            .unwrap_or_else(|| panic!("{name}: hello"));
        assert_eq!(hello.kind, EntryKind::File, "{name}");
        assert_eq!(
            hello.size,
            Some(crate::fixtures::HELLO.len() as u64),
            "{name}"
        );
        assert!(hello.modified.is_some(), "{name}: FAT dates come through");
        let docs = list(&provider, &loc(&path, "DOCS"), Depth::One);
        assert_eq!(names(&docs), ["README.TXT"], "{name}");
        assert_eq!(
            read(&provider, &loc(&path, "HELLO.TXT")),
            crate::fixtures::HELLO,
            "{name}"
        );
        assert_eq!(
            read(&provider, &loc(&path, "DOCS/README.TXT")),
            crate::fixtures::README,
            "{name}"
        );
        let all = list(&provider, &loc(&path, ""), Depth::Unlimited);
        assert_eq!(names(&all), ["DOCS", "HELLO.TXT", "README.TXT"], "{name}");
        // Read-only: no writes, and the capabilities say so.
        assert!(provider.capabilities(&loc(&path, "")).is_empty(), "{name}");
        assert!(matches!(
            runtime::block_on(provider.create_dir(&loc(&path, "new"))),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            stat(&provider, &loc(&path, "missing.txt")),
            Err(Error::NotFound)
        ));
        let root_entry = stat(&provider, &loc(&path, "")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(root_entry.kind, EntryKind::Dir);
    }
    // The first hit was written back into the setting, through the store.
    assert!(resolver.is_loaded());
    assert_eq!(resolver.setting().as_deref(), Some(library.as_path()));
    assert_eq!(store.library().as_deref(), Some(library.as_path()));
    // The settings the extension declares name the same key.
    let schema = crate::library::settings_schema();
    assert_eq!(schema.ext, ID);
    assert_eq!(schema.fields[0].key, LIBRARY_SETTING);
}

#[test]
fn a_missing_library_says_how_to_install_it_and_browsing_to_it_stores_the_path() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    // A machine without the library: nowhere to look but an empty folder.
    let store = Memory::new();
    let resolver = Resolver::with_locations(
        Some(Arc::clone(&store) as Arc<dyn SettingsStore>),
        vec![dir.path().join(ext::platform::sevenzip::LIBRARY_FILE)],
    );
    let error = resolver
        .library()
        .err()
        .unwrap_or_else(|| panic!("the library was found"));
    let message = error.to_string();
    assert!(message.contains("7-Zip library"), "{message}");
    assert!(
        message.contains(ext::platform::sevenzip::LIBRARY_FILE),
        "{message}"
    );
    assert!(
        message.contains(ext::platform::sevenzip::install_hint()),
        "{message}"
    );
    assert!(message.contains(LIBRARY_SETTING), "{message}");
    assert!(
        message.contains(&dir.path().display().to_string()),
        "{message}"
    );
    let Error::Extension { ext, source } = &error else {
        panic!("{error}");
    };
    assert_eq!(*ext, ID);
    let missing = MissingLibrary::in_error(&error).unwrap_or_else(|| panic!("{error}"));
    assert!(source.downcast_ref::<MissingLibrary>().is_some());
    // The same facts as the pane's content: the name, the install command,
    // the setting Browse… fills and where Rick looked.
    let pane = missing.for_pane();
    assert_eq!(pane.name, LIBRARY_NAME);
    assert_eq!(pane.install, ext::platform::sevenzip::install_hint());
    assert_eq!(pane.setting, (ID, LIBRARY_SETTING));
    assert_eq!(pane.filters.len(), 1);
    assert!(
        pane.detail.contains(&dir.path().display().to_string()),
        "{}",
        pane.detail
    );
    assert_eq!(store.library(), None, "nothing is written for a miss");
    // A disk image shows the same message in place of its listing.
    let fat = dir.path().join("floppy.img");
    fs::write(&fat, crate::fixtures::fat12_image()).unwrap_or_else(|e| panic!("{e}"));
    let provider =
        ArchiveProvider::with_resolver(Arc::new(Resolver::with_locations(None, Vec::new())));
    let listed = runtime::block_on(async {
        let mut stream = provider.list(
            &loc(&fat, ""),
            ListOptions {
                hidden: false,
                system: false,
                depth: Depth::One,
            },
            CancelToken::new(),
        );
        stream.next().await
    });
    let missing = match &listed {
        Some(Err(Error::Extension { source, .. })) => {
            source.downcast_ref::<MissingLibrary>().is_some()
        }
        _ => false,
    };
    assert!(missing, "{listed:?}");
    let Some(library) = sevenzip_library() else {
        return;
    };
    // Browse… chose the library: the path is stored and used from then on.
    let loaded = resolver
        .set_library(&library)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(loaded.format("FAT").is_some());
    assert_eq!(resolver.setting().as_deref(), Some(library.as_path()));
    assert_eq!(store.library().as_deref(), Some(library.as_path()));
    resolver.forget();
    assert!(!resolver.is_loaded());
    assert!(
        resolver.library().is_ok(),
        "the setting names the library now"
    );
    // A file that is not the library is refused and the setting kept.
    let text = dir.path().join("text.so");
    fs::write(&text, b"not a library").unwrap_or_else(|e| panic!("{e}"));
    assert!(resolver.set_library(&text).is_err());
    assert_eq!(resolver.setting().as_deref(), Some(library.as_path()));
    assert!(format!("{resolver:?}").contains("Resolver"));
}
