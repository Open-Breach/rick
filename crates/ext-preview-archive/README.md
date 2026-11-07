# ext-preview-archive

`ext-preview-archive` previews archives as a read-only file list without
extracting anything: zip (and jar, apk, epub), tar plain or compressed with
gzip, zstd, xz, bzip2 or lz4, 7z, RAR and ISO 9660 images. It reads the
archive's directory, or walks a tar's headers within a time budget, and
hands the pane an `ext::Listing` in explorer order (folders first, each
directory followed by its contents) with name, kind, size, compressed size,
date and whether the entry is encrypted; an archive whose directory is
itself encrypted lists nothing and says so (QUESTIONS.md 96). The status
bar shows entries, compressed and uncompressed size and the format;
`render` gives the same list as text for a pane that shows no listings. It
is a plugin (`cdylib`, loaded from `plugins/preview-archive/`) and an
`rlib` for the binary's opt-in `preview-archive` feature. It depends on
`ext` and `types` only; `rick` (behind the feature) and `tests` link it, and
the plugin host loads it.
