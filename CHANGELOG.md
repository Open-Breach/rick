# Changelog

All notable changes to Rick are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Rick follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each line names
the pull request that brought the change.

## [Unreleased]

## [0.1.0] - 2026-11-06

The first release: local files, tabs and panes, the core operations, previews
for text, code, images and PDF, and Git, on Linux, macOS and Windows.

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

[Unreleased]: https://github.com/Open-Breach/rick/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Open-Breach/rick/releases/tag/v0.1.0
