# ext-vfs-archive

`ext-vfs-archive` is the `Provider` with the id `archive`: an archive on
the local disk browsed in place as a location whose path is the archive's
OS path followed by the path inside it, so the breadcrumb shows the archive
as a segment. It reads zip (DEFLATE, zstd), the tar family (plain, gz, zst,
xz, bz2, lz4), 7z, RAR, ISO 9660 and UDF images, single-file codecs
(gzip, zstd, xz, bzip2, LZ4, Brotli, Snappy) as one-entry archives, and
the disk images of the spec's table (raw IMG, DMG, VHD, VHDX, VMDK, VDI,
QCOW2) read-only through the 7-Zip library (`7z.so` / `7z.dll`, bound in
`platform::sevenzip`), which opens the container, the partition table and
the file systems inside the way 7-Zip's browser does, one folder per
partition; the library is found lazily through the `sevenzip_library`
setting (`vfs-archive.toml`, read and written through the settings store
the registry lends), the bundle beside the plugin and the platform's usual
installs, the first hit written back, and when it is missing the error
names it and how to install it, with the same facts as the content the
archive previewer shows in the pane (QUESTIONS.md 202–205).
Formats are told apart by extension and, failing that, by their first
bytes. Each archive's
index is read once and cached by path, size and modification time; listings
come in one batch, `stat` answers from the index, `open_read` decompresses
on the fly through the blocking pool, and copying out to another provider
goes through the kernel's streamed path. Zip, tar and 7z take writes
(`open_write`, `create_dir`, `rename`, `remove`, same-archive `copy`): each
rewrites the archive into a temporary file beside it and renames it over
the original, inside the job that asked. RAR and the images are read-only
and say so in their capabilities; `watch` is `Unsupported`, the kernel
polls the archive file. An archive inside an archive is a file, not a
location. It is a plugin (`plugins/vfs-archive/`) that depends on `ext` and
`types` only; `rick` (behind `vfs-archive`), `tests`, the compress plugin
and the archive previewer link it, and the plugin host loads it.
