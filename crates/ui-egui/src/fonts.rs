//! The interface's fonts (SPEC.md *Look and feel*, Typography): the
//! system's UI font for everything that is not code and its monospace font
//! for hashes, paths and code, ahead of egui's bundled fonts, which stay as
//! the fallback for glyphs the system fonts lack and for a system that
//! names none; and the Phosphor icon fonts.
//!
//! egui looks a character up font by font in a family's order and takes
//! the first font that maps it. Two things follow. A system font file is
//! parsed once before it is handed to egui, so a file egui cannot read
//! keeps the bundled font instead of stopping the application. And the
//! icon font goes *first* in every family, because a system font may map
//! the private-use area the icons live in (Inter does, the patched "Nerd"
//! monospace fonts do) and would otherwise show its own glyphs in place of
//! the icons; for that to work the icon font must map no text, so its few
//! text characters (space, the hyphen, `a` to `z`, which Phosphor keeps for
//! typing an icon's name) are moved out of reach in Rick's copy of it
//! ([`icon_font`]).

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};
use platform::{FontFile, Fonts};

use crate::icons;

/// The name the system's UI font is registered under.
pub const SYSTEM_UI: &str = "system-ui";
/// The name the system's monospace font is registered under.
pub const SYSTEM_MONO: &str = "system-mono";
/// The name the Phosphor regular weight is registered under, as a font and
/// as a family of its own.
pub const PHOSPHOR: &str = "phosphor";

/// Installs the fonts on a context: the icons first, the system fonts where
/// they parse, egui's bundled fonts after them. Call it once per context.
pub fn install(ctx: &egui::Context, system: &Fonts) {
    ctx.set_fonts(definitions(system));
}

/// The font definitions [`install`] sets.
#[must_use]
pub fn definitions(system: &Fonts) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (file, name, family) in [
        (&system.ui, SYSTEM_UI, FontFamily::Proportional),
        (&system.mono, SYSTEM_MONO, FontFamily::Monospace),
    ] {
        let Some(file) = file else {
            continue;
        };
        match data_of(file) {
            Some(data) => {
                fonts.font_data.insert(name.to_owned(), Arc::new(data));
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, name.to_owned());
                tracing::debug!(family = %file.family, index = file.index, "system font");
            }
            None => {
                tracing::warn!(
                    family = %file.family,
                    "the system font did not parse; the bundled font stays"
                );
            }
        }
    }
    let icon = icon_font(egui_phosphor::Variant::Regular.font_bytes());
    fonts
        .font_data
        .insert(PHOSPHOR.to_owned(), Arc::new(FontData::from_owned(icon)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, PHOSPHOR.to_owned());
    }
    fonts
        .families
        .insert(FontFamily::Name(PHOSPHOR.into()), vec![PHOSPHOR.to_owned()]);
    // The fill weight is its own family so a glyph can ask for it; nothing
    // sets that family on text, so its text mappings stay.
    egui_phosphor::add_font_bytes_as_family(
        &mut fonts,
        icons::FILL_FAMILY,
        egui_phosphor::Variant::Fill.font_bytes(),
    );
    fonts
}

/// A font file as egui takes it, once it parses as a font.
fn data_of(file: &FontFile) -> Option<FontData> {
    ab_glyph::FontRef::try_from_slice_and_index(&file.bytes, file.index).ok()?;
    Some(FontData {
        index: file.index,
        ..FontData::from_owned(file.bytes.clone())
    })
}

/// The first code point of the private-use area the icons live in.
const PRIVATE_USE: u32 = 0xE000;
/// Where the text characters go: the surrogate block, which no `char` can
/// name, so a lookup never reaches them; below the private-use area, so the
/// table stays sorted.
const UNREACHABLE: u32 = 0xD800;
/// The size of that block.
const UNREACHABLE_LEN: u32 = 0x800;

