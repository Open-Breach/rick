# ext-action-compress

`ext-action-compress` holds the archive actions of the Share group:
*Compress* packs the selection into a zip next to it under a derived name,
*Compress to…* asks for the name, the format (zip, tar.gz, tar.zst, 7z,
tar, tar.xz, tar.bz2; gz, zst, xz or bz2 for a single file) and the folder
in a small form, and *Extract here*, *Extract to folder* (one named after
the archive) and *Extract to…* unpack the selected archives of every
format the archive provider reads. Each run is one pausable job in the
operations area with bytes and items done; an extraction that would land
on an existing name asks through the conflict dialogue (skip, overwrite,
rename, apply to all, cancel) like a copy, a folder over a folder merges,
and a cancelled compression leaves no archive behind. The archives are
written and read through `ext-vfs-archive`'s `create` and `extract`, so
the zip, tar and 7z code exists once; the actions work on local files
only. It is a plugin (`plugins/action-compress/`) that depends on `ext`,
`types` and `ext-vfs-archive`; `rick` (behind `action-compress`) and
`tests` link it, and the plugin host loads it.
