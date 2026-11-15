# Rick — Tarball Installer Plugin (ext-installer)

## Purpose

Many Linux applications and developer toolchains are distributed only as archives (`.tar.gz`, `.tar.xz`, `.tar.bz2`, `.tar.zst`, `.zip`). Installing them by hand means extracting somewhere, finding the right binary, writing a desktop entry, and remembering all of it at update time. This plugin turns "Install as application" into a single action inside Rick: the archive is extracted to a standard location, the app appears in the application launcher with its icon, its command is on the PATH, and updates, rollback and uninstall are clean.

Target user: someone who normally installs via the package manager or Flatpak and only occasionally meets a tarball. Prior art exists only as small standalone scripts and tools; nothing is integrated into a file manager.

## Decisions

- One plugin crate, `ext-installer`. Linux only in this version; macOS and Windows are out of scope.
- Handles both GUI applications and command-line toolchains.
- Install scope is chosen per install: **user space (recommended, default)** or **system-wide**.
- Versioned on-disk layout with a `current` symlink; launcher entry and command symlinks only ever point through `current`.
- The install record lives inside the app's own folder, not in a central manifest. The install folder is the database.
- Detection only produces defaults. The install dialog always shows them, editable, before anything is written.
- Source tarballs are refused with a clear message.
- Rick never checks for new versions itself; it has no knowledge of where an archive came from.
- Does its own extraction with the tar and zip crates: no dependency on the archives plugin and no ordering constraint. Ships with 0.2 (Linux only).

## Scope

**In:** prebuilt GUI apps and CLI toolchains in tar (gz, xz, bz2, zst) and zip archives; AppImages, run as they are or installed (see *AppImages*); user-space and system-wide installs; desktop entry, icon and PATH integration; update with one previous version kept; revert; uninstall; repair of broken state; an Installed Apps view.

**Out (this version):** `.deb`/`.rpm`, source builds, automatic update checks, AppImage self-update (zsync, AppImageUpdate) and signature verification, browsing into an AppImage like an archive, adopting apps installed manually before Rick, macOS `.app` bundles and Windows portable zips (candidates for a later version using the same model).

## Detection

Detection reads the archive index and, where needed, the headers of candidate files. It answers five questions and fills the dialog with its best guess for each.

1. **Source or prebuilt?** The presence of `configure`, `Makefile`, `CMakeLists.txt`, `Cargo.toml`, `meson.build`, `setup.py` or similar together with no compiled executables means source. Refuse: "This archive contains source code and needs to be compiled; Rick can't install it."
2. **Main executable.** Best signal: a `.desktop` file shipped in the archive — its `Exec` line names the binary. Otherwise: files whose first bytes are the ELF magic (never trust permission bits alone), excluding shared libraries (`.so*`); prefer a binary at the top level or in `bin/` whose name matches the archive or top-level folder. A name matches when its stem is a word of the app name or a suffix of the app id (`rider` matches `JetBrains Rider` and `jetbrains-rider`). When several candidates remain, a matching binary wins; a helper never wins by alphabetical order. If several candidates remain, the dialog shows the list and the user picks.
3. **Kind: GUI or CLI.** A `.desktop` file or icon files mean GUI. Otherwise read the main binary's dynamic dependencies: links against X11, Wayland, GTK, Qt or similar mean GUI. A `bin/` folder with several binaries and no icons means toolchain.
4. **Icon.** From the `.desktop` file's `Icon` line if present; else a PNG or SVG named like the app, with the same name rule as the executable; else the largest raster icon under an `icons/`, `resources/` or `share/` folder; else none. Every PNG and SVG in the archive stays available as a choice; the rule only decides which one is preselected.
5. **Name and version.** From the `.desktop` `Name`, the archive filename (version digits parsed with a conservative pattern), or the top-level folder name. The archive filename with version digits masked is kept as the *source pattern* for later update matching.

**Bonus check:** the ELF machine type of the main executable is compared with the running machine. A mismatch (e.g. an aarch64 build on x86\_64) is flagged before anything is extracted.