/// The icon font with every character below the private-use area moved
/// into the surrogate block, so it maps no text and can sit first in a
/// family. The glyph ids do not change. Works on the `cmap` format 4
/// subtables Phosphor has; a font this cannot patch is returned as it is,
/// with a warning, and then shows its text characters as before.
#[must_use]
pub fn icon_font(source: &[u8]) -> Vec<u8> {
    let mut bytes = source.to_vec();
    match patch_cmap(&mut bytes) {
        Ok(()) => bytes,
        Err(what) => {
            tracing::warn!(
                what,
                "the icon font could not be patched; it keeps its text"
            );
            source.to_vec()
        }
    }
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        *bytes.get(at + 3)?,
    ]))
}

fn put16(bytes: &mut [u8], at: usize, value: u16) -> Option<()> {
    bytes
        .get_mut(at..at + 2)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

/// Moves the text segments of every `cmap` subtable; `Err` names what stood
/// in the way, with the bytes possibly half-patched, so the caller starts
/// over from the source.
fn patch_cmap(bytes: &mut [u8]) -> Result<(), &'static str> {
    let num_tables = be16(bytes, 4).ok_or("short header")? as usize;
    let mut cmap = None;
    for i in 0..num_tables {
        let record = 12 + 16 * i;
        if bytes.get(record..record + 4) == Some(b"cmap") {
            cmap = Some(be32(bytes, record + 8).ok_or("short directory")? as usize);
        }
    }
    let cmap = cmap.ok_or("no cmap table")?;
    let encodings = be16(bytes, cmap + 2).ok_or("short cmap")? as usize;
    let mut done: Vec<usize> = Vec::new();
    for i in 0..encodings {
        let offset = be32(bytes, cmap + 8 + 8 * i).ok_or("short cmap")? as usize;
        let subtable = cmap + offset;
        if done.contains(&subtable) {
            continue;
        }
        done.push(subtable);
        if be16(bytes, subtable).ok_or("short subtable")? != 4 {
            return Err("a cmap subtable format other than 4");
        }
        patch_format4(bytes, subtable)?;
    }
    Ok(())
}

