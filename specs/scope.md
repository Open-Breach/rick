# Scope

Related: specs/gui-windows.md, specs/gui-preview.md, specs/plugins.md, specs/core-types.md

## Decisions so far

Rust with egui, Linux-first, native previews, and a small kernel that every feature plugs into.

| Area | Decision | Why |
| --- | --- | --- |
| Language | Rust | Performance; one language from syscalls to UI |
| GUI toolkit | egui (eframe) with egui\_dock for tabs, split panes and tear-out windows | Self-rendered, identical on all platforms, native Wayland through winit + wgpu; only visible rows are laid out |
| Reference platform | Linux, Wayland-native | Priority platform; no GTK or WebKitGTK in the rendering path |
| Previews | Decoded in Rust and uploaded as GPU textures; PDF through pdfium-render; video through GStreamer on all three platforms (bundled runtime on macOS and Windows) | Zero-copy path; same result on every OS |
| Core shape | Small kernel with five extension points (Provider, Decorator, Action, Previewer, SearchIndex); built-in features are extensions | Keeps the core small and the extension API honest |
| UI to core | Explicit Command and Event types between the egui layer and the core | Core stays testable; UI stays replaceable |
| Third-party plugins | WASM (wasmtime or extism) | Sandboxed; one plugin binary for all platforms |

Ruled out: iced (the preferred architecture, but no docking or tab tear-out today) and Tauri (WebKitGTK on Linux needs workarounds that disable accelerated rendering, no zero-copy preview path, 2 to 3 times the memory).

## Supported scope

Rick runs on Linux, macOS and Windows and must work correctly on every file system, mount, archive and file type below. It does not implement file systems: the OS mounts them, and "supported" means correct behaviour on each (naming rules, attributes, copy fast paths, limits) plus mount and connect actions through the platform layer.

### Platforms

Linux (reference platform), macOS, Windows.

### Local file systems

| File system | Where it appears | Rick behaviour |
| --- | --- | --- |
| ext2, ext3, ext4 | Linux | Standard POSIX semantics |
| XFS | Linux | Reflink copies (instant, space-shared) |
| Btrfs | Linux | Reflink copies; subvolumes shown as folders |
| ZFS | Linux (OpenZFS) | Datasets appear as mounts |
| NTFS | Windows; Linux via ntfs3 | Case-insensitive names, hidden attribute, ACLs |
| FAT12/16/32, exFAT | Removable media on all OSes | Case-insensitive, no permissions; FAT32 4 GB file limit flagged before a copy |
| HFS+, APFS | macOS | Case-insensitive by default; clonefile copies on APFS |
| ISO9660, UDF | Optical discs and .iso images | Read-only; an .iso file can also be browsed as an archive without mounting |

### Network file systems

NFS v3/v4, SMB/CIFS, CephFS, GlusterFS, SSHFS (FUSE) and 9p are mounted by the OS. Rick treats them as remote-latency mounts: no eager stat, background metadata, no thumbnailing by default, and connect, mount and unmount actions (udisks2 and GVFS on Linux). iSCSI is out of scope: it is block storage, and whatever the OS mounts on the attached disk is handled as a local file system.

### Object storage

| Service | Plugin | Notes |
| --- | --- | --- |
| AWS S3, Cloudflare R2, MinIO and other S3-compatible stores | `ext-vfs-s3` | One plugin with an endpoint setting; flat keys shown as folders by prefix |
| Azure Blob | `ext-vfs-azure` | Containers as top-level folders |
| Google Cloud Storage | `ext-vfs-gcs` | Buckets as top-level folders |

