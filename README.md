# Rick

**A god-like file manager.**

Rick is a lens into your files. Point it at a folder and you see what's what: what every file is, what's inside it, what changed, what's where. Then you move things, rename things, sort things out, without ever leaving the window. One tool, written in Rust, identical on Linux, macOS and Windows, and built from modules so it only carries what *you* need.

> Status: 0.3.0 is out, on all three platforms at once, as plain archives from the [Releases page](https://github.com/Open-Breach/rick/releases). It brought the design refresh — the whole window redrawn in Light and Dark and two densities, with workspace tabs in the title row and tabs and panes you drag, dock and reorder — one Rick per session with a command line and `rick://` links, the setup dialog that installs Rick into a Linux desktop, SVG and icon previews, and the About dialog. 0.2.0 brought the native plugin host and the split: every non-essential feature is now a file you can drop in or remove. Archives, disk images, the Trash, SSH hosts and S3, Azure Blob and Google Cloud Storage accounts are providers; the Office, media, archive, markup (Markdown, HTML, CSV), hex and 3D previewers, the search connectors, Subversion and the compress, compare, console and bulk-rename actions are plugins; on Linux the tarball installer turns an archive into an installed application. The spec is [SPEC.md](SPEC.md). Star the repo, watch it, and come along for the ride.

> **Use with caution.** Rick is written largely by an AI coding agent working from [SPEC.md](SPEC.md), with human review, a strict test gate and CI on all three platforms. That makes it fast to build and easy to audit, but it is still young software whose entire purpose is to move, rename and delete your files. Until 1.0, run it on data you have backed up, prefer *Move to trash* over *Delete permanently*, and treat every release as a preview. If it does something surprising, that's a bug: please report it.

## See what's what

Most file managers show you names in a column and leave the rest to a dozen other programs: an image viewer, a PDF reader, an archive tool, a Git client, something to mount an ISO. Rick's job is to make that pile of tools unnecessary.

Select a file and the preview pane shows it, whatever it is: the pane is Rick's only viewer. Press Enter to open it with the application the system has for it. No app launch to look at a file, no switching windows:

- **An image** opens scaled to fit, with 1:1, pan and zoom, EXIF and the camera data one click away.
- **A Git repository** shows its status on every entry: badges, branch, ahead and behind, folder roll-ups. Commit, stash, pull, push, checkout and blame are in the right-click menu, only when you're inside a repo. *Git clone…* is there on any folder: paste the URL, and the clone runs as a job you can watch and cancel.
- **A zip, 7z, tar or RAR** opens like a folder. Browse it, look at a file inside it, extract what you need, and never unpack the whole thing.
- **An ISO or a disk image** (raw, DMG, VHD, VHDX, VMDK, VDI, QCOW2) is browsable the same way, no mounting required; *Mount* in its menu hands it to the system where the system can mount it, and the mounted volume ejects from the side panel.
- **A PDF** renders page by page with a searchable text layer; Office documents go through the same path when LibreOffice is installed.
- **Text and code** come with syntax highlighting and pretty-print for JSON, XML and YAML, and a 500 MB log shows its first screen instantly because Rick never loads a file whole.
- **Audio and video** play right in the pane, with the stream details in the info bar.

A file no other previewer recognises opens in the hex view; without that plugin it shows its facts and whether a plugin for its type is there to switch on.

## Then put it in order

Seeing is half the job. Rick is also for the sorting-out:

- Tabs, split panes, docking and tear-out windows, so two folders side by side is one keystroke (and so is a grid of four).
- Details, list, tree and icon views; sort, group and filter per tab; type to filter the list as you go, or search a whole subtree while it streams in.
- Every copy, move, rename and delete runs as a job with progress, pause and cancel, and the risky ones are undoable. Conflicts ask once and remember your answer.
- Flatten a tree into one list, create links, bulk rename with a preview, compare two files or two folders, pin the places you keep coming back to.
- A command palette, a full rebindable keyboard map, and plain TOML settings you can read, edit and version.
- Settings for how you work: how dates are written (`date_format`: `smart-time`, `smart`, `iso` or `system`), whether a second click renames (`rename_on_click`, off unless you turn it on), and the editor that F4 and *Open in editor* start (`editor`, empty for the system's default text editor). All three are in the Settings window, under Appearance and Behaviour, and in `rick.toml`.

## One tool, your modules

The core of Rick is small: the window, the file list, the local file system and a plain text preview. Every capability above is a module that you enable, disable, install or leave on the shelf. Don't open PDFs? Don't load the PDF module. Never touch a repo? Git never loads. A photographer's Rick and a sysadmin's Rick are the same binary with different modules switched on, and neither pays for the other's features.

Modules are physically separate packages, loaded only when a matching file or action is first needed, and third-party modules use exactly the same five extension points as Rick's own features. If we can build it, so can you.

## Fast is a feature

The design targets every milestone is measured against: a 200,000-entry directory paints its first rows in about 100 ms, scrolling holds your display's refresh rate, a PDF or photo preview appears in about 150 ms, and an idle window uses no CPU. Listings stream in batches, previews decode off the UI thread, and the UI never blocks on I/O.

## Architecture in one breath

A small kernel with five extension points. Every feature plugs into them, and dependencies only point down the layers.

```
 ui-egui        the only crate that knows egui (eframe + egui_dock)
 app            command loop: Command in, Event out, undo log, settings
 model  ops     directory model; file operations as jobs
 ext            the five traits every feature implements
 vfs  runtime  platform   the only crates that touch the OS
 types          the data that crosses every boundary
```

| Extension point | Answers |
| --- | --- |
| **Provider** | Where files come from: local disk, archives, ISO, SFTP, object storage |
| **Decorator** | Status on entries: Git badges, sync state, extra columns |
| **Action** | Anything you can do to a selection or location |
| **Previewer** | How a file type is shown |
| **SearchIndex** | Querying Windows Search, Spotlight, Tracker or Baloo |

Built-in features register through the same traits as plugins, so the extension API is honest from day one. Sandboxed WASM plugins are on the roadmap, with native libraries for the heavy decoders.

The full story, from supported file systems to the trait signatures, lives in [SPEC.md](SPEC.md). It is the source of truth for this project.

## Try it

Release archives for Linux (x64 and arm64), macOS (Apple silicon) and Windows (x64) are on the [Releases page](https://github.com/Open-Breach/rick/releases). Each archive holds the `rick` binary, the shared libraries it needs beside it (Rick's kernel and the Rust standard library, which the plugins share with it), the pdfium library for PDF previews, a `plugins/` folder with the Git, SVN, archive, trash, SSH, S3, Azure Blob and Google Cloud Storage plugins (the 7-Zip library for disk images beside the archive plugin, with its licence), the Office, media, archive, markup (Markdown, HTML, CSV), hex, 3D and icon (ICO, CUR, ICNS, XPM) previewers, the subdirectory search and the platform's search-index connector (Windows Search, Spotlight, or Tracker and Baloo on Linux), the compress, compare, console and bulk-rename actions and the tarball installer (Linux only), the GStreamer runtime the media previewer plays through on macOS and Windows (on Linux it uses the system GStreamer: install `gstreamer1.0-plugins-good` and `gstreamer1.0-libav`, or your distribution's equivalents, for MP3 and H.264), the license, this README, the changelog and the third-party licenses; `SHA256SUMS.txt` has the checksums. Office previews need LibreOffice installed; without it the preview shows the document's metadata and says so. Unpack it anywhere and run it from the unpacked folder; on macOS the folder holds `Rick.app`:

```bash
# Linux
tar xzf rick-0.3.0-linux-x64.tar.gz && ./rick-0.3.0-linux-x64/rick
```

```bash
# macOS
ditto -x -k rick-0.3.0-macos-arm64.zip . && open rick-0.3.0-macos-arm64/Rick.app
```

```powershell
# Windows
Expand-Archive rick-0.3.0-windows-x64.zip -DestinationPath . ; .\rick-0.3.0-windows-x64\rick.exe
```

**Install.** From 0.3.0 on the archives also install, with no script. On Linux, unpack the archive anywhere and start `rick`: at its first start Rick checks what of the system it is not set up in yet and shows *Set up Rick on this system*, one row with a switch per item. *Install to the standard location* copies Rick with its plugins and `share/` to `~/.local/opt/rick/<version>` with a `current` link (through the tarball installer, the `ext-installer` plugin) and Rick continues from the installed copy; the *app launcher entry* puts the desktop entry and the icons into `~/.local/share`, so Rick appears in the app grid with its icon; *default file manager* makes Rick open folders (`inode/directory`) and `rick://` links in `mimeapps.list`; *“Show in folder” from other apps* installs the D-Bus activation file of `org.freedesktop.FileManager1`, which browsers, chat and download tools call, and the running Rick answers it; the *`rick` command* is a link in `~/.local/bin` plus a shell function for login shells; the *dock favourite* pins Rick to the GNOME dash, the Plasma task manager, the COSMIC dock or the favourites of the Cinnamon menu, as best it can. *Apply* sets up the rows that are on; a row you switch off is not asked about again, *Not now* asks again at the next start, and *Don’t ask again* silences the dialog. Nothing needs a password; setting Rick up for all users under `/opt` is not available yet. A file Rick did not write (a `rick.desktop` or a `rick` link from somewhere else) is never replaced unless you tick *Replace* on its row. Everything Rick writes is on record in `rick.toml`, and Settings → Integration shows each item as *Set up*, *Reset by system* (with *Restore*), *Skipped* (with *Add*) or *Unsupported*, checks again on request and resets your choices. By hand it still works as before: the archive carries the desktop entry and the icons in the XDG layout under `share/` (`share/applications/rick.desktop`, `share/icons/hicolor/<size>x<size>/apps/rick.png` and the scalable `rick.svg`), so copy `share/` into `~/.local/share/` and put `rick` on the PATH; or open the tarball in Rick and choose *Install as application*. On macOS the archive is `Rick.app` zipped: drag it to Applications. On Windows nothing changes: unpack and run `rick.exe`, which carries its icon and version.

**Command line.** One Rick runs per user session and config directory: a second `rick` hands its command line to the running one over a local socket (a named pipe on Windows) and exits with its status within a second, so a terminal, a launcher or a link always reaches the window you have open, and the window comes forward. `rick [<path>…]` opens each folder in a tab of the most recently focused window (the current directory without any); `rick reveal <file>…` opens the files' folders and selects the files; `rick --new-window [<path>…]` puts them in a window of their own; `rick --activate` is what the launcher entry runs, so a click on the launcher or on a dock favourite only brings the window forward and opens no tab; `rick --version` prints the build's details and `rick --list-plugins` the plugins it found and whether each loaded; `--config-dir <dir>` runs a Rick of its own on another config directory; `rick --dbus-service` is what the session bus runs for *“Show in folder”* when no Rick is up, and opens no folder of its own. Plugins add subcommands of their own, which `rick --help` lists beside the core ones: `rick install <archive>` opens the tarball installer's form on an archive (Linux). A `rick:///<path>` link from a document or a chat opens an existing folder or reveals an existing file, and says so in a toast otherwise; the desktop entry and `Info.plist` register the scheme. The embedded console follows the tab it was opened from and the tab follows the console's `cd` (the shell reports it through OSC 7 where it knows how: bash and zsh with the distribution's vte.sh, fish), and the tarball installer's `path.sh` defines a `rick` function for login shells that lack `~/.local/bin` on their PATH. Plugins live in the `plugins/` folder beside the binary and in a per-user one in the config directory (`~/.config/rick/plugins` on Linux, `~/Library/Application Support/Rick/plugins` on macOS, `%APPDATA%\Rick\plugins` on Windows); the Extensions screen in the burger menu lists them and switches each on or off for the next start. The archives are not signed or notarised yet: macOS may refuse an app downloaded through a browser until you allow it under System Settings → Privacy & Security, or clear the quarantine flag with `xattr -dr com.apple.quarantine rick-0.3.0-macos-arm64/Rick.app`; Windows SmartScreen may warn the first time. The Windows archives up to 0.3.0 need the Microsoft Visual C++ Redistributable (x64) installed and stop with “VCRUNTIME140.dll was not found” without it; from 0.4.0 the archive carries that runtime beside `rick.exe`. The Linux builds need glibc 2.39 or later (Ubuntu 24.04). Intel Macs are not covered yet.

0.1 shipped local files, tabs and panes, the core operations, previews for text, code, images and PDF, and the Git module. 0.2.0 adds the plugin host and moves everything non-essential into plugins: Git first, then the archive (zip, tar, 7z, RAR, ISO and UDF images, the other disk images through the 7-Zip library), Trash, SSH and cloud (S3, Azure Blob, Google Cloud Storage) providers, the Office, media, archive, markup, hex and 3D previewers, the search connectors, Subversion and the compress, compare, console and bulk-rename actions; the core gained permissions in Properties, localisation (German first), the native drag source, Compute hash and, on Linux, the tarball installer. 0.3.0 is the design refresh and the start of system integration: the GUI redrawn from tokens in Light and Dark, Comfortable and Compact, with workspace tabs in the title row and the drag of tabs and panes; a single instance with the command line and `rick://`; the integration check, the setup dialog and Settings → Integration with the Linux items; SVG and icon-container (ICO, CUR, ICNS, XPM) previews; the app icon, the desktop entry and `Rick.app`; and the About dialog with the third-party licences. Clipboard interoperability, the Windows and macOS integration items, notifications, signing and the packaged installers are not in it yet. The numbers 0.2.0 was measured against are in [BENCHMARKS.md](BENCHMARKS.md), the changes in [CHANGELOG.md](CHANGELOG.md).

To build from source instead:

```bash
git clone https://github.com/Open-Breach/rick.git
cd rick
cargo xtask bundle
./target/debug/bundle/rick
```

You need the stable Rust toolchain (`rust-toolchain.toml` pins it) and, on Linux, the usual Wayland/X11 development packages for an egui app. `cargo xtask bundle` builds the workspace and lays out the binary, the shared libraries it links and every plugin in `target/debug/bundle/` (`--release` for `target/release/bundle/`), which is also what the release archives contain; build and test the workspace as a whole (`cargo test --workspace`), since the kernel crates are shared libraries built once per feature set (QUESTIONS.md 81). PDF previews need pdfium beside the binary, or wherever the PDF previewer's `library` setting points (`preview-pdf.toml` in the config directory, which Rick fills in itself when it finds the library and which the preview pane's *Browse…* sets): the bundle copies the library from that setting when it is set, a release archive brings it along. Media playback is built only with `--features ext-preview-media/gstreamer` (`cargo test --workspace --features ext-preview-media/gstreamer`, `cargo xtask bundle --features ext-preview-media/gstreamer`), which needs the GStreamer development files (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Debian and Ubuntu, the official development package on macOS and Windows); without the feature the media previewer shows metadata only. `RICK_GSTREAMER_DIR` makes the bundle carry a GStreamer runtime on macOS and Windows (QUESTIONS.md 106). To write a plugin, start from `crates/plugins-native/README.md`.

## Join in

Rick is open source under [Apache-2.0](LICENSE) and we want company.

- **Try it and tell us.** Open an issue for anything that feels slow, wrong or missing. Performance complaints are especially welcome; they come with numbers.
- **Ask questions.** Ambiguities in the spec are raised as [issues](https://github.com/Open-Breach/rick/issues). Add yours.
- **Review the code.** Because so much of it is agent-written, a second pair of human eyes on any pull request is worth more here than in most projects. Read a crate, question a design, poke at an edge case.
- **Build something.** Pick a crate, read its one-paragraph README, and send a pull request. The engineering rules are short and in SPEC.md; CI checks them on Linux, macOS and Windows.
- **Write a module.** The five traits are small enough to fit on one screen each. A previewer for your favourite format is a great first contribution, and it ships as a module people can switch on.

Contributions are accepted under the same Apache-2.0 license.

---

*Named after a certain genius scientist who refuses to accept that anything is impossible. The file manager is ours; the name is a nod.*
