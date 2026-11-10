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

Each cloud is its own plugin implementing `Provider` directly, so a user installs only the clouds they use; there is no shared object-storage crate — the overlap (prefix-to-folder mapping, chunked upload, credential storage) is small, and a shared crate may be extracted later if the second plugin shows real duplication. Object stores have no change notifications (polling). Download and upload are ordinary cross-provider copies to and from the local provider, streamed through the machine. **Transfers inside one account never pass through the machine:** a copy or move between two locations of the same provider instance — within a bucket, or across two buckets or containers of the same account — is submitted to the service as a server-side operation (S3 `CopyObject`, Azure's asynchronous Copy Blob, GCS `Rewrite`; move adds a delete), and Rick's job only tracks its progress, polling where the service reports it and cancelling where the service allows. This is the `NATIVE_TRANSFER` capability of the Provider contract, not cloud-specific code: `ext-ssh` runs its same-host `cp` and `mv` through the same hook.

### Archives and compression

| Format | Browse in place | Extract | Create | Crate |
| --- | --- | --- | --- | --- |
| zip (DEFLATE, zstd) | Yes | Yes | Yes | `zip` |
| tar, tar.gz, tar.zst, tar.xz, tar.bz2, tar.lz4 | Yes | Yes | Yes | `tar` + codec crates |
| 7z | Yes | Yes | Yes (LZMA, LZMA2) | `sevenz-rust` |
| RAR | Yes | Yes | No | `unrar`; creation is not licensable |
| gzip, zstd, xz/LZMA, bzip2, LZ4, Brotli, Snappy (single file) | — | Decompress | Compress | Preview by streaming decompression |

Enter on an archive enters it in the current tab by default; the setting `archives.on_open` can hand it to the system instead (see *Opening files* under General behaviour).

### Disk images

Disk images are browsed exactly like archives — listed in the preview pane on selection, entered on Enter, the same `archives.on_open` setting — read-only, through `ext-vfs-archive` and `ext-preview-archive`, without mounting anything. The list is pinned; WIM and SquashFS are out.

| Format | Browse in place | Reader | *Mount* through the OS |
| --- | --- | --- | --- |
| ISO (ISO 9660, UDF) | Yes | Native, as today | Windows, macOS, Linux |
| Raw IMG | Yes | 7-Zip library | macOS, Linux |
| DMG | Yes | 7-Zip library | macOS |
| VHD, VHDX | Yes | 7-Zip library | Windows |
| VMDK, VDI, QCOW2 | Yes | 7-Zip library | — |

The 7-Zip library (`7z.so` / `7z.dll`, LGPL) is bundled in the plugins directory and loaded dynamically as *Third-party libraries* describes; it parses the image containers and the file systems inside them (FAT, NTFS, ext, HFS+, APFS, ISO), which no mature Rust crate does. The formats the native crates already read keep their readers. The *Mount* action and *Eject* are specified under *Disk images and Mount* in General behaviour.

### Preview support

| Category | Formats | How |
| --- | --- | --- |
| Text | Any text file, encoding detected | Native text view |
| CSV | CSV, TSV | Virtualised grid, delimiter sniffed, first row as header; the source one toggle away (`ext-preview-markup`) |
| Markup | Markdown, HTML | Rendered by default, the source one toggle away (`ext-preview-markup`). HTML is a basic reader — headings, paragraphs, lists, links, images, tables — with no browser engine, no JavaScript and no CSS fidelity |
| Code | JavaScript, TypeScript, Java, Rust, C#, F#, VB, C, C++, Perl, PHP, JSON, XML, YAML, HTML, Markdown | Syntax highlighting (syntect); for Markdown and HTML this is the source view the markup previewer toggles to, and what shows when that plugin is disabled |
| PDF | PDF | pdfium-render, page by page, text layer for search and selection |
| Office | DOCX, XLSX, PPTX, ODT, ODS, ODP | Converted to PDF by LibreOffice headless when it is installed (never bundled), then the PDF path; without it, the missing-library message of *Third-party libraries* |
| Images | JPEG, PNG, WebP, BMP, GIF (animated) | `image` crate, downscaled to the pane, EXIF shown |
| Vector images | SVG, SVGZ | `resvg`, rasterised at the size the pane asks for and again at every size change, so it is sharp at any zoom; the source one toggle away (see *Vector images*) |
| Icon containers | ICO, CUR, ICNS, XPM | `ext-preview-icons`: every embedded variant, stepped through as pages, magnified pixel-sharp (see *Icon containers*) |
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

A side panel on the left gives one-click access to the places that matter; the main area holds the workspaces — each a layout of panes — and the preview. The look follows GNOME and libadwaita (Linux first): calm, friendly and accessible, on a 6 px grid with rounded controls, in place of the Visual Studio Code-like chrome of 0.1 and 0.2. The design file is the Figma page *Main view* of *Rick — FileExplorer* (issue 60); this chapter records its decisions, and where the file and this document disagree the question goes to `QUESTIONS.md`, never to the code.

Default layout: the side panel on the left; one workspace tab holding one explorer pane with the preview pane to its right; the side panel is collapsible and can dock right, the preview is a pane like any other and can sit below or in its own window, and every pane's footer holds its status and its filter.

### Look and feel

**Scheme tokens.** Every colour the interface draws comes from a named token set, the *scheme*; no widget hard-codes a colour, and the egui `Visuals` are derived from the tokens, never the other way round. The token names and values are the Figma variable collection *Rick*, one to one, and its two modes are the two schemes Rick ships, *Light* and *Dark*. A scheme is a swappable value, so further schemes can follow without touching a widget; which schemes beyond these two, and whether a user may define one, is not designed yet.

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `bg/window` | `#ededed` | `#2b2b2b` | The window background behind the panes |
| `bg/titlebar` | `#ece0db` | `#332c2c` | The band of the title row. The values are the *Dusty rose* preset; the *Title bar colour* setting writes the chosen pair into the token (see *Title row*) |
| `bg/panel` | `#f7f7f7` | `#262626` | The side panel, and the header band of the tables in previews |
| `bg/pane` | `#fafafa` | `#1e1e1e` | Panes, the active workspace tab, text fields |
| `bg/card` | `#ffffff` | `#2e2e2e` | Cards, menus, the operations drawer, the tables in dialogues |
| `bg/subtle` | `#ececec` | `#333333` | The column header band, drive cards, the parent chip, hovered rows and buttons, the tracks of the drawer's bars |
| `bg/control` | `#e4e4e4` | `#3a3a3a` | Resting controls: segmented backgrounds, the icon-size group, the round cancel button, secondary buttons |
| `bg/selected` | `#90d1ff` | `#2e5e8f` | Strong selection, reserved; the lists use the soft one |
| `bg/selected-soft` | `#cfe6ff` | `#24456a` | Selected rows in the focused pane, the active menu entry, the active settings category, active toggles, the selection pill |
| `bg/selected-inactive` | `#e4e4e4` | `#3a3a3a` | Selected rows in a pane without focus |
| `bg/active` | `#90d1ff` | `#2e5e8f` | A tool button while pressed |
| `bg/track` | `#e0e0e0` | `#3d3d3d` | The track of a drive's usage bar |
| `bg/switch-off` | `#8e8e8e` | `#7a7a7a` | A switch that is off |
| `text/primary` | `#2b2b2b` | `#ededed` | Body text |
| `text/secondary` | `#6b6b6b` | `#a0a0a0` | Column headers, section headers, footer statistics, the Size, Type and Modified columns, inactive tabs, detail lines, shortcuts in menus |
| `text/placeholder` | `#737373` | `#979797` | Placeholder text in fields |
| `text/on-selected` | `#1f528c` | `#a9d2ff` | Text on `bg/selected-soft` |
| `icon/default` | `#4a4a4a` | `#cfcfcf` | Icons at rest |
| `border/default` | `#d6d6d6` | `#3d3d3d` | Hairlines, dividers, field and card borders, the window border |
| `border/strong` | `#939393` | `#777777` | Checkbox borders and other small controls that need more contrast |
| `accent/focus` | `#2873c0` | `#4ea1ff` | Blue text, focus rings, the icon of an active toggle, a switch that is on, progress bars, the portal animations, the filter-active dot |
| `accent/button` | `#2a77c6` | `#2975c7` | The background of a primary button, with white text on it |
| `accent/usage` | `#3c84c2` | `#5ab0ff` | The fill of a drive's usage bar |
| `status/success` | `#1d7943` | `#6fd49a` | A hash that matches: field border, pill text, row label |
| `status/success-bg` | `#d9f2e0` | `#1e3b2a` | The *Matches* pill |
| `status/success-row` | `#edfaf0` | `#1b2e22` | The matching hash row |
| `status/warning` | `#8c5c00` | `#f2c46b` | Warning text, such as the missing version in the installer |
| `status/warning-border` | `#d98c0d` | `#d98c0d` | The border of a field with a warning |
| `status/danger` | `#b3261a` | `#ff8a80` | Destructive menu entries (*Move to trash*, *Delete permanently*), a hash that does not match, errors |
| `trash/low-bg`, `trash/low-text` | `#e2e2e2`, `#5c5c5c` | `#3a3a3a`, `#bdbdbd` | The Trash pill under 1 % |
| `trash/mid-bg`, `trash/mid-text` | `#fce8b2`, `#8a5a00` | `#4a3a12`, `#f2c46b` | The Trash pill from 1 % to 5 % |
| `trash/high-bg`, `trash/high-text` | `#f9d3d0`, `#a61b12` | `#4d2220`, `#ff9c93` | The Trash pill over 5 % |
| `dialog/bg` | `#fafafa` | `#2a2a2a` | The background of dialogues and of the settings window |

`accent/focus` and `accent/button` stay separate: the first is for blue text, focus and active states, the second only for the filled primary button. Every text-on-background and control pair in both schemes meets WCAG contrast — 4.5:1 for text, 3:1 for controls and icons — and a unit test holds the shipped pairs to it; a pair added later is checked the same way before it ships. The scheme follows the system by default, light or dark, with a manual override (the `theme` setting: `system`, `light`, `dark`); the syntax highlighting of previews follows the scheme. The accent-colour setting of 0.1 and 0.2 is withdrawn: a free accent would break the checked pairs, and more colour comes as further schemes. `bg/titlebar` is the one token a setting writes: the scheme stays a swappable set, the *Title bar colour* setting replaces the token's two values, and what is drawn on the band is chosen by contrast with it (*On the band* under *Title row*), so the rule holds for every preset and for a colour the user picks.

**Density.** Spacing comes from a second token set, the Figma collection *Density*, with two modes the user chooses in Settings → Appearance: *Comfortable*, the default, and *Compact*. The row-density setting of 0.1 and 0.2 with its three levels is replaced by these two. Everything on the 6 px grid derives from them; no widget hard-codes a size.

| Token | Comfortable | Compact | Use |
| --- | --- | --- | --- |
| `titlebar/height` | 40 | 34 | The title row |
| `tab/height` | 34 | 28 | A workspace tab, sitting on the bottom edge of the title row |
| `button/size` | 34 | 28 | A square tool button; a toolbar and a footer are `button/size` plus twice `space/toolbar-y` tall |
| `search/height` | 28 | 22 | The search box in the title row |
| `row/height` | 32 | 24 | A row of the file list |
| `sidebar/item-height` | 34 | 22 | A row of the side panel |
| `space/row-x` | 12 | 6 | Horizontal padding of list rows and side-panel rows |
| `space/toolbar-x` | 12 | 8 | Horizontal padding of toolbars and footers |
| `space/toolbar-y` | 6 | 4 | Vertical padding of toolbars and footers |
| `radius/control` | 6 | 4 | Corner radius of buttons, fields, chips and side-panel rows |

Fixed whatever the density: 16 px icons; a 34 px column header band; 220 px workspace tabs with 10 px top corners; a 320 px search box; 4 px progress and usage bars; 30 px entries in the context menus and 32 px in the View options menu and the application menu; 8 px corners on cards, drive cards and menus; 12 px on dialogues and the operations drawer; fully round pills.

**Typography.** The system UI font — the one the desktop uses for its own windows — at 13 px for body text, 12 px for secondary text (column headers, footer statistics, detail lines, shortcuts in menus) and 11 px medium, letter-spaced capitals for section headers and pills; Ubuntu in the Figma file is a stand-in, not a choice. Hashes, paths in dialogues and tables, and code are set in the system monospace font. The font-size setting stays and scales the whole interface.

**Icons.** Phosphor is the icon set, through the `egui-phosphor` crate, regular weight at 16 px in `icon/default`. The icons in the Figma file are stand-ins drawn in Phosphor style; the app uses the real glyphs. Every glyph in the application comes from the set — tool buttons, tab icons, file-type icons, badges, menu entries, the empty-folder illustration — and no Unicode symbol character is ever used as an icon, so no glyph can fall back to a box. Back and Forward are `ARROW_LEFT` and `ARROW_RIGHT`, the parent chip `ARROW_UP`, refresh `ARROW_CLOCKWISE`, copy `COPY`, the view options `SLIDERS_HORIZONTAL`, the pane handle `DOTS_SIX_VERTICAL`, the hidden-files toggle `EYE` and `EYE_SLASH`, a waiting job `HOURGLASS`, the hash dialogue `HASH`.

**Controls.** Tool buttons are ghost buttons, `button/size` square with `radius/control` corners: no frame at rest, `bg/subtle` on hover, `bg/active` while pressed; an active toggle keeps a tint, `bg/selected-soft` behind an icon in `accent/focus` — the fill weight no longer marks an active toggle. A primary button is filled `accent/button` with white text; a secondary button is `bg/control` with `text/primary`; a caution button, for a choice that overwrites something, is `trash/mid-bg` with `trash/mid-text` — the amber of the Trash pill's middle level, not the red of a destructive entry; a disabled button fades to 50 %. A text field is `bg/pane` with a 1 px `border/default` border and `radius/control` corners, the border `accent/focus` while it has focus; a leading icon sits inside it. A switch is a 32 × 18 pill, `accent/focus` when on, `bg/switch-off` when off, with a white knob. A segmented control is a `bg/control` track with the chosen segment raised as a `bg/card` tile. A checkbox is 16 px with a `border/strong` border, filled `accent/focus` with a white check when ticked. A pill is fully round with 11 px medium text and 8 px horizontal padding. A card is `bg/card` with a 1 px `border/default` border and 8 px corners; rows inside it are separated by hairlines. A menu is a `bg/card` card with a soft shadow; its entries are 30 px tall with a 16 px icon slot, a 13 px label, and the shortcut in 12 px `text/secondary` at the right edge; the hovered or active entry is `bg/selected-soft`; groups are separated by hairlines and, where the menu has titled groups, headed by 11 px capitals in `text/secondary`.

### Title row

Rick draws its own window chrome; the platform's title bar is turned off. Where Rick draws the controls (Linux, Windows) the window is transparent behind a 10 px margin that holds a light shadow and a one-pixel border in `border/default`, with invisible resize grips along the edges and corners; a maximised or fullscreen window has no frame; macOS keeps the system's border and resize handles. Dragging an empty part of the title row moves the window; double-clicking it maximises or restores.

The top of every window is one **title row**, `titlebar/height` tall on a tinted band, `bg/titlebar` (*Title bar colour*, below), laid out left to right:

- **Menu slot.** The burger menu as a tool button, alone in a slot as wide as the side panel, so it sits above the panel and the first workspace tab starts where the panes begin. On macOS the traffic lights keep their place at the left and the burger moves right of them.
- **Workspace tabs**, from the panes' left edge, then the new-tab button directly after the last tab (below).
- **Search box.** The search box, also the command palette, 320 px wide and `search/height` tall, a text field with a leading `MAGNIFYING_GLASS` and the placeholder *Search, or > for commands*; right-aligned before the window group and centred vertically in the row. It keeps the `bg/pane` fill of a text field and has no border at rest, so it reads as a light field on the band; the focus ring still shows while it has the focus. It gives way, narrower, only when the window is too narrow to hold tabs, box and controls.
- **Window group.** Two layout toggles — the side panel and the preview pane, each an icon-only toggle with the tint of an active one — then a hairline divider, then minimise, maximise and close drawn by Rick as plain icons. The icons of the group are 16 px and sit 12 px apart, with the divider in the middle of a 25 px gap between the toggles and the window controls; each control answers over the whole of its 28 px step and the height of a tool button, so the three window controls are neither crowded nor small to hit. Where the system draws its controls (macOS) the three buttons are absent and the toggles end the row.

**Search box.** One box serves search and commands. Plain text searches by name — the current location, and the platform index once a `SearchIndex` is available — and results drop down below the box; a leading `>` switches the box to commands, listing every registered Action fuzzy-matched as you type, each with its shortcut, Enter running the chosen one on the current selection and recently used commands floating to the top. Mod+P focuses the box empty, Mod+Shift+P focuses it with `>` already typed. Escape clears it and returns focus to the list.

**Title bar colour.** The title row sits on a band of its own colour, and the active workspace tab lifts out of it in `bg/pane` — the idea of Chrome's tab strip, not its colour. The band is the token `bg/titlebar`, and its colour is a setting, *Settings → Appearance → Title bar colour*: six presets, each a light and a dark value so that one choice works in both schemes, and *Custom* with two colour fields, one per scheme. The default is *Dusty rose*. The token lives in the scheme, so the theme stays a swappable set; the setting writes its two values into the token at start and whenever it changes, and the choice is kept in `rick.toml` as `titlebar_colour` (`dusty-rose`, `warm-grey`, `slate-teal`, `sage`, `plum`, `dark-band` or `custom`) with `titlebar_custom_light` and `titlebar_custom_dark` as `#rrggbb` for *Custom*.

| Preset | Light | Dark |
| --- | --- | --- |
| Dusty rose (default) | `#ece0db` | `#332c2c` |
| Warm grey | `#e6e2dc` | `#2c2a28` |
| Slate teal | `#d9e4e4` | `#263233` |
| Sage | `#dfe6d8` | `#283027` |
| Plum | `#e4dcea` | `#3a2e4a` |
| Dark band | `#2f3237` | `#2f3237` |

**On the band.** The active tab is `bg/pane` with the scheme's own colours whatever the band; everything else in the row is drawn on the band and is chosen by contrast with it. A band is *dark* when white contrasts with it more than `#2b2b2b` does, and *light* otherwise: every preset is light in Light and dark in Dark, except *Dark band*, which is dark in both.

- **A band that agrees with the scheme** — light in Light, dark in Dark — is drawn from the scheme: icons in `icon/default`, inactive tab titles in `text/secondary`. Where `text/secondary` does not reach 4.5:1 on the band the titles are the band's own text colour at 78 % instead — `#2b2b2b` on a light band, white on a dark one — and at full strength should 78 % still fall short. That is the case of the five light presets in Light, on which `text/secondary` reaches 4.0 to 4.2:1 and `#2b2b2b` at 78 % reaches 5.8 to 6.0:1; in Dark `text/secondary` reaches 4.8:1 or more on every preset and stays.
- **A dark band in Light** — the *Dark band* preset, or a dark *Custom* colour — switches the row to light text, as frame `167:5261` draws: inactive tab titles white at 78 %; the icons — the menu button, the tab icons and close buttons, the new-tab button, the layout toggles and the window controls — white at 90 %; the divider white at 25 %; a hovered tool button and a hovered inactive tab white at 12 %; the search box white at 14 % with its placeholder white at 70 % and its text white at 90 %. A light *Custom* colour in Dark is the mirror image, with `#2b2b2b` in place of white.
- **Hover.** An inactive tab under the pointer is `bg/pane` at 50 % over the band, except in the case above, where that would give a mid grey no title reads on.

A unit test holds every preset, in both schemes, to 4.5:1 for the tab titles and the search box's text and to 3:1 for the icons, at rest and on hover, as the scheme's own pairs are held; *Custom* is covered by the same rule, not by a test.

### Workspace tabs

Tabs live in the title row, Chrome-style, and each tab is a whole **workspace**: its own layout of panes with their locations, histories, selections and dividers. Switching tabs swaps the entire window content. Panes have no tab strips of their own — the per-pane strips of 0.1 and 0.2 are gone — so the title row is the only place with tabs, and a window with two panes shows them in one workspace, not as two strips.

- **Shape.** A tab is 220 px wide and `tab/height` tall, sitting on the bottom edge of the title row with 10 px top corners and 2 px between tabs: a 16 px icon, the title in 13 px truncated with an ellipsis, and a close button (`X`). The active tab is filled `bg/pane`, lifting out of the title row's band, and joins the pane area below it with no line between and no accent line on it, its title medium in `text/primary`; an inactive tab has no fill and shows the band, its title in `text/secondary` and its close button at half opacity; hovering an inactive tab shows `bg/pane` at 50 % over the band. *On the band* under *Title row* says where the band's colour changes these. The new-tab button (`PLUS`) is a 28 px wide ghost button directly after the last tab, never at the far right.
- **Title.** A workspace is titled automatically from the folder in its focused pane — the folder's name, or a virtual location's title — and updates as focus or folder changes; there is no renaming. The icon is the one the side panel uses for that folder when it is a place (Home, Downloads, Trash and so on), the provider's icon for an archive, a remote host or a cloud account, and `FOLDER` otherwise. A workspace whose focused pane is a console is titled *Console* — not the folder the console was opened in, and not what the shell sets as its title.
- **Behaviour.** `+` opens a new workspace with one pane at the focused pane's location; Mod+T does the same, Mod+W closes the workspace tab, Mod+Tab and Mod+Shift+Tab cycle, and middle-click closes any tab. *Open in new tab* everywhere — the side panel, the context menu, a middle-click on a folder — opens a new workspace with one pane at that location, next to the current one. Tabs move by drag (next) and, from the keyboard, with Mod+Shift+PageUp and PageDown, which shift the active tab one place left or right. What happens when tabs outgrow the row is not designed yet: until it is, the row clips and the tabs stay reachable through Mod+Tab and the command palette.
- **Dragging a tab.** Hovering a tab shows the grab cursor, the open hand; dragging it shows the grabbing cursor, the closed hand, wherever the cursor goes, and never the arrow. A click activates a tab, a drag does not: the window goes on showing the workspace it showed. *Along the row* the tab reorders as in Chrome: lifted with a soft shadow, it follows the cursor along the row, held at the point where it was grabbed, and its neighbours slide apart, animated, to open the gap where it will land; there is no insertion line, and on release the tab settles into the gap. *Off the row* the lifted tab follows the cursor as its drag ghost (see Panes), and what the tab holds decides where it can go. A tab holding **exactly one pane** can be dragged down into the workspace the window shows: the dock targets appear on the pane under the cursor — left, right, top and bottom with the preview of the half, and no centre, the tab being a tab already — and a drop on one moves the tab's pane into that layout on that side, the tab leaving the row; it is the reverse of dropping a pane on the tab row. A tab holding **several panes** shows no dock targets: it can only be reordered, moved to another window's row or torn off into a window of its own, and its panes move one at a time by their handles. Any tab dropped on another window's tab row moves there, into the gap that row opens for it, and one dropped outside every window becomes a new window; released anywhere else it returns to its place.
- **Tab bar as drop target.** A pane dragged by its handle (below) and dropped on the tab row, or on the centre target of a pane, becomes a new workspace tab at the drop position; the row shows an `accent/button` slot where the tab would land. Another window's tab row takes the pane the same way.
- **Spring-loaded tabs.** While a pane, or a tab holding exactly one pane, is dragged, resting on another workspace tab for the spring-loading delay (see Drag and drop under Functions) switches the window to that workspace, so the drag can go on to its dock targets; dragged entries switch it the same way, to reach a folder in another workspace. A dragged pane rests on a tab while the cursor is on it; a dragged tab, which rides the row under the cursor, rests on a neighbour while it overlaps it by a quarter of a tab or more, short of the half where the two would swap. A pane dropped on the tab itself still becomes a new tab. The tabs of another window's row spring-load alike.

### Side panel

Docked on the left by default, 270 px wide, in `bg/panel`; the user can move it to the right side and hide it entirely, with a shortcut (Mod+B) and the title-row toggle to bring it back; a collapsed icon-strip mode comes later. Its side, width, visibility and folded sections are part of each window's session state, not of `rick.toml`. The panel's width is draggable.

The panel is navigation only: it takes the user places and never performs a file operation — the panes are the drop targets. It holds flat lists, never a tree; the folder tree lives in the Tree view mode and in the preview panel. Nothing in it highlights the current location; the breadcrumb says where the pane is. Top to bottom it shows the **Places** list, then three titled sections — **This Machine**, **Network**, **Cloud** — and, while jobs run, the **operations drawer** attached to the bottom.

**Rows.** A row is `sidebar/item-height` tall with `radius/control` corners and `space/row-x` padding, inset 6 px from the panel's edges: a 16 px icon, 12 px of gap, the label in 13 px, and at the right whatever the row carries (the Trash pill, a drive's percentage). Hover is `bg/subtle`; the keyboard focus ring is a 2 px `accent/focus` border. Rows are not highlighted for the current location.

