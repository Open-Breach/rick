# ui-egui

`ui-egui` is the only crate that knows egui. Every text it shows goes
through the `t!` lookup with a key of `locales/en.ftl`, which
`.github/scripts/check-strings.py` enforces in CI. Every colour and size it
draws comes from the scheme and density tokens of `theme` (the Figma
collections *Rick* and *Density*, one to one; Light and Dark, Comfortable
and Compact), from which egui's visuals and spacing are derived; its type
is the system's UI and monospace fonts (`fonts`, through the platform's
`SystemFonts`), egui's bundled fonts behind them, and the Phosphor icons. It renders every window from
the `model` the `app` owns, the first as eframe's root viewport and the
others as viewports of their own drawn in the same frame: the window frame
with its shadow and resize grips where the system draws none, the title row
with the burger menu (New window, the Layouts submenu), the workspace tabs
(one per workspace, titled after the folder in its focused pane, reordered
by drag with the neighbours sliding apart for the gap where it will land, the dragged tab its own ghost lifted with a shadow, a tab of one pane docking into the shown layout on the side targets of the pane under it, a tab dragged outside every window opening a new one) and the
search box, the `egui_dock` tree of the shown workspace's split panes, whose
divider positions it reports back for the session,
each pane's toolbar (Back and Forward, the parent chip and breadcrumb, the view options menu with the per-type filter, the grab handle that docks the pane on another's dock tiles, the pane's ghost — its own snapshot at a quarter, anchored to the grab point, `drag.rs` — following the cursor), path bar (breadcrumbs per provider: a `rick://` view's
title, the folders above an archive as local segments and the archive and
its contents on the archive provider), file list (the 34 px header band over the Name, Size, Type and Modified columns, rows with inset rounded hover and selection backgrounds and the cursor ring, the empty-folder message, the portal burst while a folder is read; details, tree, list or
icon view, only the visible rows laid out, names edited in place for
Rename, New file and New folder) and footer (the toolbar's height under a
hairline: the hidden-files toggle, the folder and file counts and the
files' size — *12 of 340* while a filter narrows the list — the selection
pill, and the 200 px filter field with the include-subfolders toggle
inside its right end), the Preview tab (toolbar with the media transport and *Copy*, info
badges, content, status bar; one per window, following the focused
explorer, docked and torn out like any tab; Rick's only viewer, laying out
every kind of content a previewer returns itself in `content.rs`: a file
list in the explorer's columns, a grid, a rendered document, and the
message for a missing third-party library with *Browse…* and *Retry*; the
placeholder for a file no previewer claims), the side panel (rows
`sidebar/item-height` tall inset from the panel's edges; the editable
Places list with its drag reorder, Edit… dialogue and keyboard, Favorites,
Recent and the Trash view among the places, the Trash row's pill with the
trash's size tinted by its disk share; the This Machine, Network and Cloud
sections under small-capitals headers with a folding caret, This Machine
as drive cards with a usage bar, Eject and Add… rows, nothing indented;
the section overviews drawn as drive tables in the pane) with the
operations drawer floating over its foot — along the window's foot while
the panel is hidden — (a `bg/card` card with its summary *2 running ·
2 waiting*, the overall percentage and bar, collapsible to that line; a
block per running job with its icon, title, pause and cancel, a thin bar
and the bytes or items done of total with throughput and time left; the
waiting jobs with why they wait; finished jobs as compact rows; rows that
scroll beyond a cap), the dialogue frame in `dialog.rs` (a `dialog/bg`
modal with 12 px corners and a shadow: the header with an icon, title and
secondary line, a scrolling body, the footer with the note and the
buttons; a form's fields in a label-left grid with cards, disclosure
cards, segmented controls, switches, file pickers and the preview as a
card of effects — the install dialogue's shape — taken by every form,
prompt and confirmation an action asks), the shared controls in
`widgets.rs`, the conflict dialogue, the question dialogue an extension asks through, the
Properties dialogue (the general rows and, for a local entry, its owner,
group, mode bits and ACL edited in place, applied to the enclosed entries
too when asked), the Hash dialogue (620 px in the dialogue frame: *File
hashes* with the file's name and size and a close button; the *VERIFY*
field on top, prefilled from the clipboard when it holds a hash and
compared live against every row — a `status/success` border, the
*Matches SHA-256* pill and the tinted row on a match, `status/danger` and
*No match* otherwise; a card per file with the MD5, SHA-1, SHA-256 and
BLAKE3 rows, each with its monospace value and a copy icon; the *Format*
segmented control; *Copy all* and *Done* in the footer; the portal burst
with the progress line and bar while the job runs, when *Cancel* stops it
and closes the dialogue), the context menu (a 260 px card of the spec's groups — Open, Edit with the *Copy as* submenu, Change with the destructive pair in `status/danger`, the extension groups as submenus or single entries, Info — each entry with an icon slot, its label and its shortcut, hidden when it does not apply; on the empty area New folder and New file, Paste, Select all, Open terminal here, Refresh and Folder properties) and command palette
built from the action registry, the keyboard shortcut table, the Settings
window in `settings_ui.rs` (880 × 640, a viewport of its own where the
backend has them and an area over the main window otherwise: the
`bg/panel` column with the title, the search field that narrows the
categories and rows to those whose label or description match, and the
category list — General, View, Appearance, then *PLUGINS* with a row per
plugin that declares settings; the page at the right with its title, a
round close button and grouped cards, one row per setting with its label,
description and control: a switch, a dropdown, a segmented control or a
text field) and the Keyboard shortcuts screen, and drag and drop of entries: rows are drag
sources, folders, pane space, breadcrumb segments and tab headers are drop
zones with the ghost, the live indicator and spring-loading in between (a
pane at the Trash view trashes what lands in it); the side panel is never a
file-operation target, a dragged folder becomes a place at the insertion
line and files are refused with the not-allowed indicator; a drag that
leaves the window is handed to the platform's drag source one frame later,
once the ghost is gone, and followed until the platform says it ended; a
drop from outside lands where the platform says the cursor is, or in the
focused pane's folder where it cannot say. When an integration apply
ends with Rick installed to the standard location, the shell saves the
session and hands the installed binary to the `Relaunch` hook the binary
gave it. It never mutates
the model: everything the user does becomes a `Command` handed to `app`,
and it re-renders from the model. It depends on `types`, `runtime`,
`platform`, `ext`, `vfs-local` (the local provider's id, for dropped
paths), `vfs-rick` (the virtual locations' titles and ids), `model` and
`app`; only `rick` and
`tests` may depend on it.