**Tarbombs:** if the archive has no single top-level folder, its contents go directly into the version folder.

## Install locations

The **app id** is a lowercase, hyphenated slug derived from the display name (editable in the dialog).

|  | User space | System-wide |
| --- | --- | --- |
| App files | `~/.local/opt/<app-id>/<version>/` | `/opt/<app-id>/<version>/` |
| Active version | `~/.local/opt/<app-id>/current` → version folder | `/opt/<app-id>/current` |
| Install record | `~/.local/opt/<app-id>/rick-install.toml` | `/opt/<app-id>/rick-install.toml` |
| Command symlinks | `~/.local/bin/` | `/usr/local/bin/` |
| Desktop entry | `~/.local/share/applications/<app-id>.desktop` | `/usr/share/applications/<app-id>.desktop` |
| Icon | `~/.local/share/icons/hicolor/<size>/apps/<app-id>.<ext>` | `/usr/share/icons/hicolor/...` |

System-wide installs run the write steps through a small privileged helper invoked via `pkexec`, so the user sees the standard polkit password prompt and Rick itself never runs as root. The helper is a separate milestone and may ship after the user-space path.

Before extraction, free space at the destination is checked against the archive's uncompressed size.

**Detection is a job.** Install as application returns at once. A job *Reading archive…* with byte progress over the compressed stream and a cancel button reads the archive in a single streaming pass, collecting the entry index and the first bytes of candidate files together. The dialog opens when the job finishes; cancelling leaves nothing behind. A gzip tar of several gigabytes must not freeze the window.

## Install dialog

Opened by the Action **Install as application** on an archive (context menu and command palette). Fields, all prefilled by detection and all editable:

- Name, app id, version
- Kind: GUI application / command-line tool
- Main executable (dropdown of candidates)
- Icon: one list, `(none)` then every PNG and SVG in the archive with path and pixel size, best first, the best one preselected when detection trusts it
- Category (short list; shown for GUI only)
- Commands to link on the PATH (GUI: one, named after the app id; toolchain: the `bin/` contents, all ticked by default). For toolchains the alternative is "add the bin folder to PATH" (see PATH mode under Desktop integration)
- Scope: user space (recommended) / system-wide

Switching the kind recomputes the main executable, icon and command defaults. For a GUI application the default is one command named after the app id; for a toolchain the `bin/` contents.

**Modes:** *New install* when nothing matches. *Update* when the archive matches an installed app (see Updates), showing "\<Name> \<old> is installed — update to \<new>?". If the detected version is lower than the installed one, the dialog says so and asks to confirm a **downgrade**. If matching fails but the user knows it is the same app, a dropdown "This is an update of…" lists installed apps.

The dialog ends with a plain-language summary of what will be written and where. Nothing is written before the user confirms.

**Collisions.** If a command symlink name or a desktop entry name already exists and was not created by Rick (not listed in any install record), the dialog says so and asks before overwriting. Rick never overwrites silently.

**Checksum.** An optional "expected SHA-256" field accepts the hash vendors publish next to the download. When filled, Rick hashes the archive before extracting and refuses on a mismatch, naming both values.

**Other ways in.** Dropping an archive onto the Installed Apps view opens the same dialog. With Rick's command line (0.3), `rick install <archive>` opens it too, so installs can be scripted later.

## Desktop integration

**Desktop entry.** If the archive ships a `.desktop` file, reuse it: keep the vendor's name, comment, categories, MIME types and `StartupWMClass`; rewrite `Exec` and `Icon` to go through `current`. Otherwise generate a minimal entry: `Name`, `Exec` (quoted absolute path through `current`), `Icon=<app-id>`, `Type=Application`, `Terminal=false`, the chosen `Categories`. `StartupWMClass` is editable afterwards in the Installed Apps view, since its real value is often only known after the app has run once. The desktop file is written once and left untouched by updates; `update-desktop-database` is called when it is created or its content changes.

**Icon.** Copied into the user's (or system's) hicolor theme under the matching size, or `scalable/apps` for SVG, and referenced **by name**, so launcher, dock and window switcher all resolve it consistently.