**Section headers.** Left-aligned small grey capitals — 11 px medium `text/secondary` with 0.66 px letter spacing — with a `CARET_DOWN` at 60 % opacity before the title (`CARET_RIGHT` when folded), 18 px of air above and 6 px below. Every titled section's header is itself a location: clicking it opens an overview of the section in the focused pane (a virtual location, below); clicking a row opens that row's root. Middle-click, Mod+click and the context menu's *Open in new tab* open any row or header in a new workspace tab. With the panel focused, Up and Down move, Enter opens, Mod+Enter opens in a new tab. The titled sections fold and expand independently with the caret on the header; the rows are not indented and no vertical indent guide is drawn. A section with nothing in it stays visible and shows an *Add…* row (`PLUS_CIRCLE`) — that is how connections and accounts are created — except Cloud, which is absent entirely while no cloud provider plugin is loaded.

**Places.** A flat, user-owned list at the top of the panel with no header. It is seeded from the operating system's standard folders and is fully editable. Defaults, in this order: Home, Documents, Pictures, Music, Video, Downloads, Favorites, Desktop, Recent, Trash. The standard folders are resolved from platform conventions — XDG user-dirs on Linux, the known folders on Windows, the standard directories on macOS — and a folder that does not exist on the system is left out of the defaults. Places is for the handful of folders wanted one click away; Favorites holds the larger collection of pins and opens as a view.

Three default entries are virtual locations rather than folders. Clicking one switches the focused pane to its view; the panel never lists their contents inline.

| Entry | What the view shows | Notes |
| --- | --- | --- |
| Favorites | The pinned folders and files | *Pin to side panel* adds here; the view lists the pins as entries, *Unpin* in each entry's context menu; a pinned folder opens in the pane, a pinned file opens its folder with the file selected |
| Recent | The system's recently-used list | The XDG recently-used store on Linux, best-effort equivalents on macOS and Windows; Rick keeps no recent list of its own |
| Trash | The platform trash | Restore and *Empty Trash* live inside the view only; the row carries the Trash pill below |

**Trash pill.** The Trash row shows the trash's size — not its item count — in a pill at the right end of the row (`2.3 GB`), tinted by the share of its disk the trash takes: `trash/low-*` under 1 %, `trash/mid-*` from 1 % to 5 %, `trash/high-*` over 5 %. The thresholds are to be fine-tuned during implementation. An empty trash shows no pill. The size and the share come from the trash provider's statistics and refresh with the trash.

Editing Places: a folder dragged from a file list and dropped on the list is added as a place at the insertion line; a dropped folder *always* becomes a place and is never moved or copied. Files are not accepted anywhere in the panel; the drag shows a not-allowed cursor over it. Rows reorder by drag and drop. A row's context menu offers *Remove*, *Edit…* (title and icon; the icon is picked from the Phosphor set, `FOLDER` by default) and *Open in new tab*; default rows can be removed and reordered like custom ones. The list persists in its own TOML file in the per-user config directory.

**This Machine.** Every drive connected to the machine, fixed and removable, in one list whatever the platform (mount points on Linux and macOS, drive letters on Windows). Each drive is a **drive card**: a `bg/subtle` card with 8 px corners and 6 px between cards. In Comfortable it is 44 px tall: the drive icon (`HARD_DRIVE`) centred on the left, then the name in 13 px, on one line and truncated with an ellipsis when it is long, with the utilisation percentage in 12 px `text/secondary` at the right end of the first line, and under them a 4 px usage bar on a `bg/track` track with 2 px corners. In Compact it is 26 px, one line: icon, name, the bar, the percentage. The bar's fill follows how full the drive is, like the Trash pill: `accent/usage` below 80 %, `status/warning` from 80 %, `status/danger` from 90 %. **Read-only media** — a live USB, an optical disc, a mounted ISO, anything the operating system mounted read-only — is full by design: its bar is full in `text/placeholder`, with no threshold colour and no percentage, and nothing else sits in the row; hovering the bar shows the tooltip *Read-only*, which is also the bar's accessible description. A **removable drive** carries an eject button (`EJECT`) at the right edge of the card, vertically centred and always visible, never on hover only; the name, the percentage and the bar end before it. A click unmounts the drive and ejects it; on a mounted ISO volume it unmounts. Clicking a card opens the drive's root; clicking the header opens the This Machine overview, a virtual location with the full drive table: name, mount point or drive letter, file system, total, used, free, utilisation bar and whether the drive is removable. Removable drives offer *Eject* in the card's context menu and in the overview too. The list follows mounts, unmounts and hot-plug live; there is no manual refresh.

**Network.** Anything addressed by a host name. For now it lists the network file systems already mounted on the system (an SMB mount, an NFS export) and, with `ext-ssh`, one row per SSH host (`specs/ext-ssh.md`); *Add connection…* creates a connection. Clicking a row opens its root; the header opens the Network overview. The list follows mounts and unmounts live. Discovery of hosts and shares on the local network is deferred (see Version 0.2).

**Cloud.** Anything addressed by an account: the object-storage providers contributed by plugins (`ext-vfs-s3`, `ext-vfs-azure`, `ext-vfs-gcs`), one row per configured account or container, with `CLOUD` as the icon. Clicking a row opens its root; the header opens the Cloud overview. Shown with *Add…* while a cloud plugin is loaded and no account is configured; hidden entirely while no cloud provider plugin is loaded.

**Virtual locations.** The three overviews and the Favorites, Recent and Trash views are Provider-backed virtual paths, so they sit in a pane, in the breadcrumb, in history and in session restore like any folder. The scheme is `rick://` (`QUESTIONS.md`): `rick://favorites`, `rick://recent`, `rick://trash`, `rick://machine`, `rick://network` and `rick://cloud`.

**Operations drawer.** Running long operations — copying a large file, extracting an archive, an upload to object storage — show in a drawer at the bottom of the side panel: a `bg/card` card the full width of the panel, attached to the window's bottom edge with only its top corners rounded (12 px), a hairline on top and a soft shadow upwards, floating over the end of the panel's lists. It appears only while a job runs and folds away when the queue is empty; when the side panel is hidden the drawer sits along the window's foot instead, so a running job is never invisible. Top to bottom:

