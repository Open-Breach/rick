use std::sync::Arc;

use ext::{Block, Content, ControlValue, Frame, Image, Inline, PreviewControl, Style};
use types::{Attrs, EntryId, EntryKind, ProviderId};

use super::*;

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

fn req() -> RenderRequest {
    RenderRequest {
        width: 100,
        height: 100,
        scale: 1.0,
        dark: false,
    }
}

const ALL_MARKDOWN: &str = "# Title\n\nA paragraph with *emphasis*, **strong**, ~~struck~~, `code` and a [link](https://example.org/page).  \nHard break above. Soft\nbreak here.\n\n## Lists\n\n- one\n- two\n  - nested\n\n3. third\n4. fourth\n\n- [x] done\n- [ ] open\n\n> A quote\n> with two lines\n\n```rust\nfn main() {}\n```\n\n    indented code\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n![Alt text](picture.png)\n\nInline ![icon](icon%20one.png) here and <b>raw html</b>.\n\n---\n\n### The end\n";

#[test]
fn the_three_previewers_score_their_extensions_above_the_code_previewer() {
    let code = ext_preview_code::CodePreviewer;
    for (name, previewer) in [
        ("README.md", &MarkdownPreviewer as &dyn Previewer),
        ("notes.MARKDOWN", &MarkdownPreviewer),
        ("page.html", &HtmlPreviewer),
        ("page.htm", &HtmlPreviewer),
        ("data.csv", &CsvPreviewer),
        ("data.tsv", &CsvPreviewer),
    ] {
        let score = previewer.score(&entry(name), b"text");
        assert_eq!(score, SCORE, "{name}");
        assert!(score > code.score(&entry(name), b"text"), "{name}");
        assert_eq!(previewer.id(), ID, "{name}");
        assert_eq!(previewer.reads(), Reads::Prefix, "{name}");
        assert_eq!(
            previewer.score(&entry(name), b"\x00\x01"),
            0,
            "{name}: binary"
        );
        let mut dir = entry(name);
        dir.kind = EntryKind::Dir;
        assert_eq!(previewer.score(&dir, b""), 0, "{name}: a folder");
    }
    assert_eq!(MarkdownPreviewer.score(&entry("page.html"), b"x"), 0);
    assert_eq!(HtmlPreviewer.score(&entry("README.md"), b"x"), 0);
    assert_eq!(CsvPreviewer.score(&entry("README.md"), b"x"), 0);
    assert_eq!(MarkdownPreviewer.score(&entry("main.rs"), b"fn"), 0);
    for ext in EXTENSIONS {
        assert_eq!(
            [
                &MarkdownPreviewer as &dyn Previewer,
                &HtmlPreviewer,
                &CsvPreviewer
            ]
            .iter()
            .filter(|p| p.score(&entry(&format!("f.{ext}")), b"x") == SCORE)
            .count(),
            1,
            "exactly one previewer claims .{ext}"
        );
    }
    assert_eq!(Markup::for_name("a.md"), Some(Markup::Markdown));
    assert_eq!(Markup::for_name("a.HTM"), Some(Markup::Html));
    assert_eq!(Markup::for_name("a.csv"), None);
    assert_eq!(Markup::for_name(".md"), None);
}

