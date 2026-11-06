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

| Service | Provider | Notes |
| --- | --- | --- |
| AWS S3 | `object_store` crate | Flat keys shown as folders by prefix |
| Cloudflare R2 | `object_store` (S3 API) | Same provider as S3 |
| Azure Blob | `object_store` | Containers as top-level folders |
| Google Cloud Storage | `object_store` | Buckets as top-level folders |

Object stores have no rename (copy then delete) and no change notifications (polling).

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

### Side panel

Docked on the left by default; the user can move it to the right side, and collapse it to a narrow icon strip or hide it entirely, with a shortcut to toggle it back. Its entries, top to bottom:

| Entry | What it shows | Notes |
| --- | --- | --- |
| Home | The user's home directory | Linux and macOS: `$HOME`; Windows: the user profile directory (`C:\Users\<name>`) |
| Recent | Files and folders recently changed anywhere in the file system | Fed by file-system events, not only by what Rick touched; newest first |
| Network | Network environment: discovered shares and hosts, mounted network file systems, configured object-storage connections | Connect, mount and unmount from here |
| Trash | The platform trash | Restore and empty |
| Pinned | Locations the user pins | Any path, local or remote; drag to reorder |

Each entry is a section that folds and expands independently, so a user short on room can collapse Recent or Network and keep Pinned open. The panel's width is draggable, and the folded state and width persist across sessions.

**Operations area.** The bottom of the side panel shows running long operations, such as copying a large file, extracting an archive or an upload to object storage: one row per job with name, progress bar, throughput and remaining time, plus pause, resume and cancel. The area appears only while a job runs and folds away when the queue is empty; completed jobs stay listed briefly with their result, and a failed job stays until dismissed.

### Main area

The main area is the explorer view: each tab shows one location's files, with a toolbar above the file list.

**Toolbar.** A bar of icons per tab, left to right:

| Control | Action | Shortcut |
| --- | --- | --- |
| Back | Previous location in this tab's history | Alt+Left, mouse button 4 |
| Forward | Next location in history | Alt+Right, mouse button 5 |
| Up | Parent directory | Alt+Up, Backspace |

Other toolbar controls can be added later; the toolbar is an extension point for Actions with toolbar placement.

**Path bar.** Next to the toolbar, the current path is always visible as clickable segments (breadcrumbs); clicking a segment jumps there, clicking the empty area or pressing Ctrl+L turns it into an editable text field. A Copy-path button beside it copies the full path to the clipboard with one click.

**Tabs and panes.** Any number of tabs, each an explorer view with its own toolbar, path and history (Ctrl+T new, Ctrl+W close, Ctrl+Tab cycle; middle-click a folder opens it in a new tab). Tabs are not only stacked: a tab can be picked up and dropped to the left, right, top or bottom of the current one, splitting the area into side-by-side or stacked panes, as in an IDE. Drop targets appear while dragging; splits nest, so two-pane, three-pane and grid layouts are all possible; dividers drag to resize; a tab dragged outside the window opens a new window. The layout, tabs and their paths are restored on the next start. This is egui\_dock's tree of tabs and splits.

**File list.** A details list, one row per entry, folders first. Default columns in order:

| Column | Content |
| --- | --- |
| Icon + Name | An icon for what the entry is (folder, text, image, archive, video, and so on), then the name |
| Type | The file type as text, e.g. Folder, PNG image, Rust source |
| Date modified | Last modification time, relative for recent entries, full on hover |
| Size | Human-readable; folders show nothing until a size is requested |

Columns sort on click, resize, reorder and can be hidden; further columns may be added later, and extensions can add columns through Decorators (for example a Git status column). An optional setting alternates the row background colour (zebra striping) for easier reading of wide rows.

**Type filter.** A filter control on the toolbar lists only the file types actually present in the current directory, with a count per type; ticking one or more shows just those entries. The filter is per tab, clears on navigation unless pinned, and a text filter (name contains) sits beside it.

**Status line.** A small stats line at the bottom of each explorer view shows how many folders and how many files the current directory holds, the total size of the files, and, when something is selected, the selected count and size. With a filter active it reads "12 of 340 files".

**Sorting.** Default order is by name, alphabetically; clicking a column header sorts by that column (name, type, date modified, size) and clicking again reverses it. A separate "folders first" toggle, on by default, keeps all folders above all files whatever the sort column; switched off, folders and files are mixed into one order. Both settings persist per user, with an optional per-folder override.

