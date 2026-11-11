# Functions

Related: specs/gui-dialogues.md, specs/gui-side-panel.md, specs/gui-title-row.md, specs/gui-windows.md, specs/core-types.md, specs/keyboard-shortcuts.md

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

## Operations

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
- **New file.** Create an empty file in the current location with its name in inline edit; extensions can add templates (text, markdown, and so on). Offered where the location has `CREATE_FILE`, disabled with the reason where it has not (*Capabilities decide what is offered* under Core types).
- **New folder.** Create a folder in the current location with its name in inline edit; with a selection, a variant moves the selected items into the new folder. Offered where the location has `CREATE_DIR`; at the root of an Azure Blob account it creates a container (*Object storage* under Supported scope).
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
- **Select all / invert selection.** Select everything in the list (Ctrl+A) or swap selected and unselected. *Select all* selects what the list shows and nothing else: an entry the pane is not showing is never selected invisibly. When the folder holds hidden items that are not shown, a toast says so with their number, *3 hidden items not selected* (*1 hidden item not selected*).
- **Select by pattern.** Select the entries matching a glob or regular expression entered in a small prompt (Ctrl+Shift+A), adding to or replacing the current selection.
- **Undo / redo.** Reverse the last move, rename, trash, new file or new folder (Ctrl+Z / Ctrl+Shift+Z); copies and permanent deletes are not undoable and say so.
- **Open with.** Open the selection in a chosen application from the platform's list for that type, or any executable; the choice can be remembered for the type.
- **Open in editor.** Open the selected local files in the configured editor (F4); see *Open in editor* under *Opening files* in General behaviour.
- **Download.** Bring the selected remote files and folders down to the machine as one job, into a folder asked for first, Downloads by default; see *Remote files* under *Opening files*. Offered on remote locations only.
- **Compress.** Pack the selection into an archive (zip by default, other formats in a small dialogue) next to it.
- **Extract here / extract to.** Unpack the selected archive into the current folder, into a folder named after it, or to a chosen location.
- **Compare.** Compare the two selected files, or the folders of the two panes, in a diff view; file contents via a text diff, folders by name, size and modified date with the differences highlighted.
- **Pin to side panel.** Add the selected items — folders or files — to Favorites (the Places entry that opens as a view), at the end. A pinned folder opens in the current pane; a pinned file opens its folder with the file selected. The action sits in the context menu of every entry and in the palette; on an already pinned item it reads **Unpin**, and each entry in the Favorites view offers Unpin in its context menu too. Pin and Unpin are undoable.
