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
/// Between breadcrumb segments.
pub const SEGMENT: &str = ph::CARET_RIGHT;
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
/// Unpin.
pub const UNPIN: &str = ph::PUSH_PIN_SLASH;
/// Recent.
pub const RECENT: &str = ph::CLOCK_COUNTER_CLOCKWISE;
/// Network.
pub const NETWORK: &str = ph::NETWORK;
/// Trash.
pub const TRASH: &str = ph::TRASH;
/// A folder.
pub const FOLDER: &str = ph::FOLDER;
/// A location on a provider that is not the local disk.
pub const PROVIDER: &str = ph::HARD_DRIVES;
/// An archive browsed as a location.
pub const ARCHIVE: &str = ph::FILE_ARCHIVE;
/// A link.
pub const LINK: &str = ph::LINK_SIMPLE;
/// Done.
pub const DONE: &str = ph::CHECK;
/// Failed.
pub const FAILED: &str = ph::WARNING;
/// A collapsed tree node.
pub const TREE_CLOSED: &str = ph::CARET_RIGHT;
/// An expanded tree node.
pub const TREE_OPEN: &str = ph::CARET_DOWN;
/// An open folder, for the expanded folder tree node.
pub const FOLDER_OPEN: &str = ph::FOLDER_OPEN;
/// Pause a job.
pub const PAUSE: &str = ph::PAUSE;
/// Resume a job.
pub const PLAY: &str = ph::PLAY;
/// Search in subdirectories.
pub const SUBTREE: &str = ph::FOLDERS;
/// Removable media.
pub const USB: &str = ph::USB;
/// An internal disk.
pub const DISK: &str = ph::HARD_DRIVE;
/// An optical disc.
pub const DISC: &str = ph::DISC;
/// The side panel's tree follows the focused pane.
pub const LINKED: &str = ph::LINK_SIMPLE;
/// The side panel's tree is unlinked.
pub const UNLINKED: &str = ph::LINK_SIMPLE_BREAK;
/// Open containing folder.
pub const CONTAINING: &str = ph::ARROW_SQUARE_OUT;
/// Restore from the trash.
pub const UNTRASH: &str = ph::ARROW_COUNTER_CLOCKWISE;
/// Favorites.
pub const STAR: &str = ph::STAR;
/// This Machine.
pub const MACHINE: &str = ph::DESKTOP_TOWER;
/// Cloud.
pub const CLOUD: &str = ph::CLOUD;
/// Eject removable media.
pub const EJECT: &str = ph::EJECT;
/// A drop the panel refuses.
pub const REFUSED: &str = ph::PROHIBIT;
/// Edit a place.
pub const EDIT: &str = ph::PENCIL_SIMPLE;
/// Add a connection or an account.
pub const ADD: &str = ph::PLUS_CIRCLE;
/// A job waiting for its volume.
pub const WAITING: &str = ph::HOURGLASS;

/// Line numbers in a text preview.
pub const LINE_NUMBERS: &str = ph::LIST_NUMBERS;
/// Word wrap in a text preview.
pub const WRAP: &str = ph::ARROW_U_DOWN_LEFT;
/// Pretty print structured text.
pub const PRETTY_PRINT: &str = ph::BRACKETS_CURLY;
/// Fit a raster to the pane.
pub const FIT: &str = ph::ARROWS_IN;
/// A raster at actual pixels.
pub const ACTUAL_SIZE: &str = ph::FRAME_CORNERS;
/// The File info badge.
pub const INFO: &str = ph::INFO;
/// The EXIF badge.
pub const CAMERA: &str = ph::CAMERA;
/// The Git badge.
pub const BRANCH: &str = ph::GIT_BRANCH;
/// The media stream badge.
pub const FILM: &str = ph::FILM_STRIP;
/// A tag list badge.
pub const TAG: &str = ph::TAG;
/// Letter case.
pub const TEXT_CASE: &str = ph::TEXT_AA;
/// Rotate.
pub const ROTATE: &str = ph::ARROWS_CLOCKWISE;
/// Zoom in.
pub const ZOOM_IN: &str = ph::MAGNIFYING_GLASS_PLUS;
/// Zoom out.
pub const ZOOM_OUT: &str = ph::MAGNIFYING_GLASS_MINUS;
/// A wrapped line's continuation mark in the gutter.
pub const CONTINUATION: &str = ph::ARROW_ELBOW_DOWN_RIGHT;