**Type-to-filter.** Typing while the list has focus filters it live: the typed text appears in a small bar at the foot of the list, only entries whose name contains it stay visible, Escape clears it and Enter opens the first match. This replaces the classic "jump to first entry starting with the letter" behaviour.

**Search in subdirectories.** A toggle on the filter bar (or Ctrl+Shift+F) extends the same text to the whole subtree: results stream into the list as the walk proceeds, each row showing its relative path, and the walk is cancelled the moment the text changes or the toggle goes off. The walk uses the `ignore` crate's parallel walker, respects a configurable ignore list, and is a cancellable job in the operations area when it runs long.

**Indexed search.** If the platform has a search index, a search box appears on the toolbar: it queries Windows Search, Spotlight or, on Linux, Tracker or Baloo through a search-index extension, and shows results in the list with their paths. Without an index the box is absent, not greyed out; detection happens at start and when the index service appears or disappears. Each index connection is its own plugin (search-windows, search-spotlight, search-tracker, search-baloo) against one \`SearchIndex\` trait in \`ext\`, so a connector can be replaced, and a future Rick-owned index would be just another plugin implementing the same trait.

**Context menu.** Right-clicking a file, folder or selection opens a full operations menu, built from the Action registry so extensions add entries in their own group. Default groups:

| Group | Entries |
| --- | --- |
| Open | Open, Open with (app list), Open in new tab, Open in terminal here |
| Edit | Cut, Copy, Paste, Duplicate, Rename, Move to, Copy to |
| Delete | Move to trash, Delete permanently |
| Create | New folder, New file (from templates) |
| Share | Copy path, Copy as link, Compress to archive, Extract here |
| Extensions | Git actions, archive actions, and anything a plugin registers |
| Info | Properties |

Entries that do not apply to the selection are hidden rather than greyed; the same Actions are reachable from a command palette (Ctrl+Shift+P) and from keyboard shortcuts.

### Preview

The preview is its own tab, not part of any explorer tab. It docks and moves like every other tab (right of the explorer by default, or below, or torn out to a second monitor), and there is one of it.

- It follows focus: it always shows the selection of whichever explorer tab was last focused. Select a file in tab A, switch to tab B and select a file there, and the preview switches to that file.
- Space on a file or folder opens the preview tab if it is closed and shows the entry; Space again closes it. While open, moving the selection updates it live.
- A file shows its contents through the Previewer registered for its type (see Supported scope); a folder, or an archive such as a zip, 7z or tar, shows its contents as a read-only file list in the same columns as the explorer, with a stats line; subfolders expand in place, so a user can look inside without navigating or extracting.
- Rendering is asynchronous and cancellable: a quick selection sweep never blocks the list, and a preview that takes longer shows a placeholder first.

### Menu

There is no menu bar; a burger menu at the top of the window holds the global entries.

- **Commands** opens the command palette (also Ctrl+Shift+P): a searchable list of every registered Action, fuzzy-matched as you type, each with its shortcut shown; Enter runs it on the current selection, and recently used commands float to the top.
- Further entries: New window, Settings, Extensions, Keyboard shortcuts, About.

The remaining menu entries will be specified later.

### Further GUI elements

**Tree view.** A folder tree that expands in place, available both as a section in the side panel and as its own dockable tab; selecting a node navigates the focused explorer tab, and the tree follows the explorer's location unless unlinked.

**Theming.** Light and dark themes, following the system by default, with a manual override; font size and row density (compact, normal, comfortable) are settings, and accent colour can be chosen. Syntax highlighting themes follow the app theme.

**Hidden files, symlinks and mounts.** A toggle (Ctrl+H) shows dot-files and files with the hidden attribute. Symlinks show a link badge on the icon and their target in the Type column; a broken link is marked. Mount points show a volume badge, and the side panel's Network and Pinned sections list mounted volumes.

**Icon and thumbnail view.** Besides the details list, a grid view with large icons or thumbnails for images, videos and PDFs, with an adjustable cell size; thumbnails come from the freedesktop thumbnail cache on Linux and are generated in the background elsewhere.

**Dual pane.** One command (F6) lays out two explorer tabs side by side, the classic commander layout; a second press returns to a single pane.

**Inline rename.** F2 or a slow second click edits the name in place in the list, with the extension excluded from the initial selection.

### General behaviour

**Session restore.** Closing Rick and opening it again brings back exactly what was left: every window with its position and size, its dock layout, every tab with its path, sort, filter and scroll position, and the side panel's state. The session is saved continuously, so a crash loses nothing but the last few seconds.

**Saved layouts.** A user can arrange panels and tabs, then save that arrangement as a named layout and mark it as the start layout; Rick then opens with that layout instead of the last session. Layouts are listed in the menu and the command palette for switching at any time; the last session remains available as "Continue where I left off".

**Windows.** New window is one click (menu, Ctrl+N, or the + on the tab strip with Shift) and opens with the start layout. Dragging a tab out of the tab strip and dropping it outside the window creates a new window holding that tab; dropping a tab onto another window's strip moves it there. Every window has the full side panel and shares the operations queue, the preview follows focus across windows.

**Text files.** The preview shows the file's text in a monospace view with two toggles in its small toolbar: word wrap (off = lines keep their real length and the view scrolls sideways; on = long lines break at the pane width, a wrapped continuation marked in the gutter) and line numbers. Both settings persist. Large files are never loaded whole: the previewer memory-maps the file (or reads windows of it) and renders only the visible lines, so a 500 MB log shows its top immediately and scrolls through the rest; line starts are indexed in the background for exact line numbers and scrollbar position, with a byte-offset estimate until that finishes, and highlighting and pretty print work on the visible window only. Memory use is bounded by the window, not the file size, and encoding is detected. Structured and code formats get syntax highlighting: JSON, YAML, XML, HTML, Markdown and every language in the preview list, detected by extension and, failing that, by content; the highlighter is syntect with the theme following the app theme, and the format name shows in the preview toolbar.

**Pretty print.** For JSON, XML, YAML and minified HTML, CSS or JavaScript, a Pretty-print toggle in the preview toolbar reformats the text with consistent indentation and one element per line; it is a view transformation only, the file is never modified, and the raw view is one click away. Invalid input shows the raw text with the parse error highlighted.

**Images.** By default the image is scaled to fit the available pane, keeping its aspect ratio and never upscaled beyond 100%. A 1:1 button (or double-click) switches to actual pixels, where the image can be panned by dragging or with the scroll wheel, and Ctrl+wheel zooms in steps around the cursor; Fit returns to the scaled view. EXIF orientation is applied. Large images decode a downscaled version first for an instant fit view, with the full-resolution decode following for 1:1.

**Media info bar.** Image, audio and video previews carry a small status bar with the basics: dimensions or duration, file size, format, colour depth or codec, and for photos the camera, date taken and exposure. An expander on that bar opens the full metadata panel: every EXIF, IPTC and XMP tag for images (via the `kamadak-exif` and `rexiv2` crates), ID3 or Vorbis tags for audio, and stream details for video, as a searchable key-value list with copy. The panel's open state persists.

**Audio and video.** Media files play inside the preview: play/pause (Space while the preview has focus), a seek bar with elapsed and total time, volume, and for video the frames drawn into the pane with a fit/1:1 toggle like images. Audio shows a waveform and cover art when present. Playback starts on demand, not on selection, stops when the selection changes, and decoding runs through GStreamer on every platform, one code path, with the runtime bundled on macOS and Windows.

**Previews are modules.** Every preview is an extension exposing one or more Previewers, as the architecture section describes, and each ships as its own module that a user can enable or disable. One module may cover several related types, but the split follows who needs it, not the file type: a plain-text previewer is part of the core, while syntax highlighting, pretty print, PDF, Office, images, audio and video are separate modules, so a user who is not a programmer never loads the code tooling. When several previewers match a file, the highest score wins and the user can override the choice per type in settings.

Proposed modules: preview-text (core), preview-code (highlighting, pretty print), preview-pdf, preview-office, preview-image, preview-media (audio, video), preview-archive (folder and archive listing).

**Extensions as installable packages.** Modules are not only switchable, they are physically separate: each is a package in a known extensions directory that can be removed from the disk or simply never loaded, and the core binary carries only the text previewer and the local file system. A package holds a manifest (id, version, the extension points it fills, the platforms it supports) plus its code as a WASM module or a native dynamic library; the loader reads the manifest at start and loads code only when a matching file or action is first needed.

**Extension store.** An Extensions screen in the menu lists installed packages with enable, disable, update and remove, and browses a store of available ones: a signed index hosted by the project, with official modules (the preview set above, Git, archives, object storage) and third-party ones. Install downloads the package for the current platform, verifies its signature and hash, and activates it without a restart. Store entries declare the permissions they need (file system scope, network, process spawn); the user sees them before installing.

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

**Drag and drop.** Dragging a selection onto a folder, another pane, a tab header or a side-panel entry starts a file operation. On the same volume a drag moves; across volumes it copies; holding Ctrl forces a copy, Shift forces a move, and Alt (or right-button drag) asks on drop. A drop target highlights while hovering; hovering a tab header switches to that tab, hovering a folder for a moment opens it (spring-loading). The operation runs as a job in the operations area with progress, pause and cancel, and a conflict on an existing name asks once with skip, overwrite, rename or apply-to-all. Drags to and from other applications use the platform drag-and-drop with file URIs.

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
- **Copy.** Put the selection on the clipboard for copying.
- **Cut.** Put the selection on the clipboard for moving.
- **Paste.** Copy or move the clipboard contents into the current location, running as a job with the usual conflict handling.
- **Create link.** Make a link to the selected file or folder at another location. On Linux and macOS this is a symbolic link; on Windows the user picks between a symbolic link, a directory junction and a classic .lnk shortcut. Also offered as a drag modifier (Alt) and as a paste variant ("Paste as link").
- **Move to trash.** Send the selection to the platform trash (Delete key), restorable from the Trash entry in the side panel.
- **Delete permanently.** Remove the selection without going through the trash (Shift+Delete), after confirmation.
- **Duplicate.** Make a copy of the selection in the same folder with a derived name, such as "name (copy)".
- **Inspect.** Open the system properties for the selection: the Windows Properties dialogue, the macOS Get Info window, and on Linux Rick's own properties dialogue (size, dates, owner, permissions, mime type), since there is no desktop-wide one.
- **Restore.** Put a trashed item back at its original location; available when browsing Trash.
- **Copy path.** Copy the full path of the selection to the clipboard, one per line for multiple items; a variant copies the name only.
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
- **Pin to side panel.** Add the selected folder to the Pinned section; unpin from its context menu.

### Keyboard shortcuts

All shortcuts are remappable in Settings; the defaults follow the common desktop conventions. On macOS, Cmd replaces Ctrl.

| Shortcut | Action |
| --- | --- |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+Tab, Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+N | New window |
| Alt+Left / Alt+Right | Back / forward |
| Alt+Up, Backspace | Parent directory |
| Enter | Open |
| Space | Toggle preview |
| Ctrl+L | Edit path |
| Ctrl+Shift+C | Copy path |
| Ctrl+F | Filter in list |
| Ctrl+Shift+F | Search subdirectories |
| Ctrl+Shift+P | Command palette |
| Ctrl+A | Select all |
| Shift+arrows, Shift+click | Range select |
| Ctrl+click | Toggle entry in selection |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |
| F2 | Rename |
| Delete | Move to trash |
| Shift+Delete | Delete permanently |
| Ctrl+Shift+N | New folder |
| Alt+Enter | Properties |
| Ctrl+H | Toggle hidden files |
| Ctrl+B | Toggle side panel |
| F5 | Refresh |
| F11 | Full screen |
| Ctrl+, | Settings |
| Ctrl+Q | Quit |

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

**Kernel**

- `types` — paths, entries, mime, errors, capability flags; no I/O, no async runtime and no dependency on any other Rick crate. It may use the small ecosystem crates it needs (`bitflags`, `thiserror`), nothing heavier.
- `runtime` — job system, cancellation, progress, thread pools, event bus.
- `platform` — per-OS code behind one trait set: trash, links and junctions, elevation, system properties dialogue, file associations, mounts.
- `vfs` — the Provider trait and the mount table that routes a path to a provider.
- `vfs-local` — the local filesystem provider: listing, statx, watching, copy and reflink.
- `model` — the directory model: cached listings, sort, filter, group, selection, history.
- `ops` — the file operations as jobs: copy, move, trash, delete, rename, link, undo stack, conflict policy.
- `ext` — the five extension traits, the registry, manifests and the extension context.
- `app` — the application state: windows, panes, tabs, layouts, session persistence, settings, the command palette.
- `ui-egui` — all egui widgets and the dock layout; depends on `app`, never the other way round.
- `rick` — the binary: feature flags, wiring, startup.

**Providers** (`ext-vfs-*`)

- `ext-vfs-archive` — zip, tar family, 7z, RAR read-only, single-file codecs.
- `ext-vfs-object` — S3, R2, Azure Blob, GCS through `object_store`.
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
- `ext-preview-pdf` — pdfium-render.
- `ext-preview-office` — LibreOffice headless to PDF, then the PDF path.
- `ext-preview-media` — audio and video via GStreamer, ID3 and stream info.
- `ext-preview-archive` — listing of an archive's contents without mounting it.
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

- `plugins-wasm` — the WASM runtime and the adapters that expose the five traits to guest modules.
- `plugins-native` — loading of signed native libraries for heavy decoders that WASM cannot carry.

Naming rule: a crate that contributes to one extension point carries that point as its prefix (ext-preview-pdf). A crate that is a feature in the user's eyes and contributes to several points, like Git, carries only the feature name (ext-git) and registers a Decorator, an Action group, a Previewer or whatever it needs from one manifest; the user installs, enables and disables it as one thing.

### Build order

types → runtime → vfs-local with the two-pass listing → model → ext with a fake Decorator → ui-egui showing a list with badges → ext-git → previews.

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
- **Every long operation is a Job.** A directory listing, a copy, a search, a preview decode: each runs as a `Job` with a cancellation token, a progress channel and a result. Listings stream entries in batches so the UI shows the first screen before the walk finishes; switching away from a location cancels its jobs.
- **Platform code lives in `platform` only.** No `cfg(target_os)` anywhere else. Other crates program against the `platform` traits (trash, open-with, file properties, config dir, native dialogs) and the tests run with a fake implementation.
- **The extension traits are a versioned contract.** The traits in `ext` and the manifest format carry a version. Any breaking change bumps it; the plugin host refuses manifests it does not understand and says so in the Extensions screen.
- **Optional extensions are Cargo features.** `rick` builds with the kernel alone; each `ext-*` crate is a feature on the binary, default-on for built-ins, so a minimal build and a full build come from the same workspace.
- **Milestones have acceptance tests.** Each step in the build order ends with an executable acceptance test (listing 200k entries under the target, a round-trip copy with cancellation, a plugin loaded from a manifest). The next step does not start until it passes.
- **No shared mutable state across threads without an owner.** Caches, indexes and watchers are owned by one component that exposes a message or async API; `Arc<Mutex<_>>` across crate boundaries is a design smell to justify in a comment.
- **Identity, not paths.** Entries are keyed by a provider-scoped `EntryId` (device and inode on local filesystems), not by path string, so renames and moves keep selection, history and decorations intact.

### Core types and extension traits

First cut of the contract between the kernel and the extensions. Names and shapes are binding; signatures may gain parameters as the implementation learns, but only through a spec change. Everything here is `Send + Sync`; async methods use `async fn` in traits on Rust 2024, with `BoxStream` from `futures` for streamed results.

**Value types** (crate `types`, no I/O):

```rust
/// Which extension provides a tree. "local", "archive", "sftp", "s3", "trash".
pub struct ProviderId(pub &'static str);

/// Stable identity of an entry inside one provider. Local uses (device, inode);
/// other providers choose their own 128-bit key. Never a path string.
pub struct EntryId { pub provider: ProviderId, pub key: u128 }

/// Provider-neutral path. Keeps raw bytes, offers a lossy display form.
pub struct VPath(Vec<u8>);

/// A place a pane can show.
pub struct Location { pub provider: ProviderId, pub path: VPath }

pub enum EntryKind { File, Dir, Symlink { target: VPath, broken: bool }, Other }

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

/// What `runtime` hands back for every long operation.
pub struct JobHandle<T> {
    pub id: JobId,
    pub cancel: CancelToken,
    pub progress: watch::Receiver<Progress>,
    pub result: oneshot::Receiver<Result<T, Error>>,
}

/// One error type for the whole kernel; extensions map their own into it.
pub enum Error { NotFound, PermissionDenied, Cancelled, Unsupported(&'static str), Io(std::io::Error), Extension { ext: ExtId, source: Box<dyn std::error::Error + Send + Sync> } }
```

**Provider** — a tree of entries (crate `ext`):

```rust
bitflags! { pub struct Capabilities: u32 {
    const WRITE = 1; const TRASH = 2; const RENAME = 4; const SYMLINK = 8;
    const REFLINK = 16; const WATCH = 32; const SEEK = 64; const PERMISSIONS = 128;
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

    /// Same-provider copy with the native fast path (copy_file_range, reflink,
    /// server-side copy). Cross-provider copies are composed by `ops` from
    /// `open_read` and `open_write`.
    async fn copy(&self, from: &Location, to: &Location, opts: CopyOptions,
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
    fn info(&self) -> Vec<(String, String)>;                // metadata rows
    fn transport(&mut self) -> Option<&mut dyn Transport>;  // media only: play/pause/seek
}

pub enum Frame { Rgba { width: u32, height: u32, data: Arc<[u8]> }, Text(TextBlock), External(TextureId) }
```

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
    NewTab(Location), CloseTab(TabId), Split { pane: PaneId, dir: SplitDir }, MoveTab { tab: TabId, to: DockTarget },
    CancelJob(JobId), Undo, Redo,
    Settings(SettingsChange),
}

pub enum Event {
    Listed { pane: PaneId, batch: Vec<Entry>, done: bool },
    Changed { pane: PaneId, change: FsChange },
    Decorated { pane: PaneId, items: Vec<(EntryId, Decoration)> },
    JobProgress { id: JobId, progress: Progress }, JobDone { id: JobId, result: Result<(), Error> },
    SelectionChanged { pane: PaneId, ids: Vec<EntryId> },
    PreviewReady { pane: PaneId, doc: Box<dyn PreviewDoc> },
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
| Mod+Shift+P | Command palette |
| Mod+P | Quick open (search by name) |

**Selection**

| Shortcut | Action |
| --- | --- |
| Mod+A | Select all |
| Mod+Shift+A | Invert selection |
| Mod+Shift+S | Select by pattern |
| Space | Toggle selection of the focused entry |
| Escape | Clear selection / close filter |
| Type-ahead | Jump to the first entry starting with the typed letters |

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
| Mod+N | New file |
| Mod+Shift+N | New folder |
| Mod+Shift+C | Copy path |
| Mod+I, Alt+Enter | Inspect |
| Mod+Z | Undo |
| Mod+Shift+Z, Mod+Y | Redo |
| Mod+O | Open with… |
| Mod+Shift+O | Open in system file manager |

**Tabs and panes**

| Shortcut | Action |
| --- | --- |
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
| Mod+Shift+F | Toggle flatten subfolders |
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
- Tabs, docking and split panes (right and down), list and details views, column sizing, sort by every column, filter, hidden-files toggle.
- Core operations: copy, cut, paste, duplicate, move to trash, delete permanently, rename (single), new file, new folder, open with the system default, open containing folder, copy path, back and forward, go to root, refresh.
- Keyboard navigation for all of the above and the command palette.
- Settings in TOML with a minimal Settings screen.
- Previews for text, code (syntect), images and PDF (pdfium-render, first page plus page navigation) in the inspector pane.
- `ext-git`: status badges and branch via gix, and the Git action group (Commit, Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log, Blame) in the context menu and palette inside a repository.
- All built-ins implemented through the `ext` traits, so the architecture is real from day one.
- The performance targets from the Decisions table met on the 200k-entry fixture, on all three platforms.

**Explicit non-goals for 0.1**

- The full-size viewer, office and media previews, archive previews and hex fallback.
- Archive, SFTP, object-storage and trash providers; search indexes.
- SVN and any Decorator beyond file-type icons and Git status.
- Bulk rename, compare, compress and extract, console, pin to side panel.
- The WASM and native plugin hosts and the Extensions screen; extensions are compiled in.
- Localisation beyond English.

### Still open

- Extension security model: what a WASM plugin may touch (capabilities per extension point), and whether native plugins need signing.
- Testing strategy for the performance targets: a generated 200k-entry fixture and a benchmark harness in CI.
- Accessibility: egui's AccessKit support is young; screen-reader support is a known weakness to state and track.
- Distribution: packaging still to choose — Flatpak, Homebrew, winget, or plain binaries.
