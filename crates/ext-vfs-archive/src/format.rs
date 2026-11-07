//! Which kind of archive a file is, and the codecs around the tar family
//! and the single-file formats.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use types::Error;

/// A compression codec: wrapped around a tar, or a file on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// gzip (`.gz`).
    Gzip,
    /// Zstandard (`.zst`).
    Zstd,
    /// xz / LZMA (`.xz`).
    Xz,
    /// bzip2 (`.bz2`).
    Bzip2,
    /// LZ4 frames (`.lz4`).
    Lz4,
    /// Brotli (`.br`).
    Brotli,
    /// Snappy frames (`.sz`).
    Snappy,
}

impl Codec {
    /// The codec's usual extension, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gzip => "gz",
            Self::Zstd => "zst",
            Self::Xz => "xz",
            Self::Bzip2 => "bz2",
            Self::Lz4 => "lz4",
            Self::Brotli => "br",
            Self::Snappy => "sz",
        }
    }

    /// The codec an extension names.
    #[must_use]
    pub fn by_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "gz" | "gzip" => Self::Gzip,
            "zst" | "zstd" => Self::Zstd,
            "xz" | "lzma" => Self::Xz,
            "bz2" | "bzip2" => Self::Bzip2,
            "lz4" => Self::Lz4,
            "br" => Self::Brotli,
            "sz" | "snappy" => Self::Snappy,
            _ => return None,
        })
    }

    /// The codec whose stream starts with `head`; Brotli has no signature.
    #[must_use]
    pub fn by_magic(head: &[u8]) -> Option<Self> {
        Some(if head.starts_with(&[0x1f, 0x8b]) {
            Self::Gzip
        } else if head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            Self::Zstd
        } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
            Self::Xz
        } else if head.starts_with(b"BZh") {
            Self::Bzip2
        } else if head.starts_with(&[0x04, 0x22, 0x4d, 0x18]) {
            Self::Lz4
        } else if head.starts_with(&[0xff, 0x06, 0x00, 0x00, b's', b'N', b'a', b'P', b'p', b'Y']) {
            Self::Snappy
        } else {
            return None;
        })
    }

    /// A decompressing reader over `inner`.
    ///
    /// # Errors
    ///
    /// The codec's error setting the stream up (zstd allocates its context).
    pub fn decoder(self, inner: Box<dyn Read + Send>) -> io::Result<Box<dyn Read + Send>> {
        Ok(match self {
            Self::Gzip => Box::new(flate2::read::MultiGzDecoder::new(inner)),
            Self::Zstd => Box::new(zstd::stream::read::Decoder::new(inner)?),
            Self::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(inner)),
            Self::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(inner)),
            Self::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(inner)),
            Self::Brotli => Box::new(brotli::Decompressor::new(inner, 64 * 1024)),
            Self::Snappy => Box::new(snap::read::FrameDecoder::new(inner)),
        })
    }

    /// A compressing writer over `inner`; [`Finish::finish`] ends the
    /// stream.
    ///
    /// # Errors
    ///
    /// The codec's error setting the stream up.
    pub fn encoder(self, inner: Box<dyn Write + Send>) -> io::Result<Box<dyn Finish>> {
        Ok(match self {
            Self::Gzip => Box::new(flate2::write::GzEncoder::new(
                inner,
                flate2::Compression::default(),
            )),
            Self::Zstd => Box::new(zstd::stream::write::Encoder::new(inner, 3)?),
            Self::Xz => Box::new(liblzma::write::XzEncoder::new(inner, 6)),
            Self::Bzip2 => Box::new(bzip2::write::BzEncoder::new(
                inner,
                bzip2::Compression::default(),
            )),
            Self::Lz4 => Box::new(lz4_flex::frame::FrameEncoder::new(inner)),
            Self::Brotli => Box::new(brotli::CompressorWriter::new(inner, 64 * 1024, 6, 22)),
            Self::Snappy => Box::new(snap::write::FrameEncoder::new(inner)),
        })
    }
}

/// A writer whose stream must be ended explicitly.
pub trait Finish: Write + Send {
    /// Writes the stream's trailer and flushes everything below.
    ///
    /// # Errors
    ///
    /// The codec's or the writer's I/O error.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

impl Finish for flate2::write::GzEncoder<Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        flate2::write::GzEncoder::finish(*self)?.flush()
    }
}

impl Finish for zstd::stream::write::Encoder<'static, Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        zstd::stream::write::Encoder::finish(*self)?.flush()
    }
}

impl Finish for liblzma::write::XzEncoder<Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        liblzma::write::XzEncoder::finish(*self)?.flush()
    }
}

impl Finish for bzip2::write::BzEncoder<Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        bzip2::write::BzEncoder::finish(*self)?.flush()
    }
}

impl Finish for lz4_flex::frame::FrameEncoder<Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        lz4_flex::frame::FrameEncoder::finish(*self)
            .map_err(io::Error::other)?
            .flush()
    }
}