Each cloud is its own plugin implementing `Provider` directly, so a user installs only the clouds they use; there is no shared object-storage crate — the overlap (prefix-to-folder mapping, chunked upload, credential storage) is small, and a shared crate may be extracted later if the second plugin shows real duplication. Object stores have no change notifications (polling). Download and upload are ordinary cross-provider copies to and from the local provider, streamed through the machine. **Transfers inside one account never pass through the machine:** a copy or move between two locations of the same provider instance — within a bucket, or across two buckets or containers of the same account — is submitted to the service as a server-side operation (S3 `CopyObject`, Azure's asynchronous Copy Blob, GCS `Rewrite`; move adds a delete), and Rick's job only tracks its progress, polling where the service reports it and cancelling where the service allows. This is the `NATIVE_TRANSFER` capability of the Provider contract, not cloud-specific code: `ext-ssh` runs its same-host `cp` and `mv` through the same hook.

### Archives and compression

| Format | Browse in place | Extract | Create | Crate |
| --- | --- | --- | --- | --- |
| zip (DEFLATE, zstd) | Yes | Yes | Yes | `zip` |
| tar, tar.gz, tar.zst, tar.xz, tar.bz2, tar.lz4 | Yes | Yes | Yes | `tar` + codec crates |
| 7z | Yes | Yes | Yes (LZMA, LZMA2) | `sevenz-rust` |
| RAR | Yes | Yes | No | `unrar`; creation is not licensable |
| gzip, zstd, xz/LZMA, bzip2, LZ4, Brotli, Snappy (single file) | — | Decompress | Compress | Preview by streaming decompression |

Enter on an archive enters it in the current tab by default; the setting `archives.on_open` can hand it to the system instead (see *Opening files* under General behaviour).

### Disk images

Disk images are browsed exactly like archives — listed in the preview pane on selection, entered on Enter, the same `archives.on_open` setting — read-only, through `ext-vfs-archive` and `ext-preview-archive`, without mounting anything. The list is pinned; WIM and SquashFS are out.

| Format | Browse in place | Reader | *Mount* through the OS |
| --- | --- | --- | --- |
| ISO (ISO 9660, UDF) | Yes | Native, as today | Windows, macOS, Linux |
| Raw IMG | Yes | 7-Zip library | macOS, Linux |
| DMG | Yes | 7-Zip library | macOS |
| VHD, VHDX | Yes | 7-Zip library | Windows |
| VMDK, VDI, QCOW2 | Yes | 7-Zip library | — |

The 7-Zip library (`7z.so` / `7z.dll`, LGPL) is bundled in the plugins directory and loaded dynamically as *Third-party libraries* describes; it parses the image containers and the file systems inside them (FAT, NTFS, ext, HFS+, APFS, ISO), which no mature Rust crate does. The formats the native crates already read keep their readers. The *Mount* action and *Eject* are specified under *Disk images and Mount* in General behaviour.

### Preview support

| Category | Formats | How |
| --- | --- | --- |
| Text | Any text file, encoding detected | Native text view |
| CSV | CSV, TSV | Virtualised grid, delimiter sniffed, first row as header; the source one toggle away (`ext-preview-markup`) |
| Markup | Markdown, HTML | Rendered by default, the source one toggle away (`ext-preview-markup`). HTML is a basic reader — headings, paragraphs, lists, links, images, tables — with no browser engine, no JavaScript and no CSS fidelity |
| Code | JavaScript, TypeScript, Java, Rust, C#, F#, VB, C, C++, Perl, PHP, JSON, XML, YAML, HTML, Markdown | Syntax highlighting (syntect); for Markdown and HTML this is the source view the markup previewer toggles to, and what shows when that plugin is disabled |
| PDF | PDF | pdfium-render, page by page, text layer for search and selection |
| Office | DOCX, XLSX, PPTX, ODT, ODS, ODP | Converted to PDF by LibreOffice headless when it is installed (never bundled), then the PDF path; without it, the missing-library message of *Third-party libraries* |
| Images | JPEG, PNG, WebP, BMP, GIF (animated) | `image` crate, downscaled to the pane, EXIF shown |
| Vector images | SVG, SVGZ | `resvg`, rasterised at the size the pane asks for and again at every size change, so it is sharp at any zoom; the source one toggle away (see *Vector images*) |
| Icon containers | ICO, CUR, ICNS, XPM | `ext-preview-icons`: every embedded variant, stepped through as pages, magnified pixel-sharp (see *Icon containers*) |
| Audio | MP3, WAV | Waveform and playback |
| Video | MP4, AVI | GStreamer on all platforms, hardware-decoded where available, frames to texture |
| 3D models | STL, OBJ, 3MF | Parsed natively, shaded mesh rendered through wgpu; orbit, zoom; triangle count and bounding box (0.2) |

### Source code repositories

| System | Integration | What Rick shows |
| --- | --- | --- |
| Git | Native, `gix` | Status badges per entry, folder roll-ups, branch and ahead/behind, stage/unstage/discard/diff actions |
| SVN | `svn` command line | Status badges, update and commit actions |
| TFVC | `tf` command line (Azure DevOps) | Deferred: not at launch, later as a plugin on demand (pending changes, check-in) |
