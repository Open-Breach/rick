//! The three structured text formats, rendered: Markdown and HTML as a
//! [`Document`] of headings, paragraphs, lists, code, tables, links and
//! images ([`markdown`], [`html`]), CSV and TSV as a virtualised [`Grid`](ext::Grid)
//! ([`csv`]). Each previewer renders by default and carries the *Rendered*
//! toggle ([`RENDERED`]), on for every newly shown file, to the source: the
//! highlighted text of `ext-preview-code` for Markdown and HTML, the raw
//! text for CSV. The source is also what `render` always gives, for a pane
//! that lays out no documents, and the text layer every text-based preview
//! has, so the pane's *Copy* control puts the raw source on the clipboard.
//!
//! One plugin, three previewers, one id: [`MarkdownPreviewer`],
//! [`HtmlPreviewer`] and [`csv::CsvPreviewer`] score [`SCORE`] for the
//! extensions in [`EXTENSIONS`], above the code previewer's claim on `.md`
//! and `.html`, so disabling this plugin leaves the highlighted source and
//! the plain text of a CSV file (SPEC.md "Preview support"). Images a
//! Markdown or HTML document names are read from the document's own folder
//! or below through the provider, within [`IMAGE_LIMIT`], [`IMAGES_LIMIT`]
//! and [`MAX_IMAGES`] (QUESTIONS.md 107).

#![deny(unsafe_code)]

pub mod csv;
pub mod document;
pub mod html;
pub mod markdown;

use std::sync::Arc;

use ext::Text;
use ext::{
    API_VERSION, Capabilities, Content, ControlId, ControlValue, Document, Extension, Frame, Icon,
    InfoSection, Manifest, Platforms, Points, PreviewControl, PreviewDoc, Previewer, Provider,
    Reads, Registry, RenderRequest, TextLayer, Transport, Version, async_trait,
};
use ext_preview_code::CodeDoc;
use types::{CancelToken, Entry, Error, ExtId, Location};

pub use csv::{CsvDoc, CsvPreviewer};

/// The extension id, shared by the three previewers.
pub const ID: ExtId = ExtId("preview-markup");

/// The file extensions the plugin's previewers claim, lower case: what
/// the manifest lists so the host can say *Plugin available* for one of
/// them without loading the plugin.
pub const EXTENSIONS: &[&str] = &["md", "markdown", "html", "htm", "csv", "tsv", "tab"];

/// The score for a claimed extension: above the code previewer's 120 for
/// `.md` and `.html`, and the text previewer's 80.
pub const SCORE: u8 = 150;

/// The Rendered toggle's id: the laid-out document or grid rather than the
/// source.
pub const RENDERED: ControlId = ControlId("markup.rendered");

/// How much of a Markdown or HTML file is read: large files show their
/// top, as in the code previewer.
pub const WINDOW: usize = ext_preview_code::WINDOW;

/// An image named by a document is read up to this size.
pub const IMAGE_LIMIT: usize = 16 * 1024 * 1024;

/// All images of a document together are read up to this size.
pub const IMAGES_LIMIT: usize = 64 * 1024 * 1024;

/// At most this many images of a document are read.
pub const MAX_IMAGES: usize = 50;

/// The extensions a Markdown file has.
const MARKDOWN: &[&str] = &["md", "markdown"];

/// The extensions an HTML file has.
const HTML: &[&str] = &["html", "htm"];

/// A markup the plugin renders as a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Markup {
    /// Markdown, through `pulldown-cmark`.
    Markdown,
    /// HTML, through the sanitising reader.
    Html,
}

impl Markup {
    /// The markup a file name's extension names.
    #[must_use]
    pub fn for_name(name: &str) -> Option<Self> {
        let ext = extension(name)?;
        if MARKDOWN.contains(&ext.as_str()) {
            Some(Self::Markdown)
        } else if HTML.contains(&ext.as_str()) {
            Some(Self::Html)
        } else {
            None
        }
    }

    /// The name shown in the status bar and File info.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Html => "HTML",
        }
    }

    /// Renders `text` as a document, images unread.
    #[must_use]
    pub fn render(self, text: &str) -> Document {
        match self {
            Self::Markdown => markdown::render(text),
            Self::Html => html::render(text),
        }
    }
}