impl Finish for brotli::CompressorWriter<Box<dyn Write + Send>> {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.flush()?;
        // Dropping the writer ends the stream; the inner writer is flushed
        // by `flush` above.
        Ok(())
    }
}

impl Finish for snap::write::FrameEncoder<Box<dyn Write + Send>> {
    fn finish(self: Box<Self>) -> io::Result<()> {
        snap::write::FrameEncoder::into_inner(*self)
            .map_err(|e| io::Error::other(e.to_string()))?
            .flush()
    }
}

/// What kind of archive a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// zip, DEFLATE or zstd inside.
    Zip,
    /// A tar, plain or through a codec.
    Tar(Option<Codec>),
    /// 7z.
    SevenZ,
    /// RAR; read-only, creation is not licensable.
    Rar,
    /// An ISO 9660 or UDF disc image; read-only.
    Image,
    /// One compressed file: a one-entry archive.
    Single(Codec),
}

impl Format {
    /// Whether entries can be added, renamed and removed: zip, the tar
    /// family and 7z.
    #[must_use]
    pub fn writable(self) -> bool {
        matches!(self, Self::Zip | Self::Tar(_) | Self::SevenZ)
    }

    /// The format's name for messages.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar(None) => "tar",
            Self::Tar(Some(_)) => "compressed tar",
            Self::SevenZ => "7z",
            Self::Rar => "RAR",
            Self::Image => "disc image",
            Self::Single(_) => "compressed file",
        }
    }

    /// The format a file name's extension names, if any. `.tar.gz` and its
    /// short forms are tars; a bare codec extension is a single file until
    /// the contents say otherwise (see [`detect`](Self::detect)).
    #[must_use]
    pub fn by_extension(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        let (stem, ext) = lower.rsplit_once('.')?;
        Some(match ext {
            "zip" | "jar" | "war" | "ear" | "cbz" | "xpi" | "whl" => Self::Zip,
            "tar" => Self::Tar(None),
            "tgz" => Self::Tar(Some(Codec::Gzip)),
            "tzst" => Self::Tar(Some(Codec::Zstd)),
            "txz" => Self::Tar(Some(Codec::Xz)),
            "tbz" | "tbz2" => Self::Tar(Some(Codec::Bzip2)),
            "7z" => Self::SevenZ,
            "rar" | "cbr" => Self::Rar,
            "iso" | "udf" => Self::Image,
            _ => {
                let codec = Codec::by_extension(ext)?;
                let tar_inside = Path::new(stem)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("tar"));
                if tar_inside {
                    Self::Tar(Some(codec))
                } else {
                    Self::Single(codec)
                }
            }
        })
    }

    /// The format the first bytes of a file name: zip, 7z, RAR, a tar (the
    /// `ustar` magic in its first header), a disc image (`CD001`, `BEA01`
    /// or `NSR0x` in its volume descriptors) or a codec.
    #[must_use]
    pub fn by_magic(head: &[u8], descriptors: &[u8]) -> Option<Self> {
        if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
            return Some(Self::Zip);
        }
        if head.starts_with(&[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c]) {
            return Some(Self::SevenZ);
        }
        if head.starts_with(b"Rar!\x1a\x07") {
            return Some(Self::Rar);
        }
        if is_tar_header(head) {
            return Some(Self::Tar(None));
        }
        if descriptors.len() >= 5
            && matches!(
                &descriptors[..5],
                b"CD001" | b"BEA01" | b"NSR02" | b"NSR03" | b"TEA01"
            )
        {
            return Some(Self::Image);
        }
        Codec::by_magic(head).map(Self::Single)
    }

    /// The format of the file at `path`: by extension, then by content; a
    /// codec whose stream holds a tar is a tar.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` when the file is no archive this provider
    /// knows; the I/O error when it cannot be read.
    pub fn detect(path: &Path) -> Result<Self, Error> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut file = std::fs::File::open(path).map_err(Error::from_io)?;
        let format = if let Some(format) = Self::by_extension(&name) {
            format
        } else {
            let mut head = [0u8; 512];
            let read = read_fully(&mut file, &mut head).map_err(Error::from_io)?;
            let mut descriptors = [0u8; 6];
            let marker = file
                .seek(SeekFrom::Start(0x8001))
                .and_then(|_| read_fully(&mut file, &mut descriptors))
                .unwrap_or(0);
            file.seek(SeekFrom::Start(0)).map_err(Error::from_io)?;
            Self::by_magic(&head[..read], &descriptors[..marker])
                .ok_or(Error::Unsupported("not an archive"))?
        };
        // A codec file may hold a tar: the first decompressed block tells.
        if let Self::Single(codec) = format {
            let decoder = codec.decoder(Box::new(file))?;
            let mut block = [0u8; 512];
            let read = read_fully(&mut decoder.take(512), &mut block).unwrap_or(0);
            if is_tar_header(&block[..read]) {
                return Ok(Self::Tar(Some(codec)));
            }
        }
        Ok(format)
    }
}

