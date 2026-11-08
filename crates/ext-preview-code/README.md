# ext-preview-code

`ext-preview-code` previews source files: the spec's language list by
extension, shebangs by first line, highlighted with syntect into coloured
spans the UI lays out, in a light or dark theme following the request,
with a Pretty print toggle for JSON, XML and HTML. Markdown and HTML are
highlighted as source like any other language of the list: that is the
view `ext-preview-markup` toggles to from its rendered document, built on
this crate's `CodeDoc`, and what shows when that plugin is disabled. It
depends on `ext` and `types` only; `rick`, `tests` and `ext-preview-markup`
link it.