/// The lower-case extension of a name, if any.
pub(crate) fn extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// The Rendered toggle, as every previewer of the plugin shows it.
pub(crate) fn rendered_control(on: bool) -> PreviewControl {
    PreviewControl::Toggle {
        id: RENDERED,
        icon: Icon::Rendered,
        label: "Rendered",
        on,
    }
}

/// The extension: registers the Markdown, HTML and CSV previewers.
#[derive(Debug)]
pub struct MarkupPreview {
    manifest: Manifest,
}

impl Default for MarkupPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkupPreview {
    /// The extension with its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Markup preview",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
                points: Points::PREVIEWER,
                platforms: Platforms::ALL,
                previews: EXTENSIONS,
                commands: &[],
            },
        }
    }
}

impl Extension for MarkupPreview {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        reg.translations(&[("en", include_str!("../locales/en.ftl"))]);
        reg.previewer(Arc::new(MarkdownPreviewer));
        reg.previewer(Arc::new(HtmlPreviewer));
        reg.previewer(Arc::new(CsvPreviewer));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(MarkupPreview::new()) as Arc<dyn Extension>);

/// Previews Markdown files as rendered documents.
#[derive(Debug, Default)]
pub struct MarkdownPreviewer;

/// Previews HTML files as rendered documents: the basic reader, no
/// browser engine, no JavaScript, no CSS fidelity.
#[derive(Debug, Default)]
pub struct HtmlPreviewer;

/// An opened Markdown or HTML file: the rendered document and, behind the
/// Rendered toggle, the highlighted source of the code previewer.
pub struct MarkupDoc {
    source: CodeDoc,
    markup: Markup,
    document: Arc<Document>,
    rendered: bool,
}

impl std::fmt::Debug for MarkupDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkupDoc")
            .field("markup", &self.markup)
            .field("blocks", &self.document.blocks.len())
            .field("rendered", &self.rendered)
            .finish_non_exhaustive()
    }
}

impl MarkupDoc {
    /// Builds the document from the bytes read for `name`, images unread
    /// ([`load_images`] fills them in); `size` is the file's size when
    /// known. `None` for a name that is neither Markdown nor HTML.
    #[must_use]
    pub fn new(name: &str, bytes: &[u8], truncated: bool, size: Option<u64>) -> Option<Self> {
        let markup = Markup::for_name(name)?;
        let source = CodeDoc::new(name, bytes, truncated, size);
        let document = Arc::new(markup.render(source.text()));
        Some(Self {
            source,
            markup,
            document,
            rendered: true,
        })
    }

    /// The markup the file is rendered as.
    #[must_use]
    pub fn markup(&self) -> Markup {
        self.markup
    }

    /// The decoded source text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.source.text()
    }

    /// The rendered document.
    #[must_use]
    pub fn document(&self) -> &Arc<Document> {
        &self.document
    }

    /// Replaces the rendered document: after its images were read.
    pub fn set_document(&mut self, document: Document) {
        self.document = Arc::new(document);
    }

    /// Whether the Rendered toggle is on.
    #[must_use]
    pub fn rendered(&self) -> bool {
        self.rendered
    }
}

