# Main area and further GUI elements

Related: specs/gui-look.md, specs/gui-title-row.md, specs/gui-preview.md, specs/gui-windows.md, specs/functions.md

## Main area

The main area shows the active workspace: its panes, each an explorer view with a toolbar above the file list and a footer below it, and the preview pane, which has the same frame.

**Panes.** A workspace holds one or more panes in nested splits, as in an IDE: side by side or stacked, dividers dragging to resize, two-pane, three-pane and grid layouts all possible. This is egui\_dock's tree of splits, without its tab bars. A pane is a `bg/pane` surface; panes are separated by a hairline, and the focused pane is told by its selection colour (below), never by a border. Every pane has a fixed **grab handle** at the top right of its toolbar: a 28 px wide ghost area with `DOTS_SIX_VERTICAL` at 70 % opacity after a hairline divider. Hovering shows the grab cursor, the open hand; dragging shows the grabbing cursor, the closed hand, for as long as the drag lasts and wherever the cursor goes — a drag never shows the arrow. Dragging the handle moves the pane: while it drags, the **drag ghost** (next paragraph) follows the cursor, and **dock targets** appear on the pane under it — five rounded `bg/card` tiles in a cross (left, right, top, bottom, centre), each with a split icon, the one under the cursor filled `accent/button` — with a translucent `accent/button` preview of the half the pane would take. Left, right, top and bottom split that pane and dock the dragged pane on that side; the centre, or the tab row, makes the dragged pane a new workspace tab; outside every window opens a new window. Resting on another workspace tab switches the window to that workspace, so the pane can be docked into a layout that was not showing, and another window takes the pane on its tab row or on its dock targets (see Workspace tabs, and Windows under General behaviour). A pane leaves a workspace by being dragged out, so a workspace never ends up empty. The layout, panes and their paths are restored on the next start.

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

## Further GUI elements

**Tree view.** A folder tree that expands in place, available as a view mode and as its own dockable pane, never in the side panel; selecting a node navigates the focused explorer pane, and the tree follows the explorer's location unless unlinked.

**Theme and density.** See *Look and feel*: the scheme (system, light, dark), the density (Comfortable, Compact), the font size and the title bar colour are the Appearance settings; there is no accent-colour setting. Syntax highlighting themes follow the scheme.

**Hidden files, symlinks and mounts.** The footer's eye toggle (Mod+H) shows dot-files and files with the hidden attribute. Symlinks show a link badge on the icon and their target in the Type column; a broken link is marked. Mount points show a volume badge, and the side panel's This Machine and Network sections list mounted volumes.

**Icon and thumbnail view.** Besides the details list, a grid view with large icons or thumbnails for images, PDFs and videos, in four cell sizes (small, medium, large, extra large; the menu offers the first three). A thumbnail is rendered by the Previewer registered for the entry's type, at the cell size, off the UI thread, and cached per entry and size; while it renders, or when no previewer claims the file, the cell shows the file-type icon. The freedesktop thumbnail cache on Linux may seed that cache later.

**Inline rename.** F2, or a second click on the single selected row at least 0.5 s after the first (past the double-click window), edits the name in place in the list, with the extension excluded from the initial selection; Enter commits, Escape cancels, a clashing name is refused in place. New file and New folder insert a row in the same edit mode at the top of the list.