/// The Phosphor icons a place can be given, by name, as the Edit… dialogue
/// offers them: the folder kinds first, then what people keep one click
/// away. The name is what `places.toml` stores.
pub const PICKABLE: &[(&str, &str)] = &[
    ("FOLDER", ph::FOLDER),
    ("FOLDER_OPEN", ph::FOLDER_OPEN),
    ("FOLDER_STAR", ph::FOLDER_STAR),
    ("HOUSE", ph::HOUSE),
    ("DESKTOP", ph::DESKTOP),
    ("FILE_TEXT", ph::FILE_TEXT),
    ("FILES", ph::FILES),
    ("IMAGE", ph::IMAGE),
    ("CAMERA", ph::CAMERA),
    ("MUSIC_NOTES", ph::MUSIC_NOTES),
    ("FILM_STRIP", ph::FILM_STRIP),
    ("VIDEO_CAMERA", ph::VIDEO_CAMERA),
    ("DOWNLOAD_SIMPLE", ph::DOWNLOAD_SIMPLE),
    ("STAR", ph::STAR),
    ("HEART", ph::HEART),
    ("BOOKMARK", ph::BOOKMARK),
    ("FLAG", ph::FLAG),
    ("CLOCK_COUNTER_CLOCKWISE", ph::CLOCK_COUNTER_CLOCKWISE),
    ("TRASH", ph::TRASH),
    ("BRIEFCASE", ph::BRIEFCASE),
    ("CODE", ph::CODE),
    ("TERMINAL", ph::TERMINAL),
    ("BOOK", ph::BOOK),
    ("GRADUATION_CAP", ph::GRADUATION_CAP),
    ("PALETTE", ph::PALETTE),
    ("GAME_CONTROLLER", ph::GAME_CONTROLLER),
    ("ARCHIVE", ph::ARCHIVE),
    ("GEAR", ph::GEAR),
    ("WRENCH", ph::WRENCH),
    ("ROCKET", ph::ROCKET),
    ("LIGHTBULB", ph::LIGHTBULB),
    ("SHOPPING_CART", ph::SHOPPING_CART),
    ("GLOBE", ph::GLOBE),
    ("CLOUD", ph::CLOUD),
    ("NETWORK", ph::NETWORK),
    ("DESKTOP_TOWER", ph::DESKTOP_TOWER),
    ("HARD_DRIVE", ph::HARD_DRIVE),
    ("HARD_DRIVES", ph::HARD_DRIVES),
    ("USB", ph::USB),
    ("DISC", ph::DISC),
];

/// The glyph of a Phosphor icon name from [`PICKABLE`]; the folder for a
/// name that is not in the list.
#[must_use]
pub fn named(name: &str) -> &'static str {
    PICKABLE
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(FOLDER, |(_, glyph)| glyph)
}

/// The glyph for an extension's icon name.
#[must_use]
pub fn for_icon(icon: ext::Icon) -> &'static str {
    match icon {
        ext::Icon::Info => INFO,
        ext::Icon::Camera => CAMERA,
        ext::Icon::Branch => BRANCH,
        ext::Icon::Film => FILM,
        ext::Icon::PrettyPrint => PRETTY_PRINT,
        ext::Icon::TextCase => TEXT_CASE,
        ext::Icon::LineNumbers => LINE_NUMBERS,
        ext::Icon::Wrap => WRAP,
        ext::Icon::Fit => FIT,
        ext::Icon::ActualSize => ACTUAL_SIZE,
        ext::Icon::Previous => BACK,
        ext::Icon::Next => FORWARD,
        ext::Icon::Rotate => ROTATE,
        ext::Icon::ZoomIn => ZOOM_IN,
        ext::Icon::ZoomOut => ZOOM_OUT,
        // `Tag`, and whatever later versions add.
        _ => TAG,
    }
}

/// The badge glyph for a decorator's panel section, by the extension's id.
#[must_use]
pub fn for_decorator(id: &str) -> &'static str {
    match id {
        "git" => BRANCH,
        _ => TAG,
    }
}

/// Registers the Phosphor font with egui's defaults; call once per context.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // The fill weight marks an active toggle; it is its own family so a
    // glyph can ask for it.
    egui_phosphor::add_font_bytes_as_family(
        &mut fonts,
        FILL_FAMILY,
        egui_phosphor::Variant::Fill.font_bytes(),
    );
    ctx.set_fonts(fonts);
}

/// The font family of the fill weight, for active toggles.
pub const FILL_FAMILY: &str = "phosphor-fill";

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
        SEGMENT,
        DROPDOWN,
        SORT_ASC,
        SORT_DESC,
        VIEW_LIST,
        VIEW_DETAILS,
        VIEW_TREE,
        VIEW_ICONS,
        HOME,
        PIN,
        UNPIN,
        RECENT,
        NETWORK,
        TRASH,
        FOLDER,
        PROVIDER,
        LINK,
        LINE_NUMBERS,
        WRAP,
        PRETTY_PRINT,
        FIT,
        ACTUAL_SIZE,
        INFO,
        CAMERA,
        BRANCH,
        FILM,
        TAG,
        TEXT_CASE,
        ROTATE,
        ZOOM_IN,
        ZOOM_OUT,
        CONTINUATION,
        DONE,
        FAILED,
        TREE_CLOSED,
        TREE_OPEN,
        FOLDER_OPEN,
        PAUSE,
        PLAY,
        SUBTREE,
        USB,
        DISK,
        DISC,
        LINKED,
        UNLINKED,
        CONTAINING,
        UNTRASH,
        STAR,
        MACHINE,
        CLOUD,
        EJECT,
        REFUSED,
        EDIT,
        ADD,
        WAITING,
    ];

    #[test]
    fn the_installed_fonts_have_every_icon_the_shell_uses() {
        // kittest fails a frame that renders a missing glyph.
        let text: String = ALL
            .iter()
            .copied()
            .chain(PICKABLE.iter().map(|(_, glyph)| *glyph))
            .chain(
                ["a.png", "b.pdf", "c.zip", "d.rs", "e.unknownext", "f"]
                    .iter()
                    .map(|n| by_extension(n)),
            )
            .collect();
        assert_eq!(named("STAR"), STAR);
        assert_eq!(named("nope"), FOLDER);
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