/// Format 4: parallel arrays of segment end codes, start codes, deltas
/// and range offsets. Every segment that ends below the private-use area
/// (the sentinel segment at `0xFFFF` aside) moves into the surrogate
/// block, in order, keeping its length; a segment whose glyphs come from
/// its delta keeps its glyph ids by shifting the delta the other way.
fn patch_format4(bytes: &mut [u8], at: usize) -> Result<(), &'static str> {
    let seg_x2 = be16(bytes, at + 6).ok_or("short format 4")? as usize;
    let segments = seg_x2 / 2;
    let ends = at + 14;
    let starts = at + 16 + seg_x2;
    let deltas = at + 16 + 2 * seg_x2;
    let range_offsets = at + 16 + 3 * seg_x2;
    let mut cursor = UNREACHABLE;
    for k in 0..segments {
        let end = u32::from(be16(bytes, ends + 2 * k).ok_or("short format 4")?);
        let start = u32::from(be16(bytes, starts + 2 * k).ok_or("short format 4")?);
        if end >= PRIVATE_USE || end == 0xFFFF {
            continue;
        }
        let len = end.saturating_sub(start);
        if cursor + len >= UNREACHABLE + UNREACHABLE_LEN {
            return Err("more text than the surrogate block holds");
        }
        let new_start = cursor;
        cursor += len + 1;
        // The segment keeps its length and its glyphs: with a zero range
        // offset the glyph is `code + delta`, so the delta moves by the
        // same amount in the other direction.
        if be16(bytes, range_offsets + 2 * k).ok_or("short format 4")? == 0 {
            let delta = be16(bytes, deltas + 2 * k).ok_or("short format 4")?;
            // The arithmetic is modulo 2^16 by the format's definition.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let shift = (i64::from(start) - i64::from(new_start)) as u16;
            put16(bytes, deltas + 2 * k, delta.wrapping_add(shift)).ok_or("short format 4")?;
        }
        // The code points fit in u16: both are below 0xE000.
        #[allow(clippy::cast_possible_truncation)]
        {
            put16(bytes, starts + 2 * k, new_start as u16).ok_or("short format 4")?;
            put16(bytes, ends + 2 * k, (new_start + len) as u16).ok_or("short format 4")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font as _;

    fn file(bytes: &[u8]) -> FontFile {
        FontFile {
            family: "Test".to_owned(),
            bytes: bytes.to_vec(),
            index: 0,
        }
    }

    #[test]
    fn the_icon_font_keeps_its_icons_and_maps_no_text() {
        let source = egui_phosphor::Variant::Regular.font_bytes();
        let patched = icon_font(source);
        assert_ne!(patched, source, "something moved");
        let before = ab_glyph::FontRef::try_from_slice(source).unwrap_or_else(|e| panic!("{e}"));
        let after = ab_glyph::FontRef::try_from_slice(&patched).unwrap_or_else(|e| panic!("{e}"));
        for text in [' ', '-', 'a', 'm', 'z', 'A', '0'] {
            assert_eq!(after.glyph_id(text).0, 0, "{text:?} is no longer mapped");
        }
        for icon in [
            icons::BACK,
            icons::FOLDER,
            icons::MENU,
            icons::SEARCH,
            icons::TRASH,
            icons::CLOSE,
        ] {
            let c = icon.chars().next().unwrap_or_else(|| panic!("an icon"));
            assert_ne!(after.glyph_id(c).0, 0, "{c:?} still mapped");
            assert_eq!(
                after.glyph_id(c),
                before.glyph_id(c),
                "{c:?} keeps its glyph"
            );
        }
        assert_eq!(after.glyph_count(), before.glyph_count());
    }

    #[test]
    fn a_font_that_cannot_be_patched_is_returned_as_it_is() {
        assert_eq!(icon_font(b"not a font"), b"not a font");
        // A font with no cmap: the directory alone.
        let no_cmap = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(icon_font(&no_cmap), no_cmap);
    }

    #[test]
    fn without_system_fonts_the_icons_lead_and_the_bundled_fonts_follow() {
        let fonts = definitions(&Fonts::default());
        assert_eq!(fonts.families[&FontFamily::Proportional][0], PHOSPHOR);
        assert_eq!(fonts.families[&FontFamily::Proportional][1], "Ubuntu-Light");
        assert_eq!(fonts.families[&FontFamily::Monospace][0], PHOSPHOR);
        assert_eq!(fonts.families[&FontFamily::Monospace][1], "Hack");
        assert!(!fonts.font_data.contains_key(SYSTEM_UI));
        assert!(
            fonts
                .families
                .contains_key(&FontFamily::Name(PHOSPHOR.into()))
        );
        assert!(
            fonts
                .families
                .contains_key(&FontFamily::Name(icons::FILL_FAMILY.into()))
        );
    }

    #[test]
    fn a_system_font_that_parses_goes_right_after_the_icons() {
        // Any real font file does; the fill icon font is one at hand.
        let real = egui_phosphor::Variant::Fill.font_bytes();
        let system = Fonts {
            ui: Some(file(real)),
            mono: Some(file(real)),
        };
        let fonts = definitions(&system);
        let after = |family: FontFamily, name: &str| {
            fonts.families[&family]
                .iter()
                .position(|n| n == name)
                .is_some_and(|i| i > 1)
        };
        assert_eq!(fonts.families[&FontFamily::Proportional][0], PHOSPHOR);
        assert_eq!(fonts.families[&FontFamily::Proportional][1], SYSTEM_UI);
        assert!(
            after(FontFamily::Proportional, "Ubuntu-Light"),
            "the bundled font stays as fallback"
        );
        assert_eq!(fonts.families[&FontFamily::Monospace][0], PHOSPHOR);
        assert_eq!(fonts.families[&FontFamily::Monospace][1], SYSTEM_MONO);
        assert!(after(FontFamily::Monospace, "Hack"));
        assert_eq!(fonts.font_data[SYSTEM_UI].index, 0);
    }

    #[test]
    fn a_file_that_does_not_parse_keeps_the_bundled_font() {
        let system = Fonts {
            ui: Some(file(b"\x00\x01\x00\x00 not a font at all")),
            mono: None,
        };
        let fonts = definitions(&system);
        assert_eq!(fonts.families[&FontFamily::Proportional][1], "Ubuntu-Light");
        assert!(!fonts.font_data.contains_key(SYSTEM_UI));
        // And a context takes them without complaint.
        let ctx = egui::Context::default();
        install(&ctx, &system);
    }
}
