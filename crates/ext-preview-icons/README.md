# ext-preview-icons

`ext-preview-icons` previews the icon containers: ICO and CUR (Windows),
ICNS (macOS) and XPM (X11), each claimed by its magic. It decodes in pure
Rust — the `ico` crate for ICO and CUR with their BMP and PNG entries, the
`icns` crate for ICNS with its PNG and run-length encoded types, and an
XPM parser of its own with the X11 colour names and `None` transparent —
so no third-party library is involved. The variants of an ICO, CUR or ICNS
are the document's pages, largest first, stepped through with the pane's
pager; a variant the plugin cannot decode (an ICNS JPEG 2000 entry, an
unknown type, a corrupt entry) is listed in File info as not decodable
and counted in the footer instead of being a page, and a file with no
decodable variant fails to open with a clear error. Icons are small, so
in Fit the previewer magnifies the variant by the largest whole factor up
to 8 that fits the request, nearest-neighbour, and for the pane's
actual-size request returns the variant at its own pixels, so every pixel
stays a crisp square and the pane needs nothing new; transparent pixels
stay transparent. A CUR draws its hotspot as a small crosshair, with the
`icons.hotspot` toggle to hide it. XPM is text, so its source is the text
layer and *Copy* copies it; the binary containers have none. It is a
plugin (`cdylib`, loaded from `plugins/preview-icons/`) and an `rlib` for
the binary's opt-in `preview-icons` feature. It depends on `ext` and
`types` only; `rick` (behind the feature) and `tests` link it, and the
plugin host loads it.
