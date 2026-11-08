# ext-preview-archive

`ext-preview-archive` previews archives and disk images as a read-only
file list without extracting or mounting anything: zip (and jar, apk,
epub), tar plain or compressed with gzip, zstd, xz, bzip2 or lz4, 7z, RAR
and ISO 9660 images through native readers, and the disk images of the
spec's table (raw, DMG, VHD, VHDX, VMDK, VDI, QCOW2) through the archive
provider's reader over the 7-Zip library. It reads the archive's
directory, or walks a tar's headers within a time budget, and hands the
pane an `ext::Listing` in explorer order (folders first, each directory
followed by its contents) with name, kind, size, compressed size, date and
whether the entry is encrypted; an archive whose directory is itself
encrypted lists nothing and says so (QUESTIONS.md 105); a disk image lists
one folder per partition with the layout and volume label in File info.
The status bar shows entries, compressed and uncompressed size and the
format; `render` gives the same list as text for a pane that shows no
listings. Without the 7-Zip library a disk image opens as the pane's
missing-library content, which names the library, how to install it on
this platform and the archive provider's setting; *Browse…* stores the
chosen library into that setting, shared by both plugins, and *Retry*
lists the image (QUESTIONS.md 204). It is a plugin (`cdylib`,
loaded from `plugins/preview-archive/`) and an `rlib` for the binary's
opt-in `preview-archive` feature. It depends on `ext`, `types` and
`ext-vfs-archive` (the disk-image reader and the library resolver);
`rick` (behind the feature) and `tests` link it, and the plugin host loads
it.
