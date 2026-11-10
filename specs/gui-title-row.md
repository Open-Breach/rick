# Title row, workspace tabs and menu

Related: specs/gui-look.md, specs/gui-main-area.md, specs/gui-dialogues.md, specs/functions.md, specs/about-dialog.md

## Title row

Rick draws its own window chrome; the platform's title bar is turned off. Where Rick draws the controls (Linux, Windows) it draws the window's frame too, one of two (*Window frame*, below): where the window can be transparent, it is transparent behind a 10 px margin that holds a light shadow and a one-pixel border in `border/default`, with invisible resize grips along the edges and corners; where it cannot, the frame is a plain one-pixel border at the window's edge. A maximised or fullscreen window has no frame; macOS keeps the system's border and resize handles. Dragging an empty part of the title row moves the window; double-clicking it maximises or restores.

**Window frame.** The frame Rick draws around a window it decorates itself is one of two, the same for every window of the process:

- **Shadow frame**, the frame of a transparent window: a 10 px transparent margin around the content holds a light shadow, a one-pixel border in `border/default` runs around the content, and the resize grips cover the margin and reach 4 px inside the border.
- **Plain frame**, the fallback where the window is opaque: no margin and no shadow. The content fills the window, a one-pixel border in `border/default` runs along the window's edge, the corners are square, and the resize grips sit on the edge and reach 4 px inwards. The window is cleared to the panel colour, never to transparent.

In both, a grip within 24 px of a corner resizes diagonally, and a maximised or fullscreen window has no frame.

Rick asks for a transparent window and finds out once, when the first window's surface exists, whether it got one. The window counts as transparent when the surface the renderer draws to offers an alpha mode that blends the window with the desktop, pre-multiplied or post-multiplied; a surface that offers neither is composited opaque, and the margin of a shadow frame would show as a black band. This is a check of what the surface can do and never of the machine: Rick does not look for a virtual machine, a vendor or a driver name. Until the answer is known the window counts as opaque, so no black band shows even for one frame. Rick writes one line to the log at startup with the graphics backend, the adapter's name and the frame chosen; the fallback shows no dialogue and no notice.

The choice can be overridden in `rick.toml`, key `frame` of the `[window]` table: `auto` (the default) follows the check, `shadow` always draws the shadow frame, `plain` always draws the plain frame and does not ask for a transparent window at all. It is for a system that reports a transparent surface and composites it wrongly, or the other way round. The key is in the config file only, with no entry in Settings; it is read when Rick starts, and a value Rick does not know reads as `auto`.

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

## Workspace tabs

Tabs live in the title row, Chrome-style, and each tab is a whole **workspace**: its own layout of panes with their locations, histories, selections and dividers. Switching tabs swaps the entire window content. Panes have no tab strips of their own — the per-pane strips of 0.1 and 0.2 are gone — so the title row is the only place with tabs, and a window with two panes shows them in one workspace, not as two strips.

- **Shape.** A tab is 220 px wide and `tab/height` tall, sitting on the bottom edge of the title row with 10 px top corners and 2 px between tabs: a 16 px icon, the title in 13 px truncated with an ellipsis, and a close button (`X`). The active tab is filled `bg/pane`, lifting out of the title row's band, and joins the pane area below it with no line between and no accent line on it, its title medium in `text/primary`; an inactive tab has no fill and shows the band, its title in `text/secondary` and its close button at half opacity; hovering an inactive tab shows `bg/pane` at 50 % over the band. *On the band* under *Title row* says where the band's colour changes these. The new-tab button (`PLUS`) is a 28 px wide ghost button directly after the last tab, never at the far right.
- **Title.** A workspace is titled automatically from the folder in its focused pane — the folder's name, or a virtual location's title — and updates as focus or folder changes; there is no renaming. The icon is the one the side panel uses for that folder when it is a place (Home, Downloads, Trash and so on), the provider's icon for an archive, a remote host or a cloud account, and `FOLDER` otherwise. A workspace whose focused pane is a console is titled *Console* — not the folder the console was opened in, and not what the shell sets as its title.
- **Behaviour.** `+` opens a new workspace with one pane at the focused pane's location; Mod+T does the same, Mod+W closes the workspace tab, Mod+Tab and Mod+Shift+Tab cycle, and middle-click closes any tab. *Open in new tab* everywhere — the side panel, the context menu, a middle-click on a folder — opens a new workspace with one pane at that location, next to the current one. Tabs move by drag (next) and, from the keyboard, with Mod+Shift+PageUp and PageDown, which shift the active tab one place left or right. What happens when tabs outgrow the row is not designed yet: until it is, the row clips and the tabs stay reachable through Mod+Tab and the command palette.
- **Dragging a tab.** Hovering a tab shows the grab cursor, the open hand; dragging it shows the grabbing cursor, the closed hand, wherever the cursor goes, and never the arrow. A click activates a tab, a drag does not: the window goes on showing the workspace it showed. *Along the row* the tab reorders as in Chrome: lifted with a soft shadow, it follows the cursor along the row, held at the point where it was grabbed, and its neighbours slide apart, animated, to open the gap where it will land; there is no insertion line, and on release the tab settles into the gap. *Off the row* the lifted tab follows the cursor as its drag ghost (see Panes), and what the tab holds decides where it can go. A tab holding **exactly one pane** can be dragged down into the workspace the window shows: the dock targets appear on the pane under the cursor — left, right, top and bottom with the preview of the half, and no centre, the tab being a tab already — and a drop on one moves the tab's pane into that layout on that side, the tab leaving the row; it is the reverse of dropping a pane on the tab row. A tab holding **several panes** shows no dock targets: it can only be reordered, moved to another window's row or torn off into a window of its own, and its panes move one at a time by their handles. Any tab dropped on another window's tab row moves there, into the gap that row opens for it, and one dropped outside every window becomes a new window; released anywhere else it returns to its place.
- **Tab bar as drop target.** A pane dragged by its handle (below) and dropped on the tab row, or on the centre target of a pane, becomes a new workspace tab at the drop position; the row shows an `accent/button` slot where the tab would land. Another window's tab row takes the pane the same way.
- **Spring-loaded tabs.** While a pane, or a tab holding exactly one pane, is dragged, resting on another workspace tab for the spring-loading delay (see Drag and drop under Functions) switches the window to that workspace, so the drag can go on to its dock targets; dragged entries switch it the same way, to reach a folder in another workspace. A dragged pane rests on a tab while the cursor is on it; a dragged tab, which rides the row under the cursor, rests on a neighbour while it overlaps it by a quarter of a tab or more, short of the half where the two would swap. A pane dropped on the tab itself still becomes a new tab. The tabs of another window's row spring-load alike.

## Menu

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
| About | About Rick (the About dialog, [`specs/about-dialog.md`](about-dialog.md)) | `INFO` | The version of the running build, `0.4.0` |

- Every entry has an icon. No label ends in "…": *Settings*, *Keyboard shortcuts* and *Extensions* are written as GNOME writes them. A shortcut hint comes from the keymap, never from a table of its own: the entry shows the chord bound to it — the first when it has several — and nothing when it is unbound. There is no theme switch in the menu: the scheme follows the system by default and is changed in Settings → Appearance.
- The submenus are cards of the same form and keep their entries.
