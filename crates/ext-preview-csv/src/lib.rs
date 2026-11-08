//! CSV and TSV previews: a grid with a header row over a window of the
//! file, the delimiter sniffed from the first lines, the encoding detected.
//!
//! `open` reads at most [`WINDOW`] bytes, decodes them (BOMs, then UTF-8,
//! then Windows-1252), sniffs the delimiter, decides whether the first row
//! is a header and indexes where every row starts, quotes respected, so
//! that [`Grid::cells`] parses only the rows a pane asks for: a million
//! rows cost the index once and the rows on screen after. `render` returns
//! the raw text as the fallback for a pane that lays out no grids; the
//! grid itself comes through [`PreviewDoc::content`].

#![deny(unsafe_code)]

use std::ops::Range;
use std::sync::Arc;

use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Extension, Frame, Grid, Icon,
    InfoSection, Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider,
    Registry, RenderRequest, TextBlock, TextLayer, Transport, Version, async_trait,
};
use types::{CancelToken, Entry, Error, ExtId, Location};

/// The extension id.
pub const ID: ExtId = ExtId("preview-csv");

/// How much of a file is read and indexed: larger files show their top
/// and say so.
pub const WINDOW: usize = 256 * 1024 * 1024;

/// How much of the raw text `render` hands a pane that shows no grids.
pub const FALLBACK_TEXT: usize = 1024 * 1024;

/// How many lines the delimiter is sniffed from.
const SNIFF_LINES: usize = 20;

/// Extensions this previewer claims.
const EXTENSIONS: &[&str] = &["csv", "tsv", "tab"];

/// The extension: registers [`CsvPreviewer`].
#[derive(Debug)]
pub struct CsvPreview {
    manifest: Manifest,
}

impl Default for CsvPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl CsvPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "CSV preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
            },
        }
    }
}

impl Extension for CsvPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.previewer(Arc::new(CsvPreviewer));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(CsvPreview::new()) as Arc<dyn Extension>);

/// Previews delimited text files.
#[derive(Debug, Default)]
pub struct CsvPreviewer;

/// The delimiters sniffed for, in the order that breaks ties.
const CANDIDATES: &[u8; 4] = b",;\t|";

/// The name of a delimiter for the status bar.
#[must_use]
pub fn delimiter_name(delimiter: u8) -> &'static str {
    match delimiter {
        b',' => "comma",
        b';' => "semicolon",
        b'\t' => "tab",
        b'|' => "pipe",
        _ => "other",
    }
}

/// The lower-case extension of a name, if any.
fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Decodes bytes to text: BOM-marked UTF-8 and UTF-16, then UTF-8, then
/// Windows-1252. A multi-byte sequence cut at the end counts as UTF-8 only
/// when the window was `truncated`. Returns the text and the encoding's
/// name.
#[must_use]
pub fn decode(bytes: &[u8], truncated: bool) -> (String, &'static str) {
    if let Some((encoding, bom_len)) = encoding_rs::Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(&bytes[bom_len..]);
        return (text.into_owned(), encoding.name());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), "UTF-8"),
        Err(error)
            if truncated
                && bytes.len() - error.valid_up_to() <= 3
                && error.error_len().is_none() =>
        {
            let (text, _) = bytes.split_at(error.valid_up_to());
            (String::from_utf8_lossy(text).into_owned(), "UTF-8")
        }
        Err(_) => {
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            (text.into_owned(), "windows-1252")
        }
    }
}

/// Where each record of `text` starts, quotes respected: a newline inside
/// a `"`-quoted field does not end the record. The last record needs no
/// line end; an empty trailing line is not a record.
#[must_use]
pub fn index_rows(text: &[u8]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut at = 0;
    let mut in_quotes = false;
    let mut record_start = Some(0);
    while at < text.len() {
        if in_quotes {
            match memchr::memchr(b'"', &text[at..]) {
                Some(found) => {
                    at += found + 1;
                    in_quotes = false;
                }
                None => break,
            }
            continue;
        }
        match memchr::memchr2(b'"', b'\n', &text[at..]) {
            Some(found) => {
                let here = at + found;
                if text[here] == b'"' {
                    in_quotes = true;
                } else {
                    if let Some(start) = record_start.take() {
                        starts.push(start);
                    }
                    record_start = Some(here + 1);
                }
                at = here + 1;
            }
            None => break,
        }
    }
    if let Some(start) = record_start
        && start < text.len()
    {
        starts.push(start);
    }
    starts
}

