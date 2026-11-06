//! Every glyph the shell draws, from the Phosphor icon set through
//! `egui-phosphor`. No Unicode symbol character is used as an icon anywhere,
//! so no glyph can fall back to a box; the test below renders them all.

use egui_phosphor::regular as ph;
use types::{Entry, EntryKind};

/// Back.
pub const BACK: &str = ph::ARROW_LEFT;
/// Forward.
pub const FORWARD: &str = ph::ARROW_RIGHT;
/// Parent directory.
pub const UP: &str = ph::ARROW_UP;
/// Refresh.
pub const REFRESH: &str = ph::ARROW_CLOCKWISE;
/// Copy.
pub const COPY: &str = ph::COPY;
/// Close.
pub const CLOSE: &str = ph::X;
/// New tab.
pub const PLUS: &str = ph::PLUS;
/// The burger menu.
pub const MENU: &str = ph::LIST;
/// The search box.
pub const SEARCH: &str = ph::MAGNIFYING_GLASS;
/// Undo.
pub const UNDO: &str = ph::ARROW_U_UP_LEFT;
/// Redo.
pub const REDO: &str = ph::ARROW_U_UP_RIGHT;
/// Hidden files shown.
pub const HIDDEN_SHOWN: &str = ph::EYE;
/// Hidden files hidden.
pub const HIDDEN_HIDDEN: &str = ph::EYE_SLASH;
/// The side panel.
pub const SIDE_PANEL: &str = ph::SIDEBAR_SIMPLE;
/// The inspector.
pub const INSPECTOR: &str = ph::INFO;
/// Window: minimise.
pub const MINIMISE: &str = ph::MINUS;
/// Window: maximise.
pub const MAXIMISE: &str = ph::SQUARE;
/// Window: restore.
pub const RESTORE: &str = ph::CORNERS_OUT;
/// Collapsed breadcrumb ancestors.
pub const COLLAPSED: &str = ph::CARET_DOUBLE_LEFT;
/// Dropdown marker.
pub const DROPDOWN: &str = ph::CARET_DOWN;
/// Sorted ascending.
pub const SORT_ASC: &str = ph::SORT_ASCENDING;
/// Sorted descending.
pub const SORT_DESC: &str = ph::SORT_DESCENDING;
/// The List view.
pub const VIEW_LIST: &str = ph::LIST;
/// The Details view.
pub const VIEW_DETAILS: &str = ph::LIST_DASHES;
/// The Tree view.
pub const VIEW_TREE: &str = ph::TREE_STRUCTURE;
/// The Icons view.
pub const VIEW_ICONS: &str = ph::SQUARES_FOUR;
/// Home.
pub const HOME: &str = ph::HOUSE;
/// Pinned.
pub const PIN: &str = ph::PUSH_PIN;
/// Recent.
pub const RECENT: &str = ph::CLOCK_COUNTER_CLOCKWISE;
/// Network.
pub const NETWORK: &str = ph::NETWORK;
/// Trash.
pub const TRASH: &str = ph::TRASH;
/// A folder.
pub const FOLDER: &str = ph::FOLDER;
/// A link.
pub const LINK: &str = ph::LINK_SIMPLE;
/// Done.
pub const DONE: &str = ph::CHECK;
/// Failed.
pub const FAILED: &str = ph::WARNING;

/// Registers the Phosphor font with egui's defaults; call once per context.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

/// The file-type icon of an entry: by kind, then by extension.
#[must_use]
pub fn for_entry(entry: &Entry) -> &'static str {
    match entry.kind {
        EntryKind::Dir => FOLDER,
        EntryKind::Symlink { .. } => LINK,
        _ => by_extension(&entry.name),
    }
}

/// The icon for a file name's extension.
#[must_use]
pub fn by_extension(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "ico" | "heic"
        | "avif" => ph::FILE_IMAGE,
        "svg" => ph::FILE_SVG,
        "pdf" => ph::FILE_PDF,
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" => ph::FILE_ZIP,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" => ph::FILE_VIDEO,
        "mp3" | "flac" | "wav" | "ogg" | "m4a" | "aac" | "opus" => ph::FILE_AUDIO,
        "txt" | "log" | "rtf" => ph::FILE_TEXT,
        "md" | "markdown" => ph::MARKDOWN_LOGO,
        "csv" | "tsv" => ph::FILE_CSV,
        "rs" => ph::FILE_RS,
        "ts" | "tsx" => ph::FILE_TS,
        "js" | "jsx" | "mjs" => ph::FILE_JS,
        "py" => ph::FILE_PY,
        "html" | "htm" => ph::FILE_HTML,
        "css" | "scss" => ph::FILE_CSS,
        "c" | "h" => ph::FILE_C,
        "cpp" | "cc" | "hpp" => ph::FILE_CPP,
        "sql" => ph::FILE_SQL,
        "doc" | "docx" | "odt" => ph::FILE_DOC,
        "xls" | "xlsx" | "ods" => ph::FILE_XLS,
        "ppt" | "pptx" | "odp" => ph::FILE_PPT,
        "json" | "yaml" | "yml" | "toml" | "xml" | "java" | "go" | "rb" | "sh" | "cs" | "swift"
        | "kt" | "php" | "lua" | "vb" | "fs" => ph::FILE_CODE,
        _ => ph::FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon the shell uses, so the coverage test draws them all.
    const ALL: &[&str] = &[
        BACK,
        FORWARD,
        UP,
        REFRESH,
        COPY,
        CLOSE,
        PLUS,
        MENU,
        SEARCH,
        UNDO,
        REDO,
        HIDDEN_SHOWN,
        HIDDEN_HIDDEN,
        SIDE_PANEL,
        INSPECTOR,
        MINIMISE,
        MAXIMISE,
        RESTORE,
        COLLAPSED,
        DROPDOWN,
        SORT_ASC,
        SORT_DESC,
        VIEW_LIST,
        VIEW_DETAILS,
        VIEW_TREE,
        VIEW_ICONS,
        HOME,
        PIN,
        RECENT,
        NETWORK,
        TRASH,
        FOLDER,
        LINK,
        DONE,
        FAILED,
    ];

    #[test]
    fn the_installed_fonts_have_every_icon_the_shell_uses() {
        // kittest fails a frame that renders a missing glyph.
        let text: String = ALL
            .iter()
            .copied()
            .chain(
                ["a.png", "b.pdf", "c.zip", "d.rs", "e.unknownext", "f"]
                    .iter()
                    .map(|n| by_extension(n)),
            )
            .collect();
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            ui.label(&text);
        });
        install(&harness.ctx);
        harness.run();
    }

    #[test]
    fn file_type_icons_follow_kind_then_extension() {
        assert_eq!(by_extension("photo.JPG"), ph::FILE_IMAGE);
        assert_eq!(by_extension("paper.pdf"), ph::FILE_PDF);
        assert_eq!(by_extension("Makefile"), ph::FILE);
        assert_eq!(by_extension("notes.txt"), ph::FILE_TEXT);
    }
}