#[test]
fn markdown_renders_every_element() {
    let doc = markdown::render(ALL_MARKDOWN);
    let blocks = &doc.blocks;
    assert!(
        matches!(&blocks[0], Block::Heading { level: 1, text } if text == &[Inline::Text { text: "Title".into(), style: Style::empty() }]),
        "{:?}",
        blocks[0]
    );
    let Block::Paragraph(inlines) = &blocks[1] else {
        panic!("{:?}", blocks[1]);
    };
    assert!(inlines.iter().any(|i| matches!(i, Inline::Text { text, style } if text == "emphasis" && *style == Style::ITALIC)));
    assert!(inlines.iter().any(
        |i| matches!(i, Inline::Text { text, style } if text == "strong" && *style == Style::BOLD)
    ));
    assert!(inlines.iter().any(|i| matches!(i, Inline::Text { text, style } if text == "struck" && *style == Style::STRIKETHROUGH)));
    assert!(inlines.iter().any(
        |i| matches!(i, Inline::Text { text, style } if text == "code" && *style == Style::CODE)
    ));
    assert!(inlines.iter().any(|i| matches!(i, Inline::Link { text, href } if text == "link" && href == "https://example.org/page")));
    assert!(inlines.iter().any(|i| matches!(i, Inline::Break)));
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Text { text, .. } if text.contains("Soft break here"))),
        "{inlines:?}"
    );
    assert!(matches!(&blocks[2], Block::Heading { level: 2, .. }));
    let Block::List {
        ordered: false,
        items,
        ..
    } = &blocks[3]
    else {
        panic!("{:?}", blocks[3]);
    };
    assert_eq!(items.len(), 2);
    assert!(
        items[1]
            .iter()
            .any(|b| matches!(b, Block::List { ordered: false, items, .. } if items.len() == 1)),
        "nested: {:?}",
        items[1]
    );
    assert!(
        matches!(&blocks[4], Block::List { ordered: true, start: 3, items } if items.len() == 2),
        "{:?}",
        blocks[4]
    );
    let Block::List { items, .. } = &blocks[5] else {
        panic!("{:?}", blocks[5]);
    };
    assert!(
        matches!(&items[0][0], Block::Paragraph(p) if matches!(&p[0], Inline::Text { text, .. } if text == "[x] done")),
        "{:?}",
        items[0]
    );
    assert!(
        matches!(&blocks[6], Block::Quote(inner) if inner.len() == 1),
        "{:?}",
        blocks[6]
    );
    assert!(
        matches!(&blocks[7], Block::Code { language: Some(l), text } if l == "rust" && text == "fn main() {}"),
        "{:?}",
        blocks[7]
    );
    assert!(
        matches!(&blocks[8], Block::Code { language: None, text } if text == "indented code"),
        "{:?}",
        blocks[8]
    );
    let Block::Table { header, rows } = &blocks[9] else {
        panic!("{:?}", blocks[9]);
    };
    assert_eq!(header.len(), 2);
    assert_eq!(rows.len(), 2);
    assert!(matches!(&rows[1][1][0], Inline::Text { text, .. } if text == "4"));
    assert!(
        matches!(&blocks[10], Block::Image(image) if image.alt == "Alt text" && image.source == "picture.png" && image.data.is_none()),
        "{:?}",
        blocks[10]
    );
    let Block::Paragraph(inlines) = &blocks[11] else {
        panic!("{:?}", blocks[11]);
    };
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Image(image) if image.alt == "icon")),
        "{inlines:?}"
    );
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Text { text, .. } if text.contains("<b>"))),
        "raw html stays text: {inlines:?}"
    );
    assert!(matches!(&blocks[12], Block::Rule));
    assert!(matches!(&blocks[13], Block::Heading { level: 3, .. }));
    assert_eq!(blocks.len(), 14, "{blocks:?}");
}

#[test]
fn odd_markup_renders_without_panics() {
    for text in [
        "",
        "#",
        "- ",
        "1.",
        "> > >",
        "| a |\n|",
        "```\nunterminated",
        "![]()",
        "[]()",
        "***",
        "<div><p>unclosed",
        "</p></li></ul></table>",
        "<table><td>cell without row</td></table>",
        "<ul>text outside items<li>a<ul><li>b</ul></ul>",
        "<pre><code class=\"language-rs\">let x = 1;</code>",
        "<a href=\"javascript:alert(1)\">x</a><img src=\"\"><img alt=\"only alt\">",
        "<!-- open comment",
        "<b><i>crossed</b></i>",
        "a < b && c > d",
        "<script>never</script>after<style>.x{}</style>",
    ] {
        let md = markdown::render(text);
        let html = html::render(text);
        assert!(md.blocks.len() < 100 && html.blocks.len() < 100, "{text}");
    }
}

