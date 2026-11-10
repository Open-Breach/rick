# Version 0.4 — scope

Related: specs/gui-look.md, specs/gui-title-row.md, specs/gui-side-panel.md, specs/gui-main-area.md, specs/gui-preview.md, specs/gui-dialogues.md, specs/gui-windows.md, specs/functions.md, specs/plugins.md, specs/core-types.md, specs/scope.md, specs/keyboard-shortcuts.md

0.4 opens with the GUI fixes of the test session of 2026-11-09 (issue 139): one bundle of visual and interaction changes. Every item was drawn in the Figma section *Test-session redesign — 2026-11-09* of the page *Main view* and agreed there; the frames and their yellow notes frames are the design, and this document records their rules in the places named below. The spec lands first; then one pull request per item into `dev`, in this order, each compared with its Figma frame in Light and Dark and both densities before it merges.

1. **App menu** — *Menu*.
2. **Paste-conflict dialogue** — *Dialogues*; *Execution* under Functions.
3. **Properties dialogue** — *Dialogues*; *Permission management* under Functions.
4. **Preview pane: the empty state and the File info card** — *Preview*: *File info card*, *Nothing selected*, *No previewer*; *Third-party libraries* and *Media metadata* under General behaviour.
5. **Tables inside previews** — *Tables in previews* under Preview.
6. **Remote size limit** — *Remote sources* under Preview.
7. **Keyboard shortcuts dialogue** — *Dialogues*.
8. **Extensions dialogue** — *Dialogues*.
9. **Title bar colour** — `bg/titlebar` in *Look and feel*; *Title row* and *Workspace tabs*; the Appearance page of the *Settings window*.
10. **Small items** — the spacing of the window controls (*Title row*); no second close button in the settings (*Settings window*); the hash dialogue's secondary line cut before the close button (*Dialogues*); a console's tab titled *Console* (*Workspace tabs*); an operation's error shown whole (*Operations drawer* under Side panel).

The bundle reuses the dialogue frame, the scheme and density tokens and the Portal burst of the design refresh, and adds one token, `bg/titlebar`, and no other colour. It ends with the changelog entry and the screenshots of every item in Light and Dark.

**Not in the bundle.** The functional bugs of the same session have their own issue (149), the second bundle below, and are not fixed along the way: a new folder and a new file on Azure, at the account root and inside containers; searching subdirectories on remote providers; a double-click on a remote file; the tab left on removed or ejected media; sorting by the real modification time; the date formats with the time of day; the window's position and size across launches; a *Format…* action for media; the second partition that appears only sometimes; `tracker3` missing; the hex previewer not claiming `.bin`; executables and scripts not starting on a double-click; `~` and environment variables in the path bar; *Open in editor*; single-click rename as a setting that is off by default; *Select all* with hidden files; the layout of the List view; and the corner resize that goes dead after one use.

**The functional fixes of the same session (issue 149).** The second bundle of 0.4, taken once the GUI bundle is finished: three of its items use what that bundle introduced — the download job asks with the paste-conflict dialogue, a lost connection shows the File info card, and the Tracker connector reports through the attention dot of the Extensions dialogue. Every item is a bug to fix or a behaviour that was decided, not a design; only the List view was drawn, in the frame *List view mode* of the same Figma section, with its notes frame. This document records the rules in the places named below. The spec lands first; then one pull request per item into `dev`, in this order, and every bug fixed gets a test that fails before the fix.

1. **Remote providers** (Azure Blob, S3, GCS, SSH) — the capability flags, `unavailable_reason` and `name_problem` under *Provider* in Core types; *Object storage* under Supported scope; *Remote files* under *Opening files*; *Context menu* and the *Footer*'s include-subfolders toggle under Main area; *Download* under Operations.
2. **Removable media and connections** — *Removed and unreachable locations* under General behaviour; *This Machine* under Side panel (*Format…*, and the list re-read on every mount and unmount event).
3. **File list** — *List view*, *Sorting*, *Dates* and *Inline rename* under Main area; *Select all* under Operations; the Appearance and Behaviour pages of the *Settings window*.
4. **Window** — *Window geometry* under General behaviour; and a bug without a rule of its own: the corner resize grip goes dead after one resize until a click elsewhere.
5. **Launching and opening files** — *Executables and scripts* and *Open in editor* under *Opening files*; *Path bar* under Main area; F4 in Keyboard shortcuts.
6. **Search and previewers** — *Indexed search* under Main area and *Attention* in the Extensions dialogue for the Tracker connector; *No previewer* under Preview and *Previews are modules* for the hex view as the fallback, which takes the *Open as hex* button off the *No previewer* card of the first bundle.

It ends with the changelog entry, the new settings in the README where settings are listed, and a screenshot of the List view in Light and Dark.

Item 1 raises the extension API version to 19, for the four capability flags, `unavailable_reason` and `name_problem`. Every plugin is rebuilt against it; one that was built against 18 and is not in the workspace yet — `ext-zfs`, pull request 156 — is refused by the registry until it is rebuilt.

**Not in the second bundle.** The five identical `tmp` rows at `/` of the same session were not reproduced and are left out on purpose; subfolder search over SSH; a plugin permission model. Nothing visual of the first bundle is changed along the way, but for that one button.

What else 0.4 holds is added here as it is decided. During 0.4 the workspace version is `0.4.0-dev`.
