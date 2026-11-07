# Rick — File Manager Specification

Nov 5, 2026 · @Serge

## Purpose & goals

Rick (tagline: “a god-like file manager”) is a file manager for professionals: lean and simple to use, powerful and flexible underneath, and identical in look and behaviour on Linux, macOS and Windows.

- Same UI everywhere: one self-rendered interface, no native widgets, so a user moving between machines relearns nothing.
- As fast as possible: listing, scrolling and previewing are design constraints from day one, not polish at the end.
- Lean by default, powerful through modules: the core stays small and capabilities arrive as extensions that can be enabled, disabled or replaced.
- Modular and extensible for third parties as much as for Rick's own features.
- Linux is the reference platform and gets no compromises; macOS and Windows ship from the same codebase.

Proposed performance targets, to confirm: a 200,000-entry directory paints its first rows within 100 ms of opening; scrolling holds the display's refresh rate; the first page of a PDF or a photo preview appears within 150 ms; an idle window uses no CPU.

## Decisions so far

Rust with egui, Linux-first, native previews, and a small kernel that every feature plugs into.

| Area | Decision | Why |
| --- | --- | --- |
| Language | Rust | Performance; one language from syscalls to UI |
| GUI toolkit | egui (eframe) with egui\_dock for tabs, split panes and tear-out windows | Self-rendered, identical on all platforms, native Wayland through winit + wgpu; only visible rows are laid out |
| Reference platform | Linux, Wayland-native | Priority platform; no GTK or WebKitGTK in the rendering path |
| Previews | Decoded in Rust and uploaded as GPU textures; PDF through pdfium-render; video through GStreamer on all three platforms (bundled runtime on macOS and Windows) | Zero-copy path; same result on every OS |
| Core shape | Small kernel with five extension points (Provider, Decorator, Action, Previewer, SearchIndex); built-in features are extensions | Keeps the core small and the extension API honest |
| UI to core | Explicit Command and Event types between the egui layer and the core | Core stays testable; UI stays replaceable |
| Third-party plugins | WASM (wasmtime or extism) | Sandboxed; one plugin binary for all platforms |

Ruled out: iced (the preferred architecture, but no docking or tab tear-out today) and Tauri (WebKitGTK on Linux needs workarounds that disable accelerated rendering, no zero-copy preview path, 2 to 3 times the memory).

## Supported scope

Rick runs on Linux, macOS and Windows and must work correctly on every file system, mount, archive and file type below. It does not implement file systems: the OS mounts them, and "supported" means correct behaviour on each (naming rules, attributes, copy fast paths, limits) plus mount and connect actions through the platform layer.

### Platforms

Linux (reference platform), macOS, Windows.

### Local file systems

| File system | Where it appears | Rick behaviour |
| --- | --- | --- |
| ext2, ext3, ext4 | Linux | Standard POSIX semantics |
| XFS | Linux | Reflink copies (instant, space-shared) |
| Btrfs | Linux | Reflink copies; subvolumes shown as folders |
| ZFS | Linux (OpenZFS) | Datasets appear as mounts |
| NTFS | Windows; Linux via ntfs3 | Case-insensitive names, hidden attribute, ACLs |
| FAT12/16/32, exFAT | Removable media on all OSes | Case-insensitive, no permissions; FAT32 4 GB file limit flagged before a copy |
| HFS+, APFS | macOS | Case-insensitive by default; clonefile copies on APFS |
| ISO9660, UDF | Optical discs and .iso images | Read-only; an .iso file can also be browsed as an archive without mounting |

### Network file systems

NFS v3/v4, SMB/CIFS, CephFS, GlusterFS, SSHFS (FUSE) and 9p are mounted by the OS. Rick treats them as remote-latency mounts: no eager stat, background metadata, no thumbnailing by default, and connect, mount and unmount actions (udisks2 and GVFS on Linux). iSCSI is out of scope: it is block storage, and whatever the OS mounts on the attached disk is handled as a local file system.

### Object storage

| Service | Plugin | Notes |
| --- | --- | --- |
| AWS S3, Cloudflare R2, MinIO and other S3-compatible stores | `ext-vfs-s3` | One plugin with an endpoint setting; flat keys shown as folders by prefix |
| Azure Blob | `ext-vfs-azure` | Containers as top-level folders |
| Google Cloud Storage | `ext-vfs-gcs` | Buckets as top-level folders |

