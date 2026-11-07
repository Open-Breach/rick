# Changelog

All notable changes to Rick are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Rick follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each line names
the pull request that brought the change.

## [Unreleased]

### Added

- Archives browsed in place, as the plugin `ext-vfs-archive`: Enter or a
  double-click on a zip (DEFLATE, zstd), a tar (plain, gz, zst, xz, bz2,
  lz4), a 7z, a RAR, an ISO 9660 or UDF image or a single compressed file
  (gzip, zstd, xz, bzip2, LZ4, Brotli, Snappy) enters it as a location
  with the archive as a breadcrumb segment; files inside preview and copy
  out through the streamed path; zip, tar and 7z take new files, folders,
  renames and removals by rewriting the archive; RAR and the images are
  read-only (#20).
- The Trash view, `rick://trash`, as the plugin `ext-vfs-trash`: the side
  panel's Trash row opens it in the pane, where the trashed items list with
  their original location, Restore, Restore trash items and Empty trash
  live in the Trash group of the context menu and the palette, Delete
  permanently works as everywhere, and a trashed folder is browsable (#20).
- Views served by extensions: the `rick://` provider routes a location an
  extension registered (`Registry::view`) to that extension's provider, the
  Trash view being the first; the registry lends the host's platform
  services to extensions (#20).
- A location its provider cannot watch (an archive, a trashed folder
  browsed in the Trash view) is polled: every three seconds and right after
  an operation, a change lists it again (#20).
- Five previewers as plugins, every one in `plugins/` of the archives
  (#18): `preview-office` converts DOCX, XLSX, PPTX, ODT, ODS and ODP to
  PDF with an installed LibreOffice, cached by path and modification time,
  and shows the metadata with a hint to install it otherwise; `preview-
  media` plays MP3, WAV, MP4 and AVI through GStreamer with play, pause,
  seek, volume, speed, frame step and loop, a waveform and cover art for
  audio, tags and stream details, and shows the metadata and what is
  missing without GStreamer; `preview-archive` lists zip, tar (plain and
  gzip, zstd, xz, bzip2, lz4), 7z, RAR and ISO 9660 contents without
  extracting, encrypted entries marked; `preview-csv` shows CSV and TSV as
  a grid with a sniffed delimiter and a header row, parsing only the rows
  on screen. The code previewer renders Markdown and HTML as documents
  behind a Rendered toggle, with images from the file's folder.
- The release archives bundle the GStreamer runtime on macOS and Windows;
  Linux uses the system's (#18).
- `cargo xtask bundle --features <spec>` hands features to the build, and
  `RICK_GSTREAMER_DIR` makes the bundle carry a GStreamer runtime (#18).
- The native plugin host: plugins are dynamic libraries beside a
  `plugin.toml` in `plugins/` next to the binary or in the config
  directory (per-user wins on an id clash), checked for their extension API
  version, toolchain and platform before loading, loaded through one entry
  point into the shared registry, and listed with their status; a plugin
  that panics while registering is caught and reported. `rick
  --list-plugins` prints the table. The kernel crates are shared libraries
  that the binary and every plugin link, so one runtime, one tracing
  subscriber and one registry serve them all (#16).
- Git is the first plugin: `plugins/git/` ships in every archive and loads
  at start; the `git` Cargo feature compiles it in instead, off by default
  (#16).
- The Extensions screen in the burger menu: every plugin found, with name,
  version, the points it fills and its status, and an enable/disable toggle
  that writes `rick.toml` and takes effect on the next start (#16).
- `cargo xtask bundle` lays out the binary, the shared libraries and every
  plugin in `target/<profile>/bundle/`; the release archives are that bundle
  (#16).
- The side panel reworked (0.2 milestone 2, issue #26): an editable Places
  list seeded from the system's standard folders and kept in `places.toml`
  (drag to reorder, a dropped folder becomes a place at the insertion line,
  Remove and Edit… with a Phosphor icon, open in new tab by middle click,
  Mod+click and the context menu, Up, Down, Enter and Mod+Enter with the
  panel focused); Favorites, Recent and Trash as views in the active tab
  under `rick://` (the system's recently-used list replaces 0.1's own feed;
  the Trash view keeps restore and Empty trash); This Machine, Network and
  Cloud as collapsible sections whose header opens an overview with the full
  drive table and whose rows open a root, with utilisation bars, Eject for
  removable media, live mount and unmount, Add… on empty sections and Cloud
  shown only while a cloud provider plugin is loaded. The operations area
  grew: a header with the running count and an overall bar, a progress bar
  with bytes or items done of total on every row, a height that follows the
  jobs up to a cap, progress coalesced to twenty reports a second per job,
  and per-volume scheduling: copies and moves that share a volume run in
  turn, the queued one as *Waiting*, disjoint ones together.

### Changed

- The extension API is version 4: a preview document can hand the pane
  structured content (a file listing, a grid, a document) beside its
  frames, a transport has volume, speed, frame step and loop, and a plugin
  library exports its entry point under its own crate's name so that a
  binary can link several plugin crates (#18).
- The extension API is version 3: a manifest declares the points it fills
  and the platforms it runs on (#16).
- The core actions follow the provider's capabilities: Paste, Duplicate,
  Rename, New file and New folder are hidden where nothing can be written,
  Move to trash where there is no trash (#20).
- The trash record macOS needs lives in `platform` behind the trash itself,
  so the Trash view and the local provider see one list; Restore, Restore
  trash items and Empty trash are the trash plugin's actions
  (`trash.restore`, `trash.restore_items`, `trash.empty`) (#20).
- A plugin's entry point is named after its crate
  (`rick_plugin_register_ext_git`), so several plugins link into one binary
  (#20).
- The workspace version is `0.2.0-dev` until 0.2.0 is tagged (#16).
- The side panel is navigation only: it is no longer a drop target for file
  operations (Pin and Trash drops went with the Pinned and Trash sections),
  highlights no current location and draws no indent guide; the Tree section
  is gone (the Tree view mode stays). Every drive of the machine is listed,
  the system disk included.

### Removed

- The 0.1 Recent feed (`recent.toml`, the watches and the scan at start),
  replaced by the system's recently-used list.


## [0.1.0] - 2026-11-06

The first release: local files, tabs, panes and windows with session restore,
the side panel, the core operations with conflicts and undo, previews for
text, code, images and PDF, and Git, on Linux, macOS and Windows.

### Added

- The core data types and the job runtime: every long operation (listing,
  copy, search, preview decode) is a job with progress and cancel, and
  cancelling stops it. The 200 000-entry test fixture the performance
  targets are measured against (#1).
- Linux, macOS and Windows as first-class platforms: trash with restore, open
  with the default application, reveal in the system file manager, the
  properties dialogue, the per-user config directory and native file
  dialogues on each. The local file system provider: listings streamed in
  batches so a 200 000-entry folder shows its first rows at once, change
  watching, and copies that keep sparse files sparse and use reflinks where
  the file system has them (#2).
- The directory model (sort by every column, group, filter, hidden files,
  selection, history), the core operations as jobs with undo (copy, cut,
  paste, duplicate, move to trash, delete permanently, rename, new file, new
  folder, copy path, open, open containing folder) and settings in a plain
  `rick.toml` that is reloaded when edited (#3).
- The window: tabs, split panes and docking, details and list views with
  resizable sortable columns and zebra stripes, type-to-filter, the context
  menu, the command palette, the Settings and Keyboard shortcuts screens with
  rebinding, toasts and the operations area with progress and cancel (#4).
- Previews in the preview pane: text (encodings, huge files windowed), code
  (syntax highlighting that follows the theme), images (JPEG, PNG, WebP,
  BMP, TIFF, GIF with EXIF, orientation applied, big files decoded at the
  scale the pane needs) and PDF (pdfium, page navigation, text layer) (#5).
- Git: status badges, branch and ahead/behind on every entry inside a
  repository, folder roll-ups, and the Git group (Commit, Stash, Stash pop,
  Pull, Push, Checkout branch, Discard changes, Show log, Blame) in the
  context menu and the palette (#6).
- The side panel: Home, Pinned folders and the operations area, with the
  Recent, Network and Trash sections announced; it docks left or right,
  resizes, folds per section and is toggled with Mod+B (#7).
- The window chrome: Rick's own title row with the search box (name search,
  `>` for commands), Phosphor icons throughout, the three-group toolbar with
  a collapsing breadcrumb, the tab strip rules, the line-free details view,
  the icon view with thumbnails in four sizes, and copy name, copy path and
  copy path and name (#8).
- The preview pane layout: a toolbar with each previewer's controls, the
  pinned info panel with File info, EXIF and Git sections, the status bar,
  a remembered width, word wrap, line numbers and pretty print for JSON, XML
  and HTML (#10).
- Drag and drop: folder rows, pane space, breadcrumb segments, tab headers,
  Home, Pinned and Trash as targets; move or copy by volume with Mod, Shift
  and Alt (or a right-button drag) to override; the ghost with its live
  indicator; spring-loading with the stay-or-return setting; drops from
  other applications; Pin to side panel and Unpin (#11).
- Session restore: every window, tab, split, scroll position and side panel
  comes back on the next start, written at most once a second and once more
  on exit; saved layouts (save as, switch, set as start, rename, delete,
  Continue where I left off); New window with Mod+N, tabs torn out into a
  window of their own, all windows sharing one job table and one undo log;
  a folder that vanished opens at its nearest parent with a note (#14).
- The side panel's Recent, Network and Trash sections (fed by watches, a
  bounded scan, the system's volumes and the trash, with restore, delete and
  empty); the operations area with progress, throughput, remaining time,
  pause, resume and cancel; the tree view with lazily listed children and a
  Tree section following the focused pane; inline rename, new file and new
  folder; the paste conflict dialogue (Skip, Overwrite, Rename, Apply to
  all, Cancel) with the job paused at the first conflict; search in
  subdirectories as a cancellable job; Open containing folder (#15).
- The release: archives for Linux (x64 and arm64), macOS (Apple silicon) and
  Windows (x64) with pdfium beside the binary, `rick --version` with the git
  hash, `BENCHMARKS.md` with the measured numbers on all three platforms
  and the performance gates on for every OS, this changelog (#13).

### Changed

- The command palette moved into the title row's search box as its `>`
  mode, and the burger menu lost its Commands entry (#8).
- Previewers contribute their own toolbar controls, info sections and
  status facts to the pane (extension API version 2) (#10).

### Fixed

- Clicking a name in the details view selects its row (ab5e503).
- The window is resizable again on Linux and Windows, with a border and a
  light shadow; the toolbar row is tall enough for its buttons; a pane's
  only tab can be dropped on another pane's edge zone (#9).
- The preview pane's status bar uses the body text size, a fitted picture
  or page is centred, and a picture at actual size pans by dragging (#12).
- Settings are written through a temporary file and a rename, so a reload
  in the middle of a write can no longer read a truncated file and lose
  the pinned folders (#14).

[Unreleased]: https://github.com/Open-Breach/rick/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Open-Breach/rick/releases/tag/v0.1.0
