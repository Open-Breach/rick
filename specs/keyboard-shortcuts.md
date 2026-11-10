# Keyboard shortcuts

Related: specs/functions.md, specs/gui-dialogues.md, specs/gui-main-area.md

All shortcuts are remappable in Settings → Keyboard shortcuts. The binding tables live under "Keyboard shortcuts" at the end of this document; that table is the single source of defaults.

## Keyboard shortcuts

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
