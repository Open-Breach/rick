# ext-preview-code

`ext-preview-code` previews source and markup files: the spec's language
list by extension, shebangs by first line, highlighted with syntect into
coloured spans the UI lays out, in a light or dark theme following the
request, with a Pretty print toggle for JSON, XML and HTML. Markdown and
HTML are also rendered: pulldown-cmark, or a small sanitising HTML reader
without a browser engine, turn the text into an `ext::Document` of
headings, paragraphs, lists, code, tables, links and images (images from
the file's own folder read through the provider), behind a Rendered toggle
that is on for a newly opened file; the source stays the fallback for a
pane that shows no documents. It depends on `ext` and `types` only; `rick`
and `tests` load it.
