//! The binary icon containers decoded into variants: ICO and CUR through
//! the `ico` crate (BMP and PNG entries), ICNS through the `icns` crate
//! (PNG and run-length encoded types; a JPEG 2000 entry and an unknown
//! type are listed as not decodable). The variants come out largest
//! first.

use std::io::Cursor;

use types::Error;

/// The gzip-free magic of a JPEG 2000 file, as an ICNS element carries it.
const JPEG_2000: [u8; 12] = [
    0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
];

/// The PNG signature.
const PNG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// ICNS elements that carry no picture: the table of contents, the
/// version, the name and the info dictionary.
const ICNS_METADATA: [&[u8; 4]; 4] = [b"TOC ", b"icnV", b"name", b"info"];

/// Which container a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A Windows icon.
    Ico,
    /// A Windows cursor.
    Cur,
    /// A macOS icon family.
    Icns,
    /// An X11 pixmap.
    Xpm,
}

impl Format {
    /// The name for the footer and File info.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Ico => "ICO",
            Self::Cur => "CUR",
            Self::Icns => "ICNS",
            Self::Xpm => "XPM",
        }
    }
}

/// One picture of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Width in pixels; 0 when unknown (an ICNS element of unknown type).
    pub width: u32,
    /// Height in pixels; 0 when unknown.
    pub height: u32,
    /// Bits per pixel, when the container records it (ICO and CUR).
    pub depth: Option<u16>,
    /// The ICNS element type: `ic07`.
    pub ostype: Option<String>,
    /// How the pixels are stored: `PNG`, `BMP`, `RLE`, `JPEG 2000`.
    pub encoding: String,
    /// A cursor's hotspot.
    pub hotspot: Option<(u32, u32)>,
    /// The RGBA pixels, row-major, or why the variant cannot be decoded.
    pub pixels: Result<Vec<u8>, String>,
}

impl Variant {
    /// The facts for the footer: the size, the depth or the type, and the
    /// encoding, or `not decodable (JPEG 2000)` for a variant that is no
    /// page.
    #[must_use]
    pub fn facts(&self) -> Vec<String> {
        let mut facts = Vec::new();
        if self.width > 0 && self.height > 0 {
            facts.push(format!("{} × {}", self.width, self.height));
        }
        if let Some(depth) = self.depth {
            facts.push(format!("{depth}-bit"));
        }
        if let Some(ostype) = &self.ostype {
            facts.push(ostype.clone());
        }
        match &self.pixels {
            Ok(_) => facts.push(self.encoding.clone()),
            Err(reason) => facts.push(format!("not decodable ({reason})")),
        }
        facts
    }

    /// The File info row: the facts with the hotspot of a cursor,
    /// `16 × 16 · 32-bit · BMP · hotspot 3, 4`.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut facts = self.facts();
        if let Some((x, y)) = self.hotspot {
            facts.push(format!("hotspot {x}, {y}"));
        }
        facts.join(" · ")
    }

    /// Whether the variant decoded, and so is a page.
    #[must_use]
    pub fn decodable(&self) -> bool {
        self.pixels.is_ok()
    }
}

/// A decoded container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// The format.
    pub format: Format,
    /// Every variant, largest first; the decodable ones are the pages.
    pub variants: Vec<Variant>,
}

/// An invalid-data error with a message.
fn invalid(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.into(),
    ))
}

/// Whether `sniff` opens an ICO or a CUR: the header, a count, and a
/// first directory entry that makes sense.
#[must_use]
pub fn is_ico_or_cur(sniff: &[u8]) -> bool {
    if sniff.len() < 22 || sniff[0] != 0 || sniff[1] != 0 || sniff[3] != 0 {
        return false;
    }
    let le16 = |at: usize| u16::from_le_bytes([sniff[at], sniff[at + 1]]);
    let le32 =
        |at: usize| u32::from_le_bytes([sniff[at], sniff[at + 1], sniff[at + 2], sniff[at + 3]]);
    let cursor = match sniff[2] {
        1 => false,
        2 => true,
        _ => return false,
    };
    let count = le16(4);
    if count == 0 || sniff[9] != 0 {
        return false;
    }
    // The first directory entry: width, height, colours, a reserved byte,
    // then planes and bits (the hotspot for a cursor), size and offset.
    let (planes, bits) = (le16(10), le16(12));
    let fits = if cursor {
        planes <= 256 && bits <= 256
    } else {
        planes <= 1 && matches!(bits, 0 | 1 | 2 | 4 | 8 | 16 | 24 | 32)
    };
    fits && le32(14) > 0 && le32(18) >= 6 + 16 * u32::from(count)
}

