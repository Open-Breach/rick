//! Media fixtures for the preview tests and screenshots: a JPEG with an EXIF
//! block as a camera would write it, and a PDF of any page size.

use std::io::Cursor;

use image::ImageEncoder as _;

/// A `width` × `height` JPEG, left half red and right half blue, with an
/// EXIF block carrying Make, Model, `DateTimeOriginal`, `ExposureTime` and
/// `FNumber`.
///
/// # Panics
///
/// When the image cannot be encoded, which a fixture never is.
#[must_use]
pub fn photo_jpeg(width: u32, height: u32) -> Vec<u8> {
    let rgb = image::RgbImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            image::Rgb([220, 40, 40])
        } else {
            image::Rgb([40, 40, 220])
        }
    });
    let mut out = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let jpeg = out.into_inner();
    let payload = exif_payload();
    let length = u16::try_from(payload.len() + 2).unwrap_or_else(|e| panic!("{e}"));
    let mut with_exif = jpeg[..2].to_vec();
    with_exif.extend_from_slice(&[0xFF, 0xE1]);
    with_exif.extend_from_slice(&length.to_be_bytes());
    with_exif.extend_from_slice(&payload);
    with_exif.extend_from_slice(&jpeg[2..]);
    with_exif
}

/// `Exif\0\0` and a little-endian TIFF: IFD0 with Make, Model and the Exif
/// pointer, then the Exif IFD with the date, exposure and aperture.
fn exif_payload() -> Vec<u8> {
    let len = |bytes: &[u8]| u32::try_from(bytes.len()).unwrap_or_else(|e| panic!("{e}"));
    let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    let make = b"Rick Cameras\0";
    let model = b"RC-1\0";
    let date = b"2026:11:06 12:34:56\0";
    let make_at: u32 = 50;
    let model_at = make_at + len(make);
    let exif_ifd_at = model_at + len(model);
    let date_at = exif_ifd_at + 42;
    let exposure_at = date_at + len(date);
    let fnumber_at = exposure_at + 8;
    let entry = |tiff: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: [u8; 4]| {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&kind.to_le_bytes());
        tiff.extend_from_slice(&count.to_le_bytes());
        tiff.extend_from_slice(&value);
    };
    tiff.extend_from_slice(&3u16.to_le_bytes());
    entry(&mut tiff, 0x010F, 2, len(make), make_at.to_le_bytes());
    entry(&mut tiff, 0x0110, 2, len(model), model_at.to_le_bytes());
    entry(&mut tiff, 0x8769, 4, 1, exif_ifd_at.to_le_bytes());
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(make);
    tiff.extend_from_slice(model);
    tiff.extend_from_slice(&3u16.to_le_bytes());
    entry(&mut tiff, 0x9003, 2, len(date), date_at.to_le_bytes());
    entry(&mut tiff, 0x829A, 5, 1, exposure_at.to_le_bytes());
    entry(&mut tiff, 0x829D, 5, 1, fnumber_at.to_le_bytes());
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(date);
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&250u32.to_le_bytes());
    tiff.extend_from_slice(&28u32.to_le_bytes());
    tiff.extend_from_slice(&10u32.to_le_bytes());
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    payload
}

/// A minimal PDF with `pages` pages of `width` × `height` points, each
/// showing its number in Helvetica with a frame around the page.
#[must_use]
pub fn pdf(pages: usize, width: u32, height: u32) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let first_page_object = 4;
    let kids: Vec<String> = (0..pages)
        .map(|i| format!("{} 0 R", first_page_object + 2 * i))
        .collect();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages
        )
        .into_bytes(),
    );
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for i in 0..pages {
        let content = format!(
            "4 w 20 20 {} {} re S BT /F1 36 Tf 72 {} Td (Page {} of {}) Tj ET",
            width.saturating_sub(40),
            height.saturating_sub(40),
            height.saturating_sub(92),
            i + 1,
            pages
        );
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                first_page_object + 2 * i + 1
            )
            .into_bytes(),
        );
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
        );
    }
    objects.push(b"<< /Title (Rick sample) /Author (Rick) /Producer (Rick fixture) >>".to_vec());
    let info_object = objects.len();
    let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {info_object} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
