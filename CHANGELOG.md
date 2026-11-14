# Changelog

All notable changes to Rick are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Rick follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each line names
the pull request that brought the change.

## [Unreleased]

- **The README, regenerated with every release.** `README.md` follows
  `specs/readme.md`: what Rick is, why it exists, six features, one
  screenshot, how to install it. `cargo xtask readme` writes the version,
  the Features list (the previewable file types, the storage providers and
  the platform search, from what the extensions register) and the Install
  section (the archives the release workflow builds), and renders the
  screenshot in Light and Dark with the sample model
  `assets/readme/robot-dog.stl` in the 3D preview. The test `readme` fails
  when the page of a release version is not what the code registers, and
  the release workflow stops when the status line's version is not the
  tag's. Building from source, the command line and the architecture moved
  to `docs/` (#207).

## [0.4.0] - 2026-11-11

The fixes of the test session of 2026-11-09 and what followed from them, on
Linux, macOS and Windows: the application menu, the paste-conflict,
Properties, Keyboard shortcuts and Extensions dialogues and the preview pane
redrawn, with the File info card for every previewer and a title bar colour
to choose; remote providers that say what a location offers, with Download,
containers on Azure and search in subfolders; a tab that goes Home when its
drive is removed and stays, with Retry, when a connection is lost; the List
view in columns, the date formats, the window's place and size across
launches, programs and scripts that start, Open in editor and Git clone; and
the fixes for copy, cut and paste by keyboard, the black band around the
window, the search results and the Windows archive that would not start
without the Visual C++ runtime.

The GUI fixes of the test session of 2026-11-09 (issue 139, SPEC.md *Version
0.4 — scope*): the menus, the dialogues and the preview pane redrawn from
their Figma frames on the dialogue shell, the tokens and the Portal burst of
the design refresh, in Light and Dark and in both densities. Each item has
its acceptance test and its screenshots in the four modes
(`crates/tests/tests/screenshots.rs`, taken by CI tier 2); what a rule left
open is in `QUESTIONS.md` 323 to 366. The rules went into SPEC.md first
(#155).

The functional fixes of the same test session (issue 149,
`specs/version-0.4.md`): remote providers, removable media and lost
connections, the file list, the window, launching, search and previewers.
Every bug fixed has a test that fails before the fix. The rules went into
the specification first (#174).

### Added

- `ext-zfs`, a plugin for OpenZFS snapshots, Linux only (placed by the review of
  #156): through the `zfs` command line
  it finds the dataset a folder lies in and, without `zfs` or outside a
  dataset, shows nothing. A read-only `zfs` location lists the datasets
  with snapshots, each dataset's snapshots newest first and each
  snapshot's files as they were (`zfs:/tank/home/@b/docs`), so a snapshot
  opens, previews and copies out like any folder: a file in it opens with
  the system, *Open with…* or the editor from its path under
  `.zfs/snapshot`, which providers can now name through the defaulted
  `Provider::local_path` (API 21); `/~versions/<path>`
  lists the snapshots holding a different version of one file. The "ZFS"
  group has *Browse snapshots*, *Back to live folder*, *Versions*,
  *Restore* (one question naming the snapshot; a live item it replaces
  goes to the trash first), and *Create*, *Rename* and *Destroy snapshot*
  as jobs, which offer to run once more as administrator when ZFS refuses
  for lack of permission: through the platform prompt as a call of
  `rick-zfs`, a helper beside `rick` that runs only a snapshot's
  `snapshot`, `rename` or `destroy`, under the polkit action
  `org.openbreach.rick.zfs` where it is installed. `pkexec` is called
  without the caller's `SHELL`, which it refuses when it is not in
  `/etc/shells` (a shell from Nix or Home Manager), and its own failures
  reach the user in its words. The
  info panel gets a ZFS section with the dataset, its snapshots and its
  main properties with their source. *Edit ZFS properties…* changes
  `compression`, `atime`, `relatime`, `copies`, `recordsize`, `quota`
  and `refquota` of a dataset: a form opened with the current values, *Inherit*
  labelled with the value it gives, a preview of each change with the old
  value's source that refuses a size it cannot read, a quota below the
  space used or no change; applied as one job of `zfs set` then
  `zfs inherit`, with the same permission rule; a quota reset is
  `quota=none`, as ZFS never inherits a quota. Specified in
  `specs/ext-zfs.md`.
- **Time slider** (review of #156):
  a row under a pane's toolbar that moves the pane through the earlier
  states of the folder it shows — *Now* with the count of earlier states,
  an axis with one dot per point, evenly spaced, and at a past point the
  date, name and source with *Read-only* and *Back to now*. Click a dot, drag (the pane
  follows after a 200 ms rest or on release), or Left/Right/Home/End while
  focused; a run of moves is one Back. Switched per pane with *Timeline* in
  the view options menu or *Toggle timeline* in the palette, saved with
  the session; absent where nothing offers a timeline, disabled where the
  folder has no earlier states, never empty. Extensions offer timelines
  through two new defaulted `Decorator` methods, `offers_timelines` and
  `timeline` (API 20), asked in a job and only while the bar is shown;
  several sources merge into one bar. `ext-zfs` offers a dataset's
  snapshots.
- **Title bar colour.** The title row sits on a band of its own colour, the
  new token `bg/titlebar`, and the active workspace tab lifts out of it in
  `bg/pane`, with no accent line. *Settings → Appearance → Title bar colour*
  chooses the band: six presets, each a light and a dark value so that one
  choice works in both modes — *Dusty rose* (the default), *Warm grey*,
  *Slate teal*, *Sage*, *Plum* and *Dark band* — and *Custom* with a
  `#rrggbb` field per scheme. The choice is `titlebar_colour` in
  `rick.toml`, with `titlebar_custom_light` and `titlebar_custom_dark`. On a
  band that is dark in Light — the *Dark band*, or a dark custom colour —
  the tab titles, the icons, the window controls and the search box turn to
  light text, and a light custom colour in Dark is the mirror image; the
  titles hold 4.5:1 and the icons 3:1 on every preset (#168).
- **Extensions dialogue.** Master and detail: a searchable list grouped by
  what a plugin contributes, each row with its icon, name and enable switch
  and an amber dot when it needs attention; beside it the plugin's name, id,
  version and kind, a *Built-in plugin* or *User plugin* chip, the
  description from its manifest, an attention card when there is one, and a
  card with *Contributes*, *Status*, *Location* and *Config*. *Settings*
  opens the plugin's settings page, *Open folder* its folder, and *Remove*
  deletes a user plugin's folder. *Install from file…* unpacks an `.rkp`
  package — a zip archive of the plugin's folder — into the plugins folder.
  An installed plugin loads at the next start and a removed one is gone at
  the next start; neither asks nor checks anything yet. A manifest carries a
  `description`, and the extension API version is 18 (#167).
- **Keyboard shortcuts dialogue.** The categories with their counts at the
  left, one per plugin that contributes actions, so a plugin's actions are
  no longer filed under *File operations*; at the right a search field that
  also takes a pressed chord, and the rows grouped by category with their
  shortcuts as key caps, *Not set* when unbound. A click on the shortcut
  cell records, Backspace unbinds, Esc cancels; a changed row has an accent
  dot and a reset button, and a chord that is taken shows an amber *also: …*
  chip on both rows (#165).
- **The File info card** in the preview pane: one card for every previewer —
  icon tile, name, type and size, label and value rows with a copy button
  each, *Show all n fields* for the rest. It opens under the info button on
  hover, can be pinned above the content, and sits centred in the pane when
  there is nothing to show; the info rows above each previewer's content are
  gone. A file no previewer claims shows the card with *Open as hex* and
  *Open with…*; a missing library shows it with *Download*, *Browse…*,
  *Retry* and *Where Rick looked*, which unfolds the paths and the loader's
  error. With nothing selected the pane reads *No file selected*, and
  loading is always the centred Portal burst (#162).
- **Always preview on an account.** A remote file over the size limit shows
  the File info card with *Preview this file* and *Always preview on* the
  account's name; *Always* is kept per provider account, across restarts,
  and is also in the context menu of the account's row in the side panel.
  Either button shows the Portal burst with a progress line and
  *Downloading… x of y MiB*, and Esc cancels the download and returns to the
  card. This holds for every remote provider; the limit stays 4 MiB (#164).
- **Git clone.** *Git clone…* in the right-click menu of a folder and of a
  pane's empty area clones a repository into that folder, outside a
  repository too. Its dialogue asks for the URL: an `https://`, `ssh://`,
  `git://` or `file://` URL, `user@host:path`, or the path of a local
  repository. The field is prefilled, and selected, when the clipboard
  holds one of these. The dialogue names the folder the clone creates and
  has *Branch*, *Depth* and *Submodules* as options, folded away. The clone
  is `git clone` as a job in the operations: git's own progress line, a
  cancel that stops git and removes the folder the clone had created, never
  one that was there before, and git's message in the job when it fails.
  The dialogue is built from the form fields a plugin has, so it does not
  match its Figma frames in every detail yet (#188).
- **Download.** Enter and a double-click on a remote file copy it into
  Downloads as a job and open nothing; *Download…* in the menu of remote
  files and folders does it for the whole selection, folders included, into
  a folder asked for first (#193).
- **New folder at an Azure account's root creates a container**, its name
  checked against the naming rules before anything is asked of the service.
  Inside a container, and inside an S3 or GCS bucket, New folder and New
  file work at every depth (#193).
- **Search in subfolders on Azure, S3 and GCS**, by prefix. Where a location
  cannot be searched beneath, as over SSH, the toggle is disabled with the
  reason (#193).
- **Format…** in the menu of a drive's card opens the system's own tool on
  that drive: GNOME Disks or the desktop's equivalent, Disk Utility, the
  format dialogue of Windows. Rick formats nothing itself (#194).
- **Connection lost.** A remote location that stops answering keeps its tab
  and shows *Connection lost to* with the account's name and *Retry*; Rick
  tries again by itself after 2, 5 and 15 seconds (#194).
- **List view** as its Figma frame draws it: icon and name, columns of one
  width filled top to bottom, sideways scrolling, Left and Right between
  the columns (#195).
- **Date format**, *Settings → Appearance*, `date_format` in `rick.toml`:
  *Smart with time* (the default: *Today 15:58*, *Yesterday 09:17*, *Tue
  21:24*, *Aug 24 14:12*), *Smart without time*, *ISO* and *System* (#195).
- **Settings → Behaviour**, a new page: *Rename on a second click*
  (`rename_on_click`, off by default) and *Editor* (`editor`) (#195, #197).
- **Window geometry.** A window comes back on its monitor where that is
  still connected and the window would be visible; otherwise it opens
  centred on the primary monitor at the default size, 80 % of the working
  area capped at 1920 × 1200 and at 16:9. A second window opens beside the
  first (#196).
- **Programs start.** A binary with the execute bit runs on Enter and a
  double-click; an executable script asks *Run script?* with *Cancel*,
  *Edit* and *Run* (#197).
- **Open in editor**, F4 and the context menu of local files: the system's
  default text editor, or the command of the `editor` setting (#197).
- **The path bar expands** a leading `~` and `$VAR`, `${VAR}` and on Windows
  `%VAR%`, on Enter and on paste (#197).
- **`localsearch`.** The Tracker connector finds GNOME's indexer under its
  new name too and queries it through `tinysparql` (#198).

### Changed

- **The dividers between the column headers always show**, in
  `border/default`, so that the place to grab a column by is seen; under
  the pointer a divider is `border/strong`, as before (#199).
- **Application menu.** The pattern of the View options and context menus:
  an icon on every item, the shortcut hint or the submenu caret at the
  right, three groups — *New window* and *Layouts*; *Installer*, *Settings*,
  *Keyboard shortcuts* and *Extensions*; *About Rick* with the version. No
  trailing “…” and no theme switch. The hints come from the keymap, are
  empty when an action is unbound, and spell *Mod* as the platform's key.
  The menu is at least 280 px wide and opens under the menu button with a
  6 px gap (#159).
- **Paste-conflict dialogue.** *Replace file?*, with “(n of m)” when there
  are several, replaces the “already exists” question: two cards, *Existing
  file* and *New file*, each with icon, name, size and modified time, and
  the dialogue keeps its 520 px for a long name. *Cancel* at the left;
  *Skip*, *Replace* as a caution button and *Keep both* at the right. *Keep
  both* is the primary button and the Enter default and pastes as
  `name (2).ext` with the next free number. With several conflicts, *Do the
  same for the remaining n conflicts*. Between two folders *Merge* takes the
  place of *Replace*, without *Keep both*. `Conflicts::ask` takes a
  `ConflictDetails`, and the extension API version went to 17 (#160).
- **Properties dialogue** on the dialogue shell: a header with the icon
  tile, the name and type · size, and two tabs. *General* in cards —
  *Location*, *Size* or *Link*, *Dates*, *Options* — with a copy button on
  every path row; a symbolic link has *Target* with its copy button,
  *Resolves to* and *Open target folder*. *Permissions* has the ownership
  dropdowns, the access grid with its octal row, the ACL as a row that
  unfolds into its table with an *Add entry* line, and *Apply to enclosed
  items* as a switch. *Apply* and *Revert* stay disabled until something
  changes (#161).
- **Tables in previews.** The archive tree and the CSV grid take the file
  list's header — a `bg/panel` band with a hairline below — and their
  stripes start white. The column dividers are always visible in the header,
  one between every pair of columns, and are the resize handles:
  `border/strong` on hover, a drag resizes, a double-click fits, and the
  widths are remembered per previewer (#163).
- **Window group.** The layout toggles and the window controls are 16 px
  icons 12 px apart, each answering over its whole 28 px step, with the
  divider in the middle of a 25 px gap; every control has its name, and the
  window's is *Close window* (#169).
- **Errors of operations.** A failed job's row in the operations drawer
  unfolds on a click to the whole error, wrapped, and the failure is also a
  notice at the top right of the window the job was started from, with the
  job's name and the whole text; an error notice stays until its close
  button is pressed. `Event::JobFailed` names the pane a failed job was
  started from (#169).
- **Paste in the context menu** is always there where the folder can be
  written to, greyed with *nothing to paste* until something is copied or
  cut. On a folder it pastes into that folder, and *Paste as link* with it;
  on the empty area into the folder the pane shows (issue 176, #179).
- **A click on the launcher only raises the window.** The desktop entry
  runs `rick --activate %U`: a click on the launcher entry or on a dock
  favourite brings the window of the running Rick forward and opens no
  tab, and starts Rick with its session when none runs. A folder dropped
  on the icon, *Open with* and a `rick://` link open as before, and `rick`
  typed in a terminal keeps opening the current directory. A launcher
  entry written by an earlier version reads as changed in the integration
  dialog and is rewritten when its row is applied (specs/decisions.md
  Q264, #191).
- **Three colour tokens follow the revised Figma variables.**
  `text/secondary` is `#646464` in Light and `#a6a6a6` in Dark,
  `border/strong` is `#8c8c8c` in Light and `status/warning-border` is
  `#c27d0b` in Light. Every text and control pair now meets WCAG contrast,
  and the test that holds them has no exception left. In Light the
  inactive tab titles on the *Dusty rose*, *Warm grey*, *Slate teal* and
  *Sage* bands are `text/secondary` again; *Plum*, where it still falls
  short, keeps `#2b2b2b` at 78 % (specs/decisions.md Q242, #191).
- The workspace version is `0.4.0-dev` until 0.4.0 is tagged (#154).
- **The extension API is version 19.** `Capabilities` gains `CREATE_DIR`,
  `CREATE_FILE`, `SEARCH_SUBTREE` and `DOWNLOAD`, and `Provider` gains
  `unavailable_reason` and `name_problem`; plugins built against 18 are
  rebuilt (#193).
- **A menu entry a location's capabilities rule out is greyed** with the
  reason as its tooltip, not hidden: New folder, New file and Download…
  (#193).
- **Single-click rename is off by default.** F2 and the context menu rename
  as before (#195).
- **Select all** says how many hidden items it left out (#195).
- **The hex view is the fallback previewer**: it takes whatever no other
  previewer recognises, and *Open as hex* is gone from the *No previewer*
  card (#198).

### Fixed

- **Every search result can be reached and picked.** A click on a row
  picks it, a hit of the index taking the pane to its folder, where the
  click was lost; the arrow keys move past a pointer that rests on the list
  and bring every row into view, where the row under the pointer held the
  highlight; and the scroll bar's handle shows whenever there are more rows
  than the twelve in view (#202).
- **The search box's results list** has a width of its own, 560 px, where
  the tabs squeeze the box; it grows with the hits the index sends after
  the folder's own names, where it kept the height it had opened with and
  showed two rows; and a long name and its folder each end in an ellipsis,
  the folder at its start, where they were drawn over each other (#201).
- **The Windows archive starts on a machine without the Visual C++
  Redistributable.** `rick.exe`, Rick's kernel libraries, every plugin and
  the GStreamer runtime import `vcruntime140.dll`, some of them
  `vcruntime140_1.dll` and `msvcp140.dll` too, and the archives up to 0.3.0
  carried none: on a Windows without the redistributable Rick stopped
  before its first window with “VCRUNTIME140.dll was not found”. The
  release workflow now puts the runtime's libraries beside `rick.exe`
  (`cargo xtask bundle` copies them from `RICK_VCRUNTIME_DIR`, the
  toolset's redistributable folder), and its smoke test reads the imports
  of every module in the archive and fails when a library of the Visual
  C++ runtime is not in it (#186).
- **No black band around the window where it cannot be transparent**
  (issue 158). Rick asks for a transparent window to paint its shadow into
  a 10 px margin; where the surface is opaque — as on Windows under
  DirectX 12, which is what a virtual machine without a graphics driver
  gets — that margin showed as a solid black band. Rick now checks once,
  before the first frame,
  whether the first window's surface offers an alpha mode that blends, and
  draws the plain frame where it does not: no margin and no shadow, a
  one-pixel border at the window's edge, and the resize grips on the edge.
  The shadow frame is unchanged where the window is transparent. One line
  in the log at startup names the graphics backend, the adapter and the
  frame chosen. `frame` in the `[window]` table of `rick.toml` overrides
  the check: `auto` (the default), `shadow` or `plain` (#182).
- **The plain frame is the default on X11.** An X11 window is blended with
  the desktop only while a compositing manager runs, and its surface offers
  an alpha mode either way, so without one the shadow frame's margin was
  still a black band. On X11 `auto` now draws the plain frame and asks for
  no transparent window; `frame = "shadow"` in the `[window]` table of
  `rick.toml` brings the shadow frame back where a compositing manager
  runs. Wayland, macOS and Windows are unchanged (#189).
- **Copy, cut and paste by keyboard** work in the file list (issue 176). A
  window delivers Mod+C, Mod+X and Mod+V as copy, cut and paste events, not
  as key presses, and the paste event only when the clipboard holds text;
  the list matched key presses alone, so the three chords did nothing in a
  running Rick. They now go through the keymap like any key — Mod+Shift+C,
  Mod+Alt+C and Mod+Shift+V with them — and a paste is taken at the release
  of its key, whatever the clipboard holds. A text field keeps the chords
  for its text. CI tier 2 drives the chords through a real window on Linux
  (#179).
- **A dialogue keeps the keyboard** (issue 177). Behind *File hashes* and
  *Set up Rick on this system* the file list took every key: Escape cleared
  its selection instead of closing the dialogue, and Delete would have
  trashed the selected file. Escape now closes *File hashes*, with its
  *Verify* field focused too, and cancels the job while the hashes are
  still being computed (#183).
- **Escape calls off Properties** that has not opened yet (issue 178). The
  dialogue opens once its entry is read, which takes a moment on a slow or
  remote volume; Escape pressed in between used to go to the list, and the
  dialogue opened afterwards. An answer to a request that was called off,
  or overtaken by a later one, is dropped — on a remote entry too (#185).
- EXIF in the image preview shows only the tags Rick names and that have a
  real value, with the exposure as `1/120 s` and dates without seconds: a
  photo without GPS data has no GPS rows and no “NaN” (#162).
- The two commit actions are *Git commit* and *SVN commit*; *Commit* no
  longer appears twice in the keyboard shortcuts (#165).
- A download's progress that arrives before the preview pane has seen its
  file is kept, not cleared (#165).
- A console's tab is titled *Console*, not the folder it was opened in; the
  settings in a window of their own no longer draw a second close button;
  the *File hashes* dialogue's secondary line ends in an ellipsis before the
  close button; and an operation's error is no longer cut to “Invali…”
  (#169).
- **Azure requests**: a `PUT` without a body carries its `Content-Length`,
  a space in a query is `%20` in the request as in its signature, and a
  container's name is signed encoded (#193).
- **A tab on a drive that is pulled out or ejected goes Home**, with a
  toast that names the drive, *… was removed*; so does a tab on an account
  or a host that is removed (#194).
- **The drive list is read again on the system's mount events** on Linux,
  so a partition that mounts a moment after its stick is there at once
  (#194).
- **The corner grip resizes every time**, not once (#196).
- **No raw error in the search box** on a machine with neither `tracker3`
  nor `localsearch`: the Tracker connector disables itself and the
  Extensions dialogue says why (#198).
- **The first setting changed after the start is kept.** Rick reads the
  settings file again when something in its configuration folder changes;
  when that happened before the change had been written, the window went
  back to the old value while the file got the new one (#206).

## [0.3.0] - 2026-11-10

The design refresh and the first part of system integration, on Linux, macOS
and Windows: the whole GUI redrawn from tokens in Light and Dark and in two
densities — workspace tabs in the title row, the panes, the file list, the
footer, the side panel, the context menu, the dialogues and the settings
window — with tabs and panes dragged, docked, reordered and spring-loaded;
one Rick per session with the command line and `rick://`; the integration
check, the setup dialog and Settings → Integration with the Linux items; SVG
and icon-container previews; the app icon, the Linux desktop entry and
`Rick.app` on macOS; and the About dialog with the third-party licences.

### Added

- System integration: the check, the setup dialog and Settings →
  Integration, with the Linux items (issue 120,
  [`specs/system-integration.md`](specs/system-integration.md) §6 and
  §7.1, milestone 3 of the integration track). Rick keeps one record per
  integration item under `[integration]` in `rick.toml` — `set-up`,
  `declined`, `unsupported` or `missing`, the day of the last check and
  what Rick wrote for the item — beside `ask` and the switch for the
  check at startup. After the first frame, off the UI thread, it compares
  the wanted state with the actual one, reading only; a declined item is
  never listed and never watched (#123, #126). On Linux the items are the
  install location, the app launcher entry with its icons, the default
  file manager (`inode/directory` and `x-scheme-handler/rick` in
  `mimeapps.list`), *“Show in folder” from other apps*, the `rick`
  command (the link in `~/.local/bin` and the shell function, a marked
  block of `path.sh`) and the dock favourite. Each perform answers a
  receipt of the files it wrote and the values it replaced, the undo
  removes exactly that, and a file Rick did not write is replaced only
  with the user's word (#127). A Rick started from its unpacked archive
  installs itself to `~/.local/opt/rick/<version>` with a `current` link
  through the installer plugin's new `install-rick` command, which takes
  a folder as its source, and continues from the installed copy: the
  running process frees the single-instance endpoint, starts the copy and
  exits once it answers (#128). The running instance owns
  `org.freedesktop.FileManager1` on the session bus, through `zbus`, when
  the item is set up — `ShowItems` is `reveal`, `ShowFolders` opens the
  folders, `ShowItemProperties` reveals and opens Properties — and the
  activation file starts `rick --dbus-service` when no Rick is up (#131).
  The dock favourite appends Rick to the favourites of GNOME and Cinnamon
  through `gsettings`, to the launchers of Plasma's task manager and to
  the list of COSMIC's dock, as best it can: a failure is a warning and
  blocks nothing, and every other desktop is *Unsupported* (#133). *Set
  up Rick on this system* opens once when something is missing or was
  reset: a row per item with a switch, *Install for* (*All users* is not
  available yet), *Don't ask again*, *Not now* and *Apply N items* with a
  progress line (#134). Settings → Integration, after *Appearance*, shows
  the system and the last check beside *Check again*, every item with its
  pill — *Set up*, *Reset by system* with *Restore*, *Skipped* with
  *Add*, *Unsupported* — the switches for shell integration and the check
  at startup, and *Reset choices* (#135). Windows and macOS list no item
  yet: the dialog never opens there and the page shows its line and the
  preferences. Questions 278–290, 293–307 and 313–315; the README's
  install paragraph, the screenshots of the dialog and the page, and the
  first acceptance line clicked through the dialog (#140).
- Single instance, the command line and `rick://` (issue 103,
  [`specs/system-integration.md`](specs/system-integration.md) §3 and
  §4, milestone 1 of the integration track). One Rick per user session
  and config directory: the first launch binds an endpoint named after
  both — a Unix socket in `$XDG_RUNTIME_DIR` (Linux) or the user's
  temporary directory (macOS), a named pipe on Windows — and a second
  launch hands its command line over as one length-prefixed JSON message
  with a protocol version, prints the reply and exits with the instance's
  status within a second; an instance that does not answer is reported,
  a socket a crash left behind is reclaimed, and a second window opens
  only for `--new-window` when no instance answers (#106). The command
  line is parsed with `clap`: `rick [<path>…]` opens each folder as a tab
  of the most recently focused window (the working directory without
  any), `rick reveal <file>…` opens the files' folders and selects the
  files once listed, `--new-window` puts them in a window of their own,
  `--config-dir <dir>` runs another install, and paths resolve against
  the caller's working directory; plugins declare subcommands in their
  manifest (`[[commands]]` in `plugin.toml`) and register an
  `ext::CliCommand` the running instance dispatches to, `rick install
  <archive>` of the tarball installer being the first, and `--help` lists
  core and plugin commands; a bare word that names nothing is an unknown
  command answered with the help and exit 2 (#108). The window a path
  opened in is raised through the platform — `xdg_activation_v1` with the
  launcher's token on Wayland, `_NET_ACTIVE_WINDOW` on X11,
  `NSApp.activate` on macOS, `SetForegroundWindow` after the launch's
  `AllowSetForegroundWindow` on Windows (#114). `rick:///<path>` opens an
  existing folder, reveals an existing file and is an error toast
  otherwise; the desktop entry (`%U`, `x-scheme-handler/rick`) and
  `Info.plist` (`CFBundleURLTypes`) register the scheme (#115). The
  embedded console reports its working directory through OSC 7 (the
  session tells the shells a VTE version so they do) and the tab it was
  opened from follows it; the tab's navigation types `cd` into the
  console while the shell is at its prompt (Linux and macOS can tell; a
  Windows pseudo console cannot, so nothing is typed there); the tarball
  installer's `path.sh` defines a `rick` function for login shells that
  lack `~/.local/bin` on their PATH (#116). Questions 264–267 (#117).
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
- `ext-preview-icons`, a plugin for the icon containers ICO, CUR, ICNS
  and XPM (issue 84, SPEC.md *Icon containers*): claimed by magic,
  decoded in pure Rust (the `ico` and `icns` crates, an XPM parser of the
  crate's own with the X11 colour names and `None` transparent), never
  compiled into the default binary. The variants of a container are the
  document's pages, largest first, stepped through with the pane's pager;
  a variant the plugin cannot decode (an ICNS JPEG 2000 entry, an unknown
  type, a corrupt entry) is listed in File info as not decodable and
  counted in the footer, and a file with none fails to open with a clear
  error. In Fit a variant is magnified by the largest whole factor up to
  8, nearest-neighbour, so every pixel stays a crisp square, and the
  pane's actual-size request gets the variant's own pixels; transparent
  pixels stay transparent. A CUR draws its hotspot as a crosshair, with
  the `icons.hotspot` toggle to hide it. XPM's source is the text layer,
  so *Copy* copies it; the binary containers have no *Copy*. Footer
  `ICO · 7 variants · 48 × 48 · 32-bit · PNG · 23.4 KiB`, `2 not shown`
  when variants were skipped, `XPM · 32 × 32 · 16 colours · 2.1 KiB`
  (QUESTIONS.md 257) (#99).
- The About dialog (issue 101,
  [`specs/about-dialog.md`](specs/about-dialog.md)): a modal in the
  settings window's chrome in place of the floating About window, with
  the symbol, the name, the tagline, the version and the build line,
  *Copy details* and *What's new* — this changelog's section for the
  running version — and the link, licence and credits cards. The details
  are the text `rick --version` prints: commit, target, plugin ABI, build
  date, toolchain, the OS and desktop in one line and the loaded plugins
  (#107). *Third-party licences* is its second page: the bundled runtimes
  (pdfium, GStreamer, 7-Zip, egui's fonts, Phosphor Icons), every crate
  of the release build and one section per plugin loaded from outside the
  release's `plugins/` folder, with a search field, a filter by licence,
  each licence text opening in place and *Export…* (#109). The release
  workflow generates the crates' list as `third-party.json` beside the
  binary, in `Rick.app`'s Resources on macOS, and fails when a crate the
  release links is missing from it; the build records its date, its
  target and the contributors (#105).
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
- The drag of a pane and of a workspace tab (issue 80, rules 7 and 8;
  SPEC.md *Drag ghost*). The ghost of a pane dragged by its handle is the
  pane itself — what it painted when it was grabbed, at a quarter of its
  size and 70 % opacity inside a 1 px `border/strong` border with a soft
  shadow — in place of the card with a title and skeleton rows, and it is
  anchored to the grab point: its scaled grip sits 8 px down and right of
  the cursor and moves rigidly with it. A dragged tab is its own ghost,
  lifted with the same shadow: along the title row it rides the row held
  where it was grabbed, off the row it follows the cursor the same 8 px
  away while the row closes up behind it, and released off the row inside
  the window it returns to its place, where it used to be reordered to
  the room the row last kept for it. A tab shows the open hand over its
  body, as a pane's grip does, and the closed hand stays for the whole of
  either drag, whatever the cursor passes over. A screen reader is told
  what is being dragged. The numbers — scale, opacity, offset — are
  constants of `ui_egui::drag` (#125).
- Reordering workspace tabs by drag, Chrome-style (issue 80, rule 1;
  SPEC.md *Workspace tabs*, Dragging a tab). The tabs a dragged tab passes
  slide, over 150 ms and eased out, to open the gap where it will land,
  and close it again when the tab leaves the row; let go, the tab settles
  into the gap. Nothing marks the place but the gap: there is no
  insertion line. Under the system's reduce-motion setting the tabs take
  their places at once. The slide's duration is `ui_egui::drag::TAB_SLIDE`
  (#129).
- A workspace tab that holds exactly one pane can be dragged down into the
  workspace the window shows (issue 80, rules 2 and 3; SPEC.md *Workspace
  tabs*, Dragging a tab): off the row, the pane under the cursor shows the
  four side dock targets — no centre, the tab being a tab already — with
  the preview of the half, and dropped on one the tab's pane docks there
  and the tab leaves the row; it is the reverse of dropping a pane on the
  tab row. A tab that holds several panes is offered no target: it
  reorders, changes window or tears off, and its panes move by their
  handles. Whether a tab holds one pane is the model's answer
  (`Model::sole_pane`), every pane counting, the preview too
  (QUESTIONS.md 270), and the move is the model's `dock_workspace`
  behind `Command::DockWorkspace` (#130).
- Spring-loaded workspace tabs for every kind of drag (issue 80, rules 4
  and 5; SPEC.md *Workspace tabs*, Spring-loaded tabs). A pane dragged by
  its handle and rested on another workspace's tab for the spring-loading
  delay — the one setting of Settings → General, a second by default —
  switches the window to that workspace, where the pane can be docked; a
  dragged tab that holds one pane does the same when it is held still
  leaning on a neighbour, a quarter of a tab or more over it
  (QUESTIONS.md 308). Let go where it cannot land, nothing moves and the
  window shows the workspace the drag began in. Entries dragged from a
  list already switched the window this way; a test now holds it (#132).
- Escape cancels the drag of a pane or of a workspace tab (issue 80, rule
  9; SPEC.md *No-ops and cancel*): the ghost goes, nothing moves, the
  tabs are back in their order and the window shows the workspace it
  showed when the drag began, whatever spring-loading has switched since;
  the button that is still down then drops nothing. A cancelled drag of
  entries returns the window to that workspace too, whatever *After a
  drag* says and through however many tabs it sprang (QUESTIONS.md 309);
  the folders its panes drilled into still follow the setting
  (QUESTIONS.md 276) (#137).
- A workspace tab or a pane can be dragged onto another window (issue 80,
  rule 6; SPEC.md *Windows*). Over the other window the drag shows what
  it shows over its own: a tab rides that window's tab row, the tabs
  there sliding apart for it, and lands in the gap; a tab that holds one
  pane, and a pane dragged by its handle, are offered the dock targets of
  the pane under the cursor there; a pane over the row shows the slot
  where it becomes a new tab. The tabs of the other window spring-load
  alike, the window the cursor is over paints the ghost
  (QUESTIONS.md 311), and Escape or a release that lands nowhere leaves
  every window on the workspace it showed. Where the system does not
  tell a window its place on the screen (Wayland), a drag still knows
  its own window alone (QUESTIONS.md 310) (#138).

### Removed

- The accent-colour setting: colours are the scheme's contrast-checked
  pairs, and more colour comes as further schemes; an `accent` key in
  `rick.toml` is ignored and dropped on the next write (QUESTIONS.md 226)
  (#68).

### Fixed

- A dot-file is hidden on Windows too: the listing there took the hidden
  attribute alone, so `.name` was shown with hidden files off, against
  SPEC.md's *Hidden files* (#145).
- A `rick://` link's path is resolved like a path on the command line, to
  its canonical form, so a link and a path that name the same folder
  agree where the two spellings differ — a symlinked temporary directory
  on macOS, the `\\?\` form on Windows (QUESTIONS.md 316) (#144).
- A job that reads a decorator's state, the Git status of a repository,
  counts as work in progress, so the window keeps repainting until the
  badges are there (#146).
- A job gives up what it held — its place in the queue of an operation's
  scheduler — before its result is published, not after: the next job no
  longer finds a cancelled one still queued (#148).
- A workspace tab can be dragged to the last place of its row: the row
  never opened its gap after the last of the other tabs, so the first of
  two tabs could not be dragged past the second at all, and a tab pulled
  to the end of a longer row landed one place short (issue 80, rule 1)
  (#129).
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
- Drive cards as decided for the design refresh (SPEC.md *This Machine*):
  the usage bar's fill follows how full the drive is, like the Trash pill
  — `accent/usage` below 80 %, `status/warning` from 80 %, `status/danger`
  from 90 % — in the card and in the This Machine overview; a drive the
  operating system mounted read-only (a live USB, an optical disc, a
  mounted ISO) shows a full bar in `text/placeholder`, no percentage and
  nothing else in the row, the bar's tooltip and accessible description
  *Read-only*; the name stays on one line with an ellipsis. `Volume` of the
  `platform` crate reports `read_only` on every platform: the `ro` mount
  option on Linux, `MNT_RDONLY` on macOS, `FILE_READ_ONLY_VOLUME` on
  Windows, an ISO's mount everywhere (issue 79 item 8).
- The column header as decided for the design refresh (SPEC.md *File
  list*): no permanent divider between the titles; the boundary between
  two headers has a 6 px hit zone that, hovered, shows a 1 px
  `border/strong` divider across the band and the column-resize cursor;
  dragging it resizes the column before it, Name included, a double-click
  fits that column to the title and the rows in view, no column goes under
  48 px, and the last column takes what is left. The widths are the tab's
  (`Command::ColumnWidths`, `Tab::columns` by column id) and come back
  with the session; *Size columns* puts the defaults back (issue 79
  item 9).
- Path editing as decided for the design refresh (SPEC.md *Path bar*):
  Mod+L, or a click on the empty part of the breadcrumb bar, swaps the
  breadcrumbs for a text field in place — `button/size` tall on the Back /
  Forward line and as wide as the breadcrumb, `bg/control` with a 1 px
  `accent/focus` ring and `radius/control` corners, the path in 13 px
  `text/primary` — with the whole path selected on entry, so typing
  replaces it; Enter navigates, Escape and losing the focus restore the
  breadcrumbs unchanged (issue 79 item 10).

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

[Unreleased]: https://github.com/Open-Breach/rick/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/Open-Breach/rick/releases/tag/v0.4.0
[0.3.0]: https://github.com/Open-Breach/rick/releases/tag/v0.3.0
[0.2.0]: https://github.com/Open-Breach/rick/releases/tag/v0.2.0
[0.1.0]: https://github.com/Open-Breach/rick/releases/tag/v0.1.0
