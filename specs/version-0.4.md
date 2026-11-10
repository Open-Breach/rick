# Version 0.4 — scope

Related: specs/gui-look.md, specs/gui-title-row.md, specs/gui-side-panel.md, specs/gui-preview.md, specs/gui-dialogues.md, specs/functions.md, specs/plugins.md

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

**Not in the bundle.** The functional bugs of the same session have their own issue (149) and are not fixed along the way: a new folder and a new file on Azure, at the account root and inside containers; searching subdirectories on remote providers; a double-click on a remote file; the tab left on removed or ejected media; sorting by the real modification time; the date formats with the time of day; the window's position and size across launches; a *Format…* action for media; the second partition that appears only sometimes; `tracker3` missing; the hex previewer not claiming `.bin`; executables and scripts not starting on a double-click; `~` and environment variables in the path bar; *Open in editor*; single-click rename as a setting that is off by default; *Select all* with hidden files; the layout of the List view; and the corner resize that goes dead after one use.

What else 0.4 holds is added here as it is decided. During 0.4 the workspace version is `0.4.0-dev`.
