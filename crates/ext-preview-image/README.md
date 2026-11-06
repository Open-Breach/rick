# ext-preview-image

`ext-preview-image` previews JPEG, PNG, WebP, BMP, TIFF and GIF files (GIF
frames as pages): it scores by magic bytes, reads dimensions and EXIF from
the header at open, decodes off the UI thread when a frame is rendered,
applies the EXIF orientation and downscales to the requested size. It
depends on `ext` and `types` only; `rick` and `tests` load it.
