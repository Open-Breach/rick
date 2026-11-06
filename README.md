# Rick

**A god-like file manager.**

Rick is a lens into your files. Point it at a folder and you see what's what: what every file is, what's inside it, what changed, what's where. Then you move things, rename things, sort things out, without ever leaving the window. One tool, written in Rust, identical on Linux, macOS and Windows, and built from modules so it only carries what *you* need.

> Status: 0.1, the first release, ships on all three platforms at once as plain archives from the [Releases page](https://github.com/Open-Breach/rick/releases). Star the repo, watch it, and come along for the ride.

> **Use with caution.** Rick is written largely by an AI coding agent working from [SPEC.md](SPEC.md), with human review, a strict test gate and CI on all three platforms. That makes it fast to build and easy to audit, but it is still young software whose entire purpose is to move, rename and delete your files. Until 1.0, run it on data you have backed up, prefer *Move to trash* over *Delete permanently*, and treat every release as a preview. If it does something surprising, that's a bug: please report it.

## See what's what

Most file managers show you names in a column and leave the rest to a dozen other programs: an image viewer, a PDF reader, an archive tool, a Git client, something to mount an ISO. Rick's job is to make that pile of tools unnecessary.

Select a file and the preview pane shows it. Press Enter and the viewer opens it full size. No app launch, no switching windows:

- **An image** opens scaled to fit, with 1:1, pan and zoom, EXIF and the camera data one click away.
- **A Git repository** shows its status on every entry: badges, branch, ahead and behind, folder roll-ups. Commit, stash, pull, push, checkout and blame are in the right-click menu, only when you're inside a repo.
- **A zip, 7z, tar or RAR** opens like a folder. Browse it, look at a file inside it, extract what you need, and never unpack the whole thing.
- **An ISO** is browsable the same way, no mounting required.
- **A PDF** renders page by page with a searchable text layer; Office documents go through the same path when LibreOffice is installed.
- **Text and code** come with syntax highlighting and pretty-print for JSON, XML and YAML, and a 500 MB log shows its first screen instantly because Rick never loads a file whole.
- **Audio and video** play right in the pane, with the stream details in the info bar.

Everything that doesn't fit a previewer falls back to a hex view, so nothing is a black box.

## Then put it in order

Seeing is half the job. Rick is also for the sorting-out:

- Tabs, split panes, docking and tear-out windows, so two folders side by side is one keystroke (and so is a grid of four).
- Details, list, tree and icon views; sort, group and filter per tab; type to filter the list as you go, or search a whole subtree while it streams in.
- Every copy, move, rename and delete runs as a job with progress, pause and cancel, and the risky ones are undoable. Conflicts ask once and remember your answer.
- Flatten a tree into one list, create links, bulk rename with a preview, compare two files or two folders, pin the places you keep coming back to.
- A command palette, a full rebindable keyboard map, and plain TOML settings you can read, edit and version.

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

Release archives for Linux (x64 and arm64), macOS (Apple silicon) and Windows (x64) are on the [Releases page](https://github.com/Open-Breach/rick/releases). Each archive holds the `rick` binary, the pdfium library it needs for PDF previews beside it, the license, this README, the changelog and the third-party licenses; `SHA256SUMS.txt` has the checksums. Unpack it anywhere and run it from the unpacked folder:

```bash
# Linux
tar xzf rick-0.1.0-linux-x64.tar.gz && ./rick-0.1.0-linux-x64/rick
```

```bash
# macOS
tar xzf rick-0.1.0-macos-arm64.tar.gz && ./rick-0.1.0-macos-arm64/rick
```

```powershell
# Windows
Expand-Archive rick-0.1.0-windows-x64.zip -DestinationPath . ; .\rick-0.1.0-windows-x64\rick.exe
```

`rick --version` prints the version and the git hash it was built from; `rick <folder>` starts in that folder. The archives are not signed or notarised yet (0.2 at the earliest): macOS may refuse a binary downloaded through a browser until you allow it under System Settings → Privacy & Security, or clear the quarantine flag with `xattr -dr com.apple.quarantine rick-0.1.0-macos-arm64`; Windows SmartScreen may warn the first time. The Linux builds need glibc 2.39 or later (Ubuntu 24.04). Intel Macs are not covered yet.

0.1 ships local files, tabs and panes, the core operations, previews for text, code, images and PDF, and the Git module. Archives, ISO, media and Office follow as modules in 0.2. The numbers the release was measured against are in [BENCHMARKS.md](BENCHMARKS.md), the changes in [CHANGELOG.md](CHANGELOG.md).

To build from source instead:

```bash
git clone https://github.com/Open-Breach/rick.git
cd rick
cargo run
```

You need the stable Rust toolchain (`rust-toolchain.toml` pins it) and, on Linux, the usual Wayland/X11 development packages for an egui app. PDF previews need pdfium beside the binary or in `RICK_PDFIUM_DIR`: a development build finds it there, a release archive brings it along.

## Join in

Rick is open source under [Apache-2.0](LICENSE) and we want company.

- **Try it and tell us.** Open an issue for anything that feels slow, wrong or missing. Performance complaints are especially welcome; they come with numbers.
- **Ask questions.** Ambiguities in the spec are tracked in [QUESTIONS.md](QUESTIONS.md). Add yours.
- **Review the code.** Because so much of it is agent-written, a second pair of human eyes on any pull request is worth more here than in most projects. Read a crate, question a design, poke at an edge case.
- **Build something.** Pick a crate, read its one-paragraph README, and send a pull request. The engineering rules are short and in SPEC.md; CI checks them on Linux, macOS and Windows.
- **Write a module.** The five traits are small enough to fit on one screen each. A previewer for your favourite format is a great first contribution, and it ships as a module people can switch on.

Contributions are accepted under the same Apache-2.0 license.

---

*Named after a certain genius scientist who refuses to accept that anything is impossible. The file manager is ours; the name is a nod.*