- **Summary.** One line — *2 running · 2 waiting* in 13 px medium, the overall percentage in `accent/focus` at the right, and a caret — over a 4 px overall progress bar (`accent/focus` on `bg/subtle`). Clicking the summary collapses the drawer to this summary and bar, and expands it again; the collapsed state is remembered for the window.
- **Running jobs**, one block each under a hairline: the job's icon (the Portal spinner, below, for a copy or move; `ARCHIVE` for an archive job, and so on), its title in 12 px medium on one line truncated with an ellipsis, a round 22 px `bg/control` cancel button (`X`) at the right; under them a 4 px thin bar, and under that one detail line in 11 px `text/secondary` — *473 MiB of 2.2 GiB · 33.7 MiB/s · 53 s left*, or the job's message. Text never sits inside a bar. A pausable job shows a pause or resume ghost button before the cancel button; a paused job's detail line reads *Paused*.
- **Waiting jobs**, under a *WAITING* header in 10 px letter-spaced capitals: compact one-line rows — `HOURGLASS` at half opacity, the title in 12 px `text/secondary`, the round cancel button — and under the row, indented to the title, why it waits: *Queued behind the move on the same volume*.
- **Finished jobs.** A completed job stays listed briefly as a compact row with its result, then leaves; a failed job stays as a compact row with its error in `status/danger` and a dismiss button until dismissed. An error is never lost to the ellipsis: at rest the row shows it on one line, cut with an ellipsis when it is long, and a click on the row expands it to the whole text, wrapped over as many lines as it needs, and folds it again; hovering shows the whole text as the tooltip. The failure is also reported where the user acted: an error toast at the top right of that window carries the job's name and the whole error text, wrapped and never cut, and stays until it is dismissed with its close button (`X`) — a notice that is not an error still leaves by itself after a few seconds.

The drawer grows with the number of jobs up to a cap, after which its rows scroll; progress repaints are throttled so that a job over hundreds of thousands of files does not redraw the panel on every entry.

*Concurrency.* Jobs are scheduled per volume: two jobs that share a source or destination volume run one after the other, jobs on disjoint volumes run in parallel. A queued job shows under *Waiting* with the reason. Pause, resume and cancel act per row; cancelling a running job starts the next queued job on that volume.

### Main area

The main area shows the active workspace: its panes, each an explorer view with a toolbar above the file list and a footer below it, and the preview pane, which has the same frame.

**Panes.** A workspace holds one or more panes in nested splits, as in an IDE: side by side or stacked, dividers dragging to resize, two-pane, three-pane and grid layouts all possible. This is egui\_dock's tree of splits, without its tab bars. A pane is a `bg/pane` surface; panes are separated by a hairline, and the focused pane is told by its selection colour (below), never by a border. Every pane has a fixed **grab handle** at the top right of its toolbar: a 28 px wide ghost area with `DOTS_SIX_VERTICAL` at 70 % opacity after a hairline divider. Hovering shows the grab cursor, the open hand; dragging shows the grabbing cursor, the closed hand, for as long as the drag lasts and wherever the cursor goes — a drag never shows the arrow. Dragging the handle moves the pane: while it drags, the **drag ghost** (next paragraph) follows the cursor, and **dock targets** appear on the pane under it — five rounded `bg/card` tiles in a cross (left, right, top, bottom, centre), each with a split icon, the one under the cursor filled `accent/button` — with a translucent `accent/button` preview of the half the pane would take. Left, right, top and bottom split that pane and dock the dragged pane on that side; the centre, or the tab row, makes the dragged pane a new workspace tab; outside every window opens a new window. Resting on another workspace tab switches the window to that workspace, so the pane can be docked into a layout that was not showing, and another window takes the pane on its tab row or on its dock targets (see Workspace tabs, and Windows under General behaviour). F6 (Dual pane) splits the focused pane to the right with a second pane at the same location, and back again; a pane leaves a workspace by being dragged out, so a workspace never ends up empty. The layout, panes and their paths are restored on the next start.

**Drag ghost.** What follows the cursor while a pane or a workspace tab is dragged is the thing itself, never an abstract card. For a pane it is a real snapshot of the grabbed pane — toolbar, list and footer as they looked when it was grabbed — scaled to about a quarter and drawn at 70 % opacity, with 8 px corners, a 1 px `border/strong` border and a soft shadow. For a tab it is the tab itself, lifted with the same shadow. The ghost is anchored to the grab point: the point that was grabbed — on a pane's ghost the scaled grip — sits 8 px down and right of the cursor, so that the cursor stays visible, and the ghost moves rigidly with the cursor, never trailing it and never far from it.

**Toolbar.** One row per pane, `button/size` plus twice `space/toolbar-y` tall with `space/toolbar-x` padding, made of groups that never overlap: the navigation pair on the left, the breadcrumb flexing in the middle, and the fixed group on the right. Only the breadcrumb flexes; long paths clip so the right group and the handle stay visible whatever the window size.

| Group | Control | Action | Shortcut |
| --- | --- | --- | --- |
| Left | Back (`ARROW_LEFT`) | Previous location in this pane's history | Alt+Left, mouse button 4 |
| Left | Forward (`ARROW_RIGHT`) | Next location in history | Alt+Right, mouse button 5 |
| Centre | Parent chip (`ARROW_UP`) | Parent directory; the first element of the breadcrumb | Alt+Up, Backspace |
| Centre | Breadcrumb | The current path; see Path bar | Mod+L to edit |
| Right | View options (`SLIDERS_HORIZONTAL` with `CARET_DOWN`) | The view options menu, below | Mod+Shift+1 … 4 for the view modes, Mod+H for hidden files, F5 for refresh |
| Right | Handle (`DOTS_SIX_VERTICAL`), after a divider | Drag the pane | — |

Up, hidden files and refresh have no buttons of their own any more: Up is the parent chip, the other two live in the view options menu and keep their shortcuts. Other controls are not added to the toolbar; a customisable toolbar is a later idea, not designed.

**Path bar.** The breadcrumb starts with the **parent chip** — `ARROW_UP` on a `bg/subtle` chip with `radius/control` corners, click for the parent folder — then a 14 px hairline divider, then the path as clickable segments separated by `CARET_RIGHT`, the last segment in medium weight; clicking a segment jumps there. **Editing.** Mod+L, or a click on the empty part of the breadcrumb bar, swaps the breadcrumbs for a text field in place: `button/size` tall — the height of Back and Forward, on their line — and as wide as the breadcrumb, `bg/control` with a 1 px `accent/focus` ring and `radius/control` corners, the path in 13 px `text/primary`. The full path is selected on entry, so typing replaces it. Enter navigates to the typed path; Escape restores the breadcrumbs unchanged, and so does losing the focus. The breadcrumb never overlaps its neighbours: when the path is wider than the room between the navigation pair and the right group, segments collapse from the left into a single chevron (`CARET_DOUBLE_LEFT`) that drops down the hidden ancestors, the rightmost segments stay visible, and if even the last segment does not fit it truncates with an ellipsis. A copy button (`COPY`) sits at the right end of the breadcrumb, inside it, and copies the folder's path — the Copy path action.

**View options menu.** One button in the toolbar's right group replaces the view-mode dropdown, the hidden-files toggle and the refresh button. Its menu, a 260 px card, has three titled groups:

- *VIEW*: **Details**, **List**, **Icons** with a segmented control of three icon sizes (small, medium, large) at the right of the row, the current mode as the active entry; then two switches, **Show hidden files** (Mod+H) and **Folders first** (folders always on top regardless of sort).
- *REFRESH*: **Refresh now** (F5). There is no automatic refresh; Rick reacts to file-system events.
- *FILTER*: **Show all**, then one checkbox row per file type present in the folder — its icon, the type name and its count at the right — folders included as *Folders*; ticking types shows only those entries. The filter is per pane and clears on navigation. While a type filter is active the view options button carries a 7 px `accent/focus` dot at the top right of its icon.

The Tree view mode and the extra-large icon size stay reachable through the command palette and their shortcuts; the menu lists what the design shows.

**File list.** A details list, one row per entry, `row/height` tall, folders first by default. Default columns in order:

| Column | Content |
| --- | --- |
| Icon + Name | An icon for what the entry is (folder, text, image, archive, video, and so on), then the name in `text/primary`; the column flexes |
| Size | Human-readable, right-aligned, `text/secondary`; folders show an em dash until a size is requested |
| Type | The file type as text, e.g. Folder, SVG, TOML, `text/secondary` |
| Modified | Last modification time, relative for recent entries, full on hover, `text/secondary` |

This is the basic set and will be extended later; columns sort on click, resize, reorder and can be hidden, and extensions add columns through Decorators (for example a Git status column). The **column header** is a 34 px tinted band — `bg/subtle` between two hairlines — with the titles in 12 px `text/secondary` and the sort glyph after the sorted column's title, and no permanent divider lines between the titles: the details view is line-free. **Resizing.** The boundary between two headers has a hit zone about 6 px wide; hovering it shows a 1 px divider in `border/strong` across the header row and the column-resize cursor, and the divider goes when the pointer leaves. Dragging the boundary resizes the column before it, a double-click on the boundary fits that column to its content, and no column gets narrower than about 48 px. Every column resizes, Name included; the last column takes the remaining width. Column widths are part of the view's state and come back with the session. The body draws no grid lines: no vertical column separators and no horizontal row lines; rows are set apart by their padding alone. An optional setting alternates the row background colour (zebra striping) for easier reading of wide rows; it is off by default.

**Row states.** A row's background has `radius/control` corners inside a 6 px inset from the pane's edges. At rest it has none. Hover is `bg/subtle`. A selected row in the focused pane is `bg/selected-soft` with its text in `text/on-selected`; a selected row in a pane without focus is `bg/selected-inactive` with unchanged text, so the focused pane is the one with blue selection. The keyboard cursor is a 2 px `accent/focus` ring on the row, over whatever background it has, so a selected row with the cursor shows both. In the Icons views the same colours apply to the cell behind the icon and label.

**Empty folder.** A folder with nothing to show draws, centred in the list: `FOLDER_OPEN` at 64 px in `icon/default` at reduced opacity, *This folder is empty* in 13 px medium, and under it in 12 px `text/secondary` *Drop files here, or press Ctrl+Shift+N for a new folder.*, the shortcut as the platform spells it. A folder emptied by the filter instead reads *No entries match the filter*. The permission-denied state is not designed yet; the failure banner over the list stays as it is.

**Loading.** While a listing is being read and no row has arrived yet, the list is empty but for the **Portal burst** (see Loading animations) centred in the content area and one status line under it in 12 px `text/secondary`: *Reading folder… 12,480 items so far*, the count updating as entries stream in. It replaces the grey skeleton rows. Once the first rows are drawn the animation goes and the count continues in the footer.

**Sorting.** Default order is by name, alphabetically; clicking a column header sorts by that column (name, size, type, modified) and clicking again reverses it. The *Folders first* switch, on by default, keeps all folders above all files whatever the sort column; switched off, folders and files are mixed into one order. Both settings persist per user, with an optional per-folder override.

**Footer.** One row at the foot of every pane, as tall as the toolbar (`button/size` plus twice `space/toolbar-y`), under a hairline, with `space/toolbar-x` padding; the preview pane's footer has the same height. Left to right:

- The **hidden-files toggle**, icon only: `EYE` with the active tint while hidden files are shown, `EYE_SLASH` as a plain ghost button while they are hidden; no text.
- The **statistics** in 12 px `text/secondary`: `FOLDER` and the folder count, `FILE` and the file count, the total size of the files; while a filter narrows the list the counts read as *12 of 340*.
- A hairline divider and, only while something is selected, the **selection pill**: `bg/selected-soft` with `text/on-selected`, *1 selected · 568 B*.
- The **Filter field** at the right, 200 px wide, 26 px tall: a text field with a `MAGNIFYING_GLASS` at 60 % opacity, the placeholder *Filter*, and inside its right end the **include-subfolders toggle** (`TREE_STRUCTURE`), tinted while on. The field supports wildcard patterns (`*.svg`, `rider*`), a plain substring otherwise; typing while the list has focus fills the field and narrows the list live (type-to-filter), Escape clears it and returns focus to the list, Enter opens the first match, Ctrl+F focuses it. With the toggle on, or Ctrl+Shift+F, the same text searches the subtree: results stream into the list as the walk proceeds, each row showing its relative path, and the walk is cancelled the moment the text changes or the toggle goes off. The walk uses the `ignore` crate's parallel walker, respects a configurable ignore list, and is a cancellable job in the operations drawer when it runs long.

The classic "jump to the first entry starting with the letter" behaviour does not exist; typing filters.

**Indexed search.** If the platform has a search index, the title row's search box queries it — Windows Search, Spotlight or, on Linux, Tracker or Baloo through a search-index extension — and shows results in the list with their paths. Without an index the box searches the current location only; detection happens at start and when the index service appears or disappears. Each index connection is its own plugin (search-windows, search-spotlight, search-tracker, search-baloo) against one `SearchIndex` trait in `ext`, so a connector can be replaced, and a future Rick-owned index would be just another plugin implementing the same trait.

**Context menu.** Right-clicking a file opens a 260 px menu card built from the Action registry, so extensions add entries in their own group; right-clicking the empty area of the list — below the last row, or anywhere in an empty folder — clears the selection and opens the folder's menu. Each entry has an icon slot, its label and its shortcut; destructive entries (*Move to trash*, *Delete permanently*) are in `status/danger`; a submenu opens to the right. Groups, separated by hairlines, in this order:

| Group | Entries on a file |
| --- | --- |
| Open | Open, Open with ▸, Open in new tab, Open in terminal here, Pin to side panel / Unpin |
| Edit | Cut, Copy, Paste, Paste as link, Copy as ▸ (Name, Path, Path and name — the three copy actions), Duplicate |
| Change | Rename, Move to trash, Delete permanently |
| Extensions | Git ▸, Compress ▸, Install as application, Compute hash…, and anything a plugin registers; the whole group and its separator are hidden when no entry applies |
| Info | Properties |

| Entries on empty space |
| --- |
| New folder, New file |
| Paste, Paste as link, Select all |
| Open terminal here, Open in system file manager, Refresh |
| Folder properties |

Plugin actions are contextual and appear only when the selection or folder supports them: Git inside a working tree, Compress and Extract by the selection (archives get Extract), Install as application on supported archives (tarballs, zip), Compute hash on files only. Entries that do not apply are hidden rather than greyed: each Action's `availability` decides, so the Figma frames show the groups, their order and their look, and every registered Action that applies appears in its group. `Hidden` and `Disabled` entries are both absent from the context menu; the search box in command mode (Mod+Shift+P), which lists everything, shows `Disabled` ones greyed with their reason as a tooltip. The same Actions are reachable from keyboard shortcuts. The contents of *Open with*, and the menus for a folder and for a multiple selection, are not designed yet; until they are, a folder and a selection get the file menu with the entries that apply.

### Preview

The preview is its own pane, not part of any explorer pane, and there is one of it per window. It sits in the workspace's layout like every other pane (right of the explorer by default, or below, or torn out to a second monitor), with the same toolbar height, handle and footer. **It is Rick's only viewer.** There is no full-size viewer, no viewer tab and no second preview: every file of every type, audio and video included, is shown in this one pane, which the selection updates in place, and no previewer and no command may open another pane, tab or window to show a file. Enter and a double-click never preview a file; they open it (see *Opening files* under General behaviour).