const SAMPLE_HTML: &str = "<!DOCTYPE html><html><head><title>Nope</title><style>p{color:red}</style><script>alert(1)</script></head><body><h1>Heading &amp; more</h1><p>First <b>bold</b> and <em>italic</em> with a <a href=\"docs/a.html\">link</a> and <a href=\"javascript:x\">unsafe</a>.<br>Second line</p><ul><li>one</li><li>two<ol start=\"5\"><li>five</li></ol></li></ul><blockquote><p>quoted</p></blockquote><pre><code class=\"language-rust\">fn x() {}\n</code></pre><table><tr><th>H1</th><th>H2</th></tr><tr><td>a</td><td>b &lt; c</td></tr></table><div><img src=\"pic%20one.png\" alt=\"A picture\"></div><p>Text <img src=\"http://x/y.png\" alt=\"remote\"> inline</p><hr><section>Loose text</section></body></html>";

#[test]
fn html_renders_sanitised() {
    let doc = html::render(SAMPLE_HTML);
    let blocks = &doc.blocks;
    assert!(
        matches!(&blocks[0], Block::Heading { level: 1, text } if matches!(&text[0], Inline::Text { text, .. } if text == "Heading & more")),
        "{:?}",
        blocks[0]
    );
    let Block::Paragraph(inlines) = &blocks[1] else {
        panic!("{:?}", blocks[1]);
    };
    assert!(inlines.iter().any(
        |i| matches!(i, Inline::Text { text, style } if text == "bold" && *style == Style::BOLD)
    ));
    assert!(inlines.iter().any(
        |i| matches!(i, Inline::Text { text, style } if text == "italic" && *style == Style::ITALIC)
    ));
    assert!(inlines.iter().any(
        |i| matches!(i, Inline::Link { text, href } if text == "link" && href == "docs/a.html")
    ));
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Text { text, .. } if text.contains("unsafe"))),
        "unsafe link kept as text: {inlines:?}"
    );
    assert!(
        !inlines
            .iter()
            .any(|i| matches!(i, Inline::Link { href, .. } if href.starts_with("javascript")))
    );
    assert!(inlines.iter().any(|i| matches!(i, Inline::Break)));
    let Block::List {
        ordered: false,
        items,
        ..
    } = &blocks[2]
    else {
        panic!("{:?}", blocks[2]);
    };
    assert_eq!(items.len(), 2);
    assert!(
        items[1].iter().any(|b| matches!(
            b,
            Block::List {
                ordered: true,
                start: 5,
                ..
            }
        )),
        "{:?}",
        items[1]
    );
    assert!(matches!(&blocks[3], Block::Quote(_)));
    assert!(
        matches!(&blocks[4], Block::Code { language: Some(l), text } if l == "rust" && text == "fn x() {}"),
        "{:?}",
        blocks[4]
    );
    let Block::Table { header, rows } = &blocks[5] else {
        panic!("{:?}", blocks[5]);
    };
    assert_eq!(header.len(), 2);
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0][1][0], Inline::Text { text, .. } if text == "b < c"),
        "{:?}",
        rows[0]
    );
    assert!(
        matches!(&blocks[6], Block::Image(image) if image.alt == "A picture" && image.source == "pic%20one.png"),
        "{:?}",
        blocks[6]
    );
    let Block::Paragraph(inlines) = &blocks[7] else {
        panic!("{:?}", blocks[7]);
    };
    assert!(
        inlines
            .iter()
            .any(|i| matches!(i, Inline::Image(image) if image.source == "http://x/y.png"))
    );
    assert!(matches!(&blocks[8], Block::Rule));
    assert!(
        matches!(&blocks[9], Block::Paragraph(p) if matches!(&p[0], Inline::Text { text, .. } if text == "Loose text")),
        "{:?}",
        blocks[9]
    );
    assert_eq!(blocks.len(), 10, "{blocks:?}");
    let flat = format!("{doc:?}");
    assert!(
        !flat.contains("alert") && !flat.contains("color:red") && !flat.contains("Nope"),
        "{flat}"
    );
}