impl PreviewDoc for MarkupDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    /// The highlighted source, whatever the toggle: the fallback for a pane
    /// that lays out no documents, and the view when the toggle is off.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error> {
        self.source.render(page, req)
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        self.source.text_layer()
    }

    fn content(&self) -> Option<Content> {
        self.rendered
            .then(|| Content::Document(Arc::clone(&self.document)))
    }

    fn sections(&self) -> Vec<InfoSection> {
        let mut sections = self.source.sections();
        if let Some(section) = sections.first_mut() {
            section.title = Text("preview-markup-file-info");
            section
                .rows
                .insert(0, ("Format".to_owned(), self.markup.name().to_owned()));
            section
                .rows
                .push(("Blocks".to_owned(), self.document.blocks.len().to_string()));
        }
        sections
    }

    fn status(&self) -> Vec<String> {
        let mut status = self.source.status();
        if let Some(first) = status.first_mut() {
            self.markup.name().clone_into(first);
        }
        status
    }

    /// The Rendered toggle first; the source's own controls (pretty print
    /// for HTML) while the source is shown.
    fn controls(&self) -> Vec<PreviewControl> {
        let mut controls = vec![rendered_control(self.rendered)];
        if !self.rendered {
            controls.extend(self.source.controls());
        }
        controls
    }

    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error> {
        match (id, value) {
            (RENDERED, ControlValue::Bool(on)) => {
                self.rendered = on;
                Ok(())
            }
            (RENDERED, _) => Err(Error::Unsupported("the Rendered toggle takes on or off")),
            (id, value) => self.source.set_control(id, value),
        }
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

/// Opens a Markdown or HTML file as the markup previewers do: the first
/// [`WINDOW`] bytes through the provider, the document rendered with its
/// images read from the file's folder.
async fn open_markup(
    loc: &Location,
    provider: Arc<dyn Provider>,
    cancel: CancelToken,
) -> Result<Box<dyn PreviewDoc>, Error> {
    let size = provider.stat(loc).await.ok().and_then(|e| e.size);
    let bytes = ext::read_bytes(&*provider, loc, Some(WINDOW + 1), &cancel).await?;
    let truncated = bytes.len() > WINDOW;
    let shown = &bytes[..bytes.len().min(WINDOW)];
    let name = loc
        .path
        .file_name()
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_default();
    let mut doc = MarkupDoc::new(&name, shown, truncated, size)
        .ok_or(Error::Unsupported("not a Markdown or HTML file"))?;
    let mut document = (*doc.document).clone();
    load_images(&mut document, loc, &*provider, &cancel).await?;
    doc.set_document(document);
    Ok(Box::new(doc))
}

/// The score for a file by name and content: [`SCORE`] for a text file
/// whose extension is in `extensions`, 0 otherwise.
fn score_for(entry: &Entry, sniff: &[u8], extensions: &[&str]) -> u8 {
    if entry.kind != types::EntryKind::File || sniff.contains(&0) {
        return 0;
    }
    let named = extension(&entry.name).is_some_and(|ext| extensions.contains(&ext.as_str()));
    if named { SCORE } else { 0 }
}

#[async_trait]
impl Previewer for MarkdownPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    /// The start of a file is enough: a remote file is served as a prefix
    /// with *Load more*.
    fn reads(&self) -> Reads {
        Reads::Prefix
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        score_for(entry, sniff, MARKDOWN)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        open_markup(loc, provider, cancel).await
    }
}

#[async_trait]
impl Previewer for HtmlPreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn reads(&self) -> Reads {
        Reads::Prefix
    }

    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8 {
        score_for(entry, sniff, HTML)
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        open_markup(loc, provider, cancel).await
    }
}

/// The path of an image source relative to the document, when it names a
/// file in the document's folder or below: no scheme, no leading slash, no
/// parent steps; the query and fragment dropped and percent-escapes
/// decoded.
#[must_use]
pub fn relative_source(source: &str) -> Option<String> {
    let source = source.trim();
    let source = source.split(['?', '#']).next().unwrap_or("");
    if source.is_empty()
        || source.starts_with('/')
        || source.starts_with('\\')
        || source.contains("://")
        || source.starts_with("data:")
        || source.starts_with("mailto:")
    {
        return None;
    }
    let decoded = percent_decode(source);
    if decoded.split(['/', '\\']).any(|part| part == "..") {
        return None;
    }
    Some(decoded)
}

/// Decodes `%XX` escapes; invalid ones stay as they are.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Reads the images a document names from the document's folder through
/// the provider, within the limits; a source that cannot be read keeps
/// `None` for its data.
///
/// # Errors
///
/// `Error::Cancelled` when the token fires; a file that cannot be read is
/// no error, its image stays empty.
pub async fn load_images(
    document: &mut Document,
    loc: &Location,
    provider: &dyn Provider,
    cancel: &CancelToken,
) -> Result<(), Error> {
    let Some(folder) = loc.path.parent() else {
        return Ok(());
    };
    let mut total = 0usize;
    let mut count = 0usize;
    for image in document::images_mut(document) {
        if count >= MAX_IMAGES || total >= IMAGES_LIMIT {
            break;
        }
        let Some(relative) = relative_source(&image.source) else {
            continue;
        };
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        count += 1;
        let path = folder.join(relative.as_bytes());
        let target = Location::new(loc.provider, path);
        match ext::read_bytes(provider, &target, Some(IMAGE_LIMIT + 1), cancel).await {
            Ok(bytes) if bytes.len() <= IMAGE_LIMIT => {
                total += bytes.len();
                image.data = Some(bytes.into());
            }
            Ok(_) => tracing::debug!(source = %image.source, "image larger than the limit"),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => tracing::debug!(source = %image.source, %error, "image not read"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
