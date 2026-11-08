//! Disk images for tests, written here so nothing binary is committed: a
//! FAT12 file system with two files and a folder, and that file system
//! inside a fixed VHD, a VHDX and a QCOW2 container, each the smallest
//! layout the formats allow. Raw, the FAT image is a `.img`; the three
//! containers are what the 7-Zip library opens before it reaches the file
//! system.

#![allow(clippy::missing_panics_doc)]

/// The bytes `hello.txt` holds in every fixture.
pub const HELLO: &[u8] = b"hello from fat\n";
/// The bytes `docs/readme.txt` holds in every fixture.
pub const README: &[u8] = b"readme inside docs\n";

const SECTOR: usize = 512;

/// A 64 KiB FAT12 file system: `HELLO.TXT` at the top, `DOCS/README.TXT`
/// in a folder, the volume labelled `RICKFIX`.
#[must_use]
pub fn fat12_image() -> Vec<u8> {
    const SECTORS: usize = 128;
    let mut image = vec![0u8; SECTORS * SECTOR];
    // The boot sector and BIOS parameter block.
    let b = &mut image[..SECTOR];
    b[0..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    b[3..11].copy_from_slice(b"MSWIN4.1");
    b[11..13].copy_from_slice(&512u16.to_le_bytes());
    b[13] = 1; // sectors per cluster
    b[14..16].copy_from_slice(&1u16.to_le_bytes()); // reserved sectors
    b[16] = 2; // FATs
    b[17..19].copy_from_slice(&16u16.to_le_bytes()); // root entries
    b[19..21].copy_from_slice(&(SECTORS as u16).to_le_bytes());
    b[21] = 0xF8; // media
    b[22..24].copy_from_slice(&1u16.to_le_bytes()); // sectors per FAT
    b[24..26].copy_from_slice(&32u16.to_le_bytes()); // sectors per track
    b[26..28].copy_from_slice(&4u16.to_le_bytes()); // heads
    b[36] = 0x80; // drive number
    b[38] = 0x29; // extended boot signature
    b[39..43].copy_from_slice(&0x1234_5678u32.to_le_bytes()); // volume id
    b[43..54].copy_from_slice(b"RICKFIX    ");
    b[54..62].copy_from_slice(b"FAT12   ");
    b[510] = 0x55;
    b[511] = 0xAA;
    // The FAT: media, end marker, then one cluster per file and folder,
    // each a chain of one.
    let mut fat = vec![0u8; SECTOR];
    for (cluster, value) in [
        (0u16, 0xFF8u16),
        (1, 0xFFF),
        (2, 0xFFF),
        (3, 0xFFF),
        (4, 0xFFF),
    ] {
        fat12_set(&mut fat, cluster, value);
    }
    image[SECTOR..2 * SECTOR].copy_from_slice(&fat);
    image[2 * SECTOR..3 * SECTOR].copy_from_slice(&fat);
    // The root directory: the label, HELLO.TXT in cluster 2, DOCS in 3.
    let root = 3 * SECTOR;
    image[root..root + 32].copy_from_slice(&dir_entry(b"RICKFIX    ", 0x08, 0, 0));
    image[root + 32..root + 64].copy_from_slice(&dir_entry(
        b"HELLO   TXT",
        0x20,
        2,
        u32::try_from(HELLO.len()).unwrap_or(0),
    ));
    image[root + 64..root + 96].copy_from_slice(&dir_entry(b"DOCS       ", 0x10, 3, 0));
    // Cluster n sits in sector 4 + (n - 2).
    let cluster = |n: usize| (4 + n - 2) * SECTOR;
    image[cluster(2)..cluster(2) + HELLO.len()].copy_from_slice(HELLO);
    let docs = cluster(3);
    image[docs..docs + 32].copy_from_slice(&dir_entry(b".          ", 0x10, 3, 0));
    image[docs + 32..docs + 64].copy_from_slice(&dir_entry(b"..         ", 0x10, 0, 0));
    image[docs + 64..docs + 96].copy_from_slice(&dir_entry(
        b"README  TXT",
        0x20,
        4,
        u32::try_from(README.len()).unwrap_or(0),
    ));
    image[cluster(4)..cluster(4) + README.len()].copy_from_slice(README);
    image
}

/// Sets a 12-bit FAT entry.
fn fat12_set(fat: &mut [u8], cluster: u16, value: u16) {
    let at = usize::from(cluster) + usize::from(cluster) / 2;
    if cluster % 2 == 0 {
        fat[at] = (value & 0xFF) as u8;
        fat[at + 1] = (fat[at + 1] & 0xF0) | ((value >> 8) & 0x0F) as u8;
    } else {
        fat[at] = (fat[at] & 0x0F) | ((value << 4) & 0xF0) as u8;
        fat[at + 1] = (value >> 4) as u8;
    }
}

/// A 32-byte FAT directory entry dated 2024-05-06 07:08:08.
fn dir_entry(name: &[u8; 11], attributes: u8, cluster: u16, size: u32) -> [u8; 32] {
    let mut entry = [0u8; 32];
    entry[..11].copy_from_slice(name);
    entry[11] = attributes;
    let date = 0x58A6u16.to_le_bytes();
    let time = 0x3904u16.to_le_bytes();
    entry[14..16].copy_from_slice(&time);
    entry[16..18].copy_from_slice(&date);
    entry[18..20].copy_from_slice(&date);
    entry[22..24].copy_from_slice(&time);
    entry[24..26].copy_from_slice(&date);
    entry[26..28].copy_from_slice(&cluster.to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

/// `raw` as a fixed VHD: the data followed by the 512-byte footer.
#[must_use]
pub fn vhd_fixed(raw: &[u8]) -> Vec<u8> {
    let size = raw.len() as u64;
    let mut footer = [0u8; SECTOR];
    footer[0..8].copy_from_slice(b"conectix");
    footer[8..12].copy_from_slice(&2u32.to_be_bytes()); // features: reserved bit
    footer[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // version 1.0
    footer[16..24].copy_from_slice(&u64::MAX.to_be_bytes()); // no dynamic header
    footer[24..28].copy_from_slice(&0u32.to_be_bytes()); // timestamp
    footer[28..32].copy_from_slice(b"rick");
    footer[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[36..40].copy_from_slice(b"Wi2k");
    footer[40..48].copy_from_slice(&size.to_be_bytes());
    footer[48..56].copy_from_slice(&size.to_be_bytes());
    // Geometry: cylinders, heads, sectors per track for the sector count.
    let sectors = u32::try_from(raw.len() / SECTOR).unwrap_or(0);
    let heads = 4u8;
    let per_track = 32u8;
    let cylinders = u16::try_from(sectors / (u32::from(heads) * u32::from(per_track))).unwrap_or(1);
    footer[56..58].copy_from_slice(&cylinders.to_be_bytes());
    footer[58] = heads;
    footer[59] = per_track;
    footer[60..64].copy_from_slice(&2u32.to_be_bytes()); // fixed disk
    footer[68..84].copy_from_slice(&[0x11; 16]); // unique id
    let sum: u32 = footer.iter().map(|&b| u32::from(b)).sum();
    footer[64..68].copy_from_slice(&(!sum).to_be_bytes());
    let mut out = raw.to_vec();
    out.extend_from_slice(&footer);
    out
}

/// CRC-32C (Castagnoli), as VHDX checksums its structures.
#[must_use]
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0x82F6_3B78
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

const MIB: usize = 1 << 20;

/// `raw` (at most 1 MiB, a multiple of 512) as a VHDX: the file type
/// identifier, two headers, two region tables, an empty log, the metadata
/// (file parameters, virtual disk size, sector sizes, the disk id), the
/// block allocation table with one present block, and that block. 5 MiB.
#[must_use]
pub fn vhdx(raw: &[u8]) -> Vec<u8> {
    assert!(raw.len() <= MIB && raw.len() % SECTOR == 0);
    let mut out = vec![0u8; 5 * MIB];
    // The file type identifier.
    out[0..8].copy_from_slice(b"vhdxfile");
    for (i, c) in "rick tests".encode_utf16().enumerate() {
        out[8 + 2 * i..10 + 2 * i].copy_from_slice(&c.to_le_bytes());
    }
    // The two headers at 64 KiB and 128 KiB.
    let mut header = vec![0u8; 4096];
    header[0..4].copy_from_slice(b"head");
    header[8..16].copy_from_slice(&1u64.to_le_bytes()); // sequence number
    header[0x10..0x20].copy_from_slice(&[0x22; 16]); // file write guid
    header[0x20..0x30].copy_from_slice(&[0x33; 16]); // data write guid
    // The log guid stays zero: no log to replay.
    header[0x42..0x44].copy_from_slice(&1u16.to_le_bytes()); // version
    header[0x44..0x48].copy_from_slice(&(MIB as u32).to_le_bytes()); // log length
    header[0x48..0x50].copy_from_slice(&(MIB as u64).to_le_bytes()); // log offset
    let crc = crc32c(&header);
    header[4..8].copy_from_slice(&crc.to_le_bytes());
    out[64 * 1024..64 * 1024 + 4096].copy_from_slice(&header);
    out[128 * 1024..128 * 1024 + 4096].copy_from_slice(&header);
    // The two region tables at 192 KiB and 256 KiB: BAT and metadata.
    let mut region = vec![0u8; 64 * 1024];
    region[0..4].copy_from_slice(b"regi");
    region[8..12].copy_from_slice(&2u32.to_le_bytes()); // entry count
    let entries: [([u8; 16], u64); 2] = [
        (
            [
                0x66, 0x77, 0xC2, 0x2D, 0x23, 0xF6, 0x00, 0x42, 0x9D, 0x64, 0x11, 0x5E, 0x9B, 0xFD,
                0x4A, 0x08,
            ],
            3 * MIB as u64,
        ),
        (
            [
                0x06, 0xA2, 0x7C, 0x8B, 0x90, 0x47, 0x9A, 0x4B, 0xB8, 0xFE, 0x57, 0x5F, 0x05, 0x0F,
                0x88, 0x6E,
            ],
            2 * MIB as u64,
        ),
    ];
    for (i, (guid, offset)) in entries.iter().enumerate() {
        let at = 0x10 + 0x20 * i;
        region[at..at + 16].copy_from_slice(guid);
        region[at + 0x10..at + 0x18].copy_from_slice(&offset.to_le_bytes());
        region[at + 0x18..at + 0x1C].copy_from_slice(&(MIB as u32).to_le_bytes());
        region[at + 0x1C..at + 0x20].copy_from_slice(&1u32.to_le_bytes()); // required
    }
    let crc = crc32c(&region);
    region[4..8].copy_from_slice(&crc.to_le_bytes());
    out[192 * 1024..256 * 1024].copy_from_slice(&region);
    out[256 * 1024..320 * 1024].copy_from_slice(&region);
    // The metadata region at 2 MiB: the table, then the items from 64 KiB in.
    let meta = 2 * MIB;
    out[meta..meta + 8].copy_from_slice(b"metadata");
    let items: [([u8; 16], Vec<u8>, u32); 5] = [
        // File parameters: block size 1 MiB, no flags.
        (
            [
                0x37, 0x67, 0xa1, 0xca, 0x36, 0xfa, 0x43, 0x4d, 0xb3, 0xb6, 0x33, 0xf0, 0xaa, 0x44,
                0xe7, 0x6b,
            ],
            [(MIB as u32).to_le_bytes(), 0u32.to_le_bytes()].concat(),
            4,
        ),
        // Virtual disk size.
        (
            [
                0x24, 0x42, 0xa5, 0x2f, 0x1b, 0xcd, 0x76, 0x48, 0xb2, 0x11, 0x5d, 0xbe, 0xd8, 0x3b,
                0xf4, 0xb8,
            ],
            (raw.len() as u64).to_le_bytes().to_vec(),
            6,
        ),
        // Virtual disk id.
        (
            [
                0xab, 0x12, 0xca, 0xbe, 0xe6, 0xb2, 0x23, 0x45, 0x93, 0xef, 0xc3, 0x09, 0xe0, 0x00,
                0xc7, 0x46,
            ],
            vec![0x44; 16],
            6,
        ),
        // Logical sector size.
        (
            [
                0x1d, 0xbf, 0x41, 0x81, 0x6f, 0xa9, 0x09, 0x47, 0xba, 0x47, 0xf2, 0x33, 0xa8, 0xfa,
                0xab, 0x5f,
            ],
            512u32.to_le_bytes().to_vec(),
            6,
        ),
        // Physical sector size.
        (
            [
                0xc7, 0x48, 0xa3, 0xcd, 0x5d, 0x44, 0x71, 0x44, 0x9c, 0xc9, 0xe9, 0x88, 0x52, 0x51,
                0xc5, 0x56,
            ],
            4096u32.to_le_bytes().to_vec(),
            6,
        ),
    ];
    out[meta + 10..meta + 12].copy_from_slice(&(items.len() as u16).to_le_bytes());
    let mut data_at = 64 * 1024u32;
    for (i, (guid, data, flags)) in items.iter().enumerate() {
        let at = meta + 32 + 32 * i;
        out[at..at + 16].copy_from_slice(guid);
        out[at + 0x10..at + 0x14].copy_from_slice(&data_at.to_le_bytes());
        out[at + 0x14..at + 0x18].copy_from_slice(&(data.len() as u32).to_le_bytes());
        out[at + 0x18..at + 0x1C].copy_from_slice(&flags.to_le_bytes());
        let start = meta + data_at as usize;
        out[start..start + data.len()].copy_from_slice(data);
        data_at += 64;
    }
    // The BAT at 3 MiB: one payload block, fully present, at 4 MiB.
    let bat = 3 * MIB;
    let entry: u64 = (4u64 << 20) | 6;
    out[bat..bat + 8].copy_from_slice(&entry.to_le_bytes());
    // The block.
    out[4 * MIB..4 * MIB + raw.len()].copy_from_slice(raw);
    out
}

/// `raw` (at most 64 KiB) as a QCOW2 image, version 3 with 64 KiB
/// clusters: the header, the L1 table, the refcount table and block, the
/// L2 table and one data cluster. 384 KiB.
#[must_use]
pub fn qcow2(raw: &[u8]) -> Vec<u8> {
    const CLUSTER: usize = 64 * 1024;
    assert!(raw.len() <= CLUSTER);
    let mut out = vec![0u8; 6 * CLUSTER];
    let h = &mut out[..CLUSTER];
    h[0..4].copy_from_slice(b"QFI\xfb");
    h[4..8].copy_from_slice(&3u32.to_be_bytes()); // version
    // No backing file: offset and size zero.
    h[0x14..0x18].copy_from_slice(&16u32.to_be_bytes()); // cluster bits
    h[0x18..0x20].copy_from_slice(&(raw.len() as u64).to_be_bytes()); // virtual size
    h[0x20..0x24].copy_from_slice(&0u32.to_be_bytes()); // no encryption
    h[0x24..0x28].copy_from_slice(&1u32.to_be_bytes()); // L1 entries
    h[0x28..0x30].copy_from_slice(&(CLUSTER as u64).to_be_bytes()); // L1 at cluster 1
    h[0x30..0x38].copy_from_slice(&(2 * CLUSTER as u64).to_be_bytes()); // refcount table at 2
    h[0x38..0x3C].copy_from_slice(&1u32.to_be_bytes()); // refcount table clusters
    // No snapshots; no feature bits.
    h[0x60..0x64].copy_from_slice(&4u32.to_be_bytes()); // refcount order: 16-bit
    h[0x64..0x68].copy_from_slice(&0x68u32.to_be_bytes()); // header length
    let copied = 1u64 << 63;
    // L1[0] points at the L2 table in cluster 4.
    out[CLUSTER..CLUSTER + 8].copy_from_slice(&((4 * CLUSTER as u64) | copied).to_be_bytes());
    // Refcount table[0] points at the refcount block in cluster 3, which
    // counts one reference for each of the six clusters.
    out[2 * CLUSTER..2 * CLUSTER + 8].copy_from_slice(&(3 * CLUSTER as u64).to_be_bytes());
    for i in 0..6 {
        let at = 3 * CLUSTER + 2 * i;
        out[at..at + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    // L2[0] points at the data in cluster 5.
    out[4 * CLUSTER..4 * CLUSTER + 8]
        .copy_from_slice(&((5 * CLUSTER as u64) | copied).to_be_bytes());
    out[5 * CLUSTER..5 * CLUSTER + raw.len()].copy_from_slice(raw);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixtures_have_their_signatures_in_place() {
        let fat = fat12_image();
        assert_eq!(fat.len(), 64 * 1024);
        assert_eq!(&fat[54..62], b"FAT12   ");
        assert_eq!(&fat[510..512], &[0x55, 0xAA]);
        assert_eq!(&fat[4 * SECTOR..4 * SECTOR + HELLO.len()], HELLO);
        let vhd = vhd_fixed(&fat);
        assert_eq!(vhd.len(), fat.len() + 512);
        assert_eq!(&vhd[fat.len()..fat.len() + 8], b"conectix");
        let vhdx = vhdx(&fat);
        assert_eq!(vhdx.len(), 5 * MIB);
        assert_eq!(&vhdx[..8], b"vhdxfile");
        assert_eq!(&vhdx[64 * 1024..64 * 1024 + 4], b"head");
        assert_eq!(&vhdx[2 * MIB..2 * MIB + 8], b"metadata");
        assert_eq!(&vhdx[4 * MIB..4 * MIB + 512], &fat[..512]);
        let qcow = qcow2(&fat);
        assert_eq!(qcow.len(), 384 * 1024);
        assert_eq!(&qcow[..4], b"QFI\xfb");
        assert_eq!(&qcow[5 * 64 * 1024..5 * 64 * 1024 + 512], &fat[..512]);
        // The checksum of a known vector: CRC-32C of "123456789".
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }
}