#[test]
fn relative_sources_stay_inside_the_folder() {
    assert_eq!(
        relative_source("pic%20one.png?x=1#top").as_deref(),
        Some("pic one.png")
    );
    assert_eq!(
        relative_source("sub/pic.png").as_deref(),
        Some("sub/pic.png")
    );
    assert_eq!(relative_source("./pic.png").as_deref(), Some("./pic.png"));
    assert_eq!(relative_source("../pic.png"), None);
    assert_eq!(relative_source("/etc/passwd"), None);
    assert_eq!(relative_source("https://x/y.png"), None);
    assert_eq!(relative_source("data:image/png;base64,AAAA"), None);
    assert_eq!(relative_source(""), None);
    assert_eq!(percent_decode("a%2Fb%zz"), "a/b%zz");
}

#[test]
fn markdown_renders_by_default_and_the_toggle_shows_the_highlighted_source() {
    let mut doc = MarkupDoc::new("README.md", b"# Hi\n\nText\n", false, Some(12))
        .unwrap_or_else(|| panic!("a Markdown document"));
    assert_eq!(doc.markup(), Markup::Markdown);
    assert_eq!(doc.text(), "# Hi\n\nText\n");
    assert!(doc.rendered(), "on for a newly shown file");
    assert_eq!(doc.controls(), [rendered_control(true)]);
    let Some(Content::Document(rendered)) = doc.content() else {
        panic!("no content");
    };
    assert!(matches!(
        &rendered.blocks[0],
        Block::Heading { level: 1, .. }
    ));
    assert_eq!(doc.document().blocks.len(), 2);
    // The source is always there for the pane that shows no documents:
    // highlighted by the code previewer's syntax for Markdown.
    let Frame::Text(block) = doc.render(0, req()).unwrap_or_else(|e| panic!("{e}")) else {
        panic!("not text");
    };
    assert_eq!(block.text, "# Hi\n\nText\n");
    assert_eq!(block.language.as_deref(), Some("Markdown"));
    assert!(!block.spans.is_empty(), "the source is highlighted");
    // The text layer carries the raw source for Copy and find.
    assert_eq!(
        doc.text_layer().map(|l| l.pages[0].as_str()),
        Some("# Hi\n\nText\n")
    );
    // Off: no content, the pane shows the source.
    doc.set_control(RENDERED, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!doc.rendered());
    assert!(doc.content().is_none());
    assert_eq!(doc.controls(), [rendered_control(false)]);
    doc.set_control(RENDERED, ControlValue::Bool(true))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(doc.content(), Some(Content::Document(_))));
    assert!(doc.set_control(RENDERED, ControlValue::Pressed).is_err());
    assert!(
        doc.set_control(ControlId("nope"), ControlValue::Pressed)
            .is_err()
    );
    assert!(doc.transport().is_none());
    assert_eq!(doc.pages(), None);
    // Sections and status name the format.
    let sections = doc.sections();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].title.key(), "preview-markup-file-info");
    assert_eq!(
        sections[0].rows[0],
        ("Format".to_owned(), "Markdown".to_owned())
    );
    assert!(
        sections[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Blocks" && v == "2")
    );
    assert!(
        sections[0]
            .rows
            .iter()
            .any(|(k, v)| k == "Size" && v == "12 B")
    );
    assert_eq!(doc.status(), ["Markdown", "UTF-8", "3 lines"]);
    assert!(format!("{doc:?}").contains("Markdown"));
    assert!(MarkupDoc::new("main.rs", b"fn main() {}", false, None).is_none());
}