impl Container {
    /// Decodes an ICO, CUR or ICNS file by its magic.
    ///
    /// # Errors
    ///
    /// `Error::Unsupported` for bytes that are none of the three,
    /// `Error::Io` for a container whose directory does not parse (a
    /// truncated file). A variant that does not decode is no error: it is
    /// listed with the reason.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.starts_with(b"icns") {
            Self::icns(bytes)
        } else if is_ico_or_cur(bytes) {
            Self::ico(bytes)
        } else {
            Err(Error::Unsupported("not an icon container"))
        }
    }

    /// Decodes an ICO or CUR.
    ///
    /// # Errors
    ///
    /// `Error::Io` when the directory does not parse.
    pub fn ico(bytes: &[u8]) -> Result<Self, Error> {
        let dir = ico::IconDir::read(Cursor::new(bytes)).map_err(|e| invalid(e.to_string()))?;
        let format = match dir.resource_type() {
            ico::ResourceType::Icon => Format::Ico,
            ico::ResourceType::Cursor => Format::Cur,
        };
        let variants = dir
            .entries()
            .iter()
            .map(|entry| {
                let png = entry.is_png();
                let depth = Some(entry.bits_per_pixel())
                    .filter(|d| *d != 0)
                    .or_else(|| data_depth(entry.data(), png));
                Variant {
                    width: entry.width(),
                    height: entry.height(),
                    depth,
                    ostype: None,
                    encoding: if png { "PNG" } else { "BMP" }.to_owned(),
                    hotspot: entry
                        .cursor_hotspot()
                        .map(|(x, y)| (u32::from(x), u32::from(y))),
                    pixels: entry
                        .decode()
                        .map(ico::IconImage::into_rgba_data)
                        .map_err(|e| e.to_string()),
                }
            })
            .collect();
        let mut container = Self { format, variants };
        container.sort();
        Ok(container)
    }

    /// Decodes an ICNS.
    ///
    /// # Errors
    ///
    /// `Error::Io` when the family does not parse.
    pub fn icns(bytes: &[u8]) -> Result<Self, Error> {
        let family =
            icns::IconFamily::read(Cursor::new(bytes)).map_err(|e| invalid(e.to_string()))?;
        let mut variants = Vec::new();
        for element in &family.elements {
            let ostype = element.ostype.to_string();
            let Some(icon_type) = element.icon_type() else {
                if ICNS_METADATA.contains(&&element.ostype.0) {
                    continue;
                }
                variants.push(Variant {
                    width: 0,
                    height: 0,
                    depth: None,
                    ostype: Some(ostype),
                    encoding: String::new(),
                    hotspot: None,
                    pixels: Err("unknown type".to_owned()),
                });
                continue;
            };
            if icon_type.is_mask() {
                // Masks are folded into their colour element.
                continue;
            }
            let data = &element.data;
            let encoding = if data.starts_with(&PNG) {
                "PNG"
            } else if data.starts_with(&JPEG_2000) {
                "JPEG 2000"
            } else if data.starts_with(b"ARGB") {
                "ARGB"
            } else {
                match icon_type.encoding() {
                    icns::Encoding::RLE24 | icns::Encoding::JP2PNG | icns::Encoding::ARGB => "RLE",
                    icns::Encoding::Palette4 => "4-bit",
                    icns::Encoding::Palette8 => "8-bit",
                    icns::Encoding::Mono | icns::Encoding::MonoA => "1-bit",
                    icns::Encoding::Mask8 => "mask",
                }
            };
            let pixels = if data.starts_with(&JPEG_2000) {
                Err("JPEG 2000".to_owned())
            } else {
                family
                    .get_icon_with_type(icon_type)
                    .or_else(|_| element.decode_image())
                    .map(|image| {
                        image
                            .convert_to(icns::PixelFormat::RGBA)
                            .into_data()
                            .into_vec()
                    })
                    .map_err(|e| e.to_string())
            };
            variants.push(Variant {
                width: icon_type.pixel_width(),
                height: icon_type.pixel_height(),
                depth: None,
                ostype: Some(ostype),
                encoding: encoding.to_owned(),
                hotspot: None,
                pixels,
            });
        }
        let mut container = Self {
            format: Format::Icns,
            variants,
        };
        container.sort();
        Ok(container)
    }

    /// Largest first, the deeper of two equal sizes first, the file's
    /// order otherwise.
    fn sort(&mut self) {
        self.variants.sort_by(|a, b| {
            (u64::from(b.width) * u64::from(b.height))
                .cmp(&(u64::from(a.width) * u64::from(a.height)))
                .then(b.depth.cmp(&a.depth))
        });
    }

    /// How many variants cannot be decoded.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.variants.iter().filter(|v| !v.decodable()).count()
    }
}

