//! Archives for tests and fixtures, written here: a zip, a tar (and its
//! compressed flavours), a 7z, a RAR 5 built by hand (nothing in the
//! workspace writes RAR) and an ISO 9660 image built by hand. The encoders
//! the shipped plugin has no use for (AES zip entries, 7z writing, xz)
//! sit behind the `fixtures` feature, which the tests turn on.

#![allow(clippy::missing_panics_doc)]

use std::io::{Cursor, Write};

/// A zip with a folder, two files (one AES-encrypted) and a comment.
#[cfg(feature = "fixtures")]
#[must_use]
pub fn sample_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
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
            options.with_aes_encryption(zip::AesMode::Aes256, "pw"),
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
#[must_use]
pub fn sample_tar(payload: usize) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    let mut dir = tar::Header::new_ustar();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_mode(0o755);
    dir.set_mtime(1_700_000_000);
    dir.set_path("src/").unwrap_or_else(|e| panic!("{e}"));
    dir.set_cksum();
    builder
        .append(&dir, std::io::empty())
        .unwrap_or_else(|e| panic!("{e}"));
    let data = vec![b'x'; payload];
    let mut file = tar::Header::new_ustar();
    file.set_entry_type(tar::EntryType::Regular);
    file.set_size(data.len() as u64);
    file.set_mode(0o644);
    file.set_mtime(1_700_000_000);
    file.set_path("src/main.rs")
        .unwrap_or_else(|e| panic!("{e}"));
    file.set_cksum();
    builder
        .append(&file, data.as_slice())
        .unwrap_or_else(|e| panic!("{e}"));
    let mut link = tar::Header::new_ustar();
    link.set_entry_type(tar::EntryType::Symlink);
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

/// `bytes` gzip-compressed.
#[must_use]
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// `bytes` zstd-compressed.
#[must_use]
pub fn zstd_bytes(bytes: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(bytes, 3).unwrap_or_else(|e| panic!("{e}"))
}

/// `bytes` xz-compressed.
#[cfg(feature = "fixtures")]
#[must_use]
pub fn xz(bytes: &[u8]) -> Vec<u8> {
    let mut writer = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default())
        .unwrap_or_else(|e| panic!("{e}"));
    writer.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    writer.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// `bytes` bzip2-compressed.
#[must_use]
pub fn bzip2_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// `bytes` as an lz4 frame.
#[must_use]
pub fn lz4(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
    encoder.write_all(bytes).unwrap_or_else(|e| panic!("{e}"));
    encoder.finish().unwrap_or_else(|e| panic!("{e}"))
}

/// A 7z with a directory and two files; `encrypt` puts a password on the
/// content, `hide` also on the header.
#[cfg(feature = "fixtures")]
#[must_use]
pub fn sample_7z(encrypt: bool, hide: bool) -> Vec<u8> {
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
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
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
/// entries" question is about, QUESTIONS.md 96).
#[must_use]
pub fn sample_rar() -> Vec<u8> {
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
#[must_use]
pub fn sample_iso(joliet: bool) -> Vec<u8> {
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

/// A zip of `entries` small stored files under a few folders, for the
/// first-screen timing: `dir<n>/file<m>.txt`, 32 files per folder.
#[must_use]
pub fn big_zip(entries: usize) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..entries {
        writer
            .start_file(format!("dir{:04}/file{:05}.txt", i / 32, i), options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_all(format!("entry {i}\n").as_bytes())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    writer
        .finish()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_inner()
}