- It follows focus: it always shows the selection of whichever explorer pane was last focused. Select a file in pane A, focus pane B and select a file there, and the preview switches to that file.
- Space on a file or folder opens the preview pane if it is closed and shows the entry; Space again closes it. While open, moving the selection updates it live.
- A file shows its contents through the Previewer registered for its type (see Supported scope); a folder, or an archive or disk image such as a zip, 7z, tar or ISO, shows its contents as a read-only tree in the form *Tables in previews* gives, with a stats line; subfolders expand in place, so a user can look inside without navigating or extracting.
- Every kind of content a previewer can return is laid out by the pane itself: text and raster frames, the file list of a folder or archive, the grid of a CSV file and the rendered document of a Markdown or HTML file. A previewer's textual fallback is for a pane that cannot show the richer form, never the normal case.
- Rendering is asynchronous and cancellable: a quick selection sweep never blocks the list, and a preview that takes longer shows the **Portal burst** centred in the content with *Loading preview…* under it, in place of a placeholder. Loading is always that burst — for a local file and for a download from a remote source alike — and never a small spinner or a one-line *Opening…* in a corner of the pane.

**Pane layout.** The content is the point of the pane; everything else is one row. Top to bottom: the toolbar, the File info card when it is pinned, the content, and a one-line footer. The pane shows no heading of its own — the workspace tab's title is enough — and the word "Inspector" appears nowhere in it. The look of the content itself is not part of the design refresh and stays as it is.

**Toolbar.** One row, the explorer toolbar's height, three parts:

- **Left:** the file-type icon of the current entry (Phosphor), and nothing else — no file name.
- **Middle:** the active previewer's controls, so the row changes with the content. Text and code: line numbers, word wrap, and pretty print when the format supports it. Markdown, HTML, CSV and SVG: *Rendered*, a toggle between the rendered view (on for every newly shown file) and the source. Images: Fit and 1:1. Paged documents: previous page, `n / N`, next page — a PDF's pages, a GIF's frames, an icon container's variants. Media: the transport. Controls come from two sources — the kind of document (text layer, pages, raster) supplies the standard ones, and the previewer's `controls()` adds its own, such as pretty print.
- **Copy** (`COPY`), in the middle group of every text-based preview — text, code, Markdown, HTML, CSV: it puts the whole file's text on the clipboard as text, the raw source for the rendered formats, and never the file itself. The whole file, not the window the pane has loaded.
- **Right:** the **info button** (`INFO`) — one button for every file, whatever its type — then the close button, then the hairline divider and the pane handle.

**File info card.** One card shows a file's facts, for every previewer; the info badges of 0.2 and 0.3, one per section, are gone. It is a card as *Controls* describes, in three parts.

- The **head**, 60 px tall: a 36 px `bg/subtle` tile with 8 px corners holding the file-type icon at 20 px; the file name in 13 px medium on one line, cut with an ellipsis; and under it, in 12 px `text/secondary`, the type and the size, *JPEG image · 7.0 MiB*.
- The **rows**, 32 px each, under a hairline: the label in `text/secondary` in an 88 px column, the value in `text/primary`, and at the right end a copy button (`COPY` at 14 px, faint at rest and full under the pointer) that copies the value. A value that does not fit is cut with an ellipsis and shown whole as its tooltip; nothing wraps. Paths and permission strings are monospace.
- When there are more fields than the card shows, a last line in `accent/focus`: *Show all 11 fields*, the number being that of all the fields, with a `CARET_DOWN`. It unfolds the rest in place and, its caret turned up, folds them again; an unfolded card taller than half the pane scrolls inside itself.

*The fields.* The rows are the previewer's fields in the order its sections give them — its File info section first, then its further sections (EXIF, tags, stream details), then the sections Decorators contribute (Git's branch and last commit) — with the entry's *Modified* time, which the pane supplies, after the previewer's *Size* row. The card shows the first five as a popover and the first four when pinned, as the frames draw, and the rest behind *Show all n fields*, where each further section starts under its title in 11 px capitals. Dates read `YYYY-MM-DD HH:MM`.

*Where it appears.* The same card in three places.

- **Popover.** While the pointer is over the info button, or over the card that hover opened, the card hangs under the button over the content, its right edge on the button's, at most 360 px wide, with a soft shadow. Its head ends in a pin button (`PUSH_PIN`). Leaving both closes it.
- **Pinned.** A click on the pin, or on the info button, pins the card: it sits at the top of the content, the pane's full width inside the content's margin, and pushes the content down. The pin button and the info button then carry the tint of an active toggle (`bg/selected-soft` behind an `accent/focus` icon), and a click on either unpins. Whether the card is pinned is remembered per previewer and persists across files and restarts — pinned for photos, it stays pinned while arrowing through photos and says nothing about PDFs — in `[preview.pinned]` of `rick.toml`, keyed by the previewer's id.
- **Centred.** When the pane has a file and nothing to show for it, the card sits centred in the content, 400 px wide or the pane's width less its margins, and carries what the user can do about it as buttons in an **action row** under the rows: 32 px buttons, left-aligned and 8 px apart, wrapping onto a second line when they do not fit, the first one primary. The cases are *No previewer* (below), a missing library (*Third-party libraries* under General behaviour), an error — one row, *Preview*, holding the message, and *Retry* — and the size limit of *Remote sources*. A stack trace or a path dump is never laid out in the pane.

The rows that used to sit above the content — name, type, size and modified above a placeholder; encoding, line ending, BOM and lines, or whichever section was pinned, above a text or a picture — are gone from the content area and live in the card only. The footer keeps its one-line summary.

*EXIF.* A photo's card lists only the tags Rick knows by name and that have a value: no row for an empty value, none for one that is not a number (`NaN`, a zero denominator), none under a raw tag number. A photo without GPS data has no GPS rows. An exposure time reads as a fraction, `1/120 s`, and a date as `YYYY-MM-DD HH:MM`.

**Nothing selected.** With no file selected the pane shows, centred in the content: a 72 px `bg/subtle` circle holding `FILE` at 32 px in `icon/default`; *No file selected* in 14 px medium `text/secondary`; and under it *Select a file to preview it here* in 12 px `text/placeholder`. The toolbar holds only the info button — with nothing to show, and inert — the close button and the handle, and the footer is empty.

**Text defaults.** Text and code open with line numbers on and word wrap off. Toggling either in the toolbar changes the current view and saves the new value as the default, so the choice holds for every later file and across restarts. Pretty print is the exception: it is off for every newly shown file, must be pressed each time, and is never remembered.

**Width.** The content never sets the pane's width. The pane is sized by the layout and the user's divider alone; the previewer is asked to render at the pane's width, a page or image wider than that is scaled down to fit (keeping its aspect ratio, never enlarged above 100%), and long text lines scroll sideways inside the pane. Nothing drawn in the pane may request a minimum width beyond a small fixed floor, so a wide PDF page shrinks rather than pushing the divider.

**Footer.** The bottom of the pane is the footer row, the explorer footer's height, holding a single line of 12 px `text/secondary` with the previewer's `status()` facts separated by `·`: language, encoding and line count for text and code; dimensions, format and file size for an image; page count and file size for a PDF; duration and codec for media. It never wraps; when the pane is too narrow it truncates with an ellipsis.

**No previewer.** When no previewer claims the file the pane shows the File info card, centred — not a hex dump, and not a bare line of text. The head reads the name and, under it, the type, the size and *no previewer for .bin*, naming the file's extension (*no previewer for this type* when it has none); *plugin available* takes that place when a previewer plugin in the plugins directory claims the file's type but is disabled. The rows are *Location* (the folder, monospace, the home directory written `~`), *Modified* and, where the platform has them, *Permissions* (the mode string and `owner:group`, monospace). The action row holds **Open as hex** (`CODE`, the primary button), which shows this file in the pane with the hex view of `ext-preview-hex` — this file only, until the selection moves on, and absent while that plugin is not loaded — and **Open with…** (`FOLDER_OPEN`), the *Open with* action on the file. Enabling or fetching a plugin from here waits until plugin distribution is decided. The host answers "available" from the manifests alone, without loading a disabled plugin: a plugin's manifest lists the file extensions its previewers claim. The hex view is not an automatic fallback: it claims no file on its own, and is shown on request from this card or chosen per type in settings like any other previewer.

**Tables in previews.** Every table a preview shows — the archive tree, the CSV grid, and later ones such as SQLite or a spreadsheet — has one form.

- **Header.** A 30 px band in `bg/panel` with a hairline in `border/default` below it and the column titles in 12 px medium `text/secondary`.
- **Rows.** `row/height` tall, with the file list's padding and fonts, and striped: the first data row is `bg/pane`, the second `bg/subtle`, and so on, so that a tinted row never sits directly under the header.
- **Column dividers.** Always visible, unlike the file list's: a 1 px `border/default` line between every pair of header cells, in the header row only and never through the data rows. The dividers are the resize handles. Under the pointer a divider turns `border/strong` and the cursor becomes the column-resize cursor; dragging it resizes the column before it, a double-click fits that column to its content, no column gets narrower than 48 px and the last column takes the rest. Widths are remembered per previewer, in `rick.toml` with the pane's other remembered choices — the archive tree's by column, the CSV grid's by position, a column beyond those remembered starting at its default.
- **Archive tree.** The columns are Name, Type, Modified, Size and Packed, Packed only where the format reports it. The Name column holds a caret for a folder (`CARET_RIGHT`, `CARET_DOWN` when it is open), the icon and the name, indented 18 px per level.

**Remote sources.** On a provider with the `REMOTE` capability — an SSH host or a cloud account, and every remote provider alike: SSH, Azure Blob, S3 and GCS — whole-file previewers (image, PDF, Office, media, archive) are subject to a size limit, 10 MiB by default; prefix previewers (text, code, Markdown, CSV) read a prefix, offer *Load more* and are never limited. The limit is one setting for all remote sources, on *Settings → Previews*, beside the per-host override [`specs/ext-ssh.md`](specs/ext-ssh.md) describes. Above the limit the pane shows the File info card, centred. Its head reads the name and, under it, the type, the size and the source: *JPEG image · 18.4 MiB · Azure Blob, flowcptstorage*, the provider's name and the account or host. In place of rows it has *Not previewed automatically* in 13 px medium and, in 12 px `text/secondary`, one sentence that names the limit: *This file is larger than the 10 MiB limit for automatic previews on remote storage. Previewing it downloads the whole file.* Its action row holds **Preview this file** (`DOWNLOAD_SIMPLE`, the primary button), which downloads this one file, and **Always preview on <source>**, which switches the limit off for that source from then on: every file on that account or host previews without asking, in this session and after a restart. The row wraps when the source's name is long; no button floats free of the card. The switch is per source — one SSH host, one cloud account — stored as `preview_always = true` in that host's or account's configuration beside its optional override, and it is shown and reversible on *Settings → Previews*, in the host or account editor and in the source row's context menu in the Network or Cloud section. Rick never turns it on by itself; it is the user saying the link is fast enough.

*Downloading.* Either button clears the card at once, and the pane shows the Portal burst as for every other load. While a whole file is being fetched from a remote source — after either button, on a source that is always previewed, or under the limit — the burst has under it a slim progress line, 220 × 4 px in `accent/focus` on `bg/track`, and *Downloading…  7.7 of 18.4 MiB* in 12 px `text/secondary`. Escape cancels the download — before it clears a filter or the selection — and the pane returns to the card. When the download ends the preview takes the card's place and the footer shows its usual summary.

### Menu

There is no menu bar; the burger menu at the left of the title row — the application menu — holds the global entries.

- The command palette has no menu entry: it is the title row's search box in command mode (Mod+Shift+P, or type `>`), always visible.
- It follows the pattern of the View options menu and the context menus: a `bg/card` card with a soft shadow, 32 px entries with `radius/control` corners, each with a 16 px icon, its label in 13 px and, at the right edge, its shortcut in 12 px `text/secondary` or the caret of a submenu (`CARET_RIGHT`); the entry under the pointer is `bg/selected-soft`, and the groups are separated by hairlines. The card is at least 280 px wide, wider when an entry needs it, and opens below the menu button with its left edge on the button's and a 6 px gap.
- Three groups — window and tools, configuration, about:

| Group | Entry | Icon | At the right edge |
| --- | --- | --- | --- |
| Window and tools | New window | `APP_WINDOW` | Its shortcut |
| | Layouts | `LAYOUT` | The submenu caret |
| | One submenu per extension that puts actions in the application menu: *Installer* | The icon of the extension's group in the context menu, `PUZZLE_PIECE` when it has none; `DOWNLOAD_SIMPLE` for the Installer | The submenu caret |
| Configuration | Settings | `SLIDERS` | Its shortcut |
| | Keyboard shortcuts | `KEYBOARD` | Its shortcut |
| | Extensions | `PUZZLE_PIECE` | Its shortcut |
| About | About Rick (the About dialog, [`specs/about-dialog.md`](specs/about-dialog.md)) | `INFO` | The version of the running build, `0.4.0` |

- Every entry has an icon. No label ends in "…": *Settings*, *Keyboard shortcuts* and *Extensions* are written as GNOME writes them. A shortcut hint comes from the keymap, never from a table of its own: the entry shows the chord bound to it — the first when it has several — and nothing when it is unbound. There is no theme switch in the menu: the scheme follows the system by default and is changed in Settings → Appearance.
- The submenus are cards of the same form and keep their entries.

### Dialogues

