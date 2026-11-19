# Building Rick from source

```bash
git clone https://github.com/Open-Breach/rick.git
cd rick
cargo xtask bundle
./target/debug/bundle/rick
```

You need the stable Rust toolchain (`rust-toolchain.toml` pins it) and, on Linux, the usual Wayland/X11 development packages for an egui app.

`cargo xtask bundle` builds the workspace and lays out the binary, the shared libraries it links and every plugin in `target/debug/bundle/` (`--release` for `target/release/bundle/`). A release archive is that tree without the separate plugins (see below). Build and test the workspace as a whole (`cargo test --workspace`), since the kernel crates are shared libraries built once per feature set (QUESTIONS.md 81).

## Libraries a preview needs

- **PDF.** PDF previews need pdfium beside the binary, or wherever the PDF previewer's `library` setting points (`preview-pdf.toml` in the config directory, which Rick fills in itself when it finds the library and which the preview pane's *Browse…* sets). The bundle copies the library from that setting when it is set; a release archive brings it along.
- **Audio and video.** Media playback is built only with `--features ext-preview-media/gstreamer` (`cargo test --workspace --features ext-preview-media/gstreamer`, `cargo xtask bundle --features ext-preview-media/gstreamer`), which needs the GStreamer development files (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Debian and Ubuntu, the official development package on macOS and Windows); without the feature the media previewer shows metadata only. `RICK_GSTREAMER_DIR` makes the bundle carry a GStreamer runtime on macOS and Windows, in the media plugin's folder `plugins/preview-media/`, where the plugin finds it wherever that folder lies (`crates/plugins-native/README.md`, *Where it goes*). On Windows a bundle built with the feature needs it: Rick looks for libraries beside the binary, in the folder of the plugin that links them and in the system's folder, not on the `PATH`, so a GStreamer that is installed and on the `PATH` is not found from there. On Linux Rick uses the system GStreamer: install `gstreamer1.0-plugins-good` and `gstreamer1.0-libav`, or your distribution's equivalents, for MP3 and H.264.
- **Office documents.** Office previews need LibreOffice installed; without it the preview shows the document's metadata and says so.

## What a release archive holds

Release archives are built for Linux (x64 and arm64), macOS (Apple silicon) and Windows (x64). Each holds:

- the `rick` binary and, beside it, the shared libraries it needs: Rick's kernel and the Rust standard library, which the plugins share with it, and the pdfium library;
- a `plugins/` folder with one folder per plugin of the **default set**: the trash and archive providers (the 7-Zip library for disk images beside the archive plugin, with its licence), the archive, markup, hex and icon previewers, the subdirectory search and the platform's search-index connector (Windows Search, Spotlight, or Tracker and Baloo on Linux), the compress, compare, console and bulk-rename actions and the tarball installer, and on Linux the media previewer, which uses the system GStreamer;
- on Windows, the Visual C++ runtime beside `rick.exe`;
- on Linux, the desktop entry and the icons in the XDG layout under `share/`; on macOS all of it inside `Rick.app`;
- the license, the README, the changelog and the third-party licenses.

`SHA256SUMS.txt` on the release page has the checksums of the archives, the packages and the lists below. The Linux builds need glibc 2.39 or later (Ubuntu 24.04). Intel Macs are not covered yet.

The archives are not signed or notarised yet: macOS may refuse an app downloaded through a browser until you allow it under System Settings → Privacy & Security, or clear the quarantine flag with `xattr -dr com.apple.quarantine <folder>/Rick.app`; Windows SmartScreen may warn the first time.

## The separate plugins and their packages

Ten plugins are not in the archives: Git, SVN, ZFS (Linux), SSH, S3, Azure Blob, Google Cloud Storage, the Office and 3D previewers and, on macOS and Windows, the media previewer with its GStreamer runtime. That is nine on each platform. A release publishes each as a package of its own, `<id>-<version>-<platform>.rkp` (`git-0.6.0-linux-x64.rkp`), and one `plugins-<platform>.json` per platform that lists them for the plugin registry (specs/plugin-distribution.md §4). A package is a zip archive of the plugin's folder; *Install from file…* in the Extensions dialogue unpacks it into the per-user plugins folder, and the plugin loads at the next start. A package fits the Rick release it was built with and no other.

To make them from a bundle:

```bash
cargo xtask bundle --release
cargo xtask package --release
ls target/release/packages/
```

Which plugins are separate is said by each plugin's crate, under `[package.metadata.rick]` in its `Cargo.toml`. The same bundle gives the same bytes. A development build needs none of this: the bundle has every plugin.

## The README

`README.md` describes the latest release and is brought up to date in the release pull request only (specs/readme.md). `cargo xtask readme` does it: it writes the version, the Features list and the Install section from what the extensions register, and renders the two screenshots of `docs/screenshots/` afresh (it needs a wgpu adapter; a software one such as lavapipe will do). `cargo test --workspace` fails when the README of a release version is not what that command writes.

One part of the release pull request of 0.6.0 is written by hand: the privacy paragraph of specs/readme.md, section 9, word for word, as the first line under the rule above the closing links. The same test asks for it as soon as the status line names 0.6.0, and refuses it before.

## Writing a plugin

Start from `crates/plugins-native/README.md`. The numbers 0.2.0 was measured against are in [BENCHMARKS.md](../BENCHMARKS.md).
