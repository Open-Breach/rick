# Architecture in one breath

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
| **Provider** | Where files come from: local disk, archives, disk images, SSH hosts, object storage |
| **Decorator** | Status on entries: Git badges, sync state, extra columns |
| **Action** | Anything you can do to a selection or location |
| **Previewer** | How a file type is shown |
| **SearchIndex** | Querying Windows Search, Spotlight, Tracker or Baloo |

Built-in features register through the same traits as plugins, so the extension API is honest from day one.

## One tool, your modules

The core of Rick is small: the window, the file list, the local file system and the previews for text, code, images and PDF. Every other capability is a module that you enable, disable, install or leave on the shelf. Never touch a repo? Git never loads. A photographer's Rick and a sysadmin's Rick are the same binary with different modules switched on, and neither pays for the other's features.

Modules are physically separate packages, loaded from the `plugins/` folder, and third-party modules use exactly the same five extension points as Rick's own features. If we can build it, so can you. To write one, start from `crates/plugins-native/README.md`.

## Fast is a feature

The design targets every milestone is measured against: a 200,000-entry directory paints its first rows in about 100 ms, scrolling holds your display's refresh rate, a PDF or photo preview appears in about 150 ms, and an idle window uses no CPU. Listings stream in batches, previews decode off the UI thread, and the UI never blocks on I/O. The measured numbers are in [BENCHMARKS.md](../BENCHMARKS.md).

The full story, from supported file systems to the trait signatures, lives in [SPEC.md](../SPEC.md) and the files of `specs/`. They are the source of truth for this project.