/// Counts `delimiter` in `line` outside `"` quotes.
fn count_outside_quotes(line: &[u8], delimiter: u8) -> usize {
    let mut count = 0;
    let mut in_quotes = false;
    for &b in line {
        if b == b'"' {
            in_quotes = !in_quotes;
        } else if b == delimiter && !in_quotes {
            count += 1;
        }
    }
    count
}

/// The delimiter of `text`, from its first records: the candidate that
/// appears the same number of times (more than zero) on every sampled
/// record wins, the most frequent among those; when none is consistent,
/// the most frequent on the first record; a tab for `tsv` names and a
/// comma otherwise when nothing appears at all.
#[must_use]
pub fn sniff_delimiter(text: &[u8], starts: &[usize], name: &str) -> u8 {
    let sample: Vec<&[u8]> = starts
        .iter()
        .take(SNIFF_LINES)
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(text.len());
            &text[start..end]
        })
        .collect();
    let by_name = match extension(name).as_deref() {
        Some("tsv" | "tab") => b'\t',
        _ => b',',
    };
    let Some(first) = sample.first() else {
        return by_name;
    };
    let mut consistent: Option<(usize, u8)> = None;
    let mut first_best: Option<(usize, u8)> = None;
    for &candidate in CANDIDATES {
        let counts: Vec<usize> = sample
            .iter()
            .map(|line| count_outside_quotes(line, candidate))
            .collect();
        let on_first = count_outside_quotes(first, candidate);
        if on_first > 0 && first_best.is_none_or(|(best, _)| on_first > best) {
            first_best = Some((on_first, candidate));
        }
        if on_first > 0
            && counts.iter().all(|&c| c == on_first)
            && consistent.is_none_or(|(best, _)| on_first > best)
        {
            consistent = Some((on_first, candidate));
        }
    }
    consistent
        .or(first_best)
        .map_or(by_name, |(_, delimiter)| delimiter)
}

/// Parses the records in `text` with `delimiter`, as many cells as each
/// has.
fn parse_rows(text: &[u8], delimiter: u8) -> Vec<Vec<String>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text);
    reader
        .records()
        .filter_map(Result::ok)
        .map(|record| record.iter().map(str::to_owned).collect())
        .collect()
}

/// Whether a cell reads as a number: an optional sign, digits with one
/// separator at most, or empty.
fn looks_numeric(cell: &str) -> bool {
    let cell = cell.trim();
    if cell.is_empty() {
        return true;
    }
    let digits = cell.trim_start_matches(['-', '+']);
    !digits.is_empty()
        && digits
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' ' | '\'' | '_'))
        && digits.chars().any(|c| c.is_ascii_digit())
}

/// Whether the first record is a header: it is, unless every cell of it
/// reads as a number (then the data starts on row one and the columns are
/// numbered).
#[must_use]
pub fn first_row_is_header(first: &[String]) -> bool {
    !first.is_empty() && !first.iter().all(|cell| looks_numeric(cell))
}

/// The indexed window: the text, where its rows start and how to split
/// them. Shared by the document and the grid handed to the pane.
#[derive(Debug)]
struct Table {
    text: Arc<str>,
    /// Byte offset of every record, the header included when there is one.
    starts: Vec<usize>,
    delimiter: u8,
    header: Vec<String>,
    /// `true` when `starts[0]` is the header row.
    has_header: bool,
}

impl Table {
    fn data_rows(&self) -> usize {
        self.starts.len() - usize::from(self.has_header)
    }

    /// The byte range of the data rows `range`.
    fn span(&self, range: Range<usize>) -> Option<Range<usize>> {
        let offset = usize::from(self.has_header);
        let first = range.start + offset;
        let past = range.end.min(self.data_rows()) + offset;
        if first >= past {
            return None;
        }
        let start = *self.starts.get(first)?;
        let end = self.starts.get(past).copied().unwrap_or(self.text.len());
        Some(start..end)
    }
}

impl Grid for Table {
    fn header(&self) -> &[String] {
        &self.header
    }

    fn rows(&self) -> usize {
        self.data_rows()
    }

