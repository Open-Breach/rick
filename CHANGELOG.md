# Changelog

All notable changes to Rick are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Rick follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each line names
the pull request that brought the change.

## [Unreleased]

### Added

- SVG and SVGZ previews in `ext-preview-image` (issue 83, SPEC.md *Vector
  images*): claimed by the `svg` root element, or the gzip magic with the
  `.svgz` name, never by the extension alone; rasterised with `resvg` at
  the exact size the pane asks for and again at every change of it — the
  pane's size for Fit, the intrinsic size at the display scale for the
  actual-size request — so the picture is sharp at any size and nothing
  comes from a scaled-up bitmap; the long edge capped at 8192 px with the
  footer saying so. The intrinsic size is `width` and `height`, else the
  `viewBox`, else 300 × 150. Only data URIs are honoured: no `href` to a
  file or URL is followed, no script runs, text is set through usvg's
  font database over the system fonts. The *Rendered* toggle, on for
  every newly shown file, switches to the highlighted XML, and the source
  (inflated for SVGZ) is the text layer, so *Copy* copies the raw SVG and
  find searches it; the pane's *Copy* inflates a gzip-compressed file
  before decoding it. A document usvg rejects opens on the source with
  the parser's message, line and column, in the footer. Footer
  `SVG · 512 × 512 px · 14.2 KiB`, `viewBox 0 0 24 24` in place of the
  size when only a viewBox exists; File info plus an Image section
  (QUESTIONS.md 255, 256) (#91).
- The app icon on all three platforms and a Linux desktop entry (issue
  88, [`specs/app-icons.md`](specs/app-icons.md), #92). One identifier,
  `rick`, ties the desktop file, the hicolor icon name, the Wayland app id
  and `StartupWMClass` together, so GNOME and KDE match the running window
  to its launcher entry. Every window carries the Portal burst icon from
  `assets/icons/png/rick-256.png` and the app id through eframe (the title
  bar, taskbar and Alt-Tab on Windows and X11), and the About window shows
  the rings-only symbol at 64 px, rasterised from the SVG with resvg, the
  image previewer's rasteriser, at the drawn size times the window's
  scale (#93).
- `rick.exe` carries its icon and version resource: ProductName "Rick",
  FileDescription "Rick file manager", FileVersion and ProductVersion from
  `Cargo.toml` and the Apache-2.0 notice, embedded by `build.rs` through
  `winresource` for Windows targets only, so Explorer and the Properties
  dialog show them (#95).
- The macOS archive is `Rick.app` zipped: the release workflow lays the
  whole bundle out in `Contents/MacOS` (plugins beside the binary as
  everywhere), `Info.plist` from `packaging/macos/Info.plist` with the
  release's version and `Contents/Resources/Rick.icns`, so Finder and the
  Dock show the icon; unsigned, as before (#97).
- The Linux archive carries `share/applications/rick.desktop`
  (`packaging/linux/rick.desktop`: `Exec=rick %F`, `Icon=rick`,
  `StartupWMClass=rick`, `MimeType=inode/directory;`) and the icons under
  `share/icons/hicolor/` in the XDG layout, so one copy of `share/` into
  `~/.local/share/` plus `rick` on the PATH installs it, and the tarball
  installer finds the entry and the icons where it looks (#96).
- `rick <folder>…` opens every folder on the command line in a tab of its
  own, as the desktop entry's `%F` passes the selected folders; an argument
  that names nothing is passed on as a location rather than failing the
  start (#94).
- The README's install paragraph: `share/` and the PATH or the installer
  plugin on Linux, `Rick.app` to Applications on macOS, nothing new on
  Windows; no install script (#98).

### Changed

- The workspace version is `0.3.0-dev` until 0.3.0 is tagged.
- The scheme and density tokens of the design refresh (issue 60;
  `ui_egui::theme`): every colour comes from a scheme, the Figma collection
  *Rick* one to one with *Light* and *Dark* as the two schemes Rick ships,
  and every size from the *Density* collection, *Comfortable* (the default)
  and *Compact*; egui's visuals, spacing and text styles are derived from
  them for both of egui's themes, so following the system keeps working,
  and a widget that needs a token egui has no field for reads it from the
  context. A unit test holds the shipped pairs to WCAG contrast (4.5:1
  text, 3:1 controls and icons; QUESTIONS.md 242 names the three Figma
  pairs that fall short) (#68).
- The density setting has two levels, Comfortable and Compact; `normal` in
  an existing `rick.toml` reads as Comfortable (QUESTIONS.md 225) (#68).
- The interface is set in the system's UI font and its monospace font
  (issue 60): the platform reports them (`SystemFonts`: fontconfig's
  `sans-serif` and `monospace` on Linux, San Francisco or Helvetica Neue
  and Menlo on macOS, Segoe UI and Cascadia Mono or Consolas on Windows),
  egui's bundled Ubuntu and Hack stay behind them as fallbacks, and the
  Phosphor icons come first in both families with their text code points
  moved out of a letter's way, so no letter shows for an icon and no icon
  for a letter (#69).
- Workspace tabs in the title row (issue 60, SPEC.md *Title row* and
  *Workspace tabs*): each tab is a whole workspace — a layout of panes
  with their locations, histories, selections and dividers — titled after
  the folder in its focused pane with the place's, the view's or the
  provider's icon, 220 px wide on the bottom edge of a `titlebar/height`
  row, the active one filled `bg/pane` and joined to the panes below; the
  new-tab button follows the last tab, a middle click or the close button
  closes a tab, a drag along the row reorders them, a tab dropped outside
  every window opens a new one and one dropped on another window's row
  moves there, and a pane dropped on the row, or on the centre target of
  another pane, becomes a workspace of its own. Panes hold one tab each
  and have no tab strips; *Open in new tab* everywhere, Mod+T, Mod+W,
  Mod+Tab, Mod+1 to 9 and the Move tab shortcuts act on workspaces. The
  menu slot of the title row is as wide as the side panel, so the first
  tab starts where the panes begin; the search box is 320 px before the
  window group and gives way only when the row is short of room. The
  session and the saved layouts hold the workspaces in order with the
  active one; a `session.toml` of 0.2 reads as one workspace per window,
  its panes' further tabs opening as workspaces of their own (#70).
- The panes of the design refresh (issue 60, SPEC.md *Panes*, *Toolbar*,
  *Path bar*, *View options menu*): no tab bar over a pane any more; a
  grab handle at the top right of every toolbar drags the pane, with a
  ghost card under the pointer and the five dock tiles on the pane below —
  left, right, top and bottom split it and dock the pane there, the
  centre or the tab row makes it a workspace tab, outside the window a
  new window; the toolbar holds Back and Forward, the breadcrumb starting
  with the parent chip after a hairline, and a view options button
  before the handle. The view options menu replaces the view-mode
  dropdown, the hidden-files toggle and the refresh button: Details, List
  and Icons with the three icon sizes, the Show hidden files and Folders
  first switches, Refresh now, and a per-type filter — one checkbox per
  file type in the folder with its count, folders too — that narrows the
  list, clears on navigation and lights a dot on the button while it is
  on (`Command::TypeFilter`). Panes are separated by a hairline that
  drags to resize; the focused pane shows no border (#71).
- The file list of the design refresh (issue 60, SPEC.md *File list*,
  *Row states*, *Empty folder*, *Loading*, *Loading animations*): the
  columns Name, Size, Type and Modified in that order, Size right-aligned
  with an em dash for a folder, the three secondary columns in
  `text/secondary`; the column header a 34 px `bg/subtle` band between two
  hairlines with 12 px titles and the sort glyph after the sorted one, its
  separators the only vertical lines of the list; rows with
  `radius/control` corners inset 6 px from the pane's edges, `bg/subtle`
  on hover, `bg/selected-soft` with `text/on-selected` when selected in
  the focused pane and `bg/selected-inactive` in another, a 2 px
  `accent/focus` ring on the keyboard cursor, the same in the list and
  icon views. An empty folder shows `FOLDER_OPEN`, *This folder is empty*
  and the new-folder hint, a filter that matches nothing *No entries match
  the filter*; while a folder is read and no row has arrived the list
  shows the **portal burst** — Rick's loading animation, three arc rings
  around a glow with folder and file glyphs spiralling out, in
  `accent/focus` — with *Reading folder…* and the count so far. The
  platform reports the system's reduce-motion setting (`Motion`: GNOME's
  `enable-animations`, macOS's *Reduce motion*, Windows's client area
  animation), which stills the portal (#72).
- The footer of the design refresh (issue 60, SPEC.md *Footer*): one row
  at the foot of every pane, the toolbar's height under a hairline with
  `space/toolbar-x` padding. Left to right the hidden-files toggle, icon
  only — `EYE` on `bg/selected-soft` while hidden files are shown,
  `EYE_SLASH` as a ghost button while they are hidden — the statistics in
  12 px `text/secondary` (`FOLDER` and the folder count, `FILE` and the
  file count, the files' total size; *12 of 340* while a filter narrows
  the list, *listing…* while entries still arrive), a hairline and the
  selection pill (`bg/selected-soft`, `text/on-selected`, *1 selected ·
  568 B*) while something is selected, the pane's note; at the right the
  200 × 26 filter field with a `MAGNIFYING_GLASS` at 60 %, the *Filter*
  placeholder and, inside its right end, the include-subfolders toggle
  (`TREE_STRUCTURE`, tinted while on). Escape in the field clears the
  text and returns the focus to the list; the separate clear button is
  gone. The counts and the pill are read aloud with their nouns. The
  preview pane's footer takes the same height and the 12 px secondary
  text (#73).
- The side panel of the design refresh (issue 60, SPEC.md *Rows*,
  *Section headers*, *Trash pill*, *This Machine*, *Operations drawer*):
  rows `sidebar/item-height` tall with `radius/control` corners and
  `space/row-x` padding, inset 6 px from the panel's edges, a 16 px icon,
  12 px of gap and the label in 13 px, `bg/subtle` on hover and the
  2 px `accent/focus` ring on the keyboard row; section headers in 11 px
  letter-spaced capitals with a `CARET_DOWN` at 60 % that folds the
  section (`CARET_RIGHT` when folded), 18 px of air above and 6 below, the
  title itself the overview's row, nothing indented; the Trash row's pill
  with the trash's size — not its count — tinted `trash/low-*`, `mid-*`
  or `high-*` by the share of its disk the trash takes (under 1 %, to
  5 %, over), no pill while the trash is empty; This Machine as drive
  cards (`bg/subtle`, 8 px corners, 6 px apart; 44 px in Comfortable with
  the icon, the name, the percentage and a 4 px usage bar, 26 px in
  Compact on one line), a network mount as a row with its percentage, the
  *Add…* row with `PLUS_CIRCLE`. The operations area is the **operations
  drawer**: a `bg/card` card floating over the foot of the panel — along
  the window's foot while the panel is hidden — with 12 px top corners, a
  hairline and a shadow upwards; its summary *2 running · 2 waiting* with
  the overall percentage in `accent/focus` and a caret over a 4 px bar,
  which a click collapses the drawer to; a block per running job with its
  icon (the portal spinner, `ARCHIVE` for an archive job), its title,
  pause and a round `bg/control` cancel button, a thin bar and the detail
  line *473 MiB of 2.2 GiB · 33.7 MiB/s · 53 s left*; the waiting jobs
  under *WAITING* with why each waits; finished jobs as compact rows. The
  app sums the trashed items' sizes when it lists the trash (#74).
- The context menu of the design refresh (issue 60, SPEC.md *Context
  menu*): a 260 px card of 30 px entries, each with a 16 px icon slot, its
  label in 13 px and its shortcut in 12 px `text/secondary` at the right,
  the hovered entry `bg/selected-soft`, groups separated by hairlines in
  the spec's order — Open (Open, Open with, Open in new tab, Open in
  terminal here, Pin to side panel), Edit (Cut, Copy, Paste, Paste as
  link, *Copy as* with Name, Path and Path and name, Duplicate), Change
  (Rename, Move to trash and Delete permanently in `status/danger`), the
  extension groups (*Compress* and the plugins' groups as submenus opening
  to the right, a group with one entry as that entry, Install as
  application, Compute hash…) and Info (Properties). The empty area's
  menu: New folder and New file; Paste, Paste as link, Select all; Open
  terminal here, Open in system file manager, Refresh; Folder properties,
  which opens the Properties dialogue of the folder itself (#75).
- The dialogue frame of the design refresh (issue 60, SPEC.md
  *Dialogues*, *Install dialogue*): a `dialog/bg` window with 12 px
  corners and a soft shadow in three bands — a header with an icon, the
  title and a secondary line, a body that scrolls when the window is too
  short, a footer under a hairline with a note at the left and *Cancel*
  before the primary button filled `accent/button` — which the form,
  prompt and confirmation dialogues now take. Forms gained a header with
  an icon (a glyph, a generated monogram or a PNG), a pill and a subtitle,
  a footer note, a titled preview drawn as a card of effects, sections and
  cards, disclosure cards (counted ones with a *n of m* pill and *Select
  all*), segmented controls, file fields with *Choose…*, monospace values
  and required fields whose warning disables the primary. The install
  dialogue uses them: the archive's icon or a monogram at 48 px with the
  *Update* or *Reinstall* pill and the version change; Name, App ID and
  Version; *Install for* as *Just me* / *All users*; the Kind, Main
  executable and Icon card, the icon rows reading *path · size*; the
  *WHAT RICK WILL DO* card from the live summary; *Commands to link* for
  a toolchain; *Advanced options* collapsed with its summary. A missing
  version warns under its field and holds the primary; an archive without
  an icon offers *Choose…*. Detection keeps the bytes of icon files up to
  256 KiB for the header (#76).
- The hash dialogue of the design refresh (issue 60, SPEC.md *Hash
  dialogue*): 620 px in the dialogue frame, the header with the `HASH`
  glyph, *File hashes*, the file's name and size and a round close
  button. The *VERIFY* field comes first: when the clipboard holds
  something that looks like a hash — hex of MD5, SHA-1, SHA-256 or BLAKE3
  length, or Base64 of one — the field is prefilled with it, compared at
  once and carries a *from clipboard* hint until edited; a match gives it
  a `status/success` border and a *Matches SHA-256* pill and tints the
  row `status/success-row` with its label in `status/success`, anything
  else a `status/danger` border and a *No match* pill. Under it a card
  per file with one row per algorithm — the label, the value in 11 px
  monospace, a copy icon named after the algorithm — each file under a
  row with its name and size when several are selected, then the
  *Format* segmented control (*hex*, *HEX*, *Base64*). The footer has
  *Copy all* with the `COPY` glyph and *Done*. While the job runs the
  card gives way to the portal burst with *Hashing…*, the bytes done of
  the total, the throughput and the time left over a 4 px bar; *Copy all*
  is disabled and *Cancel* stops the job and closes the dialogue at once.
  The platform's clipboard can be read (`Clipboard::text`) and the hash
  slots carry the files' sizes (#77).
- The settings window of the design refresh (issue 60, SPEC.md *Settings
  window*): 880 × 640, GNOME-style, a viewport of its own where the
  backend has them and an area over the main window in the dialogue
  frame otherwise. The left column in `bg/panel` holds the *Settings*
  title, a search field that narrows the categories and rows to those
  whose label or description match, and the category list — General,
  View, Appearance, then *PLUGINS* with a row per loaded plugin that
  declares settings, each with an icon chosen from its id. The right
  column shows the chosen page: its title, a round close button and
  grouped cards, one row per setting with its label, description and
  control — a switch, a dropdown, a segmented control or a text field.
  General holds Language, Start with, Opening an archive, After a drag,
  the spring-loading delay (a dropdown of presets) and the names the
  subtree search skips; View the Files, Sorting and Side panel groups;
  Appearance Theme, Density and Font size; a plugin's page its schema as
  a card. The 0.2 settings screen with its grid and sliders is gone
  (#78).

### Removed

- The accent-colour setting: colours are the scheme's contrast-checked
  pairs, and more colour comes as further schemes; an `accent` key in
  `rick.toml` is ignored and dropped on the next write (QUESTIONS.md 226)
  (#68).

### Fixed

- The pane toolbar sits on the pane's `bg/pane`, the active workspace
  tab's colour, so the tab reads as attached to its pane and only the
  column header band is tinted; egui's panel frame had painted it
  `bg/window` like the title row. The row keeps `space/toolbar-x` of
  padding at both ends, as the design has it, in place of the frame's
  (issue 79 item 1).
- The footer is the Figma status bar's height — the hairline, then the
  26 px filter field with `space/toolbar-y` above and below it: 39 px
  Comfortable, 35 px Compact — with the hidden-files toggle 26 px like
  the field, and one hairline over it: egui's panel separator had drawn a
  second line between the list and the footer, and the panel's margins
  had made the row taller than the toolbar. The preview pane's footer
  takes the same height and hairline (issue 79 item 4, QUESTIONS.md 254).
- The preview pane's toolbar is the explorer toolbar's height, with
  `space/toolbar-x` padding, and ends like every other pane's in the
  hairline and the six-dot grab handle (`DOTS_SIX_VERTICAL`, named *Drag
  pane*, the grab and grabbing cursors), which drags the pane onto the
  dock tiles, the tab row or out of the window; it had been a 30 px row
  with the close button alone. The handle and the hairline are one widget
  shared with the explorer toolbar (`widgets::pane_handle`, `divider`)
  (issue 79 items 2 and 3).
- The path field, while the breadcrumb is being edited, is `button/size`
  tall — the height of Back and Forward, on their line — and as wide as
  the breadcrumb; it had hugged its text. Its look and the rest of the
  editing rule are issue 79 item 10 (issue 79 item 5).
- Every column of the details list resizes by the boundary after it, Name
  included, and the last column takes the remaining width; the header
  draws its separator at every boundary, where the previous code's
  clipped cells had cut all but the one between Size and Type away. The
  hover and drag visuals of the boundaries, the fit on double-click and
  the widths' persistence are issue 79 item 9 (issue 79 item 6).
- A removable drive's card in This Machine carries an eject button
  (`EJECT`) at its right edge, vertically centred and always visible,
  named *Eject <drive>* for the tooltip and the screen reader; a click
  ejects the drive, as the context menu's *Eject* does. The usage bar's
  colours and the read-only drives are issue 79 item 8 (issue 79 item 7).

## [0.2.0] - 2026-11-08

The plugin release, on Linux, macOS and Windows: the native plugin host and
the split of every non-essential feature into a plugin file, the side panel
reworked, archives, disk images, the Trash, SSH hosts and the S3, Azure and
Google Cloud stores as providers, the Office, media, archive, markup, hex and
3D previewers, the search connectors, Subversion, the compress, compare,
console and bulk-rename actions, permissions, localisation, the native drag
source, Compute hash, and the tarball installer on Linux.

### Added

- The burger menu lists the actions extensions put in the application
  menu (`Action::app_menu`, extension API 14): after the Layouts submenu,
  one submenu per extension with the actions that apply to the focused
  pane; the installer's *Installed Apps…* is the first (#59).
- An action can open its form when a job of its finishes
  (`ActionOutcome::Later`, `FormLater`, `Continuation`,
  `UiHost::form_later`; extension API 14): the application shows the form
  and runs the rest of the action with its values, the egui shell as its
  one dialogue through `Command::Continue` (#59).
- `ext-preview-markup` (#51): one plugin with three previewers for the
  structured text formats, each rendered by default with the *Rendered*
  toggle to the source: Markdown through `pulldown-cmark` and HTML through
  the basic reader (headings, paragraphs, lists, links, images, tables; no
  browser engine, no JavaScript, no CSS fidelity) as documents the pane lays
  out, with images read from the file's own folder; CSV and TSV as the
  virtualised grid with the delimiter sniffed and the first row as header.
  The source behind the toggle is the code previewer's highlighted text for
  Markdown and HTML and the raw text for CSV, and it is the text layer the
  pane's Copy reads. The previewers score above `ext-preview-code` for
  their extensions, so disabling the plugin leaves highlighted source for
  `.md` and `.html` and plain text for `.csv`.
- Disk images (#51): raw IMG, DMG, VHD, VHDX, VMDK, VDI and QCOW2 browse
  like archives, read-only, in `ext-vfs-archive` (Enter enters the image
  as a location, files inside read and copy out) and `ext-preview-archive`
  (the file list in the pane on selection, with the layout and volume
  label in File info), through the 7-Zip library (`7z.so` / `7z.dll`,
  LGPL) loaded dynamically from `platform::sevenzip`, which parses the
  containers, the partition tables and the file systems inside (FAT, NTFS,
  ext, HFS+, APFS, UDF, ISO), one folder per partition as 7-Zip names them;
  ISO 9660 and UDF keep the native readers. The library is bundled beside
  the archive plugin in `plugins/vfs-archive/` with its licence (built from
  the pinned sources on Linux and macOS, the pinned installer's `7z.dll`
  on Windows) and resolved lazily on the first image shown: the
  `sevenzip_library` setting of `vfs-archive.toml`, then the bundle, then
  the platform's usual installs, the first hit written back into the
  setting; when nothing is found the preview shows a message naming the
  library, how to install it on this platform and the setting, and
  browsing to the library stores its path.
- *Mount* (#51): in the context menu of a disk-image file, and so in the
  command palette, hands the file to the operating system's own mount
  mechanism through `platform::Mount` and nothing else: `udisksctl
  loop-setup` and `mount` on Linux (ISO and raw images), `hdiutil attach`
  on macOS (DMG, ISO, raw), `Mount-DiskImage` on Windows (ISO, VHD, VHDX);
  the action hides itself on a file the current OS cannot mount, the
  mounted image appears as a removable volume in This Machine through the
  live volume watch, and that row's *Eject* unmounts it (deleting the loop
  device on Linux). No qemu-nbd, no FUSE, no Unmount action.

- Compute hash… in the Info group of a file's context menu, above
  Properties, and `core.hash` in the palette: MD5, SHA-1, SHA-256 and
  BLAKE3 of the selected files, each read once through its provider in
  1 MiB chunks so a remote file is hashed like a local one, as one job with
  byte progress and Cancel. The Hash dialogue opens at once and fills in: a
  block per file with a row per algorithm (the value in a read-only
  monospace field, Copy), Copy all, a format toggle (lower-case hex,
  upper-case hex, Base64; remembered as `[hash] format` in `core.toml`)
  that re-renders without recomputing, and a Verify field compared live
  against every row of the pasted length — green *Match*, red *Mismatch*,
  or *Unrecognised length*. Hidden on folders; no shortcut (#54).
- Dragging out of Rick reaches the desktop: a file dragged past the window
  is handed to the platform's drag source as `file://` URIs once the ghost
  is off the screen, through an `NSDraggingSession` on macOS (the first
  file's icon under the cursor), OLE's `DoDragDrop` with a `CF_HDROP` on
  Windows, a `wl_data_source` on Wayland and an XDND source on X11, each
  from the window `winit` describes through its raw handles; the receiver
  decides between move and copy, and Rick's pointer is put right when the
  drag ends, since the backend never sees the button come up (#25).
- A drop from another application lands where it was dropped: the platform
  supplies the cursor position the backend leaves out (the cursor over the
  window on Windows and X11, the mouse location in the view on macOS, the
  data device's motion on Wayland), so a file dropped on a folder row goes
  into that folder; without a position the drop lands in the focused pane's
  folder as before. On Wayland, where `winit` 0.30 receives no drops at
  all, Rick's own `wl_data_device` now takes them (#25).
- `BENCHMARKS.md` carries the five-run numbers of every benchmark 0.2
  added and the macOS targets re-measured and tightened towards the spec's
  numbers where the runner now meets them (the listing's first batch and
  the PDF's first frame are back at the spec's); the two the runners fell
  short of, cancelling on macOS and the fixture on Windows, follow what
  the runners do (#25).
- The preview pane is Rick's only viewer (#50): it lays out every kind
  of content a previewer returns itself, so a folder, archive or disk
  image shows its file list in the explorer's columns with the stats line
  and folding in place, a CSV file its grid with a header row, and a
  Markdown or HTML file its rendered document (headings, paragraphs,
  styled text, links, lists, code, quotes, tables, pictures, rules), with
  the *Rendered* toggle to the highlighted source; audio and video show
  in the pane with the transport in the toolbar (play or pause, seek, the
  times, volume; Space plays while the pane has the focus), playback on
  demand. *Copy* in the middle group of every text-based preview puts
  the whole file's text on the clipboard, the raw source for a rendered
  format. A file no previewer claims shows a placeholder, not a hex dump:
  its facts, its type icon and *Plugin available* or *No plugin
  available*, answered from the plugin manifests, which now list the
  file extensions a plugin's previewers claim. Enter and a double-click
  open a file: a folder navigates, an archive enters in the current tab
  unless the new setting `archives.on_open` (a row in Settings) is
  `system`, everything else goes to the system's default application;
  Mod+Enter always does.
- Third-party libraries through a setting (#50): the PDF previewer's
  `library` (`preview-pdf.toml`) and the Office previewer's `soffice`
  (`preview-office.toml`) name pdfium and LibreOffice; each is resolved
  lazily on the first file shown, from the setting, then beside the
  binary and the plugins directory, then the platform's usual places,
  and the first hit is written back into the setting. When nothing is
  found the pane shows the file's information and a message naming the
  library with the install command or download for this platform,
  *Browse…* (a file dialogue, the choice stored in the setting) and
  *Retry*. CI and `cargo xtask bundle` read and write the setting.
- *Always preview on <host>* beside *Preview anyway* in the cap message
  of a large file on a remote source (#50): it loads the file and sets
  `preview_always` for that SSH host, after which the cap no longer
  applies there; the same switch is a checkbox in the host editor and an
  entry in the host row's context menu in Network, and works for a host
  of `~/.ssh/config` too.
- Localisation (#24): every text of the shell, the dialogues, the menus,
  the core actions and the plugins goes through the `t!` lookup over Fluent
  files bundled in the binary (`locales/` for the kernel, each plugin's
  own), overridable key by key from `locales/<lang>.ftl` in the
  configuration directory; the *Language* setting (system default, or a
  listed language) applies live, English is the fallback for any missing
  key, and German is the first other language, for the Settings screen,
  the context menus with their actions and the Properties dialogue. A CI
  check fails on a user-facing string literal outside `t!` in the egui UI.
- Permissions in Properties (#24): Properties on a local entry shows and
  edits its owner, group and mode bits (`rwx` for user, group and other,
  setuid, setgid and sticky, with the octal beside them) and the access
  control list of its file system (POSIX and NFSv4 ACLs on Linux, macOS
  ACLs, NTFS permissions with the trustees and the standard rights on
  Windows); *Apply to enclosed items* changes a folder's contents too,
  files and folders chosen separately; every change is one job with
  progress and one undo step, whatever it touched. What the user may not
  change goes to `rick-perms`, the privileged helper beside the binary,
  through the platform's prompt (polkit, Authorization Services, User
  Account Control) once per job, so Rick itself never runs elevated.
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
- Providers as plugins, the clouds: `ext-vfs-s3` (S3, R2, MinIO and other
  S3-compatible stores), `ext-vfs-azure` (Azure Blob) and `ext-vfs-gcs`
  (Google Cloud Storage), each with its connections as the rows of the
  side panel's Network or Cloud section, *Add connection…* on the section
  header and the *Add…* row with a dialogue per provider, and *Connect*,
  *Disconnect* and *Remove connection* on a row. Connections live in
  `[[connections]]` tables of `rick.toml`; passwords, keys and secret keys
  in the platform keychain through the new `platform::Secrets` (Secret
  Service, Keychain, Credential Manager). A move or copy inside one
  account runs on the service (`CopyObject`, *Copy Blob*, `Rewrite`) and
  streams nothing through the machine, within a bucket or across two
  buckets of the account; uploads stream in parts and a cancelled one
  leaves no object (#21).
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
- `ext-preview-hex`, the hex view, as a plugin in `plugins/`: a file's
  bytes as a hex dump with an ASCII column, the first mebibyte of a large
  file, laid out row by row as it scrolls (#17); it claims no file on its
  own and is chosen per type in settings (#50).
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
- Three previewers as plugins, every one in `plugins/` of the archives
  (#18): `preview-office` converts DOCX, XLSX, PPTX, ODT, ODS and ODP to
  PDF with an installed LibreOffice, cached by path and modification time,
  and shows the metadata with a hint to install it otherwise; `preview-
  media` plays MP3, WAV, MP4 and AVI through GStreamer with play, pause,
  seek, volume, speed, frame step and loop, a waveform and cover art for
  audio, tags and stream details, and shows the metadata and what is
  missing without GStreamer; `preview-archive` lists zip, tar (plain and
  gzip, zstd, xz, bzip2, lz4), 7z, RAR and ISO 9660 contents without
  extracting, encrypted entries marked. The CSV grid and the rendered
  Markdown and HTML of the same change were superseded by
  `ext-preview-markup` before release (#51).
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

- The extension API is at version 14: `ActionOutcome` gained `Later`,
  `UiHost` `form_later` and `Action` `app_menu` (#59).
- The installer reads an archive as the job *Reading archive…*, in one
  streaming pass with byte progress over the compressed stream and a
  cancel button, and opens the install form when the job finishes;
  *Install as application…* returns at once, so a gzip tar of several
  gigabytes no longer freezes the window (#59).
- The installer's detection names the app by a word of the app name or a
  suffix of the app id, so `bin/rider` and `bin/rider.svg` are found in a
  JetBrains tarball where `fsnotifier` won by alphabetical order; among
  several candidates a matching binary wins and a helper never does, a
  toolchain whose commands are all equal opens the form on "(choose one)",
  every PNG and SVG in the archive is offered in one Icon list with the
  best preselected only when detection trusts it (the icon file field is
  gone), and the command defaults follow the chosen kind: a GUI
  application links one command named after the app id at the main
  executable, a toolchain its `bin/` contents (#59).
- The extension API is at version 13: `platform` gained `Mount` (which
  disk-image formats this OS mounts, and mounting one through it) and the
  `sevenzip` binding of the 7-Zip library; `ext-preview-code` highlights
  Markdown and HTML as source only, the rendering having moved to
  `ext-preview-markup` (#51).
- Eject on Linux releases the loop device of a mounted disk image after
  unmounting it (#51).
- The extension API is at version 12 (#50): `Manifest` gained `previews`,
  the registry lends a `SettingsStore` so a plugin reads and writes its
  own declared settings, `Content` gained `MissingLibrary`, and `Provider`
  gained `remote_preview_always`.
- `ext-preview-hex` claims no file by itself any more: its score is 0 for
  every file and it is chosen per type in settings (#50).
- The extension API is at version 11: `Action::label` returns a `String`
  and `Availability::Disabled` carries one, both built with `t!`; the
  labels the registry keeps (`SettingField`, `ConnectionField`,
  `ConnectionSchema`, `InfoSection`, `ColumnDef`) are `Text` keys read
  live; the registry takes an extension's translation files through
  `Registry::translations` (#24).
- Properties is Rick's own dialogue on every platform; on Windows and
  macOS *System properties…* opens the platform's own as before (#24).
- `platform::Elevate` runs a command as the administrator on macOS and
  Windows too, not Linux alone (#24).
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
  double-click), which the preview pane hands to `set_control` as
  `ControlValue::Delta` or `Pressed` instead of panning, scrolling or
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

- `ext-preview-csv`, superseded by the CSV previewer of `ext-preview-markup`
  (#51).
- `ext-vfs-sftp`, superseded by `ext-ssh` before release (it never
  shipped; #48).
- The environment variables `RICK_PDFIUM_DIR` and `RICK_SOFFICE`: the
  library paths are the plugins' settings, which development and CI write
  instead (#50).
- The viewer of 0.2 milestone 3 (#29), withdrawn before any release: the
  preview pane is the only viewer, and Enter opens files (#50).
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

[Unreleased]: https://github.com/Open-Breach/rick/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Open-Breach/rick/releases/tag/v0.2.0
[0.1.0]: https://github.com/Open-Breach/rick/releases/tag/v0.1.0
