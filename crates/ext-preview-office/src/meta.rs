//! What an office file says about itself without LibreOffice: the
//! metadata parts of the zip, read with `roxmltree`.
//!
//! Office Open XML keeps `docProps/core.xml` (title, author, dates) and
//! `docProps/app.xml` (page, word, slide counts, the application), plus
//! the sheet list in `xl/workbook.xml`; OpenDocument keeps everything in
//! `meta.xml`, the counts in `meta:document-statistic`.

use std::io::{Cursor, Read};

use crate::Kind;

/// A metadata part larger than this is not parsed; it is not metadata.
const MAX_PART: u64 = 4 * 1024 * 1024;

/// The facts read from the file: rows for File info, and the count of the
/// kind's unit (pages, sheets or slides) when the file states it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    /// Key and value rows in display order: Title, Author, …
    pub rows: Vec<(String, String)>,
    /// Pages, sheets or slides, as the file states them.
    pub count: Option<usize>,
}

/// Reads one part of the zip as text, if present and small.
fn part<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    if file.size() > MAX_PART {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

/// The text of the first element called `name` (namespace ignored).
fn element_text(doc: &roxmltree::Document<'_>, name: &str) -> Option<String> {
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// `2026-11-07T18:09:12Z` as `2026-11-07 18:09`; anything else as is.
fn short_date(text: &str) -> String {
    if text.len() >= 16 && text.as_bytes().get(10) == Some(&b'T') {
        format!("{} {}", &text[..10], &text[11..16])
    } else {
        text.to_owned()
    }
}

fn push(rows: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        rows.push((key.to_owned(), value));
    }
}

/// Reads the metadata of `bytes`, an office file of `kind`.
#[must_use]
pub fn read(bytes: &[u8], kind: Kind) -> Meta {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return Meta::default();
    };
    if kind.is_opendocument() {
        read_odf(&mut archive, kind)
    } else {
        read_ooxml(&mut archive, kind)
    }
}

fn read_ooxml<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, kind: Kind) -> Meta {
    let mut meta = Meta::default();
    if let Some(core) = part(archive, "docProps/core.xml")
        && let Ok(doc) = roxmltree::Document::parse(&core)
    {
        push(&mut meta.rows, "Title", element_text(&doc, "title"));
        push(&mut meta.rows, "Subject", element_text(&doc, "subject"));
        push(&mut meta.rows, "Author", element_text(&doc, "creator"));
        push(
            &mut meta.rows,
            "Last modified by",
            element_text(&doc, "lastModifiedBy"),
        );
        push(
            &mut meta.rows,
            "Created",
            element_text(&doc, "created").map(|d| short_date(&d)),
        );
        push(
            &mut meta.rows,
            "Modified",
            element_text(&doc, "modified").map(|d| short_date(&d)),
        );
        push(&mut meta.rows, "Keywords", element_text(&doc, "keywords"));
    }
    if let Some(app) = part(archive, "docProps/app.xml")
        && let Ok(doc) = roxmltree::Document::parse(&app)
    {
        let count_of = |name: &str| element_text(&doc, name).and_then(|t| t.parse::<usize>().ok());
        meta.count = match kind {
            Kind::Docx => count_of("Pages"),
            Kind::Pptx => count_of("Slides"),
            _ => None,
        };
        if kind == Kind::Docx {
            push(
                &mut meta.rows,
                "Words",
                count_of("Words").map(|n| n.to_string()),
            );
        }
        push(
            &mut meta.rows,
            "Application",
            element_text(&doc, "Application"),
        );
    }
    if kind == Kind::Xlsx
        && let Some(workbook) = part(archive, "xl/workbook.xml")
        && let Ok(doc) = roxmltree::Document::parse(&workbook)
    {
        let sheets: Vec<String> = doc
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "sheet")
            .filter_map(|n| n.attribute("name"))
            .map(str::to_owned)
            .collect();
        if !sheets.is_empty() {
            meta.count = Some(sheets.len());
            let shown: Vec<&str> = sheets.iter().take(8).map(String::as_str).collect();
            let mut list = shown.join(", ");
            if sheets.len() > shown.len() {
                list.push_str(", …");
            }
            meta.rows.push(("Sheets".to_owned(), list));
        }
    }
    if kind == Kind::Pptx && meta.count.is_none() {
        let slides = archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|n| {
                n.starts_with("ppt/slides/slide") && n.ends_with(".xml") && !n.contains('/')
                    || (n.starts_with("ppt/slides/slide")
                        && n.ends_with(".xml")
                        && n.matches('/').count() == 2)
            })
            .count();
        if slides > 0 {
            meta.count = Some(slides);
        }
    }
    meta
}

fn read_odf<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>, kind: Kind) -> Meta {
    let mut meta = Meta::default();
    let Some(text) = part(archive, "meta.xml") else {
        return meta;
    };
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return meta;
    };
    push(&mut meta.rows, "Title", element_text(&doc, "title"));
    push(&mut meta.rows, "Subject", element_text(&doc, "subject"));
    push(
        &mut meta.rows,
        "Author",
        element_text(&doc, "initial-creator"),
    );
    push(
        &mut meta.rows,
        "Last modified by",
        element_text(&doc, "creator"),
    );
    push(
        &mut meta.rows,
        "Created",
        element_text(&doc, "creation-date").map(|d| short_date(&d)),
    );
    push(
        &mut meta.rows,
        "Modified",
        element_text(&doc, "date").map(|d| short_date(&d)),
    );
    if let Some(stats) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "document-statistic")
    {
        let attr = |name: &str| {
            stats
                .attributes()
                .find(|a| a.name() == name)
                .and_then(|a| a.value().parse::<usize>().ok())
        };
        meta.count = match kind {
            Kind::Odt | Kind::Odp => attr("page-count"),
            Kind::Ods => attr("table-count"),
            _ => None,
        };
        if kind == Kind::Odt {
            push(
                &mut meta.rows,
                "Words",
                attr("word-count").map(|n| n.to_string()),
            );
        }
    }
    push(
        &mut meta.rows,
        "Application",
        element_text(&doc, "generator"),
    );
    meta
}
