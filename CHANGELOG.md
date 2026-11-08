# Changelog

All notable changes to Rick are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Rick follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each line names
the pull request that brought the change.

## [Unreleased]

### Added

- `ext-ssh`, SSH hosts browsed over a shell (#34): every host of
  `~/.ssh/config` (with its `Include`s, `HostName`, `User`, `Port`,
  `IdentityFile` and `ProxyJump` honoured) and every host added in the
  Network section's editor is a row of that section that opens its default
  directory; browsing, stat, read, read from an offset, write, mkdir,
  rename, delete and symlink targets run as commands on `exec` channels
  through `russh`, never SFTP, with the host probed once for its command
  profile (`gnu`, `bsd`, `busybox`) and listings streamed as NUL-separated
  records so any name survives; a same-host copy or move runs on the host
  as `cp -a` or `mv` and streams nothing. Authentication tries the agent,
  the identity files (a passphrase is asked for) and then
  keyboard-interactive and a password, with the secrets in the platform
  keychain; host keys follow `known_hosts` with OpenSSH semantics (hashed
  entries, `@revoked`, port-qualified names) and an unknown or changed key
  asks in a dialogue, nothing trusted silently. Connections pool per host
  with a second one at the server's session limit, close after the idle
  timeout and come back on the next operation; jump hosts chain hop by
  hop over `direct-tcpip` channels, a bastion shared by every target behind
  it; a connection that fails is a banner with *Retry* in the tab. *Open
  console here* opens the console pane on a PTY channel of the existing
  connection and *Open in external terminal* runs `ssh <alias>` in the
  user's terminal. Previews over a remote host serve a prefix with *Load
  more* to the text-like previewers and hold a large file behind a size
  cap with *Preview anyway*, the cap a setting with a per-host override.
  Three settings in `ssh.toml`; Windows hosts, port forwarding and
  mounting are out.
- Questions from extensions (#34): an extension working off the UI thread
  asks the user through `Prompts` (a choice or a secret with *Remember*)
  and the shell shows one modal dialogue at a time, the oldest first.
- *Edit…* on a connection row of the Network and Cloud sections opens the
  connection's dialogue filled in; a choice field draws a combo box (#34).

- `ext-preview-3d`, the 3D model previewer, as a plugin in `plugins/`:
  STL (ASCII and binary), OBJ (materials read for their colour) and 3MF
  (units and build transforms honoured) files render through wgpu into the
  preview frame as a shaded solid over a ground grid; dragging orbits, the
  wheel zooms, a double-click resets the view, and the view is kept per
  file while it is open. The status bar shows the format, the triangle
  count, the bounding box in millimetres and, for OBJ, the object count;
  File info adds the 3MF unit and metadata. A model over the triangle
  budget (`triangle_budget` in `preview-3d.toml`, 2 000 000 by default) is
  decimated for the preview and the status bar says so; without a graphics
  adapter the preview shows the facts and says rendering is unavailable
  (#19).
- `ext-installer`, the tarball installer of `specs/ext-installer.md`, as a
  plugin (#44): *Install as application…* on a tar (gz, xz, bz2, zst) or
  zip archive reads its index and the headers of its programs to tell a
  prebuilt GUI app from a command-line toolchain and a source tree, guess
  the main executable, icon, name and version, and check the ELF machine
  type against the machine, then shows a form with every guess editable
  and a summary of what will be written; the install extracts into a
  staging folder under the archive-safety rules (no path escapes, no
  outside-pointing links, no special entries, set-uid stripped, nothing
  run), lands in `~/.local/opt/<app-id>/<version>/` behind a `current`
  symlink, writes `rick-install.toml` beside it and the desktop entry,
  icon and command symlinks through `current`, or the PATH-mode file
  `~/.config/rick/path.sh` for toolchains. An archive matching an
  installed app opens the form in update mode, with a downgrade warning;
  one previous version is kept and *Revert* flips back. The Installed
  Apps view, `rick://apps`, lists the records with version, kind, scope,
  state and commands and offers *Open install folder*, *Launch*, *Update
  from archive…*, *Revert to previous version*, *Edit launcher…*,
  *Uninstall…* (exactly the record's files and the app folder), *Repair*,
  *Forget* and *Remove staging folder*. Linux only: the plugin loads on
  every platform and hides its actions elsewhere; system-wide installs
  show as unavailable until the privileged helper's milestone. The
  extension API is version 9: `ActionOutcome::Navigate` takes a pane to a
  location and `platform::Elevate` runs a program as the administrator
  (`pkexec` on Linux).
- Search as plugins (#22): `ext-search-walk` carries the subdirectory
  search of the filter bar, the same walk as 0.1's behind the
  `SearchIndex` point, and the filter bar shows its toggle only while it
  is loaded; `ext-search-windows`, `ext-search-spotlight`,
  `ext-search-tracker` and `ext-search-baloo` connect the title-row
  search box to the platform's index, each detecting its service at
  start and every ten seconds, so the box lists the index's hits with
  their folders after the current location's matches once the text has
  stood for a moment, Enter on a hit goes to its folder and selects it,
  and without an index the box searches the current location alone as
  before. The bundle and the release archives carry only the connector
  of their platform.
- `ext-svn`, Subversion beside Git (#22): status badges with Subversion's
  letters, an SVN column, a roll-up mark on folders with changes inside,
  and the revision, URL and last commit in the info panel, read through
  `svn status` and `svn info` and cached until the working copy or its
  `.svn` directory changes; the SVN group (Update, Commit, Revert, Log,
  Add) in the context menu and the palette inside a working copy, run
  through the `svn` command line.
- Providers as plugins, SFTP and the clouds: `ext-vfs-sftp` (remote hosts
  over SSH through russh and russh-sftp, a move within a host a remote
  rename), `ext-vfs-s3` (S3, R2, MinIO and other S3-compatible stores),
  `ext-vfs-azure` (Azure Blob) and `ext-vfs-gcs` (Google Cloud Storage),
  each with its connections as the rows of the side panel's Network or
  Cloud section, *Add connection…* on the section header and the *Add…*
  row with a dialogue per provider, and *Connect*, *Disconnect* and
  *Remove connection* on a row. Connections live in `[[connections]]`
  tables of `rick.toml`; passwords, keys and secret keys in the platform
  keychain through the new `platform::Secrets` (Secret Service, Keychain,
  Credential Manager). A move or copy inside one account runs on the
  service (`CopyObject`, *Copy Blob*, `Rewrite`) and streams nothing
  through the machine, within a bucket or across two buckets of the
  account; uploads stream in parts and a cancelled one leaves no object
  (#21).
- The action plugins of milestone 6, every one in `plugins/` of the
  archives (#23): `action-compress` packs the selection into a zip next to
  it, or into a zip, tar.gz, tar.zst, 7z, tar, tar.xz, tar.bz2 or a single
  compressed file named and placed in a form, and extracts the selected
  archives here, into a folder named after each or to a chosen folder, as
  pausable jobs with progress and the conflict dialogue; `action-compare`
  opens two files side by side with the differing lines highlighted and
  one scroll for both, or two folders (the two selected, or the two panes')
  entry by entry as same, added, removed or changed with a copy button
  each way; `action-terminal` opens a console at the location as a dock
  tab, the user's shell in a pseudo-terminal drawn by a small terminal
  emulator with colours, a scroll region, the alternate screen and 10 000
  lines of history, or the system terminal when its `console` setting says
  so; `action-rename` renames the selection by a pattern (`{name}`,
  `{ext}`, `{n}`, `{date}`), find and replace with regular expressions and
  a case change, every result previewed with clashes marked, as one
  undoable job.
- Core actions: Open with… (Mod+O) lists the platform's applications for
  the file's type and takes any program, the choice remembered per type;
  Paste as link (Mod+Shift+V) and Create link… make symbolic links, or a
  junction or a .lnk shortcut on Windows, undoably; Select by pattern
  (Mod+Shift+S) selects by glob or regular expression, replacing or adding
  to the selection (#23).
- Settings an extension declares appear in the Settings window and are
  kept in the extension's own TOML file in the config directory (#23).
- The viewer: Enter or a double-click opens a previewable file in a viewer
  tab of the pane, a dock tab like any other (split, dock, tear out). Left
  and Right step through the folder's files in the list's order with the
  position in the title; Home and End go to the ends; PageUp and PageDown
  step pages, then files. Fit, fit width, 1:1 and free zoom (Mod+scroll,
  Mod+Plus and Minus, Mod+0, Mod+1, Mod+2) with panning, the zoom
  remembered per file type for the session; a page strip with thumbnails, a
  page counter with direct entry and single or continuous scrolling for
  paged content; Mod+F finds in the text layer with the match count, next
  and previous and highlights in place; I opens the info panel with every
  section; the Action registry on the file from a menu and the shortcuts; a
  transport bar for media (Space, J, K, L); F11 fills the screen, Escape
  leaves it and then closes the tab. A file no previewer claims says so;
  `[viewer] external` in `rick.toml` (a row in Settings) names the types
  that open with the system application instead, and Mod+Enter always does
  (#17).
- `ext-preview-hex`, the fallback previewer, as a plugin in `plugins/`:
  anything no other previewer claims shows as a hex dump with an ASCII
  column, the first mebibyte of a large file, laid out row by row as it
  scrolls (#17).
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

- The extension API is at version 10: the registry lends `Prompts` to
  extensions, `Previewer` gained `reads()` (`Reads::Whole` or
  `Reads::Prefix`), `Provider` gained `open_read_at()` and
  `remote_preview_cap()`, `Connector::schema()` returns an owned schema
  with `ConnectionFieldKind::Choice`, and the terminal emulator moved from
  `ext-action-terminal` into `ext::terminal` so a plugin can open a console
  on its own stream (#34).
- A listing that fails is a banner with *Retry* over the empty list, not a
  toast (#34).
- The extension API is at version 8: a previewer can claim the pointer over
  its content as `PreviewControl::Pointer` controls (a drag, the wheel, a
  double-click), which the preview pane and the viewer hand to `set_control`
  as `ControlValue::Delta` or `Pressed` instead of panning, scrolling or
  switching to 1:1; a control change keeps the last frame on screen until
  the next one is rendered (#19).
- The extension API is at version 7: `Provider` gained `transfer()` behind
  `NATIVE_TRANSFER`, `Capabilities` gained `REMOTE` and the registry takes a
  `Connector` per network provider (#21).
- The extension API is version 6: actions show forms with a live preview
  (`UiHost::form`), open a console or a comparison in a tab
  (`ActionOutcome::View`) or change the selection (`ActionOutcome::Select`),
  see the other pane's location, and make junctions and shortcuts where
  the provider offers them; the name-conflict queue is the job runner's,
  so a plugin's job asks the same dialogue as a copy; the platform knows
  the applications for a file, opens with one, and opens the user's
  terminal (#23).
- The extension API is at version 5: `Filter` gained `NoHidden` and
  `SkipNames`, the two things the subdirectory search takes from the pane
  and `rick.toml`, and `SearchScope` gained `covers` (#22).
- The preview is a dock tab: one Preview tab per window, right of the
  explorer by default, docked below or torn out at will, following the
  focus across panes and windows; its toolbar, badges, pinned section and
  status bar are unchanged. Mod+Shift+I and the title row's toggle open and
  close it, and the session brings it back where it was (#17).
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

- The "not part of version 0.1" notices of Select by pattern, Paste as
  link, Bulk rename, Open with… and Open console; the five rows of the
  shortcut table dispatch to the actions (#23).
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
