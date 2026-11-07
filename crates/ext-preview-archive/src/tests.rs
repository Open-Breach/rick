use std::io::{Cursor, Write};
use std::path::Path;

use super::*;
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

/// A zip with a folder, two files (one AES-encrypted) and a comment.
fn sample_zip() -> Vec<u8> {
    let mut writer = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = ::zip::write::SimpleFileOptions::default()
        .compression_method(::zip::CompressionMethod::Deflated);
    writer
        .add_directory("docs/", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .start_file("docs/readme.txt", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .write_all(b"hello hello hello hello")
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .start_file(
            "secret.bin",
            options.with_aes_encryption(::zip::AesMode::Aes256, "pw"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    writer.write_all(b"xyz").unwrap_or_else(|e| panic!("{e}"));
    writer
        .start_file("a.txt", options)
        .unwrap_or_else(|e| panic!("{e}"));
    writer.write_all(b"a").unwrap_or_else(|e| panic!("{e}"));
    writer
        .set_comment("fixture")
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .finish()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_inner()
}

/// A tar with a directory, a file of `payload` bytes and a symlink.
fn sample_tar(payload: usize) -> Vec<u8> {
    let mut builder = ::tar::Builder::new(Vec::new());
    let mut dir = ::tar::Header::new_ustar();
    dir.set_entry_type(::tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_mode(0o755);
    dir.set_mtime(1_700_000_000);
    dir.set_path("src/").unwrap_or_else(|e| panic!("{e}"));
    dir.set_cksum();
    builder
        .append(&dir, std::io::empty())
        .unwrap_or_else(|e| panic!("{e}"));
    let data = vec![b'x'; payload];
    let mut file = ::tar::Header::new_ustar();
    file.set_entry_type(::tar::EntryType::Regular);
    file.set_size(data.len() as u64);
    file.set_mode(0o644);
    file.set_mtime(1_700_000_000);
    file.set_path("src/main.rs")
        .unwrap_or_else(|e| panic!("{e}"));
    file.set_cksum();
    builder
        .append(&file, data.as_slice())
        .unwrap_or_else(|e| panic!("{e}"));
    let mut link = ::tar::Header::new_ustar();
    link.set_entry_type(::tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_path("latest").unwrap_or_else(|e| panic!("{e}"));
    link.set_link_name("src/main.rs")
        .unwrap_or_else(|e| panic!("{e}"));
    link.set_cksum();
    builder
        .append(&link, std::io::empty())
        .unwrap_or_else(|e| panic!("{e}"));
    builder.into_inner().unwrap_or_else(|e| panic!("{e}"))
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

fn zstd_bytes(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 3).unwrap_or_else(|e| panic!("{e}"))
}

fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default())
        .unwrap_or_else(|e| panic!("{e}"));
    writer.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    writer.finish().unwrap_or_else(|e| panic!("{e}"))
}

fn bzip2_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

fn lz4(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// A 7z with a directory and two files; `encrypt` puts a password on the
/// content, `hide` also on the header.
fn sample_7z(encrypt: bool, hide: bool) -> Vec<u8> {
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderMethod, Password};
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap_or_else(|e| panic!("{e}"));
    if encrypt {
        let aes = sevenz_rust2::encoder_options::AesEncoderOptions::new(Password::from("pw"));
        writer.set_content_methods(vec![
            aes.into(),
            sevenz_rust2::encoder_options::Lzma2Options::default().into(),
        ]);
        writer.set_encrypt_header(hide);
    } else {
        writer.set_content_methods(vec![EncoderMethod::LZMA2.into()]);
    }
    writer
        .push_archive_entry::<&[u8]>(ArchiveEntry::new_directory("pictures"), None)
        .unwrap_or_else(|e| panic!("{e}"));
    let mut one = ArchiveEntry::new_file("pictures/one.txt");
    one.has_last_modified_date = true;
    one.last_modified_date = sevenz_rust2::NtTime::now();
    writer
        .push_archive_entry(one, Some(&b"one one one one one one one"[..]))
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .push_archive_entry(ArchiveEntry::new_file("two.txt"), Some(&b"two"[..]))
        .unwrap_or_else(|e| panic!("{e}"));
    writer
        .finish()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_inner()
}

/// CRC-32 (IEEE), as RAR headers use it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn vint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = u8::try_from(value & 0x7F).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// One RAR 5 header block: CRC, size, type, flags, optional extra and data
/// sizes, the type's fields, the extra area.
fn rar_header(kind: u64, flags: u64, extra: &[u8], data: Option<u64>, body: &[u8]) -> Vec<u8> {
    let mut fields = vint(kind);
    fields.extend(vint(flags));
    if !extra.is_empty() {
        fields.extend(vint(extra.len() as u64));
    }
    if let Some(data) = data {
        fields.extend(vint(data));
    }
    fields.extend_from_slice(body);
    fields.extend_from_slice(extra);
    let mut header = vint(fields.len() as u64);
    header.extend(fields);
    let mut out = crc32(&header).to_le_bytes().to_vec();
    out.extend(header);
    out
}

/// A RAR 5 archive written by hand: a directory, a stored file and a file
/// whose content is encrypted (the one case the issue's "encrypted
/// entries" question is about, QUESTIONS.md 88).
fn sample_rar() -> Vec<u8> {
    let mut out = b"Rar!\x1A\x07\x01\x00".to_vec();
    // Main archive header: type 1, no flags, archive flags 0.
    out.extend(rar_header(1, 0, &[], None, &vint(0)));
    // File header fields: file flags, unpacked size, attributes, [mtime],
    // [data CRC], compression info, host OS, name length, name.
    let file_body = |flags: u64, size: u64, mtime: Option<u32>, crc: Option<u32>, name: &str| {
        let mut body = vint(flags);
        body.extend(vint(size));
        body.extend(vint(0x20));
        if let Some(mtime) = mtime {
            body.extend(mtime.to_le_bytes());
        }
        if let Some(crc) = crc {
            body.extend(crc.to_le_bytes());
        }
        body.extend(vint(0)); // compression info: version 0, store
        body.extend(vint(1)); // host OS: Unix
        body.extend(vint(name.len() as u64));
        body.extend_from_slice(name.as_bytes());
        body
    };
    // A directory: file flag 0x01, no data area.
    out.extend(rar_header(
        2,
        0,
        &[],
        None,
        &file_body(0x01 | 0x02, 0, Some(1_700_000_000), None, "photos"),
    ));
    // A stored file with its data.
    let data = b"stored data";
    out.extend(rar_header(
        2,
        0x02,
        &[],
        Some(data.len() as u64),
        &file_body(
            0x02 | 0x04,
            data.len() as u64,
            Some(1_700_000_000),
            Some(crc32(data)),
            "photos/note.txt",
        ),
    ));
    out.extend_from_slice(data);
    // An encrypted file: a file encryption record in the extra area
    // (version 0, flags 0, KDF count, salt, IV) and 16 bytes of
    // "ciphertext".
    let mut record = vint(1); // record type: file encryption
    record.extend(vint(0)); // encryption version
    record.extend(vint(0)); // flags: no password check value
    record.push(15); // KDF count
    record.extend([0x11u8; 16]); // salt
    record.extend([0x22u8; 16]); // IV
    let mut extra = vint(record.len() as u64);
    extra.extend(record);
    let cipher = [0x33u8; 16];
    out.extend(rar_header(
        2,
        0x01 | 0x02,
        &extra,
        Some(cipher.len() as u64),
        &file_body(0x02 | 0x04, 10, Some(1_700_000_000), Some(0), "secret.txt"),
    ));
    out.extend_from_slice(&cipher);
    // End of archive header: type 5, flags 0.
    out.extend(rar_header(5, 0, &[], None, &vint(0)));
    out
}

/// A directory record for an ISO 9660 image.
fn iso_record(extent: u32, size: u32, is_dir: bool, name: &[u8], system_use: &[u8]) -> Vec<u8> {
    let mut record = vec![0u8; 33];
    record.extend_from_slice(name);
    if name.len().is_multiple_of(2) {
        record.push(0);
    }
    record.extend_from_slice(system_use);
    if !record.len().is_multiple_of(2) {
        record.push(0);
    }
    record[0] = u8::try_from(record.len()).unwrap_or(0);
    record[2..6].copy_from_slice(&extent.to_le_bytes());
    record[6..10].copy_from_slice(&extent.to_be_bytes());
    record[10..14].copy_from_slice(&size.to_le_bytes());
    record[14..18].copy_from_slice(&size.to_be_bytes());
    // 2024-05-06 07:08:09, UTC+1 (4 quarter hours).
    record[18..25].copy_from_slice(&[124, 5, 6, 7, 8, 9, 4]);
    record[25] = if is_dir { 0x02 } else { 0 };
    record[28] = 1;
    record[32] = u8::try_from(name.len()).unwrap_or(0);
    record
}

/// A minimal ISO 9660 image: a primary descriptor, a root with a
/// subdirectory and a file carrying a Rock Ridge name, the subdirectory
/// with one file; optionally a Joliet descriptor with UCS-2 names.
fn sample_iso(joliet: bool) -> Vec<u8> {
    const ROOT: u32 = 20;
    const SUB: u32 = 21;
    const DATA: u32 = 22;
    const JROOT: u32 = 23;
    const JSUB: u32 = 24;
    let sector = |n: u32| (n as usize) * 2048;
    let mut image = vec![0u8; sector(25)];
    let mut descriptor = |at: u32, kind: u8, root: u32, escapes: &[u8], ucs: bool| {
        let d = &mut image[sector(at)..sector(at + 1)];
        d[0] = kind;
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        let label: Vec<u8> = if ucs {
            "FIXTURE"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect()
        } else {
            b"FIXTURE".to_vec()
        };
        d[40..40 + label.len()].copy_from_slice(&label);
        d[88..88 + escapes.len()].copy_from_slice(escapes);
        d[128..130].copy_from_slice(&2048u16.to_le_bytes());
        let publisher = b"RICK TESTS";
        d[318..318 + publisher.len()].copy_from_slice(publisher);
        d[813..830].copy_from_slice(b"2024050607080900\0");
        let root_record = iso_record(root, 2048, true, &[0], &[]);
        d[156..156 + root_record.len()].copy_from_slice(&root_record);
    };
    descriptor(16, 1, ROOT, &[], false);
    if joliet {
        descriptor(17, 2, JROOT, b"%/E", true);
        image[sector(18)] = 255;
        image[sector(18) + 1..sector(18) + 6].copy_from_slice(b"CD001");
    } else {
        image[sector(17)] = 255;
        image[sector(17) + 1..sector(17) + 6].copy_from_slice(b"CD001");
    }
    let data = b"nested data";
    image[sector(DATA)..sector(DATA) + data.len()].copy_from_slice(data);
    // The primary tree, with a Rock Ridge NM on the file.
    let mut nm = b"NM".to_vec();
    let alternate = b"hello-world.txt";
    nm.push(u8::try_from(5 + alternate.len()).unwrap_or(0));
    nm.push(1);
    nm.push(0);
    nm.extend_from_slice(alternate);
    let mut root = Vec::new();
    root.extend(iso_record(ROOT, 2048, true, &[0], &[]));
    root.extend(iso_record(ROOT, 2048, true, &[1], &[]));
    root.extend(iso_record(SUB, 2048, true, b"DIR1", &[]));
    root.extend(iso_record(
        DATA,
        u32::try_from(data.len()).unwrap_or(0),
        false,
        b"HELLO.TXT;1",
        &nm,
    ));
    image[sector(ROOT)..sector(ROOT) + root.len()].copy_from_slice(&root);
    let mut sub = Vec::new();
    sub.extend(iso_record(SUB, 2048, true, &[0], &[]));
    sub.extend(iso_record(ROOT, 2048, true, &[1], &[]));
    sub.extend(iso_record(
        DATA,
        u32::try_from(data.len()).unwrap_or(0),
        false,
        b"NESTED.DAT;1",
        &[],
    ));
    image[sector(SUB)..sector(SUB) + sub.len()].copy_from_slice(&sub);
    if joliet {
        let ucs = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_be_bytes).collect() };
        let mut jroot = Vec::new();
        jroot.extend(iso_record(JROOT, 2048, true, &[0], &[]));
        jroot.extend(iso_record(JROOT, 2048, true, &[1], &[]));
        jroot.extend(iso_record(JSUB, 2048, true, &ucs("Dir One"), &[]));
        jroot.extend(iso_record(
            DATA,
            u32::try_from(data.len()).unwrap_or(0),
            false,
            &ucs("héllo wörld.txt"),
            &[],
        ));
        image[sector(JROOT)..sector(JROOT) + jroot.len()].copy_from_slice(&jroot);
        let mut jsub = Vec::new();
        jsub.extend(iso_record(JSUB, 2048, true, &[0], &[]));
        jsub.extend(iso_record(JROOT, 2048, true, &[1], &[]));
        jsub.extend(iso_record(
            DATA,
            u32::try_from(data.len()).unwrap_or(0),
            false,
            &ucs("nested.dat"),
            &[],
        ));
        image[sector(JSUB)..sector(JSUB) + jsub.len()].copy_from_slice(&jsub);
    }
    image
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
    let p = ArchivePreviewer;
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
    let listed = list(&path, Format::Zip, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
        let listed = list(&path, Format::Tar(compression), &limit())
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
    let listed =
        list(&path, Format::Tar(Compression::None), &limit).unwrap_or_else(|e| panic!("{e}"));
    assert!(listed.truncated);
    assert_eq!(listed.entries.len(), 256);
    let doc = ArchiveDoc::new(Format::Tar(Compression::None), listed, None);
    assert_eq!(doc.status()[0], "256+ entries");
    assert!(doc.sections()[0].rows.iter().any(|(k, _)| k == "Shown"));
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let limit = Limit::standard(cancelled);
    assert!(matches!(
        list(&path, Format::Tar(Compression::None), &limit),
        Err(Error::Cancelled)
    ));
}

#[test]
fn sevenz_lists_entries_and_marks_encryption() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let plain = write(dir.path(), "p.7z", &sample_7z(false, false));
    let listed = list(&plain, Format::SevenZ, &limit()).unwrap_or_else(|e| panic!("{e}"));
    let doc = ArchiveDoc::new(Format::SevenZ, listed, None);
    assert_eq!(names(doc.listing()), ["pictures", "  one.txt", "two.txt"]);
    assert_eq!(doc.totals().encrypted, 0);
    assert_eq!(doc.listing().entries[1].size, Some(27));
    assert!(doc.listing().entries[1].modified.is_some());

    let locked = write(dir.path(), "e.7z", &sample_7z(true, false));
    let listed = list(&locked, Format::SevenZ, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
    let listed = list(&hidden, Format::SevenZ, &limit()).unwrap_or_else(|e| panic!("{e}"));
    assert!(listed.facts.encrypted_headers);
    assert!(listed.entries.is_empty());
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
    let listed = list(&path, Format::Rar, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
    let listed = list(&plain, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
    let listed = list(&joliet, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
    assert!(list(&not_iso, Format::Iso, &limit()).is_err());
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
    let listed = list(&image, Format::Iso, &limit()).unwrap_or_else(|e| panic!("{e}"));
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
    let mut doc = runtime::block_on(ArchivePreviewer.open(&loc, provider, CancelToken::new()))
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
