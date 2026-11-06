# ext-preview-text

`ext-preview-text` is the core text previewer: it scores textual files by
their first bytes and names, detects the encoding (BOMs, UTF-8, UTF-16 and a
Windows-1252 fallback), shows a bounded window into large files and exposes
the text as a layer for find. It depends on `ext` and `types` only; `rick`
and `tests` load it.
