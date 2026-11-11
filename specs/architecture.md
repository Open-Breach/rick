# Architecture

Related: specs/core-types.md, specs/plugins.md, specs/engineering.md, specs/scope.md, specs/ext-ssh.md, specs/gui-dialogues.md

Rick is a small kernel with five extension points; every feature, built in or third party, plugs into them, and dependencies only point down the layers.

&#91;embedded content: Rick architecture · 6 layers, 4 extension points\]

`types` holds the data that crosses boundaries; `vfs`, `runtime` and `platform` are the only crates that touch the OS; `ext` is the one API an extension sees; `app` wires everything together; `ui-egui` is the only crate that knows egui.

## Extension points

All five live in `ext`, each small enough for one screen, each returning data or jobs and never drawing.

| Point | Answers | Contract |
| --- | --- | --- |
| Provider | Where files come from: local, archive, SSH, object storage | list streamed in chunks, stat, open, watch, copy, delete, plus a capability set (trash, rename, reflink, watch) |
| Decorator | Status on entries or a directory: git badges, sync state, extra columns | interested(dir) as a cheap gate; decorate(visible entries or directory) as a cancellable batch job; invalidates\_on(dir) names the file events that make it stale |
| Action | A function for a selection or location | id, label, enabled(ctx), run(ctx) as a job, placement (menu group, palette, shortcut) |
| Previewer | Rendering a file type | score(mime, path); open(source) returns a Preview with pages, render(page, size) to RGBA frames, a text layer and metadata |
| SearchIndex | Querying a file search index: Windows Search, Spotlight, Tracker, Baloo, or a future Rick-owned index | available() detected at start and on service change; query(text, scope) streamed as a cancellable job; capabilities (names only, content, metadata) |

A packaged feature implements `Extension`: a manifest (id, version, settings schema) and `register`, which contributes to any of the five points. Grouping is by domain, not by point: `ext-git` holds one Decorator, several Actions and the repository logic; `ext-preview-image` holds one Previewer and nothing else.

## Module rules

1. One crate, one noun; split before it needs a second README section.
2. Extensions depend on `ext` and `types` only; the core never depends on an extension.
3. No thread spawn outside `runtime`; every background task is a cancellable, prioritised job.
4. Extensions return data and jobs and never draw; a panel contributed by an extension is a `Panel` trait that lives in `ui-egui` only.
5. `OsStr` for names everywhere; `cfg(target_os)` only inside `vfs-local` and `platform`.
6. Built-ins register in `app` exactly like plugins; `plugins-wasm` adapts guest modules to the same five traits.

## Package layout

One Cargo workspace, one crate per concern, grouped by layer. Kernel crates carry no feature code; everything a user would call a feature is an `ext-*` crate that registers through `ext`. Official extensions compile into the binary behind Cargo features, so a lean build is a matter of turning features off.

**Kernel**, in dependency order: each crate may depend only on the crates above it, and cargo-deny enforces the order. `ext` sits below `vfs` because `vfs` and `vfs-local` implement the `Provider` trait it defines.