Every dialogue is a `dialog/bg` window with 12 px corners and a soft shadow, laid out in three bands: a fixed **header** with a 16 px icon or an app icon, the title in medium weight and a secondary line under it, and a round close button at the right where the dialogue has one; a **body** that scrolls when the window is too short; and a fixed **footer** under a hairline with the buttons right-aligned — the primary action last and filled `accent/button`, *Cancel* before it as a secondary button — and any note (the archive's name, a hint) at the left. Labels sit left of their controls in a two-column grid; grouped settings sit in cards with hairlines between rows; a group of rows gets a caps heading in `text/secondary` (*WHAT RICK WILL DO*). Paths and hashes are monospace. A title or a secondary line too long for its header is cut with an ellipsis before the close button: nothing in a header runs under the button, and no dialogue grows wider for a long name. The question and name-prompt dialogues keep their contents and take this frame; the paste-conflict, properties, keyboard shortcuts and extensions dialogues are drawn below.

**Install dialogue** (`ext-installer`, [`specs/ext-installer.md`](specs/ext-installer.md) for the behaviour). 560 px wide. The header shows the app icon at 48 px with 12 px corners, the name, and under it a mode pill — *Update*, *Reinstall*, or none for a new install — followed by the version change, *2026.2.3 → 2026.2.4.1*. The body, top to bottom: **Name**, **App ID** (monospace) and **Version** as text fields; **Install for** as a segmented control, *Just me* (the default) and *All users*; a framed card with **Kind** (GUI application / Command-line tool), **Main executable** (monospace) and **Icon** as dropdowns — the icon row shows the icon at 20 px, its path and its size, *bin/rider.svg · scalable*; the *WHAT RICK WILL DO* card with one row per effect, an icon, a label and the detail in monospace: *App files* with the destination, *Launcher* (desktop entry and icon), *Command* with the link and its directory, *Previous version* with what is kept and that it can be reverted; and an **Advanced options** disclosure card, collapsed by default with a one-line summary of what it holds (*Category, StartupWMClass, commands, PATH variable, SHA-256, overwrite, downgrade*), expanded showing those rows in the same label-left, control-right form. The footer names the archive at the left and has *Cancel* and the primary button, *Install*, *Update* or *Reinstall*. States:

- **Version missing.** The field shows the placeholder *e.g. 2026.2.4.1* with a `status/warning-border` border and, under it, a `WARNING` glyph with *Couldn't read the version from the archive — please enter it.* in `status/warning`; the header reads *2026.2.3 → ?*, the App files row shows `<version>`, and the primary button is disabled until a version is typed.
- **No icon in the archive.** Rick generates a monogram — the name's first letter, white on a colour derived from the App ID — and the icon row reads *No icon found · generated* with a *Choose…* button that picks an SVG or PNG instead; the header shows the same monogram, and the desktop entry gets it too.
- **Commands.** For a toolchain, *Commands to link* is a disclosure row with a count pill, *3 of 7*, and a *Select all* link in `accent/focus` at the right; expanded, a checklist of the executables in `bin/`. Collapsed, the count stays visible.

On confirm the dialogue closes at once and the install runs as a job in the operations drawer.

**Hash dialogue** (*Compute hash*, see Functions). 620 px wide; the header has `HASH`, *File hashes*, the file's name and size on the secondary line — cut with an ellipsis before the close button when the name is long — and the close button. The body: a *VERIFY* heading over the **Verify** field — on top, because comparing is the main use. On open the clipboard is checked: if it holds something that looks like a hash (hex of MD5, SHA-1, SHA-256 or BLAKE3 length, or Base64 of one of those) the field is prefilled with it, compared at once, and carries a small *from clipboard* hint until edited. A match gives the field a `status/success` border and a `status/success-bg` pill at its right end with a check and *Matches SHA-256*, and tints the matching row `status/success-row` with its label in `status/success`; no match gives a `status/danger` border and a *No match* pill. Under the field, a card with one row per algorithm — the label, the value in monospace, a copy icon at the right — then the **Format** segmented control (*hex*, *HEX*, *Base64*). The footer has *Copy all* as a secondary button with `COPY` at the left and *Done* as the primary at the right. While hashing, the hash card is hidden and the **Portal burst** sits centred in its place with the progress line under it — *Hashing…* in medium, then *64.0 MiB of 343 MiB · 210 MiB/s · 1 s left* — and a 4 px bar; *Copy all* is disabled and a *Cancel* button sits left of *Done*, only during the calculation; Cancel stops the job and closes the dialogue immediately. The card returns, filled, when hashing is done. With several files selected the body repeats the hash card per file, each under a row naming the file, and Verify compares against every row.

**Paste-conflict dialogue** (*Replace file?*). It replaces the "already exists" dialogue of 0.3 and is asked by every copy or move that meets an existing name, a paste and a drop alike; what the choices do is under *Execution* in Functions. 520 px wide and never wider: the file name is never in the title, and the dialogue does not grow for a long name. The header has no icon: the title is the question, *Replace file?*, followed by *(3 of 12)* when the operation has several conflicts; the secondary line names the destination folder, *A file with this name is already in ~/Work/test*; and the close button. The body holds two cards, 12 px apart: **Existing file** and **New file**, the one being pasted, which has a 1.5 px `accent/focus` border in place of the hairline. Each card has its caption in 11 px capitals in `text/secondary` and one row under it: a 36 px `bg/subtle` tile with 8 px corners holding the file-type icon, the name in 13 px medium on one line, cut with an ellipsis at its end, and under the name the size and the modified time in 12 px `text/secondary`, *48.2 MiB · Yesterday 16:52*, so that the two can be compared before choosing. Under the cards, only while further conflicts remain, a checkbox: *Do the same for the remaining 9 conflicts*, which applies the button pressed next to all of them. The footer differs from the other dialogues': **Cancel** at the left, a secondary button that stops the whole operation, and at the right **Skip** (secondary), **Replace** (a caution button) and **Keep both** (primary). *Keep both* is the default — Enter chooses it — because it loses nothing. Escape and the close button are *Cancel*. There is no rename field. Between two folders the dialogue reads *Merge folder?* with the captions *Existing folder* and *New folder*, **Merge** takes the place of *Replace*, there is no *Keep both* and Enter is *Skip*; a folder's card shows its modified time and no size. A file and a folder of one name are asked about as two files are — *Replace*, which removes the existing item first, and *Keep both* — with the title and the captions naming what each is: *Replace folder?*, *Existing folder*, *New file*.

**Properties dialogue.** 560 px wide. The header has a 40 px `bg/subtle` tile holding the entry's icon, the name as the title, cut with an ellipsis, the type and the size as the secondary line — *Folder · 12 items, 3.4 GiB*, *Symbolic link · 56 B* — and the close button. Under the header a segmented control switches between two tabs, **General** and **Permissions**; the single long page of 0.3 is gone. Both tabs are built from groups, a caps heading over a card, whose rows have the label in `text/secondary` in a 96 px column and the value after it. Paths are monospace, cut with an ellipsis, and every path row ends in a copy button (`COPY`).

- *General.* **Location**: *Location*, the folder that holds the entry; for a symbolic link also *Target*, the path the link holds, with a copy button of its own, and *Resolves to*, the type and size of what it points at — *Executable file · 2.1 MiB*, or *Missing* for a broken link; for a regular file also *Media type*. **Size**: *Contents* (*12 items · 3.4 GiB*) and *On disk* for a folder, *Size* and *On disk* for a file; a symbolic link has the group **Link** in its place, with one row, *Size*: *56 B (on disk 4 KiB)*. A folder's size is counted in the background by a cancellable job that is not listed in the operations drawer: the secondary line and *Contents* read *Calculating…* until the result is in, and closing the dialogue cancels the count. **Dates**: *Modified*, *Accessed* and *Created*, as `YYYY-MM-DD HH:MM`. **Options**, not for a symbolic link: *Hidden*, a switch that shows whether the entry is hidden; it is not changed here.
- *General, footer.* **Close** as the primary button. For a symbolic link, **Open target folder** at the left, a secondary button: it shows the folder that holds the target in the pane the dialogue was opened from, with the target selected, and closes the dialogue; it is disabled for a broken link. Where the system has a properties window of its own (Windows, macOS), *System properties…* sits at the left too.
- *Permissions.* **Ownership**: *Owner* and *Group*, each a 150 px dropdown listing the machine's users or groups; its last entry, *Other…*, opens the dropdown with a search field for a name that is not listed. **Access**, where the file system has mode bits: a grid of checkboxes with the rows *Owner*, *Group* and *Others* and the columns *Read*, *Write* and *Execute*, and under it the row *Octal* — the mode as four digits in a monospace field, which can be edited and through whose first digit setuid, setgid and sticky are set — with the special bits that are on named at its right end in 12 px `text/secondary`, *setgid · sticky*, or *setuid · setgid · sticky: off* when none is. **Advanced**: *Access control list (ACL)*, a row with a one-line summary under its title (*1 extra entry: user qemu --x*) and a caret, which opens in place, not in a second window, into the ACL table on `bg/subtle`; and, for a folder, *Apply to enclosed items* with *Set the same permissions on everything inside* under it and a switch.
- *The ACL table.* For a POSIX list: the columns *Read*, *Write* and *Execute*, and the rows *Owner (serge)*, *Owning group (serge)*, one row per named entry — the name, a small *user* or *group* tag and, at the right end, a remove button (`X`) — then *Mask* and *Others*; under them the **Add entry** line, a name field (*User or group name*), a *User* / *Group* segmented control and an *Add* button (`PLUS`) that is enabled once a name is typed. The table and the Access grid show one set of permissions and move together: *Owner* and *Others* are the same bits in both, and once the list has named entries the grid's *Group* row is the mask, as `chmod` and `ls -l` have it, the owning group's own bits being the table's *Owning group* row. The other kinds of list — NFSv4, macOS, NTFS — open in the same place with the entries and rights they showed in 0.3.
- *Permissions, footer.* **Revert** and **Apply** (primary), both disabled until something on the tab differs from what the entry has. *Apply* does what *Permission management* under Functions describes; *Revert* puts the controls back.

**Keyboard shortcuts dialogue.** 760 px wide and about 660 px tall. The header has *Keyboard shortcuts*, the secondary line *Click a shortcut to change it. Changes apply at once.* and the close button; the body is two columns.

- **Categories**, on the left, 190 px: rows 30 px tall with `radius/control` corners — an icon, the name and, at the right, the number of actions in 12 px `text/secondary` — the chosen one `bg/selected-soft`. *All* comes first; then the built-in categories *Navigation* (`FOLDER`), *File operations* (`COPY`), *View* (`EYE`), *Selection* (`LIST_CHECKS`), *Panes & tabs* (`COLUMNS`), *Search* (`MAGNIFYING_GLASS`) and *Preview pane* (`IMAGE`); then one category per plugin that contributes actions, named after the plugin and carrying its icon — *Git*, *SVN*, *SSH*, *Console* and so on. A plugin's actions are filed under the plugin and nowhere else, never under *File operations*, so every action appears exactly once. No two actions share a name either: Git's and SVN's commit are *Git commit* and *SVN commit*, here as in the command palette and in the plugins' own submenus, so the list never shows two rows alike.
- **Search**, on the right, above the list: a text field with a leading `MAGNIFYING_GLASS` and the placeholder *Search actions or press a shortcut*. Typed text narrows the list to the actions whose name contains it; a chord pressed in the field — a key with Mod or Alt, or a function key — is shown in the field as key caps and narrows the list to what that chord is bound to.
- **List.** A grey column header, 28 px in `bg/panel` between two hairlines, with *Action* and *Shortcut* in 12 px medium `text/secondary`; under it the rows, grouped by category under headings in 11 px capitals, a plugin's heading ending in *· PLUGIN* (*GIT · PLUGIN*). A row is 34 px: the action's name, the shortcut cell (200 px) and a reset button.
- **Key caps.** A shortcut is drawn as key caps, not as text: each key a 22 px `bg/card` cap with 5 px corners and a `border/strong` border, 2 px at its lower edge, the key's name in 11 px medium, and *+* in `text/secondary` between the caps — Ctrl + Shift + C. Mod is written as the platform's key, *Ctrl* on Linux and Windows and *Cmd* on macOS. An action bound to several chords shows them one after the other. An unbound action reads *Not set* in `text/placeholder`.
- **Changing a shortcut.** A click on the shortcut cell records: the row turns `bg/selected-soft`, the cell becomes a `bg/card` field with a 1.5 px `accent/focus` ring holding a 6 px `accent/focus` dot and *Press keys…  Esc to cancel*, and the next chord pressed becomes the action's shortcut, replacing what it had. Backspace clears the binding (unbinds it); Escape leaves it as it was. A row has no *Change*, *Reset* or *Unbind* buttons.
- **Reset.** A row whose shortcut differs from its default carries a 6 px `accent/focus` dot at the end of the cell and a live reset button (`ARROW_U_UP_LEFT`, the undo arrow, not the refresh circle) with the tooltip *Reset to default: Ctrl+F2*, naming the default; on a row at its default the icon is faint, at 35 %, and does nothing.
- **Conflicts.** A chord bound to two actions is allowed and is shown, not refused: both rows carry an amber chip, *also: Layouts*, naming the other action — `trash/mid-bg` with `trash/mid-text`, 11 px medium — until one of the two is changed. While it stands the chord runs the built-in action before a plugin's, and otherwise the first of the two in the list. The preview pane's shortcuts apply only while the preview has the focus, so a chord there and the same chord in the list are not in conflict.
- **Footer.** *Reset all to defaults* at the left, a secondary button, and *Close* as the primary at the right. Every change applies at once and is written to the `[shortcuts]` table of `rick.toml`.

**Extensions dialogue.** 840 px wide, master and detail in place of the flat table of 0.3. The header has *Extensions*, a secondary line that counts — *23 installed · 22 enabled · 1 needs attention* — and the close button.

- **List**, on the left, 360 px: a search field (*Search extensions*) that narrows the list by name or id, then the plugins grouped by what they contribute, under headings in 11 px capitals: *PROVIDERS*, *PREVIEWERS*, *ACTIONS*, *SEARCH*. A plugin that fills several extension points is listed once, under its main one — the first it fills of provider, previewer, search index, action — so SSH is a provider and Git an action; a plugin that fills only the decorator point, of which none ships, would get a fifth heading, *DECORATORS*. A row is 34 px: the plugin's icon, its name and its enable switch; the chosen row is `bg/selected-soft`, a disabled plugin's row is dimmed, and an 8 px `status/warning` dot before the switch marks a plugin that needs attention. The list has no version column, and a plugin that is loaded says nothing about it: only problems are shown.
- **Details**, on the right, for the chosen plugin. A head: a 48 px `bg/subtle` tile with the plugin's icon; its name in 15 px medium; under it the id, the version and the kind, *vfs-azure · 0.4.0 · Provider*, and a chip, *Built-in plugin* for one that ships beside the binary or *User plugin* for one in the per-user plugins folder; and the enable switch at the right. Then the description from the plugin's manifest; an **attention card** when the plugin needs attention; and an **info card** with the rows *Contributes* (the extension points it fills and what it fills them with: a provider's scheme, a previewer's file types), *Status* (*Loaded*, *Disabled*, or what stopped it), *Location* (the plugin's folder) and *Config* (its settings file, when it has settings), the two paths monospace, every row with a copy button. There is no *Permissions* row until the plugin permission model exists.
- **Attention.** A plugin needs attention when the host refused or failed to load it — the card gives the host's reason — or when it has reported a third-party library missing in this session, as *Third-party libraries* under General behaviour describes. The card is `bg/card` with a 1.5 px `status/warning` border, never an amber fill: the dot, *Needs attention* in 13 px medium, and the text in 12 px `text/secondary`.
- **Actions**, at the foot of the details: **Settings**, only when the plugin has a settings page, which opens the settings window on that page; **Open folder** (`FOLDER_OPEN`), which shows the plugin's folder in the system file manager; and, at the right, **Remove**, a secondary button with its label in `status/danger`. A built-in plugin can be disabled but not removed: the button is hidden for it. *Remove* deletes the user plugin's folder at once, without a question.
- **Footer.** At the left, as plain text: *Plugins live in the plugins folder beside the binary and in the config directory; a per-user plugin wins on an id clash.* At the right **Install from file…** (`DOWNLOAD_SIMPLE`), which picks a plugin package — an `.rkp` file, a zip archive of the plugin's folder — and unpacks it into the per-user plugins folder, and **Close** as the primary button. Installing checks nothing and asks nothing: the archive is unpacked as it is, under its own name and never outside the plugins folder, a plugin already there under that name is replaced, and whether the result is a plugin is found out by the loader at the next start, as for a folder put there by hand. The trust check of [`specs/system-integration.md`](specs/system-integration.md) §9 comes later, with its milestone.
- **Enabling.** Switching a plugin on or off takes effect at the next start, as before, and so do *Install from file…* and *Remove*; until then the row and the details show a small *restart* chip.

### Settings window

Settings open in a window of their own, 880 × 640 by default, GNOME-style. The left column, 220 px in `bg/panel`, holds the *Settings* title, a **search field** that filters the categories and rows to those whose label or description match, and the **category list** — rows like the side panel's, the active one `bg/selected-soft`: *General*, *View*, *Appearance*, *Previews*, then a *PLUGINS* section header with one row per loaded plugin that declares settings (PDF preview, Office preview, 3D models, Console, SSH, Archives and so on, named after the plugin), each with its icon. The right column shows the chosen page: its title and **grouped cards** under small group titles (*Files*, *Sorting*, *Side panel*) — one card per group, one row per setting, the label and its optional description at the left and the control at the right: a switch, a dropdown, a segmented control, a text field or a button. The page has no close button of its own: the window is closed with its own close control, and a second one inside the page is not drawn. Only where the settings cannot open as a window of their own and are drawn inside the main window does the page keep a round close button at its top right, being the only one there.

- **View**: Files (Folders first, Show hidden files, Show system files, Show file extensions, Zebra stripes), Sorting (Sort by, Order), Side panel (Show side panel, Position), and the rest of the list settings.
- **Appearance**: Theme (System, Light, Dark), Density (Comfortable, Compact), Font size, and **Title bar colour** (*Title row*): a card with its title, the line *The band behind the workspace tabs. Each preset is a light and a dark value; Custom lets you set both.* and the choices as tiles — a 64 × 40 px swatch with 8 px corners, split into the preset's light half and its dark half, over the preset's name in 11 px. The six presets come in the order of the table under *Title row*, then *Custom…*, whose swatch is a dashed outline reading *L / D* until it is chosen. The chosen tile has a 2 px `accent/focus` ring and its name in medium. With *Custom* chosen, two fields appear under the tiles, *Light* and *Dark*, each a `#rrggbb` text field with a swatch of its colour beside it; a value that is not a colour is not taken and the field keeps its warning border until it is one. A change shows in the title row at once.
- **Previews**: Remote files — *Preview limit*, the size above which a whole file on a remote source is not previewed automatically, 10 MiB by default; and *Always preview*, one row per host or account on which the limit is switched off, each with a switch that brings the limit back (*Remote sources* under Preview).
- **General**: Language, Start with (the last session or a layout), Opening an archive, After a drag and the spring-loading delay, the ignore list of the subtree search, and the other settings that fit no other page.
- **A plugin page** renders the plugin's settings schema as cards, grouped as the schema groups them.
- **Keyboard shortcuts** and **Extensions** stay their own screens, opened from the burger menu: the two dialogues drawn under *Dialogues*.

Only the View page and the Title bar colour card are drawn in the Figma file; the other pages follow their form. Which setting sits where is recorded in `QUESTIONS.md`.

### Further GUI elements

**Tree view.** A folder tree that expands in place, available as a view mode and as its own dockable pane, never in the side panel; selecting a node navigates the focused explorer pane, and the tree follows the explorer's location unless unlinked.

**Theme and density.** See *Look and feel*: the scheme (system, light, dark), the density (Comfortable, Compact), the font size and the title bar colour are the Appearance settings; there is no accent-colour setting. Syntax highlighting themes follow the scheme.

**Hidden files, symlinks and mounts.** The footer's eye toggle (Mod+H) shows dot-files and files with the hidden attribute. Symlinks show a link badge on the icon and their target in the Type column; a broken link is marked. Mount points show a volume badge, and the side panel's This Machine and Network sections list mounted volumes.

**Icon and thumbnail view.** Besides the details list, a grid view with large icons or thumbnails for images, PDFs and videos, in four cell sizes (small, medium, large, extra large; the menu offers the first three). A thumbnail is rendered by the Previewer registered for the entry's type, at the cell size, off the UI thread, and cached per entry and size; while it renders, or when no previewer claims the file, the cell shows the file-type icon. The freedesktop thumbnail cache on Linux may seed that cache later.

**Dual pane.** One command (F6) lays out two explorer panes side by side in the workspace, the classic commander layout; a second press returns to a single pane.

**Inline rename.** F2, or a second click on the single selected row at least 0.5 s after the first (past the double-click window), edits the name in place in the list, with the extension excluded from the initial selection; Enter commits, Escape cancels, a clashing name is refused in place. New file and New folder insert a row in the same edit mode at the top of the list.

### Loading animations

Rick's own loading animations are the **portal family**: an abstract portal in `accent/focus`, built in code with the egui painter — the Figma frames under *Loading indicators* only approximate the motion. The reduce-motion setting of the system turns both into a still portal: the rings stop and nothing flies.

**Portal spinner.** Three arc rings around a softly pulsing glow, rotating at one, two (reverse) and three times the base speed, one loop about a second; 18 px as the icon of a running copy or move job in the operations drawer (waiting jobs keep the hourglass), 24 px as its base size elsewhere.

**Portal burst.** 96 px: the spinner's rings and glow with solid folder and file glyphs spiralling out of the portal, tumbling in different directions and fading as they go. It is shown centred in the content area wherever the file list or the preview pane is loading or busy, with one status line under it (*Reading folder… N items so far*, *Loading preview…*), and in the hash dialogue while hashing. It replaces every skeleton-row placeholder.

### Accessibility

Every icon-only control — tool buttons, the toggles, the pane handle, the round cancel buttons, the per-row copy icons, the close buttons, a drive card's eject button — carries an accessible name through AccessKit, the same text its tooltip shows, so a screen reader can announce it; a read-only drive's bar carries *Read-only* as its accessible description, the text of its tooltip. The controls 0.4 adds are held to the same rule: the preview pane's info button and the pin of its card, the copy button of every row in the File info card and in the dialogues (*Copy Location*), the reset button of a shortcut (*Reset to default: Ctrl+F2*), the remove button of an ACL entry, the swatch tiles of *Title bar colour* (the preset's name), the column dividers of a table in a preview (*Resize column Name*) and the close button of a toast. Keyboard focus is always visible as the 2 px `accent/focus` ring. Colour never carries a meaning alone: a match has its pill, a destructive entry its label, a filter its dot beside the icon. Every colour pair in both schemes meets the contrast rule under *Look and feel*.

### General behaviour

**Session restore.** Closing Rick and opening it again brings back exactly what was left: every window with its position and size, its workspace tabs in order with the active one, each workspace's layout and every pane's path, sort, filter, column widths and scroll position, and the side panel's state. The session is saved continuously, so a crash loses nothing but the last few seconds. A pane whose location no longer exists opens at its nearest existing ancestor with a note in the footer; one whose provider is not mounted opens at Home.

**Saved layouts.** A user can arrange a window — its workspace tabs and their panes — then save that arrangement as a named layout and mark it as the start layout; Rick then opens with that layout instead of the last session. Layouts are listed in the menu and the command palette for switching at any time; the last session remains available as "Continue where I left off".

**Windows.** New window is one click (menu, Ctrl+N, or the + on the tab row with Shift) and opens with the start layout. Dragging a workspace tab, or a pane by its handle, and dropping it outside every window creates a new window holding it. Dragged onto another window it joins that window: on its tab row a tab moves there and a pane becomes a new workspace tab there; resting on one of that row's tabs spring-loads its workspace; and the dock targets appear on the pane under the cursor in whichever window that is, so a pane, or a tab holding exactly one pane, can be docked into a layout of the other window (see Workspace tabs). Every window has the full side panel and shares the operations queue, the preview follows focus across windows.

**Text files.** The preview shows the file's text in a monospace view with two toggles in the pane's toolbar: word wrap (off = lines keep their real length and the view scrolls sideways; on = long lines break at the pane width at any character, so a line's row count follows from its length and the view stays virtualised; a wrapped continuation is marked in the gutter) and line numbers. The defaults are line numbers on and wrap off; changing either saves the new default (see Preview, Text defaults). Large files are never loaded whole: the previewer memory-maps the file (or reads windows of it) and renders only the visible lines, so a 500 MB log shows its top immediately and scrolls through the rest; line starts are indexed in the background for exact line numbers and scrollbar position, with a byte-offset estimate until that finishes, and highlighting and pretty print work on the visible window only. Memory use is bounded by the window, not the file size, and encoding is detected. Structured and code formats get syntax highlighting: JSON, YAML, XML, HTML, Markdown and every language in the preview list, detected by extension and, failing that, by content; the highlighter is syntect with the theme following the scheme, and the format name shows in the pane's footer.

**Pretty print.** For JSON, XML and minified HTML in 0.1 — YAML, CSS and JavaScript when each has a formatter — a Pretty-print toggle in the pane's toolbar reformats the text with consistent indentation and one element per line; it is a view transformation only, the file is never modified, and the raw view is one click away. It is off for every newly shown file and is never remembered. Invalid input shows the raw text with the parse error highlighted.

**Images.** By default the image is scaled to fit the available pane, keeping its aspect ratio and never upscaled beyond 100%. A 1:1 button (or double-click) switches to actual pixels, where the image can be panned by dragging or with the scroll wheel, and Ctrl+wheel zooms in steps around the cursor; Fit returns to the scaled view. EXIF orientation is applied. Large images decode a downscaled version first for an instant fit view, with the full-resolution decode following for 1:1.

**Vector images.** SVG and SVGZ (gzip-compressed SVG) belong to `ext-preview-image`, compiled into the binary like the raster formats: the format matters on every platform and its renderer, `resvg`, is a pure Rust crate. An SVG has no pixels of its own, so the previewer rasterises it at the exact size the pane asks for — the pane's size for Fit, the intrinsic size at the display scale for 1:1, the zoomed size when the pane zooms — and rasterises again at every change of that size. A cached bitmap is never scaled up; the picture is sharp at every zoom. The intrinsic size is the document's `width` and `height`, the `viewBox` when they are missing, and 300 × 150 when both are; above 8192 px on the long edge the picture is rendered at the cap and scaled, and the footer says so. Transparent regions show over the checkerboard raster images with alpha use. Text is set in the system fonts through `usvg`'s font database with its fallbacks; this is a previewer, not a browser, and fidelity for text-heavy documents is not promised. Scripts never run; references to other files or URLs — `href` to an external image, external style sheets — are not resolved; only embedded data URIs are honoured. A broken or truncated document falls to the Source view with the parser's message (line and column when it has them) in the footer, and *Rendered* stays available for after the file is fixed. The toolbar is the raster images' Fit and 1:1 plus the *Rendered* toggle of the markup formats, on for every newly shown file; the text layer is the source, so *Copy* copies the raw SVG as text — decompressed for SVGZ — and find searches it. The footer reads `SVG · 512 × 512 px · 14.2 KiB`, with `viewBox 0 0 24 24` in place of the size when only a viewBox exists.

**Icon containers.** ICO and CUR (Windows), ICNS (macOS) and XPM (X11) are previewed by `ext-preview-icons`, a plugin: the formats are niche — mostly a Windows concern — and bring their own decoders, so they stay out of the binary. Pure Rust throughout: the `ico` crate (ICO and CUR, BMP and PNG entries), the `icns` crate (PNG and run-length encoded types), and XPM parsed in the crate itself (X11 colour names, `None` transparent); no third-party library, so *Third-party libraries* does not apply. An ICO, CUR or ICNS holds several variants of one picture, and the variants are the document's pages: the pane's standard pager steps through them, previous and next and `n / N`, the largest variant first. Each variant is listed in the File info section as one row — `256 × 256 · 32-bit · PNG`, `16 × 16 · 4-bit · BMP` — and the footer names the shown one: `ICO · 7 variants · 48 × 48 · 32-bit · PNG · 23.4 KiB`. A variant the plugin cannot decode (an ICNS JPEG 2000 entry, an unknown OSType) is not a page: its File info row reads `not decodable (JPEG 2000)`, the footer counts it (`2 not shown`), and the file still previews while any variant decodes. XPM holds one picture and has no pager. Icons are small, so the usual rule that a picture is never upscaled does not apply to them: in Fit the previewer magnifies the variant by the largest whole factor up to 8 that fits the pane, nearest-neighbour so every pixel stays a crisp square, and 1:1 shows actual pixels, one device pixel per icon pixel; both are the previewer's own work on the frame it returns at the size the pane asked for, so the pane needs nothing new. Transparent regions show over the checkerboard. CUR draws its hotspot as a small crosshair over the variant, with a toggle in the toolbar to hide it. *Copy* follows the text-based rule: XPM, which is text, copies its source; the binary containers have no *Copy*. Not in scope: exporting a variant as PNG, editing or repacking a container, `.icl` libraries, icons inside `.exe` and `.dll` resources, icon theme folders.

**Media metadata.** Image, audio and video previews put the basics in the pane's footer — dimensions or duration, file size, format, colour depth or codec — and the rest in the File info card: the camera, the date taken and the exposure for photos first, then the EXIF tags Rick knows by name and that have a value (via `kamadak-exif`; IPTC and XMP when a reader without a system library is adopted), ID3 or Vorbis tags for audio, and stream details for video — labels and values with a copy button on every row, shown on hover and pinned on click as the Preview section describes.

**Audio and video.** Media files play inside the preview pane, like every other file: selecting one shows it there — the first frame or the waveform, the transport in the pane's toolbar — and nothing opens elsewhere. Play/pause (Space while the preview has focus), a seek bar with elapsed and total time, volume, and for video the frames drawn into the pane with a fit/1:1 toggle like images. Audio shows a waveform and cover art when present. Playback starts on demand, not on selection, stops when the selection changes, and decoding runs through GStreamer on every platform, one code path, with the runtime bundled on macOS and Windows.

**Previews are modules.** Every preview is an extension exposing one or more Previewers, as the architecture section describes, and each ships as its own module that a user can enable or disable. One module may cover several related types, but the split follows who needs it, not the file type: a plain-text previewer is part of the core, while syntax highlighting, pretty print, PDF, Office, images, audio and video are separate modules, so a user who is not a programmer never loads the code tooling. When several previewers match a file, the highest score wins and the user can override the choice per type in settings.

Proposed modules: preview-text (core), preview-code (highlighting, pretty print), preview-pdf, preview-office, preview-image, preview-media (audio, video), preview-archive (folder, archive and disk-image listing), preview-markup (rendered Markdown and HTML, the CSV grid), preview-hex.

**Extensions as installable packages.** Modules are not only switchable, they are physically separate: each is a package in a known plugins directory that can be removed from the disk or simply never loaded, and the core binary carries only the local file system, the text, code, image and PDF previewers and the core actions (see Package layout). A package holds a manifest (id, name, version, API version, the extension points it fills, the platforms it supports, the toolchain it was built with and, from 0.4, a one-paragraph description that the Extensions dialogue shows — a change of the versioned contract, so the API version goes up with it) plus its code as a native dynamic library — a WASM module later; the loader reads the manifests at start, registers each plugin's contributions, and defers heavy initialisation until a matching file or action is first needed. The Extensions dialogue (*Dialogues*) lists what was found with enable and disable; a packed plugin, for *Install from file…*, is an `.rkp` file: a zip archive of the plugin's folder as it sits in the plugins directory.

**Third-party libraries.** Some plugins need a library Rick does not build: pdfium for PDF, LibreOffice for Office, GStreamer for audio and video, the 7-Zip library for disk images. Two classes. *Bundled:* a library small enough to ship and licensed for it travels with Rick — beside the binary, as pdfium does today, or in the plugins directory beside its plugin, as the 7-Zip library (`7z.so` / `7z.dll`, LGPL, loaded dynamically) will. *System:* one too large to ship is found on the machine — LibreOffice; GStreamer on Linux, where the runtime is not bundled. In both classes the path to the library is a setting of the plugin, stored in the plugin's own TOML settings file, and Rick reads no environment variable for it: `RICK_PDFIUM_DIR`, `RICK_SOFFICE` and any other override of that kind go, and development and CI write the setting instead. The plugin resolves its library lazily, on the first file it is asked to show, in this order: the setting when it is set, the directory of the binary and the plugins directory, then the usual install locations of the current platform (the `PATH` for a program); the first hit is written back into the setting, so the user can see and change where Rick looks. Nothing is probed at start. When nothing is found, the preview pane shows the File info card, centred (*File info card* under Preview): the head with the file's name, type and size; one row, *Preview*, that names what is missing, *Needs pdfium, which was not found*; and an action row with **Download pdfium** (`DOWNLOAD_SIMPLE`, the primary button), which opens the library's download page in the browser — Rick fetches and installs nothing itself — **Browse…** (`FOLDER_OPEN`; a file dialogue whose choice is stored in the setting) and **Retry**. A library that is installed with a command, not from a page, has the command for this platform as a second row, *Install*, monospace with its copy button, and no *Download* button. Under the buttons a line in `accent/focus`, *Where Rick looked* with a caret, unfolds the places that were searched and the loader's error; it is folded at first, so no path is on show until it is opened. From then on, for the session, the Extensions dialogue marks the plugin as needing attention.

**Opening files.** Enter or a double-click on a file opens it; nothing opens a viewer, because the preview pane is the only one. A folder navigates. An archive or a disk image (see *Disk images and Mount*) enters the file as a read-only location in the current pane through its provider; that is the default of the setting `archives.on_open` (`enter`), whose other value, `system`, hands the file to the OS default application instead. Every other file opens with the OS default application. Mod+Enter opens with the OS default application whatever the file and the setting, and *Open with…* stays in the context menu for all of them. Rick never writes to a file it shows; editing is the job of an external application via Open with.

**Disk images and Mount.** A disk image is browsed like an archive: selecting it lists its contents in the preview pane, Enter enters it (above), and nothing is mounted for that — the provider reads the image file itself. The formats and the readers are in the Disk images table under Supported scope. *Mount* is an Action in the context menu of a disk-image file, and so also in the command palette, that hands the file to the operating system's own mount mechanism and nothing else: `Mount-DiskImage` on Windows (ISO, VHD, VHDX), `hdiutil attach` on macOS (DMG, ISO, IMG), `udisksctl loop-setup` on Linux (ISO and raw IMG), with a clear error when udisks is not installed. Rick ships no other mounting path (no qemu-nbd, no FUSE), and the action is hidden on a file the current OS cannot mount. The mounted image appears as a removable volume in the side panel's This Machine, and that row's *Eject* unmounts it; there is no separate Unmount action. The `platform` crate gains a `Mount` trait for this (which formats this OS mounts, mount a file), and the action lives in `ext-action-core`.

**Extension store.** Later. The Extensions dialogue, which from 0.4 installs a plugin from a file and removes one, grows update and browses a store of available packages: a signed index hosted by the project, with official modules (the preview set above, Git, archives, object storage) and third-party ones. Install downloads the package for the current platform, verifies its signature and hash, and activates it without a restart. Store entries declare the permissions they need (file system scope, network, process spawn); the user sees them before installing.

## Functions

**Drag and drop.** Dragging a selection out of a file list starts a drag; where it lands decides what happens. Every target highlights while the cursor is over it.

| Target | On drop |
| --- | --- |
| A folder row in any pane | Move (or copy, see below) into that folder |
| The empty space of a pane, or its background | Move into that pane's current folder |
| A breadcrumb segment | Move into that ancestor folder |
| A tab header | Move into the folder that tab shows |
| The Places list of the side panel | A dragged folder becomes a place at the insertion line; never a file operation. Files are refused (not-allowed cursor), as they are everywhere else in the side panel, which is never a file-operation target |
| Outside the window: another application, or another Rick process | Hand over through the platform drag-and-drop with file URIs; the receiver decides. A drop arriving without a cursor position (the backend reports none on some platforms) lands in the focused pane's folder |

*Move or copy.* Inside one Rick process the rule is by volume: same volume moves, across volumes copies. Mod forces a copy, Shift forces a move, and Alt (or a right-button drag) asks on drop. Anything arriving from outside the process — another application, a browser, a second Rick window of another process — is always a copy, whatever the volume; Shift forces a move. The drag ghost shows the first item's icon and name, a count badge when several items are dragged, and a small indicator under the cursor — `ARROW_RIGHT` for move, `PLUS` for copy, `PUSH_PIN` for a new place — that updates live as the target and the modifier keys change, so the outcome is visible before the button is released.

*Spring-loading.* A drop on a folder row moves at once. Holding over a folder row for about a second (settable) navigates the pane into that folder while the drag continues, so the drop can land on a subfolder or on the empty space inside; holding over a subfolder drills down again, and holding over a workspace tab switches the window to that workspace the same way, so the drop can land in a folder of another workspace. When the drag ends or is cancelled the pane stays where it drilled to; a setting, *After a drag: stay in the folder / return to where the drag started*, defaults to stay. A drag counts as one navigation: the starting location is pushed on the tab's history once, the folders passed through are not, so one Back returns to the starting location with its scroll position and selection restored. The delay is one setting for every kind of drag: a dragged pane, and a dragged tab that holds exactly one pane, spring-load workspace tabs with it too, in their own window or another, and go on to the dock targets of the workspace that opens (see Workspace tabs).

*No-ops and cancel.* Escape cancels any drag in progress — of entries, of a pane or of a tab — and the ghost disappears: nothing moves, and the layout is exactly what it was when the drag began, every tab in its place and every window showing the workspace it showed, whatever spring-loading has switched since; only the folders that panes drilled into during a drag of entries follow *After a drag*, as above. Dropping items onto the folder they are already in, a folder onto itself or into its own subtree, or a file onto a file does nothing and shows no error; the target simply does not highlight. A pane or a tab released where it cannot land — a pane beside the dock targets, a tab holding several panes over the panes — moves nothing either and leaves the layout as Escape does.

*Execution.* A move or copy runs as a job in the operations area with progress, pause and cancel. Before it brings anything the job compares the names it brings with the destination, so the number of conflicts is known when the first one is asked about. A conflict on an existing name pauses the job and asks with the paste-conflict dialogue (*Dialogues* in the GUI chapter), which offers:

- **Skip** — the existing item stays and this one is not brought.
- **Replace** — the new item overwrites the existing one: a file replaces a file, and a folder onto a file or the reverse removes the target first. Between two folders the button reads **Merge**: same-named files inside are replaced, the rest kept.
- **Keep both** — the new item arrives under the first free numbered name, `name (2).ext`, then `name (3).ext`, the number before the extension and the extension untouched; the existing item is not touched. It is the default, and it is not offered between two folders.
- **Cancel** — the whole operation stops; what it has already brought stays.

There is no rename field: a different name is given afterwards, in the list. *Do the same for the remaining n conflicts* makes the chosen button the answer to every conflict still to come in this operation; *Replace* and *Merge* are one answer for that purpose, and a standing *Keep both* does not apply between two folders, which are still asked about. The undo of a move that overwrote brings the item back but not the overwritten target. A move within one provider uses rename, a copy uses the provider's fast path. Pin and trash go through their Actions, so they appear in the undo log and the palette like any other operation. Across processes, the receiving Rick applies these same rules from the moment the cursor enters its window; the indicator and spring-loading work there too.

**Permission management.** Properties on a file or folder shows and edits ownership and permissions: owner, group, mode bits (rwx for user, group, other, plus setuid, setgid, sticky) on Linux and macOS, ACLs where the file system has them (POSIX ACLs, NFSv4 ACLs, NTFS ACLs on Windows), and, for a folder, *Apply to enclosed items*, which sets the same permissions on every file and folder inside. Changes that need elevation ask for it through the platform (polkit, sudo, UAC). The controls are the Permissions tab of the properties dialogue (*Dialogues* in the GUI chapter).

**Compute hash.** *Compute hash…* in the Info group of a file's context menu, above Properties — the `core.hash` operation, no default shortcut, hidden on folders — computes MD5, SHA-1, SHA-256 and BLAKE3 of the selected files in a single pass: each file is read once, in 1 MiB chunks through its provider's `open_read`, every chunk fed to all four hashers, so a file on an SSH host or in a cloud account is hashed like a local one. The pass is a job with cancel and byte progress; the dialogue opens at once, shows the progress, and fills in when the job ends. The dialogue (its layout under *Dialogues* in the GUI chapter) has one row per algorithm — the label, the value in monospace, a copy icon — then *Copy all*, which copies the four lines as `<algorithm>  <hash>  <file name>`, and a format toggle: lowercase hex by default, uppercase hex or Base64, remembered as `format = "hex" | "hex-upper" | "base64"` in the `[hash]` table of `core.toml`; switching the format re-renders the values without recomputing. The *Verify* field at the top takes an expected hash pasted from a download page, or prefilled from the clipboard when the clipboard holds one. The comparison is live as the user types, case-insensitive, ignores surrounding whitespace and reads hex or Base64 according to the toggle; the algorithm is detected from the decoded length — 16 bytes MD5 (32 hex characters), 20 SHA-1 (40), 32 SHA-256 and BLAKE3 (64) — and every row of that length is compared: a match tints its row green and the field shows *Matches <algorithm>*, no match shows *No match*; a length that fits no algorithm reads *Unrecognised length*. All four algorithms are always computed; there is no per-algorithm selection. With several files selected the dialogue shows one block per file, hashed one after the other in one job. Out of scope: hashing folders, `.sha256`/`.md5` sidecar files and checksum manifests.

### Operations

- **Flatten subfolders.** A toggle that shows the contents of all subfolders in the current list, each entry with its relative path, instead of only the current level; off again returns to the normal view.
- **Toggle inspector.** Show or hide the preview pane.
- **Split right.** Split the current pane and open the same location in a new pane to the right.
- **Split down.** Same, with the new pane below.
- **New tab.** Open a new workspace tab with one pane at the current location.
- **Close tab.** Close the active workspace tab.
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
- **Move to trash.** Send the selection to the platform trash (Delete key), restorable from the Trash view (Places entry in the side panel).
- **Delete permanently.** Remove the selection without going through the trash (Shift+Delete), after confirmation.
- **Duplicate.** Make a copy of the selection in the same folder with a derived name, such as "name (copy)".
- **Inspect.** Open the system properties for the selection: the Windows Properties dialogue, the macOS Get Info window, and on Linux Rick's own properties dialogue (size, dates, owner, permissions, mime type), since there is no desktop-wide one.
- **Compute hash.** Compute MD5, SHA-1, SHA-256 and BLAKE3 of the selected files in one read and show them in a dialogue with a copy button per value, *Copy all*, a format toggle and a verify field (see *Compute hash* above); hidden on folders, no default shortcut.
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
- **Move tab left / right.** Shift the active tab one position in the window's tab row (Ctrl+Shift+PageUp / PageDown); the keyboard's counterpart of reordering a tab by drag.
- **Close other tabs.** Close every tab of the window except the active one.
- **Close tabs to the left / to the right.** Close all tabs on that side of the active one.
- **Close all tabs.** Close every tab of the window, leaving one fresh tab at Home.
- **Reopen closed tab.** Restore the most recently closed tab with its location and history (Ctrl+Shift+T), repeatable through the closed-tab stack.
- **Next tab / previous tab.** Cycle through the tabs of the window (Ctrl+Tab / Ctrl+Shift+Tab).
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
- **Pin to side panel.** Add the selected items — folders or files — to Favorites (the Places entry that opens as a view), at the end. A pinned folder opens in the current pane; a pinned file opens its folder with the file selected. The action sits in the context menu of every entry and in the palette; on an already pinned item it reads **Unpin**, and each entry in the Favorites view offers Unpin in its context menu too. Pin and Unpin are undoable.

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
| Provider | Where files come from: local, archive, SSH, object storage | list streamed in chunks, stat, open, watch, copy, delete, plus a capability set (trash, rename, reflink, watch) |
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
- `platform` — per-OS code behind one trait set: trash, links and junctions, elevation, system properties dialogue, file associations, mounts. In 0.1 the set is `Trash` (trash, list, restore, remove one item, empty), `OpenWith` (`open` with the default application, `reveal` in the system file manager), `Properties` (the native dialogue on Windows and macOS, label/value rows on Linux), `NativeDialogs` (async, through `rfd`, the XDG portal on Linux), `Clipboard` and `ConfigDir`; elevation, file associations and mounts come later. 0.2 adds `Secrets` (store, get and remove a secret by a stable key in the platform keychain: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows, a fake for tests) and `Terminal` (launch the user's terminal emulator with a command), both introduced by `ext-ssh` and reused by the cloud providers and `ext-action-terminal`, and `Mount` (which disk-image formats this OS mounts natively, and mount one through the OS: `Mount-DiskImage`, `hdiutil attach`, `udisksctl loop-setup`), used by the *Mount* action. The three RustSec "unmaintained" notices on `rfd`'s D-Bus dependencies are accepted in deny.toml with the reason recorded, to be dropped when `rfd` moves on.
- `ext` — the five extension traits, the registry, manifests and the extension context.
- `vfs` — the mount table that routes a path to its `Provider`, and cross-provider copy and move.
- `vfs-local` — the local filesystem provider: listing, statx, watching, copy and reflink.
- `model` — the directory model: cached listings, sort, filter, group, selection, history.
- `ops` — the file operations as jobs: copy, move, trash, delete, rename, link, undo stack, conflict policy.
- `app` — the application state: windows, panes, tabs, layouts, session persistence, settings, the command palette.
- `ui-egui` — all egui widgets and the dock layout; depends on `app`, never the other way round.
- `rick` — the binary: feature flags, wiring, startup.

**Providers** (`ext-vfs-*`)

- `ext-vfs-archive` — zip, tar family, 7z, RAR read-only, single-file codecs, ISO 9660 and UDF; and the disk images of the Disk images table (raw IMG, DMG, VHD, VHDX, VMDK, VDI, QCOW2) read-only through the bundled 7-Zip library.
- `ext-vfs-s3` — S3, R2, MinIO and other S3-compatible stores, one endpoint setting.
- `ext-vfs-azure` — Azure Blob.
- `ext-vfs-gcs` — Google Cloud Storage.
- `ext-ssh` — remote hosts over SSH, one plugin with several contributions like `ext-git`: a Provider driven by shell commands over `exec` channels (no SFTP), the hosts of `~/.ssh/config` and Rick's own in the Network section, and the *Open console here* and *Open in external terminal* actions. Specified in [`specs/ext-ssh.md`](specs/ext-ssh.md).
- `ext-vfs-trash` — the trash as a browsable location with restore.

**Decorators** (`ext-deco-*`)

- `ext-git` — one plugin, several contributions: a Decorator (status badges, branch, ahead and behind, via gix), an Action group for the context menu (Git commit, Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log, Blame), and an Info panel section (current branch, last commit). The Action group appears under "Git" in the Extensions part of the context menu and in the command palette, only when the location is inside a repository.
- `ext-svn` — the same shape through the svn command line: Decorator plus Update, SVN commit, Revert, Log actions.
- `ext-deco-fs` — later: Btrfs subvolumes, ZFS snapshots, NTFS alternate streams.

**Previewers** (`ext-preview-*`)

- `ext-preview-text` — plain text, encodings, huge-file windowing, pretty print.
- `ext-preview-code` — syntax highlighting via syntect, on top of text.
- `ext-preview-image` — raster and SVG decode (`image`, `resvg`), EXIF, IPTC, XMP.
- `ext-preview-icons` — ICO, CUR, ICNS and XPM: every variant of an icon container as a page, magnified pixel-sharp (`ico`, `icns`, an XPM parser of its own).
- `ext-preview-pdf` — pdfium-render; pdfium is a dynamic library that every build and release places beside the binary, found as *Third-party libraries* describes (a setting, no environment variable).
- `ext-preview-office` — LibreOffice headless to PDF, then the PDF path.
- `ext-preview-media` — audio and video via GStreamer, ID3 and stream info.
- `ext-preview-archive` — listing of an archive's or disk image's contents without mounting it.
- `ext-preview-markup` — one plugin for the three structured text formats: Markdown and HTML rendered, CSV and TSV as a virtualised grid, each with the *Rendered* toggle to the source. Supersedes `ext-preview-csv` and takes the Markdown and HTML rendering out of `ext-preview-code`.
- `ext-preview-3d` — STL, OBJ and 3MF meshes rendered through wgpu.
- `ext-preview-hex` — the hex view; claims no file on its own, chosen per type in settings.

**Actions** (`ext-action-*`)

- `ext-action-core` — the built-in operations list: clipboard, trash, rename, new, link, select, view and sort commands, and *Mount* for disk images through `platform::Mount`.
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
/// Which extension provides a tree. "local", "archive", "ssh", "s3", "trash".
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
    /// API across buckets of one account, `ext-ssh`'s remote `cp` and `mv`. The provider
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

**Previewer** — turns an entry into something the preview pane can show:

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
    /// Metadata, grouped in sections for the pane's File info card. The first section is
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

The pane's toolbar is built from these plus the document kind, and the pane itself never asks a previewer to draw: a Decorator's `panel()` and a previewer's `sections()` both feed the same File info card.

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
    /// Pin and unpin, undoable; `at` is the position in Favorites, None = end.
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

The default `default_shortcut` of every built-in Action. **Mod** is Ctrl on Linux and Windows and Cmd on macOS. All of these are rebindable in Settings → Keyboard shortcuts; extensions may claim unused combinations and never override a core one. The Keyboard shortcuts dialogue (*Dialogues* in the GUI chapter) files the tables below under its categories: each under its own name, *Tabs and panes* as *Panes & tabs* together with the window commands (new window, full screen, the side panel toggle, the layouts); the search commands — the search box in both its modes, the filter and the search in subdirectories — under *Search*; and every plugin's actions under the plugin.

**Navigation**

| Shortcut | Action |
| --- | --- |
| Enter | Open (folder: navigate; archive or disk image: enter it, or the system default when `archives.on_open` is `system`; other: system default) |
| Mod+Enter | Open with the system default application, whatever the file and the setting |
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

**Preview pane** (while it has focus)

| Shortcut | Action |
| --- | --- |
| Space | Play / pause media |

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
2. **Side panel rework.** The side panel as the GUI section now specifies it: the editable Places list with its Favorites, Recent and Trash views, the This Machine, Network and Cloud sections with their overviews, live mount and hot-plug updates, Eject, open in new tab, the drop rule (a folder becomes a place, files are refused, no file operation anywhere in the panel), collapsible sections without an indent guide, no current-location highlight, the Tree section removed, and the operations area with its larger layout, per-row progress bars and per-volume job scheduling. In this milestone the Trash view lists the platform trash with restore and empty as 0.1's section did, moved into the tab; `ext-vfs-trash` (milestone 5) later turns it into a full browsable location. Network lists mounted shares only; discovery is not part of this milestone. Cloud appears once the first cloud provider plugin exists (milestone 5). The side-panel Recent feed of 0.1 (watches plus a bounded scan) is replaced by the system's recently-used list.
3. **Viewer — withdrawn.** A full-size viewer was never wanted; the item was a misunderstanding, and the preview pane is Rick's only viewer (see Preview). The viewer built as milestone 3 (#29) is removed again by the preview-pane corrections below, together with its hex fallback, its shortcuts and the per-type "open in viewer" setting.
4. **Previewers as plugins:** `ext-preview-office` (LibreOffice when installed), `ext-preview-media` (GStreamer, bundled runtime on macOS and Windows), `ext-preview-archive` (listing without mounting), `ext-preview-markup` (rendered Markdown and HTML and the CSV grid in one plugin, replacing `ext-preview-csv` and the rendering that sat in `ext-preview-code`), `ext-preview-hex`, and `ext-preview-3d` (below).
5. **Providers as plugins:** `ext-vfs-archive` (zip, tar family, 7z, RAR read-only, single-file codecs) with ISO images browsed the same way and, through the bundled 7-Zip library, the disk images of the Disk images table, `ext-vfs-trash` (the Trash view becomes a full browsable location with restore and empty), `ext-ssh` (remote hosts over SSH through shell commands, no SFTP; hosts from `~/.ssh/config` and Rick's own in the Network section; jump hosts; console actions; its own spec in [`specs/ext-ssh.md`](specs/ext-ssh.md), which also brings the `platform::Secrets` keychain trait and the `REMOTE` capability the cloud plugins reuse), and one plugin per cloud — `ext-vfs-s3` (S3, R2 and S3-compatibles), `ext-vfs-azure`, `ext-vfs-gcs` — each with `NATIVE_TRANSFER` so moves and copies inside an account run on the service. Connect, disconnect and remove on the Network and Cloud rows; discovery of hosts and shares on the local network is deferred to a later version.
6. **Search and decorators as plugins:** `ext-search-walk` behind the subtree search, then `ext-search-windows`, `ext-search-spotlight`, `ext-search-tracker` and `ext-search-baloo` feeding the title-row search box; `ext-svn` beside `ext-git`.
7. **Actions as plugins:** `ext-action-compress` (compress, extract here, extract to), `ext-action-compare` (file and folder diff), `ext-action-terminal` (open console, with an embedded console pane), `ext-action-rename` (bulk rename with pattern, numbering, find and replace, preview), plus Open with… chooser, Paste as link and Select by pattern in `ext-action-core`.
8. **Permission management** in Properties (owner, group, mode bits, ACLs, recursive apply, elevation through the platform), and **localisation**: the `t!` lookup gains translation files and a language setting, English remains the fallback.
9. **Platform drag source and drop position.** Close 0.1's drag-and-drop gap: a native drag source per platform (an `NSDraggingSession` on macOS, `DoDragDrop` on Windows, a Wayland data source and XDND on Linux) through the `platform::DragSource` hook, and the cursor position of an incoming drop where the backend can give it, so a drop from outside lands where it was dropped. The macOS benchmarks that run against measured targets are re-measured and tightened towards the spec's numbers as the work allows.

**Preview-pane corrections (2026-11-08).** Two issues, part of 0.2, after the milestones above that they touch. The first, on the core: the viewer comes out and Enter opens files as *Opening files* says; the pane lays out every `Content` kind itself (file list, grid, document) instead of falling back to text; the *No previewer* placeholder; the *Copy* control; third-party libraries resolved through a setting with the in-pane message (pdfium and LibreOffice first, the 7-Zip library and GStreamer as their plugins follow); the `archives.on_open` setting. The second, on the plugins: `ext-preview-markup`; disk images in `ext-vfs-archive` and `ext-preview-archive` through the 7-Zip library; `platform::Mount` and the *Mount* action with *Eject* as its unmount.

**Not in 0.2:** the WASM host, the extension store and downloads, plugin signing, `ext-deco-fs`, TFVC. During 0.2 the workspace version is `0.2.0-dev`; the milestones merge to `main` behind CI on all three OSes as in 0.1, and 0.2.0 is tagged when milestone 9 and the preview-pane corrections are in.

**3D models.** A previewer for STL, OBJ and 3MF, for people who print. The mesh is parsed natively (`stl_io`, `tobj`, a small 3MF reader over its zip and XML) and rendered through wgpu into the preview frame as a shaded solid with a neutral material and a ground grid; drag orbits, wheel zooms, double-click resets the view, and the view is remembered per file while it stays open. The status bar shows triangle count, bounding box in millimetres and, for OBJ, the number of objects; the File info section adds the units declared by 3MF. Models beyond a configurable triangle budget decimate for the preview and say so. Ships as `ext-preview-3d`, a plugin.

### Version 0.3 — design refresh, system integration and distribution

0.3 has two tracks that run in parallel and ship in one release: the design refresh of issue 60, and system integration and distribution as [`specs/system-integration.md`](specs/system-integration.md) lays it out. The design refresh is the GUI chapter as it now stands, Dark included, delivered in one release. The spec lands first; then the scheme and density tokens, with the theme a swappable set; then one pull request per area into `dev`, in this order — title row and workspace tabs; panes with the handle, drag docking, the parent chip and the view options menu; the file list with its header, row states, empty and loading states; footer and filter; the side panel with drive cards, the Trash pill and the operations drawer; the context menus; the install dialogue; the hash dialogue; the settings window — each compared with its Figma frame in Light and Dark and both densities before it merges. Nothing in the *not drawn yet* list under *Still open* is invented along the way; it is asked. No unrelated fix or feature goes into the release.

**System integration and distribution (2026-11-09).** The second track, specified in [`specs/system-integration.md`](specs/system-integration.md) and decided on 2026-11-07 and 2026-11-09: one Rick process per session with a local socket, the command line with subcommands that plugins extend, the `rick://` scheme, the system clipboard as the only file clipboard with Rick's own type beside the native formats, a startup integration check with one dialog whose switched-off rows are *declined* and never asked again, *Settings → Integration*, the per-platform items (Linux may be the default file manager and owns `org.freedesktop.FileManager1`; macOS runs alongside Finder; Windows offers *Make Rick the default* as a warned, repairable opt-in), notifications and taskbar progress, Sigstore signatures and GitHub attestations on every artifact including plugin files, declarative plugin permissions with first-party verification and trust-on-first-use for third-party plugins, and the release channels — OBS, AUR and AppImage; notarised DMG, Sparkle and a Homebrew cask with a universal binary; a signed installer and winget. Ten milestones in the spec's order, single instance first because every entry point routes through it; each merges into `dev` when green. Flatpak, Snap, the stores and the portal file chooser are out by decision.

**Preview formats (2026-11-09).** Two previewer additions, independent of the design refresh: they touch `ext-preview-image` and add a plugin crate, nothing in the GUI chapter, and each is its own issue that merges to `dev` when it is green and ships with whatever release next goes out from `dev`. The first, in the binary: SVG and SVGZ in `ext-preview-image` as *Vector images* under General behaviour describes — rasterised with `resvg` at the size the pane asks for, the *Rendered* toggle to the source, *Copy* copying the source. The second, a plugin: `ext-preview-icons` for ICO, CUR, ICNS and XPM as *Icon containers* describes — the variants as pages, pixel-sharp magnification, the CUR hotspot — built as a plugin library into the plugins directory like every other `ext-*` crate.

### Version 0.4 — scope

0.4 opens with the GUI fixes of the test session of 2026-11-09 (issue 139): one bundle of visual and interaction changes. Every item was drawn in the Figma section *Test-session redesign — 2026-11-09* of the page *Main view* and agreed there; the frames and their yellow notes frames are the design, and this document records their rules in the places named below. The spec lands first; then one pull request per item into `dev`, in this order, each compared with its Figma frame in Light and Dark and both densities before it merges.

1. **App menu** — *Menu*.
2. **Paste-conflict dialogue** — *Dialogues*; *Execution* under Functions.
3. **Properties dialogue** — *Dialogues*; *Permission management* under Functions.
4. **Preview pane: the empty state and the File info card** — *Preview*: *File info card*, *Nothing selected*, *No previewer*; *Third-party libraries* and *Media metadata* under General behaviour.
5. **Tables inside previews** — *Tables in previews* under Preview.
6. **Remote size limit** — *Remote sources* under Preview; the Previews page of the *Settings window*.
7. **Keyboard shortcuts dialogue** — *Dialogues*.
8. **Extensions dialogue** — *Dialogues*.
9. **Title bar colour** — `bg/titlebar` in *Look and feel*; *Title row* and *Workspace tabs*; the Appearance page of the *Settings window*.
10. **Small items** — the spacing of the window controls (*Title row*); no second close button in the settings (*Settings window*); the hash dialogue's secondary line cut before the close button (*Dialogues*); a console's tab titled *Console* (*Workspace tabs*); an operation's error shown whole (*Operations drawer* under Side panel).

The bundle reuses the dialogue frame, the scheme and density tokens and the Portal burst of the design refresh, and adds one token, `bg/titlebar`, and no other colour. It ends with the changelog entry and the screenshots of every item in Light and Dark.

**Not in the bundle.** The functional bugs of the same session have their own issue (149) and are not fixed along the way: a new folder and a new file on Azure, at the account root and inside containers; searching subdirectories on remote providers; a double-click on a remote file; the tab left on removed or ejected media; sorting by the real modification time; the date formats with the time of day; the window's position and size across launches; a *Format…* action for media; the second partition that appears only sometimes; `tracker3` missing; the hex previewer not claiming `.bin`; executables and scripts not starting on a double-click; `~` and environment variables in the path bar; *Open in editor*; single-click rename as a setting that is off by default; *Select all* with hidden files; the layout of the List view; and the corner resize that goes dead after one use.

What else 0.4 holds is added here as it is decided. During 0.4 the workspace version is `0.4.0-dev`.

### Separate specs

Some plugins are specified in their own file under `specs/`, so they can be handed to an implementer on their own. Each is binding in the same way as this document; where one disagrees with this document, the question goes to `QUESTIONS.md`.

- [`specs/ext-ssh.md`](specs/ext-ssh.md) — `ext-ssh`, remote hosts over SSH: a Provider driven by shell commands on `exec` channels (no SFTP) with per-host command profiles for GNU, BSD and BusyBox userlands, hosts from `~/.ssh/config` plus Rick's own in the Network section, agent, key and password authentication with secrets in the OS keychain, `known_hosts` dialogues, jump hosts and chains, pooled connections, same-host `cp`/`mv` through `NATIVE_TRANSFER`, *Open console here* and *Open in external terminal*, prefix and size-capped previews. Unix-like hosts only. Milestone 5 of 0.2, in place of `ext-vfs-sftp`.
- [`specs/ext-installer.md`](specs/ext-installer.md) — `ext-installer`, the tarball installer: *Install as application* on a tar or zip archive, versioned layout with a `current` symlink, desktop entry, icon and commands on the PATH, user-space or system-wide scope, update with one previous version kept, revert, uninstall, repair, and an Installed Apps view. Linux only. Ships with 0.2 (Linux only). It extracts with the tar and zip crates itself, so it has no dependency on `ext-vfs-archive` and no ordering constraint within 0.2.
- [`specs/app-icons.md`](specs/app-icons.md) — the app icon on all three platforms and the Linux desktop entry: one identifier, `rick`, for the desktop file, the hicolor icon name, the Wayland app id and `StartupWMClass`; the window icon through eframe and the rings-only symbol in the About dialog; the icon and version resource of `rick.exe`; the `Rick.app` bundle the release workflow assembles, plugins still beside the binary; the desktop entry and the `share/` layout of the Linux archive, which installs by one copy of `share/` or through `ext-installer`. No signing, notarisation or package formats: those are Version 0.3 System Integration & Distribution.
- [`specs/about-dialog.md`](specs/about-dialog.md) — the About dialog: a modal in the settings window's chrome with the build details (commit, target, plugin ABI, build date, toolchain, OS and desktop, loaded plugins; *Copy details* and `rick --version` print the same text), *What's new* from the embedded changelog, the link, licence and credits cards, and a second page listing every third-party component — the bundled runtimes from `about/bundled.toml` and every crate from `third-party.json`, which `cargo about` generates at release time — with search, a licence filter, expandable licence texts and *Export…*. Version 0.3.
- [`specs/system-integration.md`](specs/system-integration.md) — system integration and distribution, the second track of 0.3: single instance and the command line, `rick://`, clipboard interoperability, the integration check and its dialog, the integration items per platform and desktop (GNOME, Plasma, COSMIC, Cinnamon first-class), notifications and progress, signing and provenance, plugin load rules and trust, the release channels per platform, update checks, uninstall, the operator checklist and the ten milestones.

### Still open

- Extension security model: what a WASM plugin may touch (capabilities per extension point). Native plugins are settled in `specs/system-integration.md` §9: declared permissions, release-signature verification for first-party plugins, trust on first use for third-party ones.
- Testing strategy for the performance targets: a generated 200k-entry fixture and a benchmark harness in CI.
- Accessibility: egui's AccessKit support is young; screen-reader support is a known weakness to state and track.
- Design, not drawn yet (issue 60): what happens when workspace tabs outgrow the title row; the context menus for a folder and for a multiple selection; the contents of *Open with*; the permission-denied state of a pane; the look of the preview pane's content; further colour schemes beyond Light and Dark; a customisable toolbar.
- Design, not drawn yet (issue 139): the paste-conflict dialogue between two folders; the File info card unfolded to all its fields; the search field behind *Other…* in the properties dialogue's dropdowns, and the ACL table for the lists that are not POSIX; the Previews page of the settings and the two fields of a *Custom* title bar colour; Dark for the properties, keyboard shortcuts and extensions dialogues, which follow the variables. Each is built from the rules of this document and asked about in `QUESTIONS.md` where a rule had to be assumed.
