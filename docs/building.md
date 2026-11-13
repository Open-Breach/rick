# Building Rick from source

```bash
git clone https://github.com/Open-Breach/rick.git
cd rick
cargo xtask bundle
./target/debug/bundle/rick
```

You need the stable Rust toolchain (`rust-toolchain.toml` pins it) and, on Linux, the usual Wayland/X11 development packages for an egui app.

`cargo xtask bundle` builds the workspace and lays out the binary, the shared libraries it links and every plugin in `target/debug/bundle/` (`--release` for `target/release/bundle/`), which is also what the release archives contain. Build and test the workspace as a whole (`cargo test --workspace`), since the kernel crates are shared libraries built once per feature set (QUESTIONS.md 81).

## Libraries a preview needs

- **PDF.** PDF previews need pdfium beside the binary, or wherever the PDF previewer's `library` setting points (`preview-pdf.toml` in the config directory, which Rick fills in itself when it finds the library and which the preview pane's *Browse…* sets). The bundle copies the library from that setting when it is set; a release archive brings it along.
- **Audio and video.** Media playback is built only with `--features ext-preview-media/gstreamer` (`cargo test --workspace --features ext-preview-media/gstreamer`, `cargo xtask bundle --features ext-preview-media/gstreamer`), which needs the GStreamer development files (`libgstreamer1.0-dev` and `libgstreamer-plugins-base1.0-dev` on Debian and Ubuntu, the official development package on macOS and Windows); without the feature the media previewer shows metadata only. `RICK_GSTREAMER_DIR` makes the bundle carry a GStreamer runtime on macOS and Windows (QUESTIONS.md 106). On Linux Rick uses the system GStreamer: install `gstreamer1.0-plugins-good` and `gstreamer1.0-libav`, or your distribution's equivalents, for MP3 and H.264.
- **Office documents.** Office previews need LibreOffice installed; without it the preview shows the document's metadata and says so.

## What a release archive holds

Release archives are built for Linux (x64 and arm64), macOS (Apple silicon) and Windows (x64). Each holds:

- the `rick` binary and, beside it, the shared libraries it needs: Rick's kernel and the Rust standard library, which the plugins share with it, and the pdfium library;
- a `plugins/` folder with one folder per plugin: the Git and SVN plugins, the archive, trash, SSH, S3, Azure Blob and Google Cloud Storage providers (the 7-Zip library for disk images beside the archive plugin, with its licence), the Office, media, archive, markup, hex, 3D and icon previewers, the subdirectory search and the platform's search-index connector (Windows Search, Spotlight, or Tracker and Baloo on Linux), the compress, compare, console and bulk-rename actions and the tarball installer;
- the GStreamer runtime the media previewer plays through, on macOS and Windows;
- on Windows, the Visual C++ runtime beside `rick.exe`;
- on Linux, the desktop entry and the icons in the XDG layout under `share/`; on macOS all of it inside `Rick.app`;
- the license, the README, the changelog and the third-party licenses.

`SHA256SUMS.txt` on the release page has the checksums. The Linux builds need glibc 2.39 or later (Ubuntu 24.04). Intel Macs are not covered yet.

The archives are not signed or notarised yet: macOS may refuse an app downloaded through a browser until you allow it under System Settings → Privacy & Security, or clear the quarantine flag with `xattr -dr com.apple.quarantine <folder>/Rick.app`; Windows SmartScreen may warn the first time.

## The README

`README.md` describes the latest release and is brought up to date in the release pull request only (specs/readme.md). `cargo xtask readme` does it: it writes the version, the Features list and the Install section from what the extensions register, and renders the two screenshots of `docs/screenshots/` afresh (it needs a wgpu adapter; a software one such as lavapipe will do). `cargo test --workspace` fails when the README of a release version is not what that command writes.

## Writing a plugin

Start from `crates/plugins-native/README.md`. The numbers 0.2.0 was measured against are in [BENCHMARKS.md](../BENCHMARKS.md).