- `types` — paths, entries, mime, errors, capability flags; no I/O, no async runtime and no dependency on any other Rick crate. It may use the small ecosystem crates it needs (`bitflags`, `thiserror`), nothing heavier.
- `runtime` — job system, cancellation, progress, thread pools, event bus.
- `platform` — per-OS code behind one trait set: trash, links and junctions, elevation, system properties dialogue, file associations, mounts. In 0.1 the set is `Trash` (trash, list, restore, remove one item, empty), `OpenWith` (`open` with the default application, `reveal` in the system file manager), `Properties` (the native dialogue on Windows and macOS, label/value rows on Linux), `NativeDialogs` (async, through `rfd`, the XDG portal on Linux), `Clipboard` and `ConfigDir`; elevation, file associations and mounts come later. 0.2 adds `Secrets` (store, get and remove a secret by a stable key in the platform keychain: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows, a fake for tests) and `Terminal` (launch the user's terminal emulator with a command), both introduced by `ext-ssh` and reused by the cloud providers and `ext-action-terminal`, and `Mount` (which disk-image formats this OS mounts natively, and mount one through the OS: `Mount-DiskImage`, `hdiutil attach`, `udisksctl loop-setup`), used by the *Mount* action. The three RustSec "unmaintained" notices on `rfd`'s D-Bus dependencies are accepted in deny.toml with the reason recorded, to be dropped when `rfd` moves on.
- `ext` — the five extension traits, the registry, manifests and the extension context.
- `vfs` — the mount table that routes a path to its `Provider`, and cross-provider copy and move.
- `vfs-local` — the local filesystem provider: listing, statx, watching, copy and reflink.
- `model` — the directory model: cached listings, sort, filter, group, selection, history.
- `ops` — the file operations as jobs: copy, move, trash, delete, rename, link, undo stack, conflict policy.
- `app` — the application state: windows, panes, tabs, layouts, session persistence, settings, the command palette.
- `ui-egui` — all egui widgets and the dock layout; depends on `app`, never the other way round.
- `rick` — the binary: feature flags, wiring, startup.

**Providers** (`ext-vfs-*`)

- `ext-vfs-archive` — zip, tar family, 7z, RAR read-only, single-file codecs, ISO 9660 and UDF; and the disk images of the Disk images table (raw IMG, DMG, VHD, VHDX, VMDK, VDI, QCOW2) read-only through the bundled 7-Zip library.
- `ext-vfs-s3` — S3, R2, MinIO and other S3-compatible stores, one endpoint setting.
- `ext-vfs-azure` — Azure Blob.
- `ext-vfs-gcs` — Google Cloud Storage.
- `ext-ssh` — remote hosts over SSH, one plugin with several contributions like `ext-git`: a Provider driven by shell commands over `exec` channels (no SFTP), the hosts of `~/.ssh/config` and Rick's own in the Network section, and the *Open console here* and *Open in external terminal* actions. Specified in [`specs/ext-ssh.md`](ext-ssh.md).
- `ext-zfs` — OpenZFS snapshots, one plugin with several contributions like `ext-git`: a read-only Provider (`zfs:`: the datasets with snapshots, each dataset's snapshots newest first, each snapshot's files as they were, and the versions of one file across snapshots), the "ZFS" Action group (*Browse snapshots*, *Back to live folder*, *Versions*, *Restore*, *Create*, *Rename* and *Destroy snapshot*), and an Info panel section (dataset, snapshots, main properties with their source). It talks to the `zfs` command line; changes ZFS refuses for lack of permission run through a privileged helper under its own polkit action, as `ext-installer` does, and Rick itself never runs as root. Linux only. Specified in [`specs/ext-zfs.md`](ext-zfs.md).
- `ext-vfs-trash` — the trash as a browsable location with restore.

**Decorators** (`ext-deco-*`)

- `ext-git` — one plugin, several contributions: a Decorator (status badges, branch, ahead and behind, via gix), an Action group for the context menu (Git commit, Stash, Stash pop, Pull, Push, Checkout branch, Discard changes, Show log, Blame, Git clone…), and an Info panel section (current branch, last commit). The Action group appears under "Git" in the Extensions part of the context menu and in the command palette, only when the location is inside a repository — except *Git clone…*, which needs none:
  - **Git clone…** (`git.clone`, no default shortcut, issue 173) clones a repository into a local folder. It is offered on the empty area of a pane, where the target is the folder the pane shows, and on one selected folder, where the target is that folder, so the clone lands inside it. On a file or on a selection of several entries it is not in the context menu and is greyed in the command palette; on a location that is not a local folder it is hidden. It is the last entry of the Git group; outside a repository, where the group's other actions are hidden, it is the group's one entry and stands in the menu by itself.
  - *URL and clipboard.* The dialogue (its layout under *Dialogues*) asks for the repository's URL. The field takes a Git URL — `https://…`, `ssh://…` or `git://…` with a host and a path, `file://…` with a path, or the scp-like `user@host:path` — or a local repository by its absolute path. A URL is taken by its shape; a local path is taken when a Git repository, a work tree or a bare one, is there. Nothing else is taken: not `http://`, not a relative path, not another transport. On open the clipboard is read: when its text, without the white space around it, is one of these, the field opens with it, selected, so that typing replaces it; otherwise the field opens empty. *Clone* is enabled only while the field holds one of these; Enter is *Clone*, Escape cancels. The dialogue names the target folder and the folder the clone creates.
  - *Options*, folded away at first and all optional: **Branch** (`--branch`), **Depth**, a whole number of commits of 1 or more (`--depth`), and **Submodules** (`--recurse-submodules`). What is set is passed to git as that flag; what is not set adds nothing, so git's defaults apply. There is no custom folder name and no other flag.
  - *The clone* is what `git clone <url>` does in the target folder, run through the `git` command line like the group's other actions: git creates the folder named after the repository — the last segment of the URL's path, or of the local path, without `.git` — and where a folder of that name exists and is not empty git fails and nothing is overwritten. Credentials are git's own business, its credential helpers and the SSH agent; Rick asks for none.
  - *The job.* The clone is a job in the operations drawer, *Git clone <repository>*: its progress is git's own status line (the phase, the percentage, the counts, the rate), and it can be cancelled, which stops git and removes the folder the clone had created: only that folder, never one that was there before the clone started, and only once no process of the clone is left to write into it; where git cannot be stopped whole, the folder is left alone and a warning is logged. Every failure — a host that does not answer, a refused login, a branch that is not there, the folder that exists — is the job's, with what git said, never a dialogue. When the job ends, the pane it was started from is refreshed, so the new folder appears.
- `ext-svn` — the same shape through the svn command line: Decorator plus Update, SVN commit, Revert, Log actions.
- `ext-deco-fs` — later: Btrfs subvolumes, NTFS alternate streams. ZFS snapshots are `ext-zfs`.

**Previewers** (`ext-preview-*`)

- `ext-preview-text` — plain text, encodings, huge-file windowing, pretty print.
- `ext-preview-code` — syntax highlighting via syntect, on top of text.
- `ext-preview-image` — raster and SVG decode (`image`, `resvg`), EXIF, IPTC, XMP.
- `ext-preview-icons` — ICO, CUR, ICNS and XPM: every variant of an icon container as a page, magnified pixel-sharp (`ico`, `icns`, an XPM parser of its own).
- `ext-preview-pdf` — pdfium-render; pdfium is a dynamic library that every build and release places beside the binary, found as *Third-party libraries* describes (a setting, no environment variable).
- `ext-preview-office` — LibreOffice headless to PDF, then the PDF path.
- `ext-preview-media` — audio and video via GStreamer, ID3 and stream info.
- `ext-preview-archive` — listing of an archive's or disk image's contents without mounting it.
- `ext-preview-markup` — one plugin for the three structured text formats: Markdown and HTML rendered, CSV and TSV as a virtualised grid, each with the *Rendered* toggle to the source. Supersedes `ext-preview-csv` and takes the Markdown and HTML rendering out of `ext-preview-code`.
- `ext-preview-3d` — STL, OBJ and 3MF meshes rendered through wgpu.
- `ext-preview-hex` — the hex view: the fallback for a file no other previewer recognises (the lowest score, for every file), and chosen per type in settings.

**Actions** (`ext-action-*`)

- `ext-action-core` — the built-in operations list: clipboard, trash, rename, new, link, select, view and sort commands, and *Mount* for disk images through `platform::Mount`.
- `ext-action-compress` — compress and extract.
- `ext-action-compare` — file and folder diff.
- `ext-action-terminal` — open console, open in system file manager.

**Search** (`ext-search-*`)

- `ext-search-walk` — the non-indexed subdirectory search via the `ignore` walker.
- `ext-search-windows`, `ext-search-spotlight`, `ext-search-tracker`, `ext-search-baloo` — one connector per platform index.

Compiled into the binary: `vfs-local`, `ext-preview-text`, `ext-preview-code`, `ext-preview-image`, `ext-preview-pdf` and `ext-action-core`. Every other `ext-*` crate builds as a plugin library into the plugins directory; the Cargo features on the binary remain for a build that compiles a plugin in instead.

Naming rule: a crate that contributes to one extension point carries that point as its prefix (ext-preview-pdf). A crate that is a feature in the user's eyes and contributes to several points, like Git, carries only the feature name (ext-git) and registers a Decorator, an Action group, a Previewer or whatever it needs from one manifest; the user installs, enables and disables it as one thing.

## Build order

types → runtime → vfs-local with the streamed listing → model → ext with a fake Decorator → ui-egui showing a list with badges → ext-git → previews.

## Architecture rules

These govern how the crates fit together. They are the decisions an implementer must not re-make.

- **Unidirectional state flow.** The UI never mutates the model directly. It emits `Command` values; `app` handles them, updates the model and publishes `Event`s; the UI re-renders from the model. The model is the single source of truth for everything the window shows. Undo/redo is a log of reversible commands kept in `app`.
- **Every long operation is a Job.** A directory listing, a copy, a search, a preview decode: each runs as a `Job` with a cancellation token, a progress channel and a result. Listings stream entries in batches so the UI shows the first screen before the walk finishes; switching away from a location cancels its jobs. Cancelling drops the job's future, so a job that never polls its token still stops at its next await point and its handle resolves with `Error::Cancelled`. A job that leaves partial state behind, such as a half-written copy target, must check the token before each irreversible step or hold a guard whose `Drop` undoes or finishes that step, because being dropped is its only notice. Long synchronous work on the compute pool cannot be dropped mid-call and checks `is_cancelled()` between chunks. Listings, their change watches and decorator lookups are jobs for cancellation and progress but are not listed in the operations area, which shows the jobs an action, an undo or a redo started; their output is the event stream.
- **Platform code lives in `platform` only.** No `cfg(target_os)` anywhere else. Other crates program against the `platform` traits (trash, open-with, file properties, config dir, native dialogs) and the tests run with a fake implementation.
- **The extension traits are a versioned contract.** The traits in `ext` and the manifest format carry a version. Any breaking change bumps it; the plugin host refuses manifests it does not understand and says so in the Extensions screen.
- **Optional extensions are Cargo features.** `rick` builds with the kernel alone; each `ext-*` crate is a feature on the binary, default-on for built-ins, so a minimal build and a full build come from the same workspace.
- **Milestones have acceptance tests.** Each step in the build order ends with an executable acceptance test (listing 200k entries under the target, a round-trip copy with cancellation, a plugin loaded from a manifest). The next step does not start until it passes.
- **No shared mutable state across threads without an owner.** Caches, indexes and watchers are owned by one component that exposes a message or async API; `Arc<Mutex<_>>` across crate boundaries is a design smell to justify in a comment.
- **Identity, not paths.** Entries are keyed by a provider-scoped `EntryId` (device and inode on local filesystems), not by path string, so renames and moves keep selection, history and decorations intact.
