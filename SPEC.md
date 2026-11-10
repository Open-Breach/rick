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

## How to read this specification

- This file is an index. The specification lives in `specs/`, one file per area; a citation such as `SPEC.md "Engineering guidelines"` names a section heading, and the table says which file holds it.
- Read only the files your issue lists under **Spec files**.
- `specs/history.md` and `specs/decisions.md` are reference, not reading.

| File | Sections it holds | Read it when |
| --- | --- | --- |
| `specs/scope.md` | Decisions so far · Supported scope: Platforms, Local file systems, Network file systems, Object storage, Archives and compression, Disk images, Preview support, Source code repositories | The task turns on what Rick supports: a file system, an archive or disk-image format, a preview format, a version-control system, or one of the technology decisions |
| `specs/gui-look.md` | GUI (the two intro paragraphs) · Look and feel · Loading animations · Accessibility | Anything is drawn: scheme and density tokens, typography, icons, controls, the portal animations, accessible names |
| `specs/gui-title-row.md` | Title row · Workspace tabs · Menu | The window chrome, the search box, the title bar colour, workspace tabs and their drags, the application menu |
| `specs/gui-side-panel.md` | Side panel | Places, the Trash pill, drive cards, Network and Cloud, the `rick://` virtual locations, the operations drawer and job scheduling |
| `specs/gui-main-area.md` | Main area · Further GUI elements | Panes and docking, the toolbar and path bar, the view options menu, the file list, sorting, the footer and its filter, indexed search, the context menu, tree and icon views, inline rename |
| `specs/gui-preview.md` | Preview · from General behaviour: Text files, Pretty print, Images, Vector images, Icon containers, Media metadata, Audio and video | The preview pane or a previewer: toolbar, File info card, tables, the remote size limit, how each kind of file is shown |
| `specs/gui-dialogues.md` | Dialogues · Settings window | A dialogue (install, hash, Git clone, paste-conflict, properties, keyboard shortcuts, extensions) or the settings window |
| `specs/gui-windows.md` | General behaviour: Session restore, Saved layouts, Windows, Opening files, Disk images and Mount | Session and layouts, several windows, what Enter and a double-click open, mounting a disk image |
| `specs/plugins.md` | from General behaviour: Previews are modules, Extensions as installable packages, Third-party libraries, Extension store · from Package layout: Plugin host | Plugin packages, manifests and loading, or a library a plugin needs (pdfium, LibreOffice, GStreamer, 7-Zip) |
| `specs/functions.md` | Functions (Drag and drop, Permission management, Compute hash) · Operations | Drag and drop, move and copy with their conflicts, permissions, hashing, or any operation of the list |
| `specs/keyboard-shortcuts.md` | Keyboard shortcuts (the pointer under Functions) · Keyboard shortcuts (the table of defaults) | A default binding is added, changed or looked up |
| `specs/architecture.md` | Architecture · Extension points · Module rules · Package layout (without Plugin host) · Build order · Architecture rules | A crate is added, a dependency changes or the task crosses layers: which crate owns what, and what is not re-decided |
| `specs/core-types.md` | Core types and extension traits | A value type, a job type, an extension trait, the registry, a `Command` or an `Event` is used or changed |
| `specs/engineering.md` | Engineering guidelines · Non-goals & open questions (the two intro paragraphs, Configuration, License, Language) | A coding rule is in doubt, or the task touches where settings live, the licence or translations |
| `specs/version-0.4.md` | Version 0.4 — scope | The issue is a 0.4 item: its place in the order and the sections that hold its rules |
| `specs/history.md` | Version 0.1 — definition of done · Version 0.2 — scope · Version 0.3 — design refresh, system integration and distribution | Reference: what an earlier version shipped and why |
| `specs/open.md` | Still open | Before designing something: it may be listed as not decided or not drawn yet |
| `specs/decisions.md` | The entries kept from the retired `QUESTIONS.md`, under their original numbers | Reference: look up one entry with grep, never load the file |
| `specs/ext-ssh.md` | Separate specs: `ext-ssh`, remote hosts over SSH | The SSH provider, its hosts in the Network section, its console actions, or the `REMOTE` capability |
| `specs/ext-installer.md` | Separate specs: `ext-installer`, the tarball installer | *Install as application* and the Installed Apps view (Linux only) |
| `specs/app-icons.md` | Separate specs: the app icon on all three platforms and the Linux desktop entry | The icon, the `rick` identifier, the `Rick.app` bundle or the Linux `share/` layout |
| `specs/about-dialog.md` | Separate specs: the About dialog | The About dialog, the build details or the third-party component list |
| `specs/system-integration.md` | Separate specs: system integration and distribution, the second track of 0.3 | Single instance, the command line, `rick://`, the system clipboard, the integration check, signing, plugin trust, the release channels |
| `specs/ext-zfs.md` | Separate specs: `ext-zfs`, OpenZFS snapshots | Version 0.4, Linux only; the file is not in the repository yet |

A citation `QUESTIONS.md N` anywhere in the repository means entry `QN` in `specs/decisions.md`.

## Separate specs

Some plugins are specified in their own file under `specs/`, so they can be handed to an implementer on their own. Each is binding in the same way as this document; where one disagrees with this document, the question goes into the pull request.