    fn cells(&self, range: Range<usize>) -> Vec<Vec<String>> {
        let Some(span) = self.span(range.clone()) else {
            return Vec::new();
        };
        let mut rows = parse_rows(&self.text.as_bytes()[span], self.delimiter);
        // A record the parser skipped (an empty line) leaves the window
        // short; the caller asked for `range`, so pad rather than shift.
        let wanted = range.end.min(self.data_rows()).saturating_sub(range.start);
        rows.truncate(wanted);
        while rows.len() < wanted {
            rows.push(Vec::new());
        }
        rows
    }
}

/// An opened delimited file.
pub struct CsvDoc {
    table: Arc<Table>,
    layer: TextLayer,
    encoding: &'static str,
    size: Option<u64>,
    bytes_read: usize,
    truncated: bool,
}

impl std::fmt::Debug for CsvDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvDoc")
            .field("rows", &self.table.data_rows())
            .field("columns", &self.table.header.len())
            .field("delimiter", &delimiter_name(self.table.delimiter))
            .finish_non_exhaustive()
    }
}

impl CsvDoc {
    /// Builds the document from the bytes read for `name`, noting whether
    /// the window cut the file short; `size` is the file's size when known.
    #[must_use]
    pub fn new(name: &str, bytes: &[u8], truncated: bool, size: Option<u64>) -> Self {
        let (text, encoding) = decode(bytes, truncated);
        let mut starts = index_rows(text.as_bytes());
        if truncated {
            // The last record may be cut mid-way: keep whole ones only.
            starts.pop();
        }
        let delimiter = sniff_delimiter(text.as_bytes(), &starts, name);
        let first: Vec<String> = starts
            .first()
            .map(|&start| {
                let end = starts.get(1).copied().unwrap_or(text.len());
                parse_rows(&text.as_bytes()[start..end], delimiter)
                    .into_iter()
                    .next()
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let has_header = first_row_is_header(&first);
        let header = if has_header {
            first
        } else {
            (1..=first.len()).map(|n| format!("Column {n}")).collect()
        };
        let fallback_end = text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= FALLBACK_TEXT)
            .last()
            .unwrap_or(0)
            .min(text.len());
        let fallback = text[..fallback_end].to_owned();
        Self {
            table: Arc::new(Table {
                text: text.into(),
                starts,
                delimiter,
                header,
                has_header,
            }),
            layer: TextLayer {
                pages: vec![fallback],
            },
            encoding,
            size,
            bytes_read: bytes.len(),
            truncated,
        }
    }

    /// The grid: header, row count and cells on demand.
    #[must_use]
    pub fn grid(&self) -> Arc<dyn Grid> {
        Arc::clone(&self.table) as Arc<dyn Grid>
    }

    /// The delimiter sniffed.
    #[must_use]
    pub fn delimiter(&self) -> u8 {
        self.table.delimiter
    }

    /// Whether the first row was taken as the header.
    #[must_use]
    pub fn has_header(&self) -> bool {
        self.table.has_header
    }
}

/// A byte count as `n B`, `n.n KiB` or `n.n MiB`.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    // Sizes shown to one decimal; the precision loss is immaterial.
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    }
}