**Commands.** Symlinks in `~/.local/bin` (or `/usr/local/bin`) pointing through `current`. If `~/.local/bin` is not on the PATH, the dialog shows a one-line hint; Rick does not edit shell profiles.

**PATH mode (command-line tools only).** Instead of symlinks, the toolchain's `bin/` folder (through `current`) is put on the PATH. This is the better choice for toolchains with many binaries, for tools that locate their sibling files from their own path without resolving symlinks, and for anything that also wants a home variable such as `JAVA_HOME`. Rick never scatters edits across dotfiles: it maintains one file, `~/.config/rick/path.sh`, regenerated from the install records whenever a PATH-mode toolchain is installed, updated or removed, and adds a single line sourcing it, once, to `~/.profile` (and `~/.zprofile` if present); fish gets a drop-in file in its `conf.d` instead. An optional environment variable pointing at the version folder (for example `JAVA_HOME`) is offered in the dialog and lives in the same managed file. The record stores `path_mode` (`symlinks` or `path`), `bin_dir` and any variable. The change applies to new shells only, and the dialog says so. The sourcing line is removed only when the last PATH-mode toolchain is uninstalled, after asking.

**Avoiding the vanishing-launcher problem.** GNOME hides a launcher entry whose `Exec` target does not exist. Three rules prevent it: the desktop entry points only at `current`, never at a version folder; the `current` symlink is replaced atomically (create under a temporary name, rename over the old one); the desktop file is never deleted and recreated during an update.

**Library check.** After extraction, the dynamic dependencies of the main executable and of every linked command are resolved against the app's own folder (rpath, `lib/`) and the system's dynamic loader cache. Unresolved libraries are listed in the install summary by file name ("this app needs libxcb-cursor.so.0, which is not installed"). Looking up the matching package name through the distro's package manager is a later nicety.

## Bookkeeping: the install record

`rick-install.toml` in the app folder is the single source of truth. Rick scans `~/.local/opt/*/` and `/opt/*/` for these files; there is no central index to drift. The record survives Rick being uninstalled and reinstalled, and is readable without Rick.

```toml
id = "postman"
name = "Postman"
kind = "gui"            # gui | cli
scope = "user"          # user | system
version = "11.3.0"
previous_version = "11.2.0"
installed_at = 2026-11-07T18:42:00Z
source_archive = "Postman-linux-x64-11.3.0.tar.gz"
source_pattern = "Postman-linux-x64-*.tar.gz"
main_executable = "Postman"            # relative to the version folder
startup_wm_class = "Postman"

# Every file Rick created outside the app folder. Uninstall removes exactly these.
[[external_files]]
path = "~/.local/share/applications/postman.desktop"
kind = "desktop-entry"

[[external_files]]
path = "~/.local/share/icons/hicolor/512x512/apps/postman.png"
kind = "icon"

[[external_files]]
path = "~/.local/bin/postman"
kind = "symlink"
```

## Updates and rollback

**Matching.** A new archive is matched against installed records in order: app id derived from the detected name; `source_pattern` against the archive filename; `main_executable` path inside the archive. Any hit switches the dialog to update mode.

**Update.** Extract into a new version folder next to the current one; verify the main executable exists; flip `current` atomically; rewrite the record (`version`, `previous_version`, `installed_at`, `source_archive`). The desktop entry, icon and symlinks are not touched unless the main executable's relative path changed, in which case the symlinks are updated. Any version older than `previous_version` is deleted — one previous version is kept.

**Revert.** Flip `current` back to `previous_version` and swap the two version fields in the record. Available from the Installed Apps view.

**User data** is never touched: well-behaved apps keep settings in `~/.config` and `~/.local/share`, outside the install folder.

**Running app.** Before flipping `current`, Rick checks whether any process is running an executable from the current version folder (via `/proc/*/exe`). If so, it asks the user to close the app first; the staging folder is kept, so the update resumes without re-extracting.

## Uninstall

Delete every path listed in `external_files`, then the app folder including all version folders and the record. Then `update-desktop-database`. The dialog lists exactly what will be removed. Nothing outside that list is ever deleted.