Each cloud is its own plugin implementing `Provider` directly, so a user installs only the clouds they use; there is no shared object-storage crate — the overlap (prefix-to-folder mapping, chunked upload, credential storage) is small, and a shared crate may be extracted later if the second plugin shows real duplication. Object stores have no change notifications (polling). Download and upload are ordinary cross-provider copies to and from the local provider, streamed through the machine. **Transfers inside one account never pass through the machine:** a copy or move between two locations of the same provider instance — within a bucket, or across two buckets or containers of the same account — is submitted to the service as a server-side operation (S3 `CopyObject`, Azure's asynchronous Copy Blob, GCS `Rewrite`; move adds a delete), and Rick's job only tracks its progress, polling where the service reports it and cancelling where the service allows. This is the `NATIVE_TRANSFER` capability of the Provider contract, not cloud-specific code: SFTP gets its server-side rename through the same hook.

### Archives and compression

| Format | Browse in place | Extract | Create | Crate |
| --- | --- | --- | --- | --- |
| zip (DEFLATE, zstd) | Yes | Yes | Yes | `zip` |
| tar, tar.gz, tar.zst, tar.xz, tar.bz2, tar.lz4 | Yes | Yes | Yes | `tar` + codec crates |
| 7z | Yes | Yes | Yes (LZMA, LZMA2) | `sevenz-rust` |
| RAR | Yes | Yes | No | `unrar`; creation is not licensable |
| gzip, zstd, xz/LZMA, bzip2, LZ4, Brotli, Snappy (single file) | — | Decompress | Compress | Preview by streaming decompression |

### Preview support

| Category | Formats | How |
| --- | --- | --- |
| Text | Any text file, encoding detected | Native text view |
| CSV | CSV, TSV | Virtualised grid |
| Code | JavaScript, TypeScript, Java, Rust, C#, F#, VB, C, C++, Perl, PHP, JSON, XML, YAML, HTML, Markdown | Syntax highlighting (syntect); Markdown and HTML also rendered |
| PDF | PDF | pdfium-render, page by page, text layer for search and selection |
| Office | DOCX, XLSX, PPTX, ODT, ODS, ODP | Converted to PDF by LibreOffice headless when it is installed (never bundled), then the PDF path; without it, metadata and a hint to install LibreOffice |
| Images | JPEG, PNG, WebP, BMP, GIF (animated) | `image` crate, downscaled to the pane, EXIF shown |
| Audio | MP3, WAV | Waveform and playback |
| Video | MP4, AVI | GStreamer on all platforms, hardware-decoded where available, frames to texture |
| 3D models | STL, OBJ, 3MF | Parsed natively, shaded mesh rendered through wgpu; orbit, zoom; triangle count and bounding box (0.2) |

### Source code repositories

| System | Integration | What Rick shows |
| --- | --- | --- |
| Git | Native, `gix` | Status badges per entry, folder roll-ups, branch and ahead/behind, stage/unstage/discard/diff actions |
| SVN | `svn` command line | Status badges, update and commit actions |
| TFVC | `tf` command line (Azure DevOps) | Deferred: not at launch, later as a plugin on demand (pending changes, check-in) |

## GUI

A side panel on the left gives one-click access to the places that matter; the main area holds the tabs, panes and preview.

&#91;embedded content: Rick main window wireframe · default layout\]

Default layout: the side panel is collapsible and can dock right; the explorer tabs and the Preview tab are all dockable, so the preview can sit below or in its own window; the status line and type-to-filter share the explorer's foot.

### Window chrome

Rick draws its own window chrome; the platform's title bar is turned off. The top of every window is one compact **title row**, the height of a tab, laid out like Visual Studio Code's:

- **Left:** the burger menu.
- **Centre:** the search box, which is also the command palette (below).
- **Right:** a small group of layout toggles, then the window controls (minimise, maximise, close) drawn by Rick. Which toggles sit there is still to be decided; the candidates are side panel, inspector and split. On macOS the traffic lights keep their place on the left and the burger menu moves right of them.

Dragging an empty part of the title row moves the window; double-clicking it maximises or restores. Where Rick draws the controls (Linux, Windows) the window is transparent behind a 10-point margin that holds a light shadow and a one-point border in the theme's window-stroke colour, with invisible resize grips along the edges and corners; a maximised or fullscreen window has no frame; macOS keeps the system's border and resize handles. Tabs do not live in the title row: each pane keeps its own tab strip directly below it, as VS Code's editor groups do, so a window with two panes shows two strips and the title row never changes when focus moves between panes.

**Search box.** One box serves search and commands. Plain text searches by name — the current location in 0.1, the platform index as well once a `SearchIndex` is available — and results drop down below the box; a leading `>` switches the box to commands, listing every registered Action fuzzy-matched as you type, each with its shortcut, Enter running the chosen one on the current selection and recently used commands floating to the top. Mod+P focuses the box empty, Mod+Shift+P focuses it with `>` already typed. Escape clears it and returns focus to the list.

**Spacing.** No wasted space, but room to breathe: controls and rows get VS Code-like padding rather than touching, and the row-density setting (compact, normal, comfortable) is the single knob that scales it.

**Icons.** Phosphor is the icon set, through the `egui-phosphor` crate, regular weight by default and fill weight for an active toggle. Every glyph in the application comes from it — toolbar buttons, tab icons, file-type icons, badges, menu entries — and no Unicode symbol character is ever used as an icon, so no glyph can fall back to a box. The navigation buttons are `ARROW_LEFT`, `ARROW_RIGHT` and `ARROW_UP`; refresh is `ARROW_CLOCKWISE`; copy is `COPY`.

### Side panel

Docked on the left by default; the user can move it to the right side and hide it entirely, with a shortcut (Mod+B) to toggle it back; a collapsed icon-strip mode comes later. Its side, width, visibility and folded sections are part of each window's session state, not of `rick.toml`. Its entries, top to bottom:

| Entry | What it shows | Notes |
| --- | --- | --- |
| Home | The user's home directory | Linux and macOS: `$HOME`; Windows: the user profile directory (`C:\Users\<name>`) |
| Recent | Files and folders recently changed anywhere in the file system | Fed by the watches on Home, the pinned folders and every open pane, plus a bounded scan at start (the newest 200 of up to 50 000 entries per root); newest first |
| Network | Network environment: discovered shares and hosts, mounted network file systems, configured object-storage connections | Connect, mount and unmount from here |
| Trash | The platform trash | Restore and empty |
| Pinned | Folders and files the user pins | Any path, local or remote; drag to reorder; dropping items from a file list here pins them (see Drag and drop) |
| Tree | The folder tree of the focused pane's volume | Lazy, follows the focused pane unless unlinked; the same tree view is also a dockable tab (Further GUI elements) |

Each entry is a section that folds and expands independently, so a user short on room can collapse Recent or Network and keep Pinned open. The panel's width is draggable, and the folded state and width persist across sessions.

**Operations area.** The bottom of the side panel shows running long operations, such as copying a large file, extracting an archive or an upload to object storage: one row per job with name, progress bar, throughput and remaining time, plus pause, resume and cancel. The area appears only while a job runs and folds away when the queue is empty; completed jobs stay listed briefly with their result, and a failed job stays until dismissed. When the side panel is hidden the area sits at the bottom of the window instead, so a running job is never invisible.

### Main area

The main area is the explorer view: each tab shows one location's files, with a toolbar above the file list.

**Toolbar.** One row per tab, made of three groups that never overlap: a fixed group of navigation icons on the left, the breadcrumb filling the middle, and a fixed group of view controls on the right. Only the breadcrumb flexes; the two icon groups keep their width whatever the window size.

| Group | Control | Action | Shortcut |
| --- | --- | --- | --- |
| Left | Back (`ARROW_LEFT`) | Previous location in this tab's history | Alt+Left, mouse button 4 |
| Left | Forward (`ARROW_RIGHT`) | Next location in history | Alt+Right, mouse button 5 |
| Left | Up (`ARROW_UP`) | Parent directory | Alt+Up, Backspace |
| Centre | Breadcrumb | The current path; see Path bar | Mod+L to edit |
| Right | View mode | A dropdown; see View mode control | Mod+Shift+1 … 4 |
| Right | Hidden files | Toggle | Mod+H |
| Right | Refresh (`ARROW_CLOCKWISE`) | Re-read the location | F5 |

Other controls can be added to the right group later; the toolbar is an extension point for Actions with toolbar placement.

**Path bar.** The breadcrumb shows the current path as clickable segments; clicking a segment jumps there, clicking the empty area or pressing Mod+L turns it into an editable text field, Escape leaves edit mode. It never overlaps its neighbours: when the path is wider than the space between the two icon groups, segments collapse from the left into a single chevron (`CARET_DOUBLE_LEFT`, or `DOTS_THREE` with a chevron) that drops down the hidden ancestors, the rightmost segments stay visible, and if even the last segment does not fit it truncates with an ellipsis. A copy button (`COPY`) sits at the right end of the breadcrumb, inside it, and copies the folder's path — the Copy path action below.

**View mode control.** One dropdown in the toolbar's right group. Its icon shows the current mode (`LIST` for List, `LIST_DASHES` for Details, `TREE_STRUCTURE` for Tree, `SQUARES_FOUR` for Icons); the menu lists List, Details, Tree, and Icons in small, medium, large and extra large, with the current entry marked. The icon sizes show thumbnails where a previewer can supply one, so the large sizes work as an image browser.

**Tabs and panes.** Any number of tabs, each an explorer view with its own toolbar, path and history (Mod+T new, Mod+W close, Mod+Tab cycle; middle-click a folder opens it in a new tab). Tabs are not only stacked: a tab can be picked up and dropped to the left, right, top or bottom of the current one, splitting the area into side-by-side or stacked panes, as in an IDE. Drop targets appear while dragging; splits nest, so two-pane, three-pane and grid layouts are all possible; dividers drag to resize. Dropping a tab on another pane's tab strip moves it into that pane at the drop position; dropping it on a pane's edge zone splits; dropping it outside the window opens a new window. The layout, tabs and their paths are restored on the next start. This is egui\_dock's tree of tabs and splits.

**Tab strip.** Each pane's strip sits at the top of the pane, below the title row. A tab shows an icon, then its name: the folder icon for a local folder, the provider's icon otherwise (archive, trash, remote), so tabs read differently at a glance. The close button (`X`) appears only on the active tab and on the hovered tab; a middle click closes any tab. Tabs have VS Code-like padding inside and between them and slightly rounded top corners (about 4 px); the new-tab button (`PLUS`) sits directly after the last tab, not at the right edge of the strip. When tabs outgrow the strip, the strip scrolls horizontally and the plus button stays reachable at the end of the row.

**File list.** A details list, one row per entry, folders first. Default columns in order:

| Column | Content |
| --- | --- |
| Icon + Name | An icon for what the entry is (folder, text, image, archive, video, and so on), then the name |
| Type | The file type as text, e.g. Folder, PNG image, Rust source |
| Date modified | Last modification time, relative for recent entries, full on hover |
| Size | Human-readable; folders show nothing until a size is requested |

Columns sort on click, resize, reorder and can be hidden; further columns may be added later, and extensions can add columns through Decorators (for example a Git status column). The body of the list draws no grid lines: no vertical column separators and no horizontal row lines, rows are set apart by their padding alone, which follows the row-density setting. The only vertical separators are in the header row, between column titles, where they double as the resize handles. An optional setting alternates the row background colour (zebra striping) for easier reading of wide rows; it is off by default.

**Type filter.** A filter control on the toolbar lists only the file types actually present in the current directory, with a count per type; ticking one or more shows just those entries. The filter is per tab, clears on navigation unless pinned, and a text filter (name contains) sits beside it.

**Status line.** A small stats line at the bottom of each explorer view shows how many folders and how many files the current directory holds, the total size of the files, and, when something is selected, the selected count and size. With a filter active it reads "12 of 340 files".

**Sorting.** Default order is by name, alphabetically; clicking a column header sorts by that column (name, type, date modified, size) and clicking again reverses it. A separate "folders first" toggle, on by default, keeps all folders above all files whatever the sort column; switched off, folders and files are mixed into one order. Both settings persist per user, with an optional per-folder override.

**Type-to-filter.** Typing while the list has focus filters it live: the typed text appears in a small bar at the foot of the list, only entries whose name contains it stay visible, Escape clears it and Enter opens the first match. This replaces the classic "jump to first entry starting with the letter" behaviour.

**Search in subdirectories.** A toggle on the filter bar (or Ctrl+Shift+F) extends the same text to the whole subtree: results stream into the list as the walk proceeds, each row showing its relative path, and the walk is cancelled the moment the text changes or the toggle goes off. The walk uses the `ignore` crate's parallel walker, respects a configurable ignore list, and is a cancellable job in the operations area when it runs long.

**Indexed search.** If the platform has a search index, a search box appears on the toolbar: it queries Windows Search, Spotlight or, on Linux, Tracker or Baloo through a search-index extension, and shows results in the list with their paths. Without an index the box is absent, not greyed out; detection happens at start and when the index service appears or disappears. Each index connection is its own plugin (search-windows, search-spotlight, search-tracker, search-baloo) against one \`SearchIndex\` trait in \`ext\`, so a connector can be replaced, and a future Rick-owned index would be just another plugin implementing the same trait.

**Context menu.** Right-clicking a file, folder or selection opens a full operations menu, built from the Action registry so extensions add entries in their own group. Right-clicking the empty area of the list — below the last row, or anywhere in an empty folder — clears the selection and opens the same menu for the current folder. Default groups:

| Group | Entries |
| --- | --- |
| Open | Open, Open with (app list), Open in new tab, Open in terminal here, Pin to side panel / Unpin |
| Edit | Cut, Copy, Paste, Duplicate, Rename, Move to, Copy to |
| Delete | Move to trash, Delete permanently |
| Create | New folder, New file (from templates) |
| Share | Copy name, Copy path, Copy path and name, Copy as link, Compress to archive, Extract here |
| Extensions | Git actions, archive actions, and anything a plugin registers |
| Info | Properties |

Entries that do not apply are hidden rather than greyed: each Action's `availability` decides, so with no selection the menu holds only what works on the folder itself — New folder, New file, Paste, Paste as link, Open in terminal here, Open in system file manager, Select all, Refresh, the view, sort and group entries, and the folder's Properties — and nothing that needs an item. `Hidden` and `Disabled` entries are both absent from the context menu; the search box in command mode (Mod+Shift+P), which lists everything, shows `Disabled` ones greyed with their reason as a tooltip. The same Actions are reachable from keyboard shortcuts.

### Preview

The preview is its own tab, not part of any explorer tab. It docks and moves like every other tab (right of the explorer by default, or below, or torn out to a second monitor), and there is one of it. In 0.1 it is a panel on the right of the window, toggled with "Toggle preview pane"; it becomes a dock tab together with the viewer in 0.2.

- It follows focus: it always shows the selection of whichever explorer tab was last focused. Select a file in tab A, switch to tab B and select a file there, and the preview switches to that file.
- Space on a file or folder opens the preview tab if it is closed and shows the entry; Space again closes it. While open, moving the selection updates it live.
- A file shows its contents through the Previewer registered for its type (see Supported scope); a folder, or an archive such as a zip, 7z or tar, shows its contents as a read-only file list in the same columns as the explorer, with a stats line; subfolders expand in place, so a user can look inside without navigating or extracting.
- Rendering is asynchronous and cancellable: a quick selection sweep never blocks the list, and a preview that takes longer shows a placeholder first.

**Pane layout.** The content is the point of the pane; everything else is one row. Top to bottom: the toolbar, the pinned info panel when one is pinned, the content, and a one-line status bar. The pane shows no heading of its own — the dock tab's label is enough — and the word "Inspector" appears nowhere in it.

**Toolbar.** One row, tab-height, three parts:

- **Left:** the file-type icon of the current entry (Phosphor), and nothing else — no file name.
- **Middle:** the active previewer's controls, so the row changes with the content. Text and code: line numbers, word wrap, and pretty print when the format supports it. Images: Fit and 1:1. Paged documents: previous page, `n / N`, next page. Media: the transport. Controls come from two sources — the kind of document (text layer, pages, raster) supplies the standard ones, and the previewer's `controls()` adds its own, such as pretty print.
- **Right:** one **info badge** per section available for this file — File info, EXIF, Git, media stream details, whatever applies — then the close button. A section that does not apply has no badge. Sections come from the previewer's `sections()` and from the Decorators' `panel()` contributions (Git's branch and last commit, for example).

**Info badges.** Hovering a badge shows its section as an overlay card over the content, like a tooltip; moving away closes it. Clicking a badge pins it: the section becomes a panel between the toolbar and the content, pushing the content down, and the badge stays highlighted until it is clicked again. One section is pinned at a time; pinning another replaces it. The pin is remembered per content kind — `pages` (anything with `pages()`), `text` (a text layer, no pages), `image` (a raster frame), `other` (no previewer, a decorator section only) — and persists across files and restarts, so a pinned EXIF stays pinned while arrowing through photos. Each section is a key–value list, searchable when long, with copy on every row.

**Text defaults.** Text and code open with line numbers on and word wrap off. Toggling either in the toolbar changes the current view and saves the new value as the default, so the choice holds for every later file and across restarts. Pretty print is the exception: it is off for every newly shown file, must be pressed each time, and is never remembered.

**Width.** The content never sets the pane's width. The pane is sized by the dock layout and the user's divider alone; the previewer is asked to render at the pane's width, a page or image wider than that is scaled down to fit (keeping its aspect ratio, never enlarged above 100%), and long text lines scroll sideways inside the pane. Nothing drawn in the pane may request a minimum width beyond a small fixed floor, so a wide PDF page shrinks rather than pushing the divider.

**Status bar.** The bottom of the pane is a single line of small text with the previewer's `status()` facts, separated by `·`: language, encoding and line count for text and code; dimensions, format and file size for an image; page count and file size for a PDF; duration and codec for media. It never wraps; when the pane is too narrow it truncates with an ellipsis.

### Menu

There is no menu bar; the burger menu at the left of the title row holds the global entries.

- The command palette has no menu entry: it is the title row's search box in command mode (Mod+Shift+P, or type `>`), always visible.
- Entries: New window, Settings, Extensions, Keyboard shortcuts, About.

The remaining menu entries will be specified later.

### Further GUI elements

**Tree view.** A folder tree that expands in place, available both as a section in the side panel and as its own dockable tab; selecting a node navigates the focused explorer tab, and the tree follows the explorer's location unless unlinked.

**Theming.** Light and dark themes, following the system by default, with a manual override; font size and row density (compact, normal, comfortable) are settings, and accent colour can be chosen. Syntax highlighting themes follow the app theme.

**Hidden files, symlinks and mounts.** A toggle (Ctrl+H) shows dot-files and files with the hidden attribute. Symlinks show a link badge on the icon and their target in the Type column; a broken link is marked. Mount points show a volume badge, and the side panel's Network and Pinned sections list mounted volumes.

**Icon and thumbnail view.** Besides the details list, a grid view with large icons or thumbnails for images, PDFs and videos, in four cell sizes (small, medium, large, extra large). A thumbnail is rendered by the Previewer registered for the entry's type, at the cell size, off the UI thread, and cached per entry and size; while it renders, or when no previewer claims the file, the cell shows the file-type icon. The freedesktop thumbnail cache on Linux may seed that cache later. This view ships in 0.1 (see the definition of done).

**Dual pane.** One command (F6) lays out two explorer tabs side by side, the classic commander layout; a second press returns to a single pane.

**Inline rename.** F2, or a second click on the single selected row at least 0.5 s after the first (past the double-click window), edits the name in place in the list, with the extension excluded from the initial selection; Enter commits, Escape cancels, a clashing name is refused in place. New file and New folder insert a row in the same edit mode at the top of the list.

### General behaviour

**Session restore.** Closing Rick and opening it again brings back exactly what was left: every window with its position and size, its dock layout, every tab with its path, sort, filter and scroll position, and the side panel's state. The session is saved continuously, so a crash loses nothing but the last few seconds. A tab whose location no longer exists opens at its nearest existing ancestor with a note in the status line; one whose provider is not mounted opens at Home.

**Saved layouts.** A user can arrange panels and tabs, then save that arrangement as a named layout and mark it as the start layout; Rick then opens with that layout instead of the last session. Layouts are listed in the menu and the command palette for switching at any time; the last session remains available as "Continue where I left off".

**Windows.** New window is one click (menu, Ctrl+N, or the + on the tab strip with Shift) and opens with the start layout. Dragging a tab out of the tab strip and dropping it outside the window creates a new window holding that tab; dropping a tab onto another window's strip moves it there. Every window has the full side panel and shares the operations queue, the preview follows focus across windows.

**Text files.** The preview shows the file's text in a monospace view with two toggles in the pane's toolbar: word wrap (off = lines keep their real length and the view scrolls sideways; on = long lines break at the pane width at any character, so a line's row count follows from its length and the view stays virtualised; a wrapped continuation is marked in the gutter) and line numbers. The defaults are line numbers on and wrap off; changing either saves the new default (see Preview, Text defaults). Large files are never loaded whole: the previewer memory-maps the file (or reads windows of it) and renders only the visible lines, so a 500 MB log shows its top immediately and scrolls through the rest; line starts are indexed in the background for exact line numbers and scrollbar position, with a byte-offset estimate until that finishes, and highlighting and pretty print work on the visible window only. Memory use is bounded by the window, not the file size, and encoding is detected. Structured and code formats get syntax highlighting: JSON, YAML, XML, HTML, Markdown and every language in the preview list, detected by extension and, failing that, by content; the highlighter is syntect with the theme following the app theme, and the format name shows in the pane's status bar.

**Pretty print.** For JSON, XML and minified HTML in 0.1 — YAML, CSS and JavaScript when each has a formatter — a Pretty-print toggle in the pane's toolbar reformats the text with consistent indentation and one element per line; it is a view transformation only, the file is never modified, and the raw view is one click away. It is off for every newly shown file and is never remembered. Invalid input shows the raw text with the parse error highlighted.

**Images.** By default the image is scaled to fit the available pane, keeping its aspect ratio and never upscaled beyond 100%. A 1:1 button (or double-click) switches to actual pixels, where the image can be panned by dragging or with the scroll wheel, and Ctrl+wheel zooms in steps around the cursor; Fit returns to the scaled view. EXIF orientation is applied. Large images decode a downscaled version first for an instant fit view, with the full-resolution decode following for 1:1.

**Media metadata.** Image, audio and video previews put the basics in the pane's status bar — dimensions or duration, file size, format, colour depth or codec — and the rest behind info badges: a File info section with the camera, date taken and exposure for photos, an EXIF section with every EXIF tag (via `kamadak-exif`; IPTC and XMP when a reader without a system library is adopted), ID3 or Vorbis tags for audio, and stream details for video. Each is a searchable key-value list with copy, shown on hover and pinned on click as the Preview section describes.

**Audio and video.** Media files play inside the preview: play/pause (Space while the preview has focus), a seek bar with elapsed and total time, volume, and for video the frames drawn into the pane with a fit/1:1 toggle like images. Audio shows a waveform and cover art when present. Playback starts on demand, not on selection, stops when the selection changes, and decoding runs through GStreamer on every platform, one code path, with the runtime bundled on macOS and Windows.

**Previews are modules.** Every preview is an extension exposing one or more Previewers, as the architecture section describes, and each ships as its own module that a user can enable or disable. One module may cover several related types, but the split follows who needs it, not the file type: a plain-text previewer is part of the core, while syntax highlighting, pretty print, PDF, Office, images, audio and video are separate modules, so a user who is not a programmer never loads the code tooling. When several previewers match a file, the highest score wins and the user can override the choice per type in settings.

Proposed modules: preview-text (core), preview-code (highlighting, pretty print), preview-pdf, preview-office, preview-image, preview-media (audio, video), preview-archive (folder and archive listing).

**Extensions as installable packages.** Modules are not only switchable, they are physically separate: each is a package in a known plugins directory that can be removed from the disk or simply never loaded, and the core binary carries only the local file system, the text, code, image and PDF previewers and the core actions (see Package layout). A package holds a manifest (id, version, API version, the extension points it fills, the platforms it supports, the toolchain it was built with) plus its code as a native dynamic library — a WASM module later; the loader reads the manifests at start, registers each plugin's contributions, and defers heavy initialisation until a matching file or action is first needed. The Extensions screen (0.2) lists what was found with enable and disable.

**Extension store.** Later. The Extensions screen grows update and remove, and browses a store of available packages: a signed index hosted by the project, with official modules (the preview set above, Git, archives, object storage) and third-party ones. Install downloads the package for the current platform, verifies its signature and hash, and activates it without a restart. Store entries declare the permissions they need (file system scope, network, process spawn); the user sees them before installing.

**Viewer.** The full-size counterpart of the preview pane. Enter or a double-click on a file of a previewable type opens it in the viewer rather than in an external application (a setting flips that default per type); Ctrl+Enter forces the external application. The viewer opens as a tab in the current pane, so it can be split, docked or torn out into its own window like any other tab, and a window-mode viewer fills the screen with F11. It uses the same previewer modules as the preview pane, with the full feature set unlocked:

- **Navigation.** Left and Right, or PageUp and PageDown, step to the previous and next file of the same folder in the list's current sort and filter, so the viewer doubles as a slideshow; Home and End jump to the first and last; the title shows name and position ("12 of 240").
- **Pages.** For paged content (PDF, Office, multi-page TIFF) a page strip on the side, a page counter with direct entry, and continuous or single-page scrolling.
- **Zoom.** Fit, fit width, 1:1 and free zoom (Ctrl+scroll, Ctrl+plus and minus, Ctrl+0 to reset), with panning by drag or scroll; the zoom level is remembered per type.
- **Find.** Ctrl+F searches the text layer of text, code, PDF and Office content, with match count, next and previous, and highlights in place.
- **Media.** Audio and video get full transport: seek bar, volume, playback speed, frame step, loop, and a keyboard set matching common players (Space, J, K, L, arrows).
- **Info.** The media info bar from the preview is available as a collapsible side panel, with all metadata.
- **Actions.** The viewer exposes the same Action registry as the list, so the current file can be renamed, trashed, copied, opened with or pinned from inside the viewer, and the list stays in sync.
- **Unsupported types.** Falls back to a hex view with ASCII column for anything no previewer claims.

The viewer never writes to the file; editing is the job of an external application via Open with.

## Functions

**Drag and drop.** Dragging a selection out of a file list starts a drag; where it lands decides what happens. Every target highlights while the cursor is over it.

| Target | On drop |
| --- | --- |
| A folder row in any pane | Move (or copy, see below) into that folder |
| The empty space of a pane, or its background | Move into that pane's current folder |
| A breadcrumb segment | Move into that ancestor folder |
| A tab header | Move into the folder that tab shows |
| The Pinned section of the side panel: its header, its empty space, or any pinned entry | Pin the dragged items, inserted at the drop position; never a file operation |
| The Trash entry of the side panel | Move to trash — the same job and the same undo as the action |
| The Home entry, or a mounted volume under Network | Move into that location |
| Outside the window: another application, or another Rick process | Hand over through the platform drag-and-drop with file URIs; the receiver decides. A drop arriving without a cursor position (the backend reports none on some platforms) lands in the focused pane's folder |

*Move or copy.* Inside one Rick process the rule is by volume: same volume moves, across volumes copies. Mod forces a copy, Shift forces a move, and Alt (or a right-button drag) asks on drop. Anything arriving from outside the process — another application, a browser, a second Rick window of another process — is always a copy, whatever the volume; Shift forces a move. The drag ghost shows the first item's icon and name, a count badge when several items are dragged, and a small indicator under the cursor — `ARROW_RIGHT` for move, `PLUS` for copy, `PUSH_PIN` for pin, `TRASH` for trash — that updates live as the target and the modifier keys change, so the outcome is visible before the button is released.

*Spring-loading.* A drop on a folder row moves at once. Holding over a folder row for about a second (settable) navigates the pane into that folder while the drag continues, so the drop can land on a subfolder or on the empty space inside; holding over a subfolder drills down again, and holding over a tab header switches the pane to that tab the same way. When the drag ends or is cancelled the pane stays where it drilled to; a setting, *After a drag: stay in the folder / return to where the drag started*, defaults to stay. A drag counts as one navigation: the starting location is pushed on the tab's history once, the folders passed through are not, so one Back returns to the starting location with its scroll position and selection restored.

*No-ops and cancel.* Escape cancels a drag in progress and the ghost disappears. Dropping items onto the folder they are already in, a folder onto itself or into its own subtree, or a file onto a file does nothing and shows no error; the target simply does not highlight.

*Execution.* A move or copy runs as a job in the operations area with progress, pause and cancel; a conflict on an existing name pauses the job and asks once with Skip, Overwrite, Rename, Apply to all and Cancel — overwriting a folder with a folder merges (same-named files replaced, the rest kept), a file replaces a file, a folder onto a file or the reverse removes the target first; Rename proposes `name (2).ext`, and applied to all numbers each remaining name with its first free number; the undo of a move that overwrote brings the item back but not the overwritten target. A move within one provider uses rename, a copy uses the provider's fast path. Pin and trash go through their Actions, so they appear in the undo log and the palette like any other operation. Across processes, the receiving Rick applies these same rules from the moment the cursor enters its window; the indicator and spring-loading work there too.

**Permission management.** Properties on a file or folder shows and edits ownership and permissions: owner, group, mode bits (rwx for user, group, other, plus setuid, setgid, sticky) on Linux and macOS, ACLs where the file system has them (POSIX ACLs, NFSv4 ACLs, NTFS ACLs on Windows), and an apply-recursively option with a separate choice for files and folders. Changes that need elevation ask for it through the platform (polkit, sudo, UAC). Details to be discussed.

### Operations

- **Flatten subfolders.** A toggle that shows the contents of all subfolders in the current list, each entry with its relative path, instead of only the current level; off again returns to the normal view.
- **Toggle inspector.** Show or hide the inspector (the preview and info pane).
- **Split right.** Split the current pane and open the same location in a new pane to the right.
- **Split down.** Same, with the new pane below.
- **New tab.** Open a new tab at the current location.
- **Close tab.** Close the active tab.
- **Open options.** Open the application settings.
- **Open console.** Open a terminal at the current location.
- **Open in system file manager.** Reveal the current location in the platform's default file explorer.
- **Toggle hidden files.** Show or hide hidden files.
- **Toggle file extensions.** Show or hide file extensions in the name column.
- **Toggle system files.** Show or hide system files.
- **Open in new tab.** Open the selected folder in a new tab, leaving the current one in place.
- **Copy.** Put the selection on the clipboard for copying. Rick's file clipboard is its own, in memory (entries plus a copy-or-move mode; Paste after Cut empties it, Paste after Copy keeps it); only the Copy name / path / path and name actions write the system clipboard. Exchanging file lists with other applications through the system clipboard is later.
- **Cut.** Put the selection on the clipboard for moving.
- **Paste.** Copy or move the clipboard contents into the current location, running as a job with the usual conflict handling.
- **Create link.** Make a link to the selected file or folder at another location. On Linux and macOS this is a symbolic link; on Windows the user picks between a symbolic link, a directory junction and a classic .lnk shortcut. 0.1 offers symbolic and hard links only (`LinkKind::{Symbolic, Hard}`); junctions and .lnk shortcuts come in a later issue. Also offered as a drag modifier (Alt) and as a paste variant ("Paste as link").
- **Move to trash.** Send the selection to the platform trash (Delete key), restorable from the Trash entry in the side panel.
- **Delete permanently.** Remove the selection without going through the trash (Shift+Delete), after confirmation.
- **Duplicate.** Make a copy of the selection in the same folder with a derived name, such as "name (copy)".
- **Inspect.** Open the system properties for the selection: the Windows Properties dialogue, the macOS Get Info window, and on Linux Rick's own properties dialogue (size, dates, owner, permissions, mime type), since there is no desktop-wide one.
- **Restore.** Put a trashed item back at its original location; available when browsing Trash. In 0.1 macOS can restore only items Rick trashed in the same session, because the system keeps the original paths of trashed items in a private store; and an original path that is taken again is reported, not resolved (QUESTIONS.md 11, to revisit with the Trash location).
- **Copy name.** Copy the file name of each selected item to the clipboard (`report.pdf`), one per line for multiple items (Mod+Alt+C).
- **Copy path.** Copy the folder path of each selected item, without the name (`/home/serge/docs`), one per line, duplicates removed (Mod+Alt+Shift+C). With nothing selected, and from the breadcrumb's copy button, this is the current folder's path.
- **Copy path and name.** Copy the full path of each selected item (`/home/serge/docs/report.pdf`), one per line (Mod+Shift+C). The three sit together in the Share group of the context menu.
- **New file.** Create an empty file in the current location with its name in inline edit; extensions can add templates (text, markdown, and so on).
- **New folder.** Create a folder in the current location with its name in inline edit; with a selection, a variant moves the selected items into the new folder.
- **Open containing folder.** Navigate the current pane to the folder that holds the selected item and select it there; useful from search results, the flattened view and Recent.
- **Open containing folder in new tab.** The same, in a new tab.
- **Show only folders.** Filter the list to folders.
- **Show only files.** Filter the list to files. The two are exclusive; selecting the active one again shows everything.
- **Size all columns to fit.** Resize every column to its widest visible content; a double-click on a column divider does the same for that column.
- **Filter.** Focus the text filter for the current list (Ctrl+F); Escape clears it and returns focus to the list.
- **Refresh.** Re-read the current location (F5), for providers without change notifications or when in doubt.
- **Go to root.** Jump to the top of the current volume or provider: the drive root on Windows, the filesystem root on Linux and macOS, the bucket root for object storage.
- **Back.** Go to the previous location in the tab's history (Alt+Left, mouse button 4).
- **Forward.** Go to the next location in the tab's history (Alt+Right, mouse button 5).
- **Empty trash.** Permanently delete everything in the trash, after confirmation.
- **Restore trash items.** Put the selected trashed items back where they came from; the Restore operation above, for several items at once.
- **View mode.** Switch the current pane between view modes, each remembered per tab: *List* (name and icon only, in columns that wrap), *Details* (the default: sortable columns), *Tree* (expandable folders inline in the same list), and *Icons* in small, medium, large and extra large sizes with thumbnails where a previewer can supply one. Ctrl+scroll steps through the icon sizes and into list and details.
- **Sort by.** Sort the list by name, size, date created, date modified, date accessed, total items (entry count for folders) or type; each as ascending or descending, remembered per tab, with the folders-first toggle applied on top.
- **Group by.** Group the list under collapsible headers by name (first letter), size (bands such as tiny, small, medium, large, huge), date created or date modified (today, yesterday, this week, this month, earlier) or type; sorting applies within each group.
- **Clear grouping.** Return to the flat list.
- **Duplicate tab.** Open a new tab next to the current one with the same location, view mode, sort, filter and selection.
- **Duplicate tab to new window.** The same, into a new window.
- **Move tab left / right.** Shift the active tab one position within its pane (Ctrl+Shift+PageUp / PageDown).
- **Close other tabs.** Close every tab in the pane except the active one.
- **Close tabs to the left / to the right.** Close all tabs on that side of the active one.
- **Close all tabs.** Close every tab in the pane, leaving one fresh tab at Home.
- **Reopen closed tab.** Restore the most recently closed tab with its location and history (Ctrl+Shift+T), repeatable through the closed-tab stack.
- **Next tab / previous tab.** Cycle through the tabs of the pane (Ctrl+Tab / Ctrl+Shift+Tab).
- **Select first tab.** Jump to the first tab (Ctrl+1); Ctrl+2 to Ctrl+8 select by position and Ctrl+9 the last.
- **Rename.** Edit the name inline (F2), with the base name pre-selected and the extension left alone; with several items selected, opens the bulk rename dialogue (pattern, numbering, find and replace, preview of the result).
- **To lower case / to upper case.** Rename the selected items to all-lower or all-upper case names in one step, extension included; on case-insensitive filesystems this goes through a temporary name.
- **Select all / invert selection.** Select everything in the list (Ctrl+A) or swap selected and unselected.
- **Select by pattern.** Select the entries matching a glob or regular expression entered in a small prompt (Ctrl+Shift+A), adding to or replacing the current selection.
- **Undo / redo.** Reverse the last move, rename, trash, new file or new folder (Ctrl+Z / Ctrl+Shift+Z); copies and permanent deletes are not undoable and say so.
- **Open with.** Open the selection in a chosen application from the platform's list for that type, or any executable; the choice can be remembered for the type.
- **Compress.** Pack the selection into an archive (zip by default, other formats in a small dialogue) next to it.
- **Extract here / extract to.** Unpack the selected archive into the current folder, into a folder named after it, or to a chosen location.
- **Compare.** Compare the two selected files, or the folders of the two panes, in a diff view; file contents via a text diff, folders by name, size and modified date with the differences highlighted.
- **Pin to side panel.** Add the selected items — folders or files — to the Pinned section, at the end, or at the drop position when pinned by drag. A pinned folder opens in the current pane; a pinned file opens its folder with the file selected. The action sits in the context menu of every entry and in the palette; on an already pinned item it reads **Unpin**, and the Pinned entry's own context menu offers Unpin too. Pin and Unpin are undoable.

### Keyboard shortcuts

All shortcuts are remappable in Settings → Keyboard shortcuts. The binding tables live under "Keyboard shortcuts" at the end of this document; that table is the single source of defaults.

## Architecture

Rick is a small kernel with five extension points; every feature, built in or third party, plugs into them, and dependencies only point down the layers.

&#91;embedded content: Rick architecture · 6 layers, 4 extension points\]

`types` holds the data that crosses boundaries; `vfs`, `runtime` and `platform` are the only crates that touch the OS; `ext` is the one API an extension sees; `app` wires everything together; `ui-egui` is the only crate that knows egui.

### Extension points

All five live in `ext`, each small enough for one screen, each returning data or jobs and never drawing.

| Point | Answers | Contract |
| --- | --- | --- |
| Provider | Where files come from: local, archive, SFTP, object storage | list streamed in chunks, stat, open, watch, copy, delete, plus a capability set (trash, rename, reflink, watch) |
| Decorator | Status on entries or a directory: git badges, sync state, extra columns | interested(dir) as a cheap gate; decorate(visible entries or directory) as a cancellable batch job; invalidates\_on(dir) names the file events that make it stale |
| Action | A function for a selection or location | id, label, enabled(ctx), run(ctx) as a job, placement (menu group, palette, shortcut) |
| Previewer | Rendering a file type | score(mime, path); open(source) returns a Preview with pages, render(page, size) to RGBA frames, a text layer and metadata |
| SearchIndex | Querying a file search index: Windows Search, Spotlight, Tracker, Baloo, or a future Rick-owned index | available() detected at start and on service change; query(text, scope) streamed as a cancellable job; capabilities (names only, content, metadata) |

A packaged feature implements `Extension`: a manifest (id, version, settings schema) and `register`, which contributes to any of the five points. Grouping is by domain, not by point: `ext-git` holds one Decorator, several Actions and the repository logic; `ext-preview-image` holds one Previewer and nothing else.

### Module rules

1. One crate, one noun; split before it needs a second README section.
2. Extensions depend on `ext` and `types` only; the core never depends on an extension.
3. No thread spawn outside `runtime`; every background task is a cancellable, prioritised job.
4. Extensions return data and jobs and never draw; a panel contributed by an extension is a `Panel` trait that lives in `ui-egui` only.
5. `OsStr` for names everywhere; `cfg(target_os)` only inside `vfs-local` and `platform`.
6. Built-ins register in `app` exactly like plugins; `plugins-wasm` adapts guest modules to the same five traits.

### Package layout

One Cargo workspace, one crate per concern, grouped by layer. Kernel crates carry no feature code; everything a user would call a feature is an `ext-*` crate that registers through `ext`. Official extensions compile into the binary behind Cargo features, so a lean build is a matter of turning features off.

**Kernel**, in dependency order: each crate may depend only on the crates above it, and cargo-deny enforces the order. `ext` sits below `vfs` because `vfs` and `vfs-local` implement the `Provider` trait it defines.

- `types` — paths, entries, mime, errors, capability flags; no I/O, no async runtime and no dependency on any other Rick crate. It may use the small ecosystem crates it needs (`bitflags`, `thiserror`), nothing heavier.
- `runtime` — job system, cancellation, progress, thread pools, event bus.
- `platform` — per-OS code behind one trait set: trash, links and junctions, elevation, system properties dialogue, file associations, mounts. In 0.1 the set is `Trash` (trash, list, restore, remove one item, empty), `OpenWith` (`open` with the default application, `reveal` in the system file manager), `Properties` (the native dialogue on Windows and macOS, label/value rows on Linux), `NativeDialogs` (async, through `rfd`, the XDG portal on Linux), `Clipboard` and `ConfigDir`; elevation, file associations and mounts come later. The three RustSec "unmaintained" notices on `rfd`'s D-Bus dependencies are accepted in deny.toml with the reason recorded, to be dropped when `rfd` moves on.
- `ext` — the five extension traits, the registry, manifests and the extension context.
- `vfs` — the mount table that routes a path to its `Provider`, and cross-provider copy and move.
- `vfs-local` — the local filesystem provider: listing, statx, watching, copy and reflink.
- `model` — the directory model: cached listings, sort, filter, group, selection, history.
- `ops` — the file operations as jobs: copy, move, trash, delete, rename, link, undo stack, conflict policy.
- `app` — the application state: windows, panes, tabs, layouts, session persistence, settings, the command palette.
- `ui-egui` — all egui widgets and the dock layout; depends on `app`, never the other way round.
- `rick` — the binary: feature flags, wiring, startup.

**Providers** (`ext-vfs-*`)

- `ext-vfs-archive` — zip, tar family, 7z, RAR read-only, single-file codecs.
- `ext-vfs-s3` — S3, R2, MinIO and other S3-compatible stores, one endpoint setting.
- `ext-vfs-azure` — Azure Blob.
- `ext-vfs-gcs` — Google Cloud Storage.
- `ext-vfs-sftp` — SSH and SFTP.
- `ext-vfs-trash` — the trash as a browsable location with restore.

**Decorators** (`ext-deco-*`)

- `ext-git` — one plugin, several contributions: a Decorator (status badges, branch, ahead and behind, via gix), an Action group for the context menu (Commit, Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log, Blame), and an Info panel section (current branch, last commit). The Action group appears under "Git" in the Extensions part of the context menu and in the command palette, only when the location is inside a repository.
- `ext-svn` — the same shape through the svn command line: Decorator plus Update, Commit, Revert, Log actions.
- `ext-deco-fs` — later: Btrfs subvolumes, ZFS snapshots, NTFS alternate streams.

**Previewers** (`ext-preview-*`)

- `ext-preview-text` — plain text, encodings, huge-file windowing, pretty print.
- `ext-preview-code` — syntax highlighting via syntect, on top of text.
- `ext-preview-image` — raster and SVG decode, EXIF, IPTC, XMP.
- `ext-preview-pdf` — pdfium-render; pdfium is a dynamic library that every build and release places beside the binary (`RICK_PDFIUM_DIR` overrides for development).
- `ext-preview-office` — LibreOffice headless to PDF, then the PDF path.
- `ext-preview-media` — audio and video via GStreamer, ID3 and stream info.
- `ext-preview-archive` — listing of an archive's contents without mounting it.
- `ext-preview-csv` — CSV and TSV as a virtualised grid.
- `ext-preview-3d` — STL, OBJ and 3MF meshes rendered through wgpu.
- `ext-preview-hex` — the fallback hex view.

**Actions** (`ext-action-*`)

- `ext-action-core` — the built-in operations list: clipboard, trash, rename, new, link, select, view and sort commands.
- `ext-action-compress` — compress and extract.
- `ext-action-compare` — file and folder diff.
- `ext-action-terminal` — open console, open in system file manager.

**Search** (`ext-search-*`)

- `ext-search-walk` — the non-indexed subdirectory search via the `ignore` walker.
- `ext-search-windows`, `ext-search-spotlight`, `ext-search-tracker`, `ext-search-baloo` — one connector per platform index.

**Plugin host**

- `plugins-native` — discovery, manifest checks and loading of native plugin libraries from the plugins directory; the host for every `ext-*` crate that is not compiled in (0.2).
- `plugins-wasm` — later: the WASM runtime and the adapters that expose the five traits to guest modules.

Compiled into the binary: `vfs-local`, `ext-preview-text`, `ext-preview-code`, `ext-preview-image`, `ext-preview-pdf` and `ext-action-core`. Every other `ext-*` crate builds as a plugin library into the plugins directory; the Cargo features on the binary remain for a build that compiles a plugin in instead.

Naming rule: a crate that contributes to one extension point carries that point as its prefix (ext-preview-pdf). A crate that is a feature in the user's eyes and contributes to several points, like Git, carries only the feature name (ext-git) and registers a Decorator, an Action group, a Previewer or whatever it needs from one manifest; the user installs, enables and disables it as one thing.

### Build order

types → runtime → vfs-local with the streamed listing → model → ext with a fake Decorator → ui-egui showing a list with badges → ext-git → previews.

## Non-goals & open questions

Rick does not implement file systems, mount network shares itself, or bundle a browser engine. Rick builds no file search index of its own. Where the platform has one (Windows Search, Spotlight, Tracker or Baloo on Linux) a search box appears on the toolbar and queries it; where none is present, there is no search box and only the live subtree walk described under the GUI. The items below need a call before they enter the plan.

All open questions from the first draft are settled (2026-11-05): iSCSI out of scope, RAR extract-only, LibreOffice used when installed and never bundled, TFVC deferred to a plugin, tar and zip read separately, GStreamer on every platform, and "supported file system" means correct basic behaviour in the core — file-system-specific features such as Btrfs subvolumes and ZFS snapshots come later as Decorator extensions. New questions go here as they come up.

**Configuration.** Settings are plain TOML files the user can read, edit and version: a main `rick.toml` plus one file per extension. They live in the platform's standard location — `$XDG_CONFIG_HOME/rick` (usually `~/.config/rick`) on Linux, `~/Library/Application Support/Rick` on macOS, `%APPDATA%\Rick` on Windows. The Settings screen writes the same files; the app watches them and reloads on change.

**License.** Rick is open source under Apache-2.0: permissive, with an explicit patent grant and contributor protection, and the usual choice in the Rust ecosystem, so plugins and embedders face no friction. Every crate in the workspace carries the same license; a `CONTRIBUTING` note states that contributions are accepted under it.

**Language.** English only for the first releases. User-facing strings still go through one `t!` lookup function from the start, so adding languages later is a translation file, not a code change.

### Engineering guidelines

This spec is written to be implemented by an AI coding agent as well as by people. The rules below are meant to be applied literally.

- **Toolchain.** Rust 2024 edition on the stable toolchain, pinned in `rust-toolchain.toml`. `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` with the pedantic group enabled must pass before any change is considered done.
- **Unsafe.** No `unsafe` outside the `platform` crate. Every `unsafe` block carries a `// SAFETY:` comment stating the invariant it relies on.
- **Errors.** Library crates define their error types with `thiserror`; only the `rick` binary uses `anyhow`. Never panic on user data, file content or filesystem state: `unwrap` and `expect` are allowed only where the invariant is provable and commented.
- **Crate discipline.** Every crate has a `README.md` of one paragraph stating its responsibility and the crates it may depend on. The layer order in the architecture section is enforced with `cargo-deny` bans and checked in CI; a dependency that would point upward is a design error, not a shortcut.
- **Logging.** `tracing` everywhere, `println!` and `eprintln!` nowhere outside tests. Spans around every filesystem operation so the performance targets can be measured.
- **Async and threads.** One `tokio` runtime owned by `runtime`; the UI thread never blocks on I/O. CPU-heavy work (sorting, hashing, decoding) goes through `rayon` or a dedicated pool, never on the runtime's worker threads.
- **Tests.** Unit tests next to the code; integration tests in a `tests` crate with a generated 200k-entry fixture that the performance targets are benchmarked against. A bug fix adds a test that fails before the fix.
- **Documentation.** Public items have doc comments; `cargo doc` builds without warnings. Each extension point has a worked example extension in its crate's docs.
- **Commits.** Small, one concern each, conventional-commit messages (`feat(vfs): …`, `fix(ui): …`). Each commit builds and passes tests on its own.
- **The spec wins.** When the code and this document disagree, the document is right until it is changed. Ambiguities or contradictions in the spec are raised as questions, listed in a `QUESTIONS.md` in the repository, and never resolved silently in code.
- **Scope.** Implement what the spec says, nothing more. No speculative abstractions, no extra features, no configuration options the spec does not name.

### Architecture rules

These govern how the crates fit together. They are the decisions an implementer must not re-make.

- **Unidirectional state flow.** The UI never mutates the model directly. It emits `Command` values; `app` handles them, updates the model and publishes `Event`s; the UI re-renders from the model. The model is the single source of truth for everything the window shows. Undo/redo is a log of reversible commands kept in `app`.
- **Every long operation is a Job.** A directory listing, a copy, a search, a preview decode: each runs as a `Job` with a cancellation token, a progress channel and a result. Listings stream entries in batches so the UI shows the first screen before the walk finishes; switching away from a location cancels its jobs. Cancelling drops the job's future, so a job that never polls its token still stops at its next await point and its handle resolves with `Error::Cancelled`. A job that leaves partial state behind, such as a half-written copy target, must check the token before each irreversible step or hold a guard whose `Drop` undoes or finishes that step, because being dropped is its only notice. Long synchronous work on the compute pool cannot be dropped mid-call and checks `is_cancelled()` between chunks. Listings, their change watches and decorator lookups are jobs for cancellation and progress but are not listed in the operations area, which shows the jobs an action, an undo or a redo started; their output is the event stream.
- **Platform code lives in `platform` only.** No `cfg(target_os)` anywhere else. Other crates program against the `platform` traits (trash, open-with, file properties, config dir, native dialogs) and the tests run with a fake implementation.
- **The extension traits are a versioned contract.** The traits in `ext` and the manifest format carry a version. Any breaking change bumps it; the plugin host refuses manifests it does not understand and says so in the Extensions screen.
- **Optional extensions are Cargo features.** `rick` builds with the kernel alone; each `ext-*` crate is a feature on the binary, default-on for built-ins, so a minimal build and a full build come from the same workspace.
- **Milestones have acceptance tests.** Each step in the build order ends with an executable acceptance test (listing 200k entries under the target, a round-trip copy with cancellation, a plugin loaded from a manifest). The next step does not start until it passes.
- **No shared mutable state across threads without an owner.** Caches, indexes and watchers are owned by one component that exposes a message or async API; `Arc<Mutex<_>>` across crate boundaries is a design smell to justify in a comment.
- **Identity, not paths.** Entries are keyed by a provider-scoped `EntryId` (device and inode on local filesystems), not by path string, so renames and moves keep selection, history and decorations intact.

### Core types and extension traits

First cut of the contract between the kernel and the extensions. Names and shapes are binding; signatures may gain parameters as the implementation learns, but only through a spec change. Everything here is `Send + Sync`; async methods are written with the `async_trait` attribute so the traits stay usable as `dyn` objects (`Arc<dyn Provider>`), to be replaced by native `async fn` once `dyn`-compatible async traits are stable; streamed results use `BoxStream` from `futures`.

The supporting types the signatures name without defining them here (`ListOptions`, `WriteOptions`, `RemoveMode`, `LinkKind`, `CopyOptions`, `Badge`, `Color`, `ColumnDef`, `ColumnId`, `CellValue`, `PanelSection`, `DecorateCtx`, `PaneId`, `Invocation`, `Shortcut`, `UiHost`, `RenderRequest`, `TextLayer`, `TextBlock`, `TextureId`, `Transport`, `SearchScope`, `QueryKind`, `Filter`, `SearchHit`, `SettingsSchema`, `Version`) are defined and documented in crate `ext`, with the smallest shape that makes the trait usable. Their shapes are binding as implemented; a change to one goes through a numbered question like a change to a signature.

**Value types** (crate `types`, no I/O):

```rust
/// Which extension provides a tree. "local", "archive", "sftp", "s3", "trash".
pub struct ProviderId(pub &'static str);

/// Identity of an extension, as declared in its manifest. "git", "preview-pdf".
pub struct ExtId(pub &'static str);

/// Stable identity of an entry inside one provider. Local uses (device, inode);
/// other providers choose their own 128-bit key. Never a path string.
pub struct EntryId { pub provider: ProviderId, pub key: u128 }

/// Provider-neutral path. Keeps raw bytes, offers a lossy display form.
pub struct VPath(Vec<u8>);

/// A place a pane can show.
pub struct Location { pub provider: ProviderId, pub path: VPath }

pub enum EntryKind { File, Dir, Symlink { target: VPath, broken: bool }, Other }

/// Media type as `type/subtype` without parameters, not validated. Filled
/// lazily by sniffing, so most entries carry `None` until something asks.
pub struct Mime(Box<str>);

pub struct Entry {
    pub id: EntryId,
    pub name: String,            // display name; raw bytes live in `path`
    pub path: VPath,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub attrs: Attrs,            // bitflags: HIDDEN, SYSTEM, READONLY, EXECUTABLE
    pub mime: Option<Mime>,      // filled lazily by sniffing
}

/// Cooperative cancellation. Cloneable; `cancelled().await` for async code,
/// `is_cancelled()` for loops.
pub struct CancelToken;

pub enum Progress {
    Items { done: u64, total: Option<u64> },
    Bytes { done: u64, total: Option<u64> },
    Message(String),
}

/// One error type for the whole kernel; extensions map their own into it.
pub enum Error { NotFound, PermissionDenied, Cancelled, Unsupported(&'static str), Io(std::io::Error), Extension { ext: ExtId, source: Box<dyn std::error::Error + Send + Sync> } }
```

**Job types** (crate `runtime`, which owns the tokio runtime and re-exports `tokio::sync::{watch, oneshot}` so callers need no `tokio` dependency):

```rust
/// What `runtime` hands back for every long operation.
pub struct JobHandle<T> {
    pub id: JobId,
    pub cancel: CancelToken,
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<Result<T, Error>>,
}

/// What a job receives when it starts. Cancelling drops the job's future, so
/// a job that never looks at the token still stops at its next await point
/// and its handle resolves with `Error::Cancelled`.
pub struct JobCtx { pub id: JobId, pub cancel: CancelToken, pub progress: ProgressSink, pub pause: PauseGate }

/// Pausable jobs await the gate before each item, directory, file and streamed
/// buffer; `JobRunner::pause` closes it, `resume` opens it. Copy and move jobs
/// are pausable; listings and decorations are not.
pub struct PauseGate;

/// Where a job publishes progress; `report` replaces the previous value and never fails.
pub struct ProgressSink(watch::Sender<Progress>);

impl JobRunner {
    pub fn spawn<T, F, Fut>(&self, job: F) -> JobHandle<T>
        where F: FnOnce(JobCtx) -> Fut, Fut: Future<Output = Result<T, Error>> + Send + 'static;
    pub fn spawn_pausable<T, F, Fut>(&self, job: F) -> JobHandle<T>
        where F: FnOnce(JobCtx) -> Fut, Fut: Future<Output = Result<T, Error>> + Send + 'static;
    pub fn running(&self) -> Vec<JobId>;
    pub fn cancel(&self, id: JobId) -> bool;
    pub fn pause(&self, id: JobId) -> bool;
    pub fn resume(&self, id: JobId) -> bool;
}
```

**Provider** — a tree of entries (crate `ext`):

```rust
bitflags! { pub struct Capabilities: u32 {
    const WRITE = 1; const TRASH = 2; const RENAME = 4; const SYMLINK = 8;
    const REFLINK = 16; const WATCH = 32; const SEEK = 64; const PERMISSIONS = 128;
    /// Copy and move between two locations of this provider instance happen
    /// on the provider's side (server-side copy, remote rename); nothing
    /// streams through the machine and the job only tracks progress.
    const NATIVE_TRANSFER = 256;
} }

pub trait Provider: Send + Sync + 'static {
    fn id(&self) -> ProviderId;
    fn capabilities(&self, loc: &Location) -> Capabilities;

    /// Streams entries in batches. The first batch must arrive within the
    /// first-screen budget; `opts` carries hidden/system flags and depth (1 or
    /// unlimited for flattened views). Cancelling the token ends the stream.
    fn list(&self, loc: &Location, opts: ListOptions, cancel: CancelToken)
        -> BoxStream<'static, Result<Vec<Entry>, Error>>;

    async fn stat(&self, loc: &Location) -> Result<Entry, Error>;
    async fn open_read(&self, loc: &Location) -> Result<Box<dyn AsyncRead + Send + Unpin>, Error>;
    async fn open_write(&self, loc: &Location, opts: WriteOptions) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error>;
    async fn create_dir(&self, loc: &Location) -> Result<Entry, Error>;
    async fn rename(&self, from: &Location, to: &Location) -> Result<Entry, Error>;
    async fn remove(&self, loc: &Location, mode: RemoveMode) -> Result<(), Error>;   // Trash | Permanent
    async fn link(&self, target: &Location, at: &Location, kind: LinkKind) -> Result<Entry, Error>;

    /// Copies one regular file with the native fast path (copy_file_range,
    /// reflink, server-side copy). `vfs::copy` composes directory trees from
    /// it and cross-provider copies from `open_read` and `open_write`; it
    /// owns recursion, progress totals and cancellation.
    async fn copy(&self, from: &Location, to: &Location, opts: CopyOptions,
                  progress: ProgressSink, cancel: CancelToken) -> Result<(), Error>;

    /// Moves or copies a whole selection within this provider instance on the
    /// provider's side, if NATIVE_TRANSFER is set: the object store's copy
    /// API across buckets of one account, SFTP's remote rename. The provider
    /// submits the operation, reports progress as the service exposes it and
    /// honours `cancel` where the service allows. `ops` calls this first when
    /// source and target share a provider instance and falls back to
    /// `rename`, `copy` and the streamed path otherwise.
    async fn transfer(&self, items: &[Location], to: &Location, mode: TransferMode, // Move | Copy
                      progress: ProgressSink, cancel: CancelToken) -> Result<(), Error>;

    /// Change notifications for one location, if WATCH is set.
    fn watch(&self, loc: &Location) -> Result<BoxStream<'static, FsChange>, Error>;
}

pub enum FsChange { Created(Entry), Removed(EntryId), Modified(Entry), Renamed { from: EntryId, to: Entry }, Rescan }
```

**Decorator** — adds badges, columns and panel sections to entries:

```rust
pub trait Decorator: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    /// Cheap check per location, e.g. "inside a Git work tree".
    fn applies_to(&self, loc: &Location) -> bool;
    /// Extra columns this decorator can fill (e.g. Git status).
    fn columns(&self) -> &[ColumnDef];
    /// Called for each visible batch. Must return fast; slow lookups return
    /// `Pending` and complete later through `ctx.publish`.
    fn decorate(&self, entries: &[Entry], ctx: &DecorateCtx) -> Vec<(EntryId, Decoration)>;
    /// Optional section for the info panel.
    fn panel(&self, entry: &Entry) -> Option<PanelSection>;
    /// Locations whose changes stale this decorator's cache for `loc` (the
    /// `.git` directory of a work tree); the kernel watches them for the pane.
    fn invalidates_on(&self, loc: &Location) -> Vec<Location>;
    /// One of those locations changed: drop the cache; the kernel decorates
    /// the listing again.
    fn invalidate(&self, loc: &Location);
}

pub struct Decoration {
    pub badge: Option<Badge>,              // small glyph in the icon corner
    pub tint: Option<Color>,               // name colour
    pub cells: SmallVec<[(ColumnId, CellValue); 2]>,
}
```

**Action** — a command the user can invoke:

```rust
pub struct ActionId(pub &'static str);     // "core.copy", "git.commit"

pub enum ActionGroup { Core, Extension(&'static str) }   // Extension("Git") → context-menu submenu

pub enum Availability { Hidden, Disabled(&'static str), Enabled }

pub struct ActionCtx {
    pub pane: PaneId,
    pub location: Location,
    pub selection: Vec<Entry>,
    pub invoked_from: Invocation,          // ContextMenu | Palette | Shortcut | Menu
}

pub enum ActionOutcome { Done, Started(JobId), Undoable(Box<dyn UndoCommand>) }

pub trait Action: Send + Sync + 'static {
    fn id(&self) -> ActionId;
    fn label(&self) -> &str;
    fn group(&self) -> ActionGroup;
    fn default_shortcut(&self) -> Option<Shortcut>;
    fn availability(&self, ctx: &ActionCtx) -> Availability;
    /// Runs on the app thread; anything long is spawned as a Job through `jobs`.
    fn run(&self, ctx: ActionCtx, jobs: &JobRunner, ui: &dyn UiHost) -> Result<ActionOutcome, Error>;
}

pub trait UndoCommand: Send { fn undo(&self, jobs: &JobRunner) -> Result<(), Error>; fn redo(&self, jobs: &JobRunner) -> Result<(), Error>; fn label(&self) -> String; }
```

**Previewer** — turns an entry into something the inspector or viewer can show:

```rust
pub trait Previewer: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    /// 0 = cannot handle; the highest score wins. `sniff` is the first 4 KiB.
    fn score(&self, entry: &Entry, sniff: &[u8]) -> u8;
    async fn open(&self, loc: &Location, provider: Arc<dyn Provider>, cancel: CancelToken)
        -> Result<Box<dyn PreviewDoc>, Error>;
}

pub trait PreviewDoc: Send {
    fn pages(&self) -> Option<usize>;                       // None = single surface
    /// Produces a frame for one page at the requested size; the UI uploads it
    /// to a texture. Text documents return `Frame::Text` and let the UI lay out.
    fn render(&mut self, page: usize, req: RenderRequest) -> Result<Frame, Error>;
    fn text_layer(&self) -> Option<&TextLayer>;             // for find
    /// Metadata, grouped for the pane's info badges. The first section is
    /// "File info"; others (EXIF, stream details) appear only when they apply.
    fn sections(&self) -> Vec<InfoSection>;
    /// One-line facts for the pane's status bar, in display order.
    fn status(&self) -> Vec<String>;
    /// The previewer's own toolbar controls (e.g. pretty print); the standard
    /// ones (line numbers, wrap, pager, fit/1:1) come from the kind of document.
    fn controls(&self) -> Vec<PreviewControl>;
    /// A toolbar control was used; the next `render` reflects it.
    fn set_control(&mut self, id: ControlId, value: ControlValue) -> Result<(), Error>;
    fn transport(&mut self) -> Option<&mut dyn Transport>;  // media only: play/pause/seek
}

pub struct InfoSection { pub id: &'static str, pub title: &'static str, pub icon: Icon, pub rows: Vec<(String, String)> }

pub enum PreviewControl {
    Toggle { id: ControlId, icon: Icon, label: &'static str, on: bool },
    Button { id: ControlId, icon: Icon, label: &'static str, enabled: bool },
}

pub enum Frame { Rgba { width: u32, height: u32, data: Arc<[u8]> }, Text(TextBlock), External(TextureId) }

/// Text the UI lays out itself. `spans` are coloured byte ranges the previewer
/// computed (syntax highlighting); `RenderRequest.dark` tells it which theme.
pub struct TextBlock { pub text: String, pub spans: Vec<(Range<usize>, Color)> }
```

The pane's toolbar is built from these plus the document kind, and the pane itself never asks a previewer to draw: a Decorator's `panel()` and a previewer's `sections()` both feed the same info badges.

**SearchIndex** — a query backend:

```rust
pub trait SearchIndex: Send + Sync + 'static {
    fn id(&self) -> ExtId;
    fn availability(&self) -> Availability;                 // platform backend present?
    fn scope(&self) -> SearchScope;                         // Everywhere | Under(Location)
    fn query(&self, q: &Query, cancel: CancelToken) -> BoxStream<'static, Result<Vec<SearchHit>, Error>>;
}

pub struct Query { pub text: String, pub root: Option<Location>, pub kind: QueryKind /* Name | Content | Both */, pub filters: Vec<Filter> }
```

**Registration** — how a crate like `ext-git` contributes to several points at once:

```rust
pub struct Manifest { pub id: ExtId, pub name: &'static str, pub version: Version, pub api_version: u32, pub requires: Capabilities }

pub trait Extension: Send + Sync + 'static {
    fn manifest(&self) -> &Manifest;
    fn register(&self, reg: &mut Registry) -> Result<(), Error>;
}

impl Registry {
    pub fn provider(&mut self, p: Arc<dyn Provider>);
    pub fn decorator(&mut self, d: Arc<dyn Decorator>);
    pub fn action(&mut self, a: Arc<dyn Action>);
    pub fn previewer(&mut self, p: Arc<dyn Previewer>);
    pub fn search_index(&mut self, s: Arc<dyn SearchIndex>);
    pub fn settings(&mut self, schema: SettingsSchema);   // renders in the Settings screen, stored in the extension's TOML
}
```

**Commands and events** — the only two things that cross the UI boundary:

```rust
pub enum Command {
    Navigate { pane: PaneId, to: Location }, Back(PaneId), Forward(PaneId), Refresh(PaneId),
    Select { pane: PaneId, change: SelectionChange },
    SetView { pane: PaneId, mode: ViewMode }, Sort { pane: PaneId, by: Column, dir: SortDir }, GroupBy { pane: PaneId, by: Option<Column> },
    Filter { pane: PaneId, text: String }, ToggleHidden(PaneId), Flatten { pane: PaneId, on: bool },
    Run { action: ActionId, ctx: ActionCtx },
    NewTab(Location), CloseTab(TabId), ActivateTab(TabId), FocusPane(PaneId),
    Split { pane: PaneId, dir: SplitDir }, MoveTab { tab: TabId, to: DockTarget },
    CancelJob(JobId), DismissJob(JobId), Undo, Redo,
    Settings(SettingsChange),
    /// Drag-and-drop (and later Move to / Copy to): move or copy `items` into `to`.
    /// `Ask` opens the move/copy prompt first. Runs as one job; a move within a
    /// provider is a rename, a copy uses the provider's fast path.
    Transfer { items: Vec<EntryId>, from: PaneId, to: Location, mode: TransferMode /* Move | Copy | Ask */ },
    /// Spring-loading step while a drag is in progress: navigates like `Navigate`
    /// but records history only on the first step of a drag (`first: true`).
    SpringLoad { pane: PaneId, to: Location, first: bool },
    /// Pin and unpin, undoable; `at` is the position in the Pinned list, None = end.
    Pin { items: Vec<Location>, at: Option<usize> }, Unpin(Location),
    /// Items dropped from outside the process (file URIs, no EntryId): the same
    /// job as Transfer with the outside rule (copy unless Shift) already applied.
    Receive { items: Vec<Location>, to: Location, mode: TransferMode },
    /// Thumbnail for the icon view, rendered by the best-scoring previewer as an
    /// internal job; answered by Event::Thumbnail.
    Thumbnail { pane: PaneId, id: EntryId, size: u32 },
    /// Session, layouts and windows (General behaviour).
    TabOrder { pane: PaneId, tabs: Vec<TabId>, active: TabId }, Layout(DockLayout), Scroll { pane: PaneId, offset: f32 },
    WindowGeometry { window: WindowId, rect: Rect, maximised: bool }, SidePanel { window: WindowId, state: SidePanelState },
    FocusWindow(WindowId), NewWindow, CloseWindow(WindowId),
    SaveLayout(String), SwitchLayout(String), DeleteLayout(String), RenameLayout { from: String, to: String },
}

pub enum Event {
    Listed { pane: PaneId, batch: Vec<Entry>, done: bool },
    Changed { pane: PaneId, change: FsChange },
    Decorated { pane: PaneId, items: Vec<(EntryId, Decoration)> },
    JobProgress { id: JobId, progress: Progress }, JobDone { id: JobId, result: Result<(), Error> },
    SelectionChanged { pane: PaneId, ids: Vec<EntryId> },
    PreviewReady { pane: PaneId, doc: Box<dyn PreviewDoc> },
    PreviewFailed { pane: PaneId, error: Error },   // no previewer claims the file, or open failed
    Thumbnail { pane: PaneId, id: EntryId, size: u32, frame: Frame },
    Notify { level: Level, text: String },
}
```

### Keyboard shortcuts

The default `default_shortcut` of every built-in Action. **Mod** is Ctrl on Linux and Windows and Cmd on macOS. All of these are rebindable in Settings → Keyboard shortcuts; extensions may claim unused combinations and never override a core one.

**Navigation**

| Shortcut | Action |
| --- | --- |
| Enter | Open (folder: navigate; previewable file: viewer; other: system default) |
| Mod+Enter | Open with system default, bypassing the viewer |
| Backspace, Alt+Left | Back |
| Alt+Right | Forward |
| Alt+Up | Go to parent |
| Mod+Shift+Home | Go to root |
| Mod+L | Focus the path bar |
| F5, Mod+R | Refresh |
| Mod+Shift+P | Focus the search box in command mode (`>` typed) |
| Mod+P | Focus the search box (search by name) |

**Selection**

| Shortcut | Action |
| --- | --- |
| Mod+A | Select all |
| Mod+Shift+A | Invert selection |
| Mod+Shift+S | Select by pattern |
| Space | Toggle selection of the focused entry |
| Escape | Clear selection / close filter |
| Typing | Type-to-filter: the typed text filters the list (Main area); there is no jump-to-letter |

**File operations**

| Shortcut | Action |
| --- | --- |
| Mod+C | Copy |
| Mod+X | Cut |
| Mod+V | Paste |
| Mod+Shift+V | Paste as link |
| Mod+D | Duplicate |
| Delete | Move to trash |
| Shift+Delete | Delete permanently |
| F2 | Rename |
| Mod+F2 | Bulk rename |
| — | New file (no default; rebindable) |
| Mod+Shift+N | New folder |
| Mod+Shift+C | Copy path and name |
| Mod+Alt+C | Copy name |
| Mod+Alt+Shift+C | Copy path |
| Mod+I, Alt+Enter | Inspect |
| Mod+Z | Undo |
| Mod+Shift+Z, Mod+Y | Redo |
| Mod+O | Open with… |
| Mod+Shift+O | Open in system file manager |

**Tabs and panes**

| Shortcut | Action |
| --- | --- |
| Mod+N | New window |
| Mod+T | New tab |
| Mod+W | Close tab |
| Mod+Shift+T | Reopen closed tab |
| Mod+Shift+D | Duplicate tab |
| Mod+Tab, Mod+PageDown | Next tab |
| Mod+Shift+Tab, Mod+PageUp | Previous tab |
| Mod+1 … Mod+9 | Tab 1 to 9 |
| Mod+Shift+PageUp / PageDown | Move tab left / right |
| Mod+\\ | Split right |
| Mod+Shift+\\ | Split down |
| Mod+Alt+Left / Right / Up / Down | Focus pane in that direction |
| Mod+Shift+W | Close other tabs |

**View**

| Shortcut | Action |
| --- | --- |
| Mod+F | Filter |
| Mod+H | Toggle hidden files |
| Mod+Shift+H | Toggle system files |
| Mod+E | Toggle file extensions |
| Mod+Shift+F | Search in subdirectories (focuses the filter) |
| — | Toggle flatten subfolders (no default; rebindable) |
| Mod+Shift+I | Toggle inspector |
| Mod+\` | Open console |
| Mod+, | Settings |
| Mod+Shift+1 / 2 / 3 / 4 | List / Details / Tree / Icons |
| Mod+scroll, Mod+Plus / Minus | Icon size |
| Mod+Shift+R | Size columns to fit |
| Mod+Alt+N / S / M / T | Sort by name / size / modified / type |
| Mod+Alt+G | Clear grouping |

**Viewer**

| Shortcut | Action |
| --- | --- |
| F11 | Fullscreen |
| Escape | Leave fullscreen / close viewer |
| Left / Right, PageUp / PageDown | Previous / next item or page |
| Home / End | First / last |
| Mod+0 | Zoom to fit |
| Mod+1 | Actual size |
| Mod+2 | Fit width |
| Mod+Plus / Minus | Zoom in / out |
| Space | Play / pause |
| J / K / L | Rewind / pause / fast-forward |
| Mod+F | Find in text layer |
| I | Toggle info panel |

Always open the **Keyboard shortcuts** screen from the menu to see the live table; this one is the source for defaults, not the running state.

### Version 0.1 — definition of done

The smallest Rick worth using every day. Everything in this list ships; nothing outside it does.

**In scope**

- All three platforms from the first release: Linux, macOS and Windows, built and tested in CI, with the `platform` crate implementing trash, open-with, file properties and the config directory on each.
- Local filesystem provider only.
- Tabs, docking and split panes (right and down), list, details and icon views (the icon view with thumbnails from the previewers), column sizing, sort by every column, filter, hidden-files toggle.
- The window chrome of the GUI section: custom title row with the search box, Phosphor icons throughout, the three-group toolbar with a collapsing breadcrumb, the tab strip rules and the line-free details view.
- Core operations: copy, cut, paste, duplicate, move to trash, delete permanently, rename (single), new file, new folder, open with the system default, open containing folder, copy name, copy path, copy path and name, pin to side panel and unpin, back and forward, go to root, refresh.
- Drag and drop as the Functions section specifies: every internal target (folder rows, pane space, breadcrumb segments, tab headers, Pinned, Trash), the move/copy rule with modifiers, the ghost with its indicator, spring-loading with the stay-or-return setting, and drops to and from other applications and other Rick processes through the platform.
- Keyboard navigation for all of the above and the search box in command mode.
- Settings in TOML with a minimal Settings screen.
- Previews for text, code (syntect), images and PDF (pdfium-render, first page plus page navigation) in the inspector pane.
- `ext-git`: status badges and branch via gix, and the Git action group (Commit, Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log, Blame) in the context menu and palette inside a repository.
- All built-ins implemented through the `ext` traits, so the architecture is real from day one.
- The performance targets from the Decisions table met on the 200k-entry fixture, on all three platforms.

**Explicit non-goals for 0.1**

- The full-size viewer, office and media previews, archive previews and hex fallback.
- Archive, SFTP, object-storage and trash providers; search indexes.
- SVN and any Decorator beyond file-type icons and Git status.
- Bulk rename, compare, compress and extract, console.
- The WASM and native plugin hosts and the Extensions screen; extensions are compiled in.
- Localisation beyond English.

**Released.** 0.1.0 was tagged on 2026-11-06 from `02a03ba` and published for Linux (x64, arm64), macOS (arm64) and Windows (x64) by the release workflow; `BENCHMARKS.md` holds the measured numbers and `CHANGELOG.md` the list of changes. Known gaps carried into 0.2: dragging items out of Rick into another application is not delivered on any platform (the native drag source returns `Unsupported`; QUESTIONS.md 62), a drop from outside lands in the focused pane's folder, and macOS runs four benchmarks against measured targets rather than the spec's numbers (`BENCHMARKS.md`).

### Version 0.2 — scope

0.2 turns the architecture outward: everything that is not essential becomes a plugin file, and the bulk of the specified-but-unbuilt features arrive as plugins. The order below is the build order; the plugin host comes first because every later item lands as a plugin, and each milestone ships as its own issue.

1. **Plugin host and the split.** `plugins-native` loads plugins from the plugins directory at start: each plugin is one native dynamic library beside a manifest file, discovered by scanning the directory, loaded through one `extern "C"` entry point that receives the `Registry`. The manifest (TOML) carries id, name, version, `api_version`, the extension points filled, the platforms supported, and the toolchain the library was built with; the host refuses a plugin whose `api_version` or toolchain does not match and says so in the Extensions screen. Plugins are built from this workspace as `cdylib` crates with the same toolchain, so a workspace build produces the binary plus every plugin in the plugins directory with nothing to install by hand; the mechanism that lets a plugin and the host share one copy of `ext`, `types` and `runtime` (dynamic linking of the kernel crates, or a C-ABI vtable) is the implementer's call, recorded in `QUESTIONS.md`. **Compiled in:** the local provider, the text, code, image and PDF previewers, and `ext-action-core`. **Plugins:** everything else, starting with `ext-git`, which moves out of the binary in this milestone to prove the host. The **Extensions screen** lists the plugins found, with name, version, the points they fill, and enable/disable (a disabled plugin stays on disk and is not loaded on the next start); no store, no download, no update. The plugins directory sits beside the binary and in the per-user config directory; per-user wins on an id clash. Distribution and signing stay open.
2. **Viewer.** The full-size viewer of the GUI section: Enter opens previewable files in a viewer tab, F11 fullscreen, folder navigation as a slideshow, pages, zoom, find, media transport, info panel, the Action registry inside the viewer, hex fallback. Compiled in, since it is a frame for the previewers rather than a feature of its own.
3. **Previewers as plugins:** `ext-preview-office` (LibreOffice when installed), `ext-preview-media` (GStreamer, bundled runtime on macOS and Windows), `ext-preview-archive` (listing without mounting), `ext-preview-csv` (virtualised grid), rendered Markdown and HTML in `ext-preview-code`, `ext-preview-hex`, and `ext-preview-3d` (below).
4. **Providers as plugins:** `ext-vfs-archive` (zip, tar family, 7z, RAR read-only, single-file codecs) with ISO images browsed the same way, `ext-vfs-trash` (the Trash side-panel section becomes a browsable location with restore and empty), `ext-vfs-sftp`, and one plugin per cloud — `ext-vfs-s3` (S3, R2 and S3-compatibles), `ext-vfs-azure`, `ext-vfs-gcs` — each with `NATIVE_TRANSFER` so moves and copies inside an account run on the service. The Network side-panel section gains discovered shares, connect, mount and unmount.
5. **Search and decorators as plugins:** `ext-search-walk` behind the subtree search, then `ext-search-windows`, `ext-search-spotlight`, `ext-search-tracker` and `ext-search-baloo` feeding the title-row search box; `ext-svn` beside `ext-git`.
6. **Actions as plugins:** `ext-action-compress` (compress, extract here, extract to), `ext-action-compare` (file and folder diff), `ext-action-terminal` (open console, with an embedded console pane), `ext-action-rename` (bulk rename with pattern, numbering, find and replace, preview), plus Open with… chooser, Paste as link and Select by pattern in `ext-action-core`.
7. **Permission management** in Properties (owner, group, mode bits, ACLs, recursive apply, elevation through the platform), and **localisation**: the `t!` lookup gains translation files and a language setting, English remains the fallback.
8. **Platform drag source and drop position.** Close 0.1's drag-and-drop gap: a native drag source per platform (an `NSDraggingSession` on macOS, `DoDragDrop` on Windows, a Wayland data source and XDND on Linux) through the `platform::DragSource` hook, and the cursor position of an incoming drop where the backend can give it, so a drop from outside lands where it was dropped. The macOS benchmarks that run against measured targets are re-measured and tightened towards the spec's numbers as the work allows.

**Not in 0.2:** the WASM host, the extension store and downloads, plugin signing, `ext-deco-fs`, TFVC. During 0.2 the workspace version is `0.2.0-dev`; the milestones merge to `main` behind CI on all three OSes as in 0.1, and 0.2.0 is tagged when milestone 8 is in.

**3D models.** A previewer for STL, OBJ and 3MF, for people who print. The mesh is parsed natively (`stl_io`, `tobj`, a small 3MF reader over its zip and XML) and rendered through wgpu into the preview frame as a shaded solid with a neutral material and a ground grid; drag orbits, wheel zooms, double-click resets the view, and the view is remembered per file while it stays open. The status bar shows triangle count, bounding box in millimetres and, for OBJ, the number of objects; the File info section adds the units declared by 3MF. Models beyond a configurable triangle budget decimate for the preview and say so. Ships as `ext-preview-3d`, a plugin.

### Still open

- Extension security model: what a WASM plugin may touch (capabilities per extension point), and whether native plugins need signing.
- Testing strategy for the performance targets: a generated 200k-entry fixture and a benchmark harness in CI.
- Accessibility: egui's AccessKit support is young; screen-reader support is a known weakness to state and track.
- Distribution: 0.1 ships plain archives per OS from the release workflow (binary, pdfium, licenses); Flatpak, Homebrew, winget and installers are still to choose.
- Title row toggles: which layout toggles sit right of the search box (candidates: side panel, inspector, split), and whether the current path belongs in the title row at all.