#[test]
fn html_renders_by_default_and_offers_pretty_print_on_the_source() {
    let mut doc = MarkupDoc::new("page.html", b"<p>x</p><p>y</p>", false, None)
        .unwrap_or_else(|| panic!("an HTML document"));
    assert_eq!(doc.markup(), Markup::Html);
    assert!(matches!(doc.content(), Some(Content::Document(d)) if d.blocks.len() == 2));
    assert_eq!(
        doc.controls(),
        [rendered_control(true)],
        "only the Rendered toggle while rendered"
    );
    doc.set_control(RENDERED, ControlValue::Bool(false))
        .unwrap_or_else(|e| panic!("{e}"));
    let controls = doc.controls();
    assert_eq!(controls.len(), 2, "{controls:?}");
    assert!(matches!(
        &controls[1],
        PreviewControl::Toggle {
            id: ext_preview_code::PRETTY,
            on: false,
            ..
        }
    ));
    doc.set_control(ext_preview_code::PRETTY, ControlValue::Bool(true))
        .unwrap_or_else(|e| panic!("{e}"));
    let Frame::Text(block) = doc.render(0, req()).unwrap_or_else(|e| panic!("{e}")) else {
        panic!("not text");
    };
    assert_eq!(block.text, "<p>\n  x\n</p>\n<p>\n  y\n</p>");
    assert_eq!(doc.status()[0], "HTML");
}

#[test]
fn open_reads_the_images_of_the_folder_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir(dir.path().join("img")).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("img").join("a b.png"), b"\x89PNG fake")
        .unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("secret.txt"), b"outside").unwrap_or_else(|e| panic!("{e}"));
    let sub = dir.path().join("docs");
    std::fs::create_dir(&sub).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(sub.join("local.png"), b"\x89PNG local").unwrap_or_else(|e| panic!("{e}"));
    let md = sub.join("page.md");
    std::fs::write(
        &md,
        "![local](local.png)\n\n![up](../img/a%20b.png)\n\n![missing](nope.png)\n\n![web](https://example.org/x.png)\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&md));
    let doc = runtime::block_on(MarkdownPreviewer.open(&loc, provider, CancelToken::new()))
        .unwrap_or_else(|e| panic!("{e}"));
    let Some(Content::Document(document)) = doc.content() else {
        panic!("no document");
    };
    let images: Vec<&Image> = document
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Image(image) => Some(image),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 4, "{:?}", document.blocks);
    assert_eq!(
        images[0].data.as_deref(),
        Some(&b"\x89PNG local"[..]),
        "a file next to the document"
    );
    assert!(images[1].data.is_none(), "no parent steps");
    assert!(images[2].data.is_none(), "missing files stay empty");
    assert!(images[3].data.is_none(), "no network");
    assert_eq!(doc.status()[0], "Markdown");
}

#[test]
fn open_refuses_a_name_that_is_no_markup_and_html_opens_too() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let html = dir.path().join("page.html");
    std::fs::write(&html, "<h1>Hi</h1>").unwrap_or_else(|e| panic!("{e}"));
    let other = dir.path().join("notes.txt");
    std::fs::write(&other, "plain").unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(platform::Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let provider: Arc<dyn Provider> = Arc::new(vfs_local::LocalProvider::new(fake));
    let at = |path: &std::path::Path| {
        Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(path))
    };
    let doc = runtime::block_on(HtmlPreviewer.open(
        &at(&html),
        Arc::clone(&provider),
        CancelToken::new(),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(doc.content(), Some(Content::Document(d)) if d.blocks.len() == 1));
    assert!(
        runtime::block_on(HtmlPreviewer.open(&at(&other), provider, CancelToken::new())).is_err()
    );
}

#[test]
fn the_extension_registers_three_previewers_under_one_id() {
    let mut registry = Registry::new();
    registry
        .load(&MarkupPreview::new())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(registry.previewers().len(), 3);
    assert!(registry.previewers().iter().all(|p| p.id() == ID));
    assert_eq!(registry.extensions(), &[ID]);
    assert_eq!(MarkupPreview::default().manifest().name, "Markup preview");
}
