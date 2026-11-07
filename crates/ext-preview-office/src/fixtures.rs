//! Minimal office documents for tests and fixtures, written with the zip
//! crate: one of each of the six kinds, with a title and author and a
//! little content. LibreOffice opens and converts them.

use std::fmt::Write as _;
use std::io::{Cursor, Write};

use crate::Kind;

/// Escapes text for an XML text node or attribute.
fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Writes `parts` as a zip; the first part is stored (OpenDocument wants
/// `mimetype` first and uncompressed), the rest deflated.
fn zip_parts(parts: &[(&str, String)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (index, (name, body)) in parts.iter().enumerate() {
        let method = if index == 0 && *name == "mimetype" {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        writer
            .start_file(*name, options)
            .unwrap_or_else(|e| panic!("{e}"));
        writer
            .write_all(body.as_bytes())
            .unwrap_or_else(|e| panic!("{e}"));
    }
    writer
        .finish()
        .unwrap_or_else(|e| panic!("{e}"))
        .into_inner()
}

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn ooxml_core(title: &str, author: &str) -> String {
    format!(
        "{XML_HEAD}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>{}</dc:creator><cp:lastModifiedBy>{}</cp:lastModifiedBy><dcterms:created xsi:type=\"dcterms:W3CDTF\">2026-11-07T10:00:00Z</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">2026-11-07T11:30:00Z</dcterms:modified></cp:coreProperties>",
        esc(title),
        esc(author),
        esc(author)
    )
}

fn ooxml_rels(main: &str) -> String {
    format!(
        "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"{main}\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>"
    )
}

/// A Word document of `pages` pages, each with a paragraph.
#[must_use]
pub fn sample_docx(title: &str, author: &str, pages: usize) -> Vec<u8> {
    let mut body = String::new();
    for page in 1..=pages {
        if page > 1 {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        let _ = write!(
            body,
            "<w:p><w:r><w:t>{} — page {page} of {pages}</w:t></w:r></w:p>",
            esc(title)
        );
    }
    let document = format!(
        "{XML_HEAD}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/></w:sectPr></w:body></w:document>"
    );
    let app = format!(
        "{XML_HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Rick fixture</Application><Pages>{pages}</Pages><Words>{}</Words></Properties>",
        pages * 6
    );
    let types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"{}+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>",
        Kind::Docx
            .media_type()
            .replace(".document", ".document.main")
    );
    zip_parts(&[
        ("[Content_Types].xml", types),
        ("_rels/.rels", ooxml_rels("word/document.xml")),
        ("word/document.xml", document),
        ("docProps/core.xml", ooxml_core(title, author)),
        ("docProps/app.xml", app),
    ])
}

/// An Excel workbook with the named sheets, each holding one cell.
#[must_use]
pub fn sample_xlsx(title: &str, author: &str, sheets: &[&str]) -> Vec<u8> {
    let mut parts: Vec<(&str, String)> = Vec::new();
    let mut overrides = String::new();
    let mut sheet_list = String::new();
    let mut rels = String::new();
    let mut owned_names: Vec<String> = Vec::new();
    for (i, name) in sheets.iter().enumerate() {
        let n = i + 1;
        let _ = write!(
            overrides,
            "<Override PartName=\"/xl/worksheets/sheet{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        );
        let _ = write!(
            sheet_list,
            "<sheet name=\"{}\" sheetId=\"{n}\" r:id=\"rId{n}\"/>",
            esc(name)
        );
        let _ = write!(
            rels,
            "<Relationship Id=\"rId{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        );
        owned_names.push(format!("xl/worksheets/sheet{n}.xml"));
    }
    let types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"{}.main+xml\"/>{overrides}<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>",
        Kind::Xlsx.media_type()
    );
    let workbook = format!(
        "{XML_HEAD}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>{sheet_list}</sheets></workbook>"
    );
    let workbook_rels = format!(
        "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>"
    );
    let app = format!(
        "{XML_HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Rick fixture</Application></Properties>"
    );
    parts.push(("[Content_Types].xml", types));
    parts.push(("_rels/.rels", ooxml_rels("xl/workbook.xml")));
    parts.push(("xl/workbook.xml", workbook));
    parts.push(("xl/_rels/workbook.xml.rels", workbook_rels));
    parts.push(("docProps/core.xml", ooxml_core(title, author)));
    parts.push(("docProps/app.xml", app));
    let sheet_bodies: Vec<String> = sheets
        .iter()
        .map(|name| {
            format!(
                "{XML_HEAD}<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>{}</t></is></c><c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>",
                esc(name)
            )
        })
        .collect();
    for (name, body) in owned_names.iter().zip(sheet_bodies) {
        parts.push((name.as_str(), body));
    }
    zip_parts(&parts)
}

/// A `PowerPoint` presentation of `slides` slides, each with a title box.
#[must_use]
pub fn sample_pptx(title: &str, author: &str, slides: usize) -> Vec<u8> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut overrides = String::new();
    let mut slide_ids = String::new();
    let mut pres_rels = String::from(
        "<Relationship Id=\"rIdM\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>",
    );
    for n in 1..=slides {
        let _ = write!(
            overrides,
            "<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        );
        let _ = write!(slide_ids, "<p:sldId id=\"{}\" r:id=\"rId{n}\"/>", 255 + n);
        let _ = write!(
            pres_rels,
            "<Relationship Id=\"rId{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{n}.xml\"/>"
        );
        let slide = format!(
            "{XML_HEAD}<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"457200\" y=\"457200\"/><a:ext cx=\"8229600\" cy=\"1143000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"3200\"/><a:t>{} — slide {n} of {slides}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
            esc(title)
        );
        parts.push((format!("ppt/slides/slide{n}.xml"), slide));
        parts.push((
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            format!(
                "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/></Relationships>"
            ),
        ));
    }
    let types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"{}.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>{overrides}<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>",
        Kind::Pptx.media_type()
    );
    let presentation = format!(
        "{XML_HEAD}<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdM\"/></p:sldMasterIdLst><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx=\"9144000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );
    let empty_tree = "<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>";
    let master = format!(
        "{XML_HEAD}<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">{empty_tree}<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    );
    let layout = format!(
        "{XML_HEAD}<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\">{empty_tree}<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    );
    let font = |name: &str| {
        format!(
            "<a:{name}><a:latin typeface=\"Liberation Sans\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{name}>"
        )
    };
    let theme = format!(
        "{XML_HEAD}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Rick\"><a:themeElements><a:clrScheme name=\"Rick\"><a:dk1><a:srgbClr val=\"000000\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"1F497D\"/></a:dk2><a:lt2><a:srgbClr val=\"EEECE1\"/></a:lt2><a:accent1><a:srgbClr val=\"4F81BD\"/></a:accent1><a:accent2><a:srgbClr val=\"C0504D\"/></a:accent2><a:accent3><a:srgbClr val=\"9BBB59\"/></a:accent3><a:accent4><a:srgbClr val=\"8064A2\"/></a:accent4><a:accent5><a:srgbClr val=\"4BACC6\"/></a:accent5><a:accent6><a:srgbClr val=\"F79646\"/></a:accent6><a:hlink><a:srgbClr val=\"0000FF\"/></a:hlink><a:folHlink><a:srgbClr val=\"800080\"/></a:folHlink></a:clrScheme><a:fontScheme name=\"Rick\">{}{}</a:fontScheme><a:fmtScheme name=\"Rick\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"25400\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln><a:ln w=\"38100\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>",
        font("majorFont"),
        font("minorFont")
    );
    let app = format!(
        "{XML_HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>Rick fixture</Application><Slides>{slides}</Slides></Properties>"
    );
    let mut all: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", types),
        ("_rels/.rels", ooxml_rels("ppt/presentation.xml")),
        ("ppt/presentation.xml", presentation),
        (
            "ppt/_rels/presentation.xml.rels",
            format!(
                "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{pres_rels}</Relationships>"
            ),
        ),
        ("ppt/slideMasters/slideMaster1.xml", master),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            format!(
                "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>"
            ),
        ),
        ("ppt/slideLayouts/slideLayout1.xml", layout),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            format!(
                "{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>"
            ),
        ),
        ("ppt/theme/theme1.xml", theme),
        ("docProps/core.xml", ooxml_core(title, author)),
        ("docProps/app.xml", app),
    ];
    for (name, body) in &parts {
        all.push((name.as_str(), body.clone()));
    }
    zip_parts(&all)
}

fn odf(kind: Kind, title: &str, author: &str, body: &str, statistic: &str) -> Vec<u8> {
    let ns = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" office:version=\"1.3\"";
    // `Break` is the paragraph style the text fixture starts a new page with;
    // a style a paragraph names has to exist, or LibreOffice ignores it.
    let styles = "<office:automatic-styles><style:style style:name=\"Break\" style:family=\"paragraph\"><style:paragraph-properties fo:break-before=\"page\"/></style:style></office:automatic-styles>";
    let content = format!(
        "{XML_HEAD}<office:document-content {ns}>{styles}<office:body>{body}</office:body></office:document-content>"
    );
    let meta = format!(
        "{XML_HEAD}<office:document-meta {ns}><office:meta><dc:title>{}</dc:title><meta:initial-creator>{}</meta:initial-creator><dc:creator>{}</dc:creator><meta:creation-date>2026-11-07T10:00:00</meta:creation-date><dc:date>2026-11-07T11:30:00</dc:date><meta:generator>Rick fixture</meta:generator><meta:document-statistic {statistic}/></office:meta></office:document-meta>",
        esc(title),
        esc(author),
        esc(author)
    );
    let manifest = format!(
        "{XML_HEAD}<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"{}\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>",
        kind.media_type()
    );
    zip_parts(&[
        ("mimetype", kind.media_type().to_owned()),
        ("META-INF/manifest.xml", manifest),
        ("content.xml", content),
        ("meta.xml", meta),
    ])
}

/// An OpenDocument text of `pages` pages.
#[must_use]
pub fn sample_odt(title: &str, author: &str, pages: usize) -> Vec<u8> {
    let mut body = String::from("<office:text>");
    for page in 1..=pages {
        let _ = write!(
            body,
            "<text:p>{} — page {page} of {pages}</text:p>",
            esc(title)
        );
        if page < pages {
            body.push_str("<text:p text:style-name=\"Break\"/>");
        }
    }
    body.push_str("</office:text>");
    let statistic = format!(
        "meta:page-count=\"{pages}\" meta:word-count=\"{}\"",
        pages * 6
    );
    odf(Kind::Odt, title, author, &body, &statistic)
}

/// An OpenDocument spreadsheet with the named sheets.
#[must_use]
pub fn sample_ods(title: &str, author: &str, sheets: &[&str]) -> Vec<u8> {
    let mut body = String::from("<office:spreadsheet>");
    for name in sheets {
        let _ = write!(
            body,
            "<table:table table:name=\"{}\"><table:table-column/><table:table-row><table:table-cell office:value-type=\"string\"><text:p>{}</text:p></table:table-cell></table:table-row></table:table>",
            esc(name),
            esc(name)
        );
    }
    body.push_str("</office:spreadsheet>");
    let statistic = format!(
        "meta:table-count=\"{}\" meta:cell-count=\"{}\"",
        sheets.len(),
        sheets.len()
    );
    odf(Kind::Ods, title, author, &body, &statistic)
}

/// An OpenDocument presentation of `slides` slides.
#[must_use]
pub fn sample_odp(title: &str, author: &str, slides: usize) -> Vec<u8> {
    let mut body = String::from("<office:presentation>");
    for n in 1..=slides {
        let _ = write!(
            body,
            "<draw:page draw:name=\"page{n}\"><draw:frame svg:width=\"20cm\" svg:height=\"3cm\" svg:x=\"2cm\" svg:y=\"2cm\"><draw:text-box><text:p>{} — slide {n} of {slides}</text:p></draw:text-box></draw:frame></draw:page>",
            esc(title)
        );
    }
    body.push_str("</office:presentation>");
    let statistic = format!("meta:page-count=\"{slides}\"");
    odf(Kind::Odp, title, author, &body, &statistic)
}

/// A sample of `kind` with `n` pages, sheets or slides.
#[must_use]
pub fn sample(kind: Kind, title: &str, author: &str, n: usize) -> Vec<u8> {
    let names: Vec<String> = (1..=n).map(|i| format!("Sheet {i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match kind {
        Kind::Docx => sample_docx(title, author, n),
        Kind::Xlsx => sample_xlsx(title, author, &names),
        Kind::Pptx => sample_pptx(title, author, n),
        Kind::Odt => sample_odt(title, author, n),
        Kind::Ods => sample_ods(title, author, &names),
        Kind::Odp => sample_odp(title, author, n),
    }
}