The dialog also states that the app's own settings and data, for instance under `~/.config` and `~/.local/share`, are left untouched, so nobody expects a scorched-earth removal.

## Installed Apps view

A virtual view provided by the plugin, opened from the command palette and the app menu (the burger menu lists it under *Installer*; a side-panel entry can follow the side-panel rework). One row per record: icon, name, version, kind, scope, install date, and state. Actions per row: Open folder, Launch, Update from archive…, Revert to previous version, Edit launcher (name, icon, category, StartupWMClass), Uninstall.

**Messy states** are shown, not errored: *version folder missing*, *desktop entry missing*, *symlink broken*, *record found but current symlink dangling*. Each offers **Repair** (rewrite what is missing from the record) or **Forget** (delete the record only).

**Launch** runs a GUI app through its desktop entry (`gio launch` or equivalent) rather than executing the binary directly, so environment, startup notification and window matching behave exactly as from the app menu. For command-line tools Launch is not shown; the row lists the command names instead.

## AppImages

An AppImage is an application in one file: a small ELF program, the *runtime*, with a file-system image behind it that holds the app; started, the runtime mounts the image and runs what is inside. Rick runs such a file as it is, and installs it on the archive model, followed exactly: the install locations, the `current` symlink, the install record, the dialog with its modes, collisions and checksum, the job, *Failure handling*, update with one previous version kept, revert, uninstall and the Installed Apps view are the ones of this document. This section holds only what differs. Linux only, like the rest of the plugin.

**Detection.** A file is an AppImage when its name ends in `.AppImage`, in any case, or its ELF header carries the AppImage magic: the bytes `AI` and then `0x01` (type 1) or `0x02` (type 2) at offset 8. An AppImage never goes through the archive detection of *Detection*.

**Actions.** An AppImage on the local file system has two entries in its context menu, and so in the command palette, side by side where an archive has *Install as application…*: **Run** and **Install as application…**. Neither appears on a remote location or on a file that is not an AppImage.

**Run** starts the AppImage as it is; nothing is installed. In this order:

1. *Architecture.* Where the file's ELF machine type is not the running machine's, Rick says so and stops, as the *Bonus check* does.
2. *Execute bit.* A downloaded AppImage usually has none. It is set for the owner when it is missing; nothing else about the file changes.
3. *FUSE check*, below. Where FUSE 2 is needed and missing, Rick stops with the warning.
4. The file is started detached from Rick, with its folder as the working directory, the way *Opening files* starts a binary.

Enter and a double-click are not *Run*. They follow *Opening files* (specs/gui-windows.md), which decides by the execute bit: with it an AppImage starts like any binary, without the FUSE check; without it the file is handed to the system.

**Install.** *Install as application…* on an AppImage opens the install dialog in AppImage mode: new install or update, the scope, the collision rules and the checksum are those of an archive. Dropping an AppImage onto the Installed Apps view and `rick install <file>` open it too. What differs:

- *Nothing is run.* *Archive safety* holds for an AppImage as for an archive, so the file is never asked to unpack itself (`--appimage-extract`): the desktop entry, the icon and the version are read out of it. An AppImage of type 2 is its runtime followed by a SquashFS image that starts where the ELF ends. Rick reads from that image the `.desktop` file at its top, the icon — `.DirIcon`, or the one the entry names — and the AppStream metainfo, following links only inside the image. The reading is the detection job (*Detection is a job*), here *Reading AppImage…*.
- *An image that cannot be read* — an AppImage of type 1, a compression the reader does not have, a damaged image — is not an error. The dialog opens with a minimal entry from the file name and the generic icon, as for an archive without one, and the version from the file name.
- *Version.* Tried in this order, the first hit fills the field: `X-AppImage-Version` in the bundled desktop entry; the newest release in the AppStream metainfo (`usr/share/metainfo/*.xml`); the file name, on the usual `Name-<version>-<arch>.AppImage` pattern. With none of them the field stays empty and *Install* waits until the user has typed a version. Rick generates none and uses no date.
- *The file.* The version folder `<app-id>/<version>/` holds the AppImage itself under the fixed name `<app-id>.AppImage`, with the execute bit set, and `current` points at the version folder as usual. The fixed name keeps the path through `current` the same after every update, whatever the downloaded files were called, so the desktop entry is never rewritten (*Avoiding the vanishing-launcher problem*). The copy is the job, with progress and cancel, through the staging folder of *Failure handling*. The downloaded file is left where it was and as it was: the same bytes and the same permissions.
- *Desktop entry and icon.* The entry read from the image is reused as a vendor's is (*Desktop integration*): `Exec` goes through `current` and keeps its arguments (`%U`, `%F`), `TryExec` is dropped, and `Icon` is the installed icon's name. Entry, icon and install record are written exactly as for an archive, per user and system-wide; in the record `main_executable` is `<app-id>.AppImage` and `source_archive` the downloaded file's name.
- *Architecture.* The *Bonus check* and the verification of *Failure handling* read the AppImage's own ELF header. The AppImage is the main executable, and the dialog has no list to pick one from.
- *Commands.* No command symlink and no PATH mode by default; the dialog offers one command name, not ticked, as it offers them for a toolchain.
- *Update, revert and uninstall* are unchanged: an AppImage with the id of an installed app is offered as an update, one previous version is kept, and uninstall removes exactly what the record lists.

**FUSE check.** An AppImage on the older runtime needs `libfuse.so.2` to mount its image, and newer distributions no longer ship that library by default; one on the newer, static runtime does not need it.

- Whether a file needs it is read from its runtime, the ELF at the head of the file, without running it. The older runtime loads `libfuse.so.2` by name, so the name is in it, among its needed libraries or as the string it opens the library by; the static runtime carries no such name. An AppImage that does not need FUSE 2 is never held back.
- Where it is needed, the library is looked up in the dynamic loader cache as the *Library check* does — the plugin reads `/etc/ld.so.cache` — and not by calling `ldconfig`.
- Needed and missing is a warning, and nothing automatic: *FUSE 2 is required to run this AppImage*, the install line of each distribution — Debian and Ubuntu `sudo apt install libfuse2`, `libfuse2t64` on Ubuntu 24.04 and later; Fedora `sudo dnf install fuse-libs`; Arch `sudo pacman -S fuse2` — and a link to the AppImage documentation on FUSE, <https://docs.appimage.org/user-guide/troubleshooting/fuse.html>.
- *Run* stops with the warning. *Install* works without FUSE 2 and carries the same warning in its summary, where the *Library check* lists a missing library, so nobody installs an app that then does not start without being told why.

## Plugin architecture

- Crate `ext-installer`, loadable plugin file, removable like any other plugin.
- Contributes the **Actions** "Install as application", on archive files and on AppImages, and "Run", on AppImages (see *AppImages*), and a **Provider** (the Installed Apps virtual view). A **Decorator** badge on archives that are already installed is optional garnish for later.
- Extracts archives itself with the tar and zip crates, so it does not depend on the archives plugin; permissions and symlinks inside the archive are preserved. It reads an AppImage's image itself as well, and *Run* starts the file through the platform services the host lends, so the plugin stays free of `cfg`. The only kernel support it needs is generic: an Action opening a plugin-drawn dialog, and a plugin submitting a job with progress to the operations area. Bulk rename and compress need the same two hooks.
- Declares in its manifest that it writes outside the browsed location (`~/.local/opt`, `~/.local/share`, `~/.local/bin`) and may invoke `pkexec`; a good first real test of the declarative permission model.
- All extraction runs as a job in the operations area with progress, on the per-volume queue.

## Failure handling

Every install and update is atomic from the user's point of view: either the app is fully installed, or nothing changed.

1. Extract into a staging folder beside the destination (`<app-id>/.staging-<version>`), on the same filesystem, so the final step is a rename.
2. Verify: the main executable exists, is ELF, and matches the running architecture.
3. Rename the staging folder to the version folder.
4. Write (or rewrite) `rick-install.toml`.
5. Flip `current` atomically (temporary name, then rename over the old link).
6. Write the external files: icon, desktop entry, command symlinks; then `update-desktop-database`.

