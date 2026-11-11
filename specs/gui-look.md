# GUI

Related: specs/gui-title-row.md, specs/gui-dialogues.md

A side panel on the left gives one-click access to the places that matter; the main area holds the workspaces — each a layout of panes — and the preview. The look follows GNOME and libadwaita (Linux first): calm, friendly and accessible, on a 6 px grid with rounded controls, in place of the Visual Studio Code-like chrome of 0.1 and 0.2. The design file is the Figma page *Main view* of *Rick — FileExplorer* (issue 60); this chapter records its decisions, and where the file and this document disagree the question goes into the pull request, never to the code.

Default layout: the side panel on the left; one workspace tab holding one explorer pane with the preview pane to its right; the side panel is collapsible and can dock right, the preview is a pane like any other and can sit below or in its own window, and every pane's footer holds its status and its filter.

## Look and feel

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
| `text/secondary` | `#646464` | `#a6a6a6` | Column headers, section headers, footer statistics, the Size, Type and Modified columns, inactive tabs, detail lines, shortcuts in menus |
| `text/placeholder` | `#737373` | `#979797` | Placeholder text in fields |
| `text/on-selected` | `#1f528c` | `#a9d2ff` | Text on `bg/selected-soft` |
| `icon/default` | `#4a4a4a` | `#cfcfcf` | Icons at rest |
| `border/default` | `#d6d6d6` | `#3d3d3d` | Hairlines, dividers, field and card borders, the window border |
| `border/strong` | `#8c8c8c` | `#777777` | Checkbox borders and other small controls that need more contrast |
| `accent/focus` | `#2873c0` | `#4ea1ff` | Blue text, focus rings, the icon of an active toggle, a switch that is on, progress bars, the portal animations, the filter-active dot |
| `accent/button` | `#2a77c6` | `#2975c7` | The background of a primary button, with white text on it |
| `accent/usage` | `#3c84c2` | `#5ab0ff` | The fill of a drive's usage bar |
| `status/success` | `#1d7943` | `#6fd49a` | A hash that matches: field border, pill text, row label |
| `status/success-bg` | `#d9f2e0` | `#1e3b2a` | The *Matches* pill |
| `status/success-row` | `#edfaf0` | `#1b2e22` | The matching hash row |
| `status/warning` | `#8c5c00` | `#f2c46b` | Warning text, such as the missing version in the installer |
| `status/warning-border` | `#c27d0b` | `#d98c0d` | The border of a field with a warning |
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

## Loading animations

Rick's own loading animations are the **portal family**: an abstract portal in `accent/focus`, built in code with the egui painter — the Figma frames under *Loading indicators* only approximate the motion. The reduce-motion setting of the system turns both into a still portal: the rings stop and nothing flies.

**Portal spinner.** Three arc rings around a softly pulsing glow, rotating at one, two (reverse) and three times the base speed, one loop about a second; 18 px as the icon of a running copy or move job in the operations drawer (waiting jobs keep the hourglass), 24 px as its base size elsewhere.

**Portal burst.** 96 px: the spinner's rings and glow with solid folder and file glyphs spiralling out of the portal, tumbling in different directions and fading as they go. It is shown centred in the content area wherever the file list or the preview pane is loading or busy, with one status line under it (*Reading folder… N items so far*, *Loading preview…*), and in the hash dialogue while hashing. It replaces every skeleton-row placeholder.

## Accessibility

Every icon-only control — tool buttons, the toggles, the pane handle, the round cancel buttons, the per-row copy icons, the close buttons, a drive card's eject button — carries an accessible name through AccessKit, the same text its tooltip shows, so a screen reader can announce it; a read-only drive's bar carries *Read-only* as its accessible description, the text of its tooltip. The controls 0.4 adds are held to the same rule: the preview pane's info button and the pin of its card, the copy button of every row in the File info card and in the dialogues (*Copy Location*), the reset button of a shortcut (*Reset to default: Ctrl+F2*), the remove button of an ACL entry, the open-folder button of a missing library's *Destination* row (*Open folder*), the swatch tiles of *Title bar colour* (the preset's name), the column dividers of a table in a preview (*Resize column Name*) and the close button of a toast. Keyboard focus is always visible as the 2 px `accent/focus` ring. Colour never carries a meaning alone: a match has its pill, a destructive entry its label, a filter its dot beside the icon. Every colour pair in both schemes meets the contrast rule under *Look and feel*.