impl PreviewDoc for CsvDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        if page != 0 {
            return Err(Error::NotFound);
        }
        Ok(Frame::Text(TextBlock {
            text: self.layer.pages[0].clone(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        Some(&self.layer)
    }

    fn content(&self) -> Option<Content> {
        Some(Content::Grid(self.grid()))
    }

    fn sections(&self) -> Vec<InfoSection> {
        let size = self.size.unwrap_or(self.bytes_read as u64);
        let mut rows = vec![
            ("Size".to_owned(), format_size(size)),
            ("Encoding".to_owned(), self.encoding.to_owned()),
            (
                "Delimiter".to_owned(),
                delimiter_name(self.table.delimiter).to_owned(),
            ),
            ("Rows".to_owned(), self.table.data_rows().to_string()),
            ("Columns".to_owned(), self.table.header.len().to_string()),
            (
                "Header row".to_owned(),
                if self.table.has_header { "Yes" } else { "No" }.to_owned(),
            ),
        ];
        if self.truncated {
            rows.push((
                "Shown".to_owned(),
                format!("first {} MiB", self.bytes_read / (1024 * 1024)),
            ));
        }
        vec![InfoSection {
            id: "file",
            title: "File info",
            icon: Icon::Info,
            rows,
        }]
    }

    fn status(&self) -> Vec<String> {
        vec![
            format!(
                "{}{} rows × {} columns",
                self.table.data_rows(),
                if self.truncated { "+" } else { "" },
                self.table.header.len()
            ),
            delimiter_name(self.table.delimiter).to_owned(),
            self.encoding.to_owned(),
        ]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported(
            "CSV files have no controls of their own",
        ))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

#[async_trait]
impl Previewer for CsvPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        if entry.kind != types::EntryKind::File {
            return 0;
        }
        let named = extension(&entry.name).is_some_and(|ext| EXTENSIONS.contains(&ext.as_str()));
        if named && !sniff.contains(&0) { 150 } else { 0 }
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        let size = provider.stat(loc).await.ok().and_then(|e| e.size);
        let bytes = ext::read_bytes(&*provider, loc, Some(WINDOW + 1), &cancel).await?;
        let truncated = bytes.len() > WINDOW;
        let name = loc
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        // Indexing a quarter gigabyte is CPU work: off the runtime's
        // worker threads.
        let doc = ext::blocking(move || {
            let shown = &bytes[..bytes.len().min(WINDOW)];
            Ok(CsvDoc::new(&name, shown, truncated, size))
        })
        .await?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(Box::new(doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{Attrs, EntryId, EntryKind, ProviderId};

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

    #[test]
    fn scores_by_name_and_text_only() {
        let p = CsvPreviewer;
        assert_eq!(p.score(&entry("data.csv"), b"a,b\n1,2\n"), 150);
        assert_eq!(p.score(&entry("data.TSV"), b"a\tb\n"), 150);
        assert_eq!(p.score(&entry("data.csv"), b"\x00\x01"), 0);
        assert_eq!(p.score(&entry("data.txt"), b"a,b\n1,2\n"), 0);
        let mut dir = entry("x.csv");
        dir.kind = EntryKind::Dir;
        assert_eq!(p.score(&dir, b""), 0);
    }

    #[test]
    fn rows_are_indexed_with_quotes_respected() {
        let text = b"a,b\n1,\"two\nlines\"\n3,4\n";
        assert_eq!(index_rows(text), vec![0, 4, 18]);
        assert_eq!(index_rows(b"a,b\n1,2"), vec![0, 4]);
        assert_eq!(index_rows(b""), Vec::<usize>::new());
        assert_eq!(index_rows(b"\n\n"), vec![0, 1]);
        // An escaped quote inside a quoted field: two quotes toggle twice.
        assert_eq!(index_rows(b"\"say \"\"hi\"\"\",x\ny,z\n"), vec![0, 15]);
    }

    #[test]
    fn the_delimiter_is_the_consistent_one() {
        let comma = b"a,b,c\n1,2,3\n4,5,6\n";
        assert_eq!(sniff_delimiter(comma, &index_rows(comma), "x.csv"), b',');
        let semicolon = b"a;b;c\n1,5;2;3\n4;5;6\n";
        assert_eq!(
            sniff_delimiter(semicolon, &index_rows(semicolon), "x.csv"),
            b';'
        );
        let tab = b"a\tb\n1\t2\n";
        assert_eq!(sniff_delimiter(tab, &index_rows(tab), "x.csv"), b'\t');
        let quoted = b"\"a,b\";c\n\"1,2\";3\n";
        assert_eq!(sniff_delimiter(quoted, &index_rows(quoted), "x.csv"), b';');
        let pipe = b"a|b\n1|2\n";
        assert_eq!(sniff_delimiter(pipe, &index_rows(pipe), "x.csv"), b'|');
        assert_eq!(sniff_delimiter(b"", &[], "x.tsv"), b'\t');
        assert_eq!(sniff_delimiter(b"", &[], "x.csv"), b',');
        let single = b"just one column\nand another\n";
        assert_eq!(sniff_delimiter(single, &index_rows(single), "x.csv"), b',');
        // Inconsistent counts: the most frequent on the first line.
        let ragged = b"a,b,c\n1,2\n";
        assert_eq!(sniff_delimiter(ragged, &index_rows(ragged), "x.csv"), b',');
    }

    #[test]
    fn the_first_row_is_the_header_unless_it_is_numbers() {
        assert!(first_row_is_header(&["name".into(), "age".into()]));
        assert!(!first_row_is_header(&[
            "1".into(),
            "2.5".into(),
            "-3".into()
        ]));
        assert!(!first_row_is_header(&["1 000".into(), String::new()]));
        assert!(!first_row_is_header(&[]));
        assert!(first_row_is_header(&["id".into(), "2".into()]));
    }

    #[test]
    fn the_grid_answers_any_window_of_rows() {
        let mut text = String::from("id;name;score\n");
        for i in 0..1000 {
            use std::fmt::Write as _;
            let _ = writeln!(text, "{i};\"name {i}\";{}", i * 2);
        }
        let doc = CsvDoc::new("big.csv", text.as_bytes(), false, None);
        let grid = doc.grid();
        assert_eq!(grid.header(), ["id", "name", "score"]);
        assert_eq!(grid.rows(), 1000);
        assert_eq!(doc.delimiter(), b';');
        assert!(doc.has_header());
        let window = grid.cells(500..503);
        assert_eq!(
            window,
            vec![
                vec!["500", "name 500", "1000"],
                vec!["501", "name 501", "1002"],
                vec!["502", "name 502", "1004"],
            ]
        );
        assert_eq!(grid.cells(999..1005).len(), 1);
        assert_eq!(grid.cells(0..1)[0][0], "0");
        assert_eq!(grid.cells(1000..1001), [] as [Vec<String>; 0]);
        assert_eq!(grid.cells(5..5), [] as [Vec<String>; 0]);
        assert_eq!(doc.status()[0], "1000 rows × 3 columns");
        assert_eq!(doc.status()[1], "semicolon");
        assert_eq!(doc.status()[2], "UTF-8");
        assert!(matches!(doc.content(), Some(Content::Grid(_))));
    }

    #[test]
    fn numeric_first_rows_get_numbered_columns() {
        let doc = CsvDoc::new("n.csv", b"1,2,3\n4,5,6\n", false, None);
        let grid = doc.grid();
        assert_eq!(grid.header(), ["Column 1", "Column 2", "Column 3"]);
        assert_eq!(grid.rows(), 2);
        assert_eq!(
            grid.cells(0..2),
            vec![vec!["1", "2", "3"], vec!["4", "5", "6"]]
        );
        assert!(!doc.has_header());
        let rows = doc.sections().remove(0).rows;
        assert!(rows.iter().any(|(k, v)| k == "Header row" && v == "No"));
    }

    #[test]
    fn multi_line_fields_and_encodings_survive() {
        let mut doc = CsvDoc::new(
            "q.csv",
            b"a,b\n1,\"two\nlines\"\n\"caf\xE9\",4\n",
            false,
            Some(40),
        );
        let grid = doc.grid();
        assert_eq!(grid.rows(), 2);
        assert_eq!(
            grid.cells(0..2),
            vec![vec!["1", "two\nlines"], vec!["café", "4"]]
        );
        assert_eq!(doc.status()[2], "windows-1252");
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
        assert!(block.text.starts_with("a,b\n"), "{:?}", block.text);
        assert!(block.text.contains("café"));
        assert!(doc.text_layer().is_some());
        assert!(
            doc.render(
                1,
                RenderRequest {
                    width: 1,
                    height: 1,
                    scale: 1.0,
                    dark: false
                }
            )
            .is_err()
        );
        assert_eq!(doc.controls().len(), 0);
        assert!(
            doc.set_control(ControlId("x"), ControlValue::Pressed)
                .is_err()
        );
        assert!(doc.transport().is_none());
    }

    #[test]
    fn a_truncated_window_drops_its_cut_last_row_and_says_so() {
        let doc = CsvDoc::new("t.csv", b"a,b\n1,2\n3,4\n5,", true, Some(1 << 30));
        assert_eq!(doc.grid().rows(), 2, "the cut row is left out");
        assert!(doc.status()[0].starts_with("2+ rows"));
        let rows = doc.sections().remove(0).rows;
        assert!(rows.iter().any(|(k, _)| k == "Shown"), "{rows:?}");
        assert!(rows.iter().any(|(k, v)| k == "Size" && v == "1.00 GiB"));
    }

    #[test]
    fn open_reads_through_the_provider() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("people.tsv");
        std::fs::write(&path, "name\tage\nAda\t36\nAlan\t41\n").unwrap_or_else(|e| panic!("{e}"));
        let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
        let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
        let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&path));
        let doc = runtime::block_on(CsvPreviewer.open(&loc, provider, CancelToken::new()))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.status(), ["2 rows × 2 columns", "tab", "UTF-8"]);
        let Some(Content::Grid(grid)) = doc.content() else {
            panic!("no grid");
        };
        assert_eq!(grid.cells(1..2), vec![vec!["Alan", "41"]]);
    }
}