Any failure before step 3 deletes the staging folder and reports the cause; nothing else was touched. A failure after step 3 keeps the version folder, records what succeeded, and shows the app in the Installed Apps view as *needs repair*; **Repair** retries the missing steps. Cancelling from the operations area takes the before-step-3 path.

Disk full, permission denied and a corrupt archive each get their own message. Rick never deletes the source archive. If Rick itself dies mid-install, at most a `.staging-*` folder remains; the Installed Apps scan offers to remove orphaned staging folders.

## Archive safety

The archive is untrusted input. Nothing from it is ever executed during install, and nothing it contains can write outside its own version folder.

- Entry paths are normalised; entries that are absolute, contain `..`, or resolve outside the version folder are rejected.
- Symlinks and hard links are kept only when their target resolves inside the version folder; others are skipped and listed in the install summary.
- Device nodes, FIFOs and other special entries are skipped.
- Set-uid and set-gid bits are stripped; other permission bits are preserved. The execute bit is then set on the main executable, on every linked command and on shebang scripts in bin/, whether or not the archive carried it: zip archives and tarballs packed on other systems often lose it, which is why a freshly extracted app sometimes refuses to start.
- Duplicate entry names (an archive that writes the same path twice) are rejected.
- Uncompressed size is checked against free space before extraction and tracked as it proceeds; an entry-count cap guards against pathological archives.
- No post-install scripts. An `install.sh` or similar is never run; if one is present, the summary says so, and the user may run it by hand.
- Detection reads headers and text only (ELF magic, dynamic section, `.desktop` content). It never runs a candidate binary, for instance to ask it for its version.

**An AppImage** is untrusted input in the same way, and all of this holds for it. Nothing from it is run during detection or install: asking the file to unpack itself (`--appimage-extract`) would be running it, so its desktop entry, icon and version are read out of its image as data, and a link in the image is followed only when its target is inside the image. The file is started only when the user asks for exactly that, with *Run* or as the installed app.

## Privileged helper (system-wide installs)

System-wide installs go through `rick-install-helper`, a small separate binary invoked via `pkexec` under the polkit action `org.openbreach.rick.install`. Rick itself never runs as root, and the helper never parses an archive.

**Division of work.** Rick does detection, extraction, verification and sanitising unprivileged, into a staging folder under the user's home. The helper only moves an already-sanitised tree into place and writes the system-level files.

**Input.** One plan file (TOML) passed by path, holding the operation (`install`, `flip-current`, `revert`, `repair`, `uninstall`), the app id, the version, the staging folder, and the external files to write: desktop entry content, icon source path and target size, symlink names and targets.

**Validation inside the helper**, before touching anything:

- app id matches `[a-z0-9][a-z0-9-]*`; destination is exactly `/opt/<app-id>/<version>`
- symlinks only in `/usr/local/bin`, desktop entries only in `/usr/share/applications`, icons only under `/usr/share/icons/hicolor`, all named `<app-id>` or `<app-id>-<cmd>`
- the staging tree is re-checked for path escapes, outside-pointing links and special entries (same rules as Archive safety)
- for `uninstall` and `revert`, it reads `/opt/<app-id>/rick-install.toml` and deletes or flips only what that record lists

**Output.** Exit code plus a structured result on stdout (what was written, what was skipped, the error if any); a line in the system journal per operation. Installed files are owned by root, directories 755, files 644, executables keep their x bit.

The helper is its own milestone and may ship after the user-space path. Until it exists, the Scope choice in the dialog shows system-wide as unavailable with a short note.

## Test fixtures

The plugin crate carries a `fixtures/` folder of small archives, generated by a script at build time so no binaries are committed. Each exercises one detection or safety rule.