/// The bits per pixel from an entry's data when the directory has none
/// (a CUR, whose directory holds the hotspot instead): the BMP header's
/// `biBitCount`, or the PNG header's bit depth times its channels.
fn data_depth(data: &[u8], png: bool) -> Option<u16> {
    if png {
        let (depth, kind) = (*data.get(24)?, *data.get(25)?);
        let channels = match kind {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => return None,
        };
        Some(u16::from(depth) * channels)
    } else {
        Some(u16::from_le_bytes([*data.get(14)?, *data.get(15)?]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ico_sniff_wants_a_header_that_makes_sense() {
        let mut ico = vec![
            0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 40, 0, 0, 0, 22, 0, 0, 0,
        ];
        assert!(is_ico_or_cur(&ico));
        ico[2] = 2;
        assert!(
            is_ico_or_cur(&ico),
            "a cursor: planes and bits are the hotspot"
        );
        ico[2] = 3;
        assert!(!is_ico_or_cur(&ico));
        ico[2] = 1;
        ico[12] = 7;
        assert!(!is_ico_or_cur(&ico), "seven bits a pixel");
        ico[12] = 32;
        ico[9] = 1;
        assert!(!is_ico_or_cur(&ico), "a reserved byte set");
        assert!(!is_ico_or_cur(&[0, 0, 1, 0, 0, 0]), "no entries");
        assert!(!is_ico_or_cur(&[0, 0, 1, 0, 1, 0]), "cut short");
        assert!(!is_ico_or_cur(
            b"\x89PNG\r\n\x1a\n and then some more bytes"
        ));
    }

    #[test]
    fn a_variant_describes_itself() {
        let v = Variant {
            width: 16,
            height: 16,
            depth: Some(32),
            ostype: None,
            encoding: "BMP".to_owned(),
            hotspot: Some((3, 4)),
            pixels: Ok(Vec::new()),
        };
        assert_eq!(v.describe(), "16 × 16 · 32-bit · BMP · hotspot 3, 4");
        assert_eq!(v.facts(), ["16 × 16", "32-bit", "BMP"]);
        let skipped = Variant {
            width: 256,
            height: 256,
            depth: None,
            ostype: Some("ic08".to_owned()),
            encoding: "JPEG 2000".to_owned(),
            hotspot: None,
            pixels: Err("JPEG 2000".to_owned()),
        };
        assert_eq!(
            skipped.describe(),
            "256 × 256 · ic08 · not decodable (JPEG 2000)"
        );
        assert!(!skipped.decodable());
    }

    #[test]
    fn depths_come_from_the_data_when_the_directory_has_none() {
        let mut png = PNG.to_vec();
        png.extend_from_slice(&[
            0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 0, 16, 0, 0, 0, 16, 8, 6,
        ]);
        assert_eq!(data_depth(&png, true), Some(32));
        png[25] = 2;
        assert_eq!(data_depth(&png, true), Some(24));
        png[25] = 3;
        png[24] = 4;
        assert_eq!(data_depth(&png, true), Some(4));
        let mut bmp = vec![40, 0, 0, 0, 16, 0, 0, 0, 32, 0, 0, 0, 1, 0, 24, 0];
        assert_eq!(data_depth(&bmp, false), Some(24));
        bmp.truncate(10);
        assert_eq!(data_depth(&bmp, false), None);
    }
}
