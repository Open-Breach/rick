# Rick — App Icons & Desktop Entry

Small spec for shipping Rick's app icon on all three platforms and a Linux desktop entry, so that a release archive installs cleanly — by hand or through Rick's own installer plugin. No install script is shipped; the Linux archive just carries the files the installer plugin (ext-installer) already knows how to handle.

## The icon

The app icon is the Portal burst icon — the three rings turned against each other with folders and files spiralling out, bright blue on a dark navy rounded tile. Sizes of 32 px and below use the rings-only fallback already baked into the icon set (`rick-small.svg`, `rick-macos-small.svg`), because the flying files turn into noise at that size.

The committed files are the source of truth for builds. They are not regenerated in CI; the generator lives outside the repo.

## Repository layout

```
assets/icons/
  svg/rick.svg              master icon (square tile)
  svg/rick-small.svg        rings only on the tile, for 32 px and below
  svg/rick-macos.svg        Apple grid, margin around the tile
  svg/rick-macos-small.svg  rings-only macOS variant
  svg/rick-symbol.svg       rings only, transparent — About dialog, website
  png/rick-<size>.png       16, 20, 22, 24, 32, 40, 48, 64, 96, 128, 256, 512, 1024
  linux/hicolor/<size>x<size>/apps/rick.png   16, 22, 24, 32, 48, 64, 128, 256, 512
  linux/hicolor/scalable/apps/rick.svg
  windows/rick.ico          16 … 256
  macos/Rick.icns           plus Rick.iconset/ and rick-macos-1024.png
packaging/
  linux/rick.desktop
  macos/Info.plist          template, version substituted at release time
```

## Identifier

One identifier ties everything together and must be identical everywhere: `rick`.

- Desktop entry file name: `rick.desktop`
- Icon name in the hicolor theme and in `Icon=`: `rick`
- Wayland app id and X11 `WM_CLASS`, set in eframe with `ViewportBuilder::with_app_id("rick")`
- `StartupWMClass=rick` in the desktop entry

If these differ, GNOME and KDE show a generic icon for the running window and cannot match it to the launcher entry. A reverse-DNS identifier would also work but changing it later breaks pinned launchers, so this is decided now and stays.

## Window icon — all platforms

The running window gets its icon from eframe: `ViewportBuilder::with_icon` loaded from `assets/icons/png/rick-256.png` via `include_bytes!` and `eframe::icon_data::from_png_bytes`. This covers the title bar, taskbar and Alt-Tab on Windows and X11. On Wayland the compositor ignores it and takes the icon from the desktop entry matched by app id — hence the identifier rule above. On macOS the Dock icon comes from the app bundle, not from this call.

The About dialog shows `assets/icons/svg/rick-symbol.svg` (rings only) at 64 px, not the tile.

## Windows

`rick.ico` is embedded into `rick.exe` as the executable's icon resource with the `winresource` crate from `build.rs` (Windows targets only). The same build step sets the version resource — ProductName "Rick", FileDescription "Rick file manager", FileVersion and ProductVersion from `Cargo.toml`, LegalCopyright with the Apache-2.0 notice — so Explorer and the Properties dialog show the icon and version. The release zip stays as it is: `rick.exe` and `plugins/` side by side.

## macOS

The release workflow assembles an app bundle from the plain binary:

```
Rick.app/Contents/
  Info.plist               from packaging/macos/Info.plist
  MacOS/rick               the binary
  MacOS/plugins/           the plugin files, unchanged lookup (next to the binary)
  Resources/Rick.icns      from assets/icons/macos/Rick.icns
```

`Info.plist` keys: `CFBundleName` Rick, `CFBundleDisplayName` Rick, `CFBundleIdentifier` `org.open-breach.rick`, `CFBundleExecutable` rick, `CFBundleIconFile` Rick, `CFBundlePackageType` APPL, `CFBundleShortVersionString` and `CFBundleVersion` from `Cargo.toml`, `LSMinimumSystemVersion` 11.0, `NSHighResolutionCapable` true, `LSApplicationCategoryType` `public.app-category.utilities`. Plugins stay in `Contents/MacOS/plugins` so the plugin folder lookup needs no change.

The macOS archive ships `Rick.app` zipped. Signing, notarisation and the DMG remain Version 0.3 work; an unsigned bundle still shows the right icon in Finder and the Dock.

## Linux

### Desktop entry

`packaging/linux/rick.desktop`:

```
[Desktop Entry]
Type=Application
Name=Rick
GenericName=File Manager
Comment=A god-like file manager
Exec=rick %F
Icon=rick
Terminal=false
Categories=System;FileTools;FileManager;Utility;
MimeType=inode/directory;
Keywords=files;folders;explorer;browse;
StartupNotify=true
StartupWMClass=rick
```

`%F` passes selected folders as arguments; Rick opens them as tabs, or ignores them if argument handling is not there yet (it must at least not fail on them). `Exec=rick` relies on the binary being on the PATH — the installer plugin rewrites `Exec` to the absolute path through its `current` symlink, and a manual install is expected to put `rick` on the PATH. The MIME type registration makes Rick appear in "Open with" for folders; making it the default file manager stays Version 0.3.

### Archive layout

```
rick-<version>-linux-<arch>/
  rick
  plugins/
  share/applications/rick.desktop
  share/icons/hicolor/<size>x<size>/apps/rick.png
  share/icons/hicolor/scalable/apps/rick.svg
  README.md, LICENSE
```

`share/` mirrors the XDG layout, so a manual install is one copy of `share/` into `~/.local/share/` plus the binary on the PATH; the installer plugin finds the desktop file and the icons where it expects them and does the rest (desktop entry with rewritten `Exec`, icons into the user's hicolor theme, `current` symlink). The README gets a short "Install" paragraph stating exactly that — no script.

## Acceptance

- Linux, GNOME on Wayland and KDE on X11: after copying `share/` and launching from the app grid, the launcher shows the burst icon, the running window shows the same icon in the dock or taskbar, and the two are matched (no duplicate generic entry). Installing the same tarball through Rick's installer plugin gives the same result.
- Windows: `rick.exe` shows the icon in Explorer and in Properties with the right version; the running window and taskbar show it.
- macOS: `Rick.app` shows the icon in Finder and in the Dock while running; plugins load from inside the bundle.
- All platforms: the 16 px and 32 px sizes render the rings-only fallback, not the full burst.

## Out of scope

Signing, notarisation, DMG, winget, OBS, AUR, AppImage and the FileManager1 D-Bus service — all Version 0.3 System Integration & Distribution.
