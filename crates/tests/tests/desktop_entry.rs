//! Acceptance (issue 88, specs/app-icons.md *Identifier* and *Desktop
//! entry*): the desktop entry is the spec's, its file name, its `Icon=` and
//! its `StartupWMClass=` are the app id every window carries, so GNOME and
//! KDE match the running window to the launcher entry; every icon the
//! archive ships under `share/icons/hicolor/` is in the repository at its
//! size; and `desktop-file-validate` passes the entry where the tool is
//! installed (required on Linux CI, `RICK_REQUIRE_DESKTOP_FILE_VALIDATE`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The repository root: two directories above this crate.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The desktop entry, named after the app id.
fn desktop_entry() -> PathBuf {
    repo()
        .join("packaging/linux")
        .join(format!("{}.desktop", ui_egui::APP_ID))
}

/// The value of `key` in the entry's one group.
fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        line.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
    })
}

#[test]
fn the_desktop_entry_names_the_app_id_as_icon_and_window_class() {
    let path = desktop_entry();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(ui_egui::APP_ID, "rick");
    assert_eq!(value(&text, "Icon"), Some(ui_egui::APP_ID));
    assert_eq!(value(&text, "StartupWMClass"), Some(ui_egui::APP_ID));
    // `%U`: the selected folders, and a `rick://` link the entry handles.
    assert_eq!(
        value(&text, "Exec"),
        Some(format!("{} %U", ui_egui::APP_ID).as_str())
    );
    assert_eq!(text.lines().next(), Some("[Desktop Entry]"));
    assert_eq!(value(&text, "Type"), Some("Application"));
    assert_eq!(value(&text, "Name"), Some("Rick"));
    assert_eq!(value(&text, "GenericName"), Some("File Manager"));
    assert_eq!(value(&text, "Comment"), Some("A god-like file manager"));
    assert_eq!(value(&text, "Terminal"), Some("false"));
    assert_eq!(
        value(&text, "Categories"),
        Some("System;FileTools;FileManager;Utility;")
    );
    assert_eq!(
        value(&text, "MimeType"),
        Some("inode/directory;x-scheme-handler/rick;")
    );
    assert_eq!(
        value(&text, "Keywords"),
        Some("files;folders;explorer;browse;")
    );
    assert_eq!(value(&text, "StartupNotify"), Some("true"));
}

#[test]
fn the_hicolor_icons_are_there_at_every_size() {
    let hicolor = repo().join("assets/icons/linux/hicolor");
    for size in [16, 22, 24, 32, 48, 64, 128, 256, 512] {
        let icon = hicolor
            .join(format!("{size}x{size}/apps"))
            .join(format!("{}.png", ui_egui::APP_ID));
        assert!(icon.is_file(), "{} is missing", icon.display());
        let bytes = std::fs::read(&icon).unwrap_or_else(|e| panic!("{}: {e}", icon.display()));
        // A PNG of that size: the IHDR's width and height.
        assert_eq!(
            &bytes[..8],
            b"\x89PNG\r\n\x1a\n",
            "{} is no PNG",
            icon.display()
        );
        let dim = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        assert_eq!(
            (dim(16), dim(20)),
            (size, size),
            "{} is not {size} px",
            icon.display()
        );
    }
    let scalable = hicolor
        .join("scalable/apps")
        .join(format!("{}.svg", ui_egui::APP_ID));
    assert!(scalable.is_file(), "{} is missing", scalable.display());
}

#[test]
fn desktop_file_validate_passes_the_entry() {
    let path = desktop_entry();
    let Ok(output) = Command::new("desktop-file-validate")
        .arg(&path)
        .stdin(Stdio::null())
        .output()
    else {
        assert!(
            std::env::var_os("RICK_REQUIRE_DESKTOP_FILE_VALIDATE").is_none(),
            "desktop-file-validate is required here and missing"
        );
        eprintln!("desktop-file-validate is not installed; skipping");
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}: {text}", path.display());
    // Hints are allowed (the entry's several main categories are the
    // spec's); errors and warnings are not.
    assert!(
        !text.contains("error:") && !text.contains("warning:"),
        "{}: {text}",
        path.display()
    );
}