/// Reads until `buf` is full or the reader ends; the count read.
fn read_fully(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Whether `block` is a tar header: the `ustar` magic, or an old-style
/// header whose checksum field adds up.
#[must_use]
pub fn is_tar_header(block: &[u8]) -> bool {
    if block.len() < 512 {
        return false;
    }
    if &block[257..262] == b"ustar" {
        return true;
    }
    // The checksum field holds the octal sum of the header with the field
    // itself read as spaces.
    let recorded = std::str::from_utf8(&block[148..156])
        .ok()
        .map(|s| s.trim_matches(|c: char| c == '\0' || c == ' '))
        .and_then(|s| u32::from_str_radix(s, 8).ok());
    let Some(recorded) = recorded else {
        return false;
    };
    let sum: u32 = block
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if (148..156).contains(&i) {
                32
            } else {
                u32::from(*b)
            }
        })
        .sum();
    sum == recorded && block[..100].iter().any(|b| *b != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_name_formats() {
        assert_eq!(Format::by_extension("a.zip"), Some(Format::Zip));
        assert_eq!(Format::by_extension("A.JAR"), Some(Format::Zip));
        assert_eq!(Format::by_extension("a.tar"), Some(Format::Tar(None)));
        assert_eq!(
            Format::by_extension("a.tar.gz"),
            Some(Format::Tar(Some(Codec::Gzip)))
        );
        assert_eq!(
            Format::by_extension("a.tgz"),
            Some(Format::Tar(Some(Codec::Gzip)))
        );
        assert_eq!(
            Format::by_extension("a.tar.zst"),
            Some(Format::Tar(Some(Codec::Zstd)))
        );
        assert_eq!(
            Format::by_extension("a.tar.lz4"),
            Some(Format::Tar(Some(Codec::Lz4)))
        );
        assert_eq!(Format::by_extension("a.7z"), Some(Format::SevenZ));
        assert_eq!(Format::by_extension("a.rar"), Some(Format::Rar));
        assert_eq!(Format::by_extension("a.iso"), Some(Format::Image));
        assert_eq!(
            Format::by_extension("notes.txt.gz"),
            Some(Format::Single(Codec::Gzip))
        );
        assert_eq!(
            Format::by_extension("a.br"),
            Some(Format::Single(Codec::Brotli))
        );
        assert_eq!(Format::by_extension("notes.txt"), None);
        assert_eq!(Format::by_extension("Makefile"), None);
        assert!(Format::Zip.writable());
        assert!(Format::Tar(Some(Codec::Xz)).writable());
        assert!(!Format::Rar.writable());
        assert!(!Format::Image.writable());
        assert!(!Format::Single(Codec::Gzip).writable());
    }

    #[test]
    fn magic_names_formats() {
        assert_eq!(Format::by_magic(b"PK\x03\x04rest", &[]), Some(Format::Zip));
        assert_eq!(
            Format::by_magic(&[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c, 0], &[]),
            Some(Format::SevenZ)
        );
        assert_eq!(
            Format::by_magic(b"Rar!\x1a\x07\x01\x00", &[]),
            Some(Format::Rar)
        );
        assert_eq!(
            Format::by_magic(&[0x1f, 0x8b, 8], &[]),
            Some(Format::Single(Codec::Gzip))
        );
        assert_eq!(Format::by_magic(b"", b"CD001"), Some(Format::Image));
        assert_eq!(Format::by_magic(b"", b"NSR03"), Some(Format::Image));
        assert_eq!(Format::by_magic(b"hello", b""), None);
        let mut header = [0u8; 512];
        header[..5].copy_from_slice(b"a.txt");
        header[257..262].copy_from_slice(b"ustar");
        assert_eq!(Format::by_magic(&header, &[]), Some(Format::Tar(None)));
    }

    /// A writer into a shared buffer, so the encoder can own it.
    struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn codecs_round_trip() {
        for codec in [
            Codec::Gzip,
            Codec::Zstd,
            Codec::Xz,
            Codec::Bzip2,
            Codec::Lz4,
            Codec::Brotli,
            Codec::Snappy,
        ] {
            let text: Vec<u8> = b"the quick brown fox ".repeat(100);
            let out: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = std::sync::Arc::default();
            let mut encoder = codec
                .encoder(Box::new(Shared(out.clone())))
                .unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            encoder
                .write_all(&text)
                .unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            encoder
                .finish()
                .unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            let compressed = out
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            assert!(compressed.len() < text.len(), "{codec:?} compresses");
            if codec != Codec::Brotli {
                assert_eq!(Codec::by_magic(&compressed), Some(codec), "{codec:?}");
            }
            let mut decoder = codec
                .decoder(Box::new(io::Cursor::new(compressed)))
                .unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            let mut back = Vec::new();
            decoder
                .read_to_end(&mut back)
                .unwrap_or_else(|e| panic!("{codec:?}: {e}"));
            assert_eq!(back, text, "{codec:?}");
        }
    }
}