| Fixture | What it exercises | Expected result |
| --- | --- | --- |
| `gui-with-desktop.tar.gz` | vendor `.desktop`, PNG icon, single top-level folder | GUI; executable from `Exec`; icon from `Icon`; `StartupWMClass` kept |
| `gui-no-desktop.tar.xz` | one ELF linking GTK, `resources/icon.svg` | GUI via dependencies; generated `.desktop`; SVG into `scalable/apps` |
| `toolchain.tar.gz` | `bin/` with three ELF files, no icons | CLI; three symlinks offered, all ticked |
| `tarbomb.zip` | no top-level folder | contents land directly in the version folder |
| `source.tar.gz` | `Cargo.toml` + `src/`, no ELF | refused with the source-code message |
| `wrong-arch.tar.gz` | ELF for the other architecture | refused with the architecture message |
| `ambiguous.tar.gz` | two top-level ELF candidates | candidate list shown in the dialog |
| `hostile.tar` | `..` entry, absolute path, symlink to `/etc`, set-uid bit, `install.sh` | entries rejected or skipped per Archive safety; nothing executed |
| `update-v2.tar.gz` | same app id, higher version | update mode; `current` flipped; previous kept |
| `update-v1.tar.gz` | same app id, lower version | downgrade warning |

The AppImage fixtures are generated the same way and never committed: the tiny ELF program of the other fixtures with the AppImage magic in its header and, for type 2, a small SquashFS image behind it. The first one's app is the one `update-v2.AppImage` updates.

| Fixture | What it exercises | Expected result |
| --- | --- | --- |
| `entry.AppImage` | type 2 without the execute bit; desktop entry with `X-AppImage-Version`, `.DirIcon` linking to a PNG in the image, metainfo naming another version | AppImage mode; name, icon and version from the desktop entry; nothing run; the file left as it was |
| `metainfo.AppImage` | entry without a version, metainfo with two releases | version of the newer release |
| `Fixname-3.2.1-x86_64.AppImage` | entry without a version, no metainfo | version `3.2.1` from the file name |
| `no-version.AppImage` | none of the three version sources | version field empty; *Install* waits for one |
| `type1.AppImage` | magic `AI` `0x01`, no SquashFS image | minimal entry from the file name; generic icon |
| `magic-only` | the magic under a name without the extension | an AppImage all the same: *Run* and *Install as application…* offered |
| `fuse2.AppImage` | a runtime that names `libfuse.so.2`; every other fixture's runtime does not | FUSE 2 needed: where the loader cache lacks it *Run* stops with the warning and the install summary carries it; the others are never held back |
| `wrong-arch.AppImage` | ELF for the other architecture | *Run* and install refused with the architecture message |
| `outside-links.AppImage` | `.DirIcon` and the entry's icon are links that point out of the image | links not followed; generic icon |
| `update-v2.AppImage` | the app of `entry.AppImage`, a higher version, another file name | update mode; `current` flipped; previous kept; desktop entry untouched |

In addition: unit tests for the version parser and the app-id slug; an integration test that runs install, update, revert and uninstall against a temporary `HOME` and asserts the filesystem is empty afterwards, once from the archives and once from the AppImages, where it also asserts that the desktop entry is the same after the update and that the downloaded file kept its bytes and its permissions; and `desktop-file-validate` on every generated desktop entry in CI.

## Settled questions

All open questions were settled on 2026-11-07.

- **Target version:** ships with 0.2 (Linux only).
- **Zip support:** yes, from the first version.
- **Previous versions kept:** exactly one, not configurable.
- **`StartupWMClass` auto-detection:** no; editable in the Installed Apps view is enough.
- **Adopting apps installed by hand before Rick:** deferred to a later version.

AppImage support was settled on 2026-11-13 and 2026-11-14.

- **FUSE 2 missing:** a warning only, with the install line of each distribution. No fallback that unpacks and runs (`--appimage-extract-and-run`), and nothing is installed for the user.
- **Version of an AppImage:** the desktop entry, then the metainfo, then the file name. No generated fallback and no date: with none of the three the user types one.
- **Model:** the archive model, followed exactly — version folder, `current`, record, one previous version, the same dialog — with the AppImage kept as the one file it is, never unpacked.
