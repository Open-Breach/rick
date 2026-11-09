# ext-preview-image

`ext-preview-image` previews JPEG, PNG, WebP, BMP, TIFF and GIF files (GIF
frames as pages): it scores by magic bytes, reads dimensions and EXIF from
the header at open, decodes off the UI thread when a frame is rendered,
applies the EXIF orientation and downscales to the requested size. SVG and
SVGZ are its vector formats: sniffed by the `svg` root element (the gzip
magic with the `.svgz` name), rasterised with `resvg` at the exact size the
pane asks for and again at every change of it, never scaled up from a
cached bitmap, with the *Rendered* toggle to the highlighted XML source and
the source as the text layer, so *Copy* copies the raw SVG. Scripts never
run and nothing outside the file is read: only data URIs are honoured. It
depends on `ext`, `types` and, for the source view, `ext-preview-code`;
`rick` and `tests` load it.
