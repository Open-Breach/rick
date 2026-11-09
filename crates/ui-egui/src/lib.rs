//! The egui user interface: the eframe shell around [`app::App`].
//!
//! [`RickUi`] implements [`eframe::App`]. Each frame it polls the app for
//! what listings, watches and jobs delivered, turns key presses into
//! [`Intent`]s through the [`KeyMap`], draws every window from the model,
//! and hands the user's requests back to the app as `Command`s. Nothing here
//! mutates the model. [`dnd`] carries a drag of entries across the frame:
//! the rows are drag sources, the folders, panes, breadcrumbs, tab headers
//! and side-panel entries register as drop zones, and the end of the frame
//! turns a release into a `Command`. The first window is eframe's root
//! viewport; every further window of the model is a viewport of its own
//! drawn from the same state in the same frame, so all of them share one
//! app, one job table and one undo log.
//!
//! [`run`] opens the first window; the binary calls it after building the
//! app with an [`EguiHost`], which is how actions ask the user questions
//! without blocking a frame.

#![forbid(unsafe_code)]
// egui speaks f32 and rows are counted in usize; the conversions between
// counts, offsets and sizes are intended.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

pub mod about;
pub mod compare;
pub mod conflict;
pub mod console;
pub mod content;
pub mod dialog;
pub mod dnd;
pub mod dock;
pub mod fonts;
pub mod format;
pub mod frame;
pub mod hash;
pub mod host;
pub mod icons;
pub mod inline;
pub mod inspector;
pub mod keys;
pub mod menus;
pub mod operations;
pub mod overview;
pub mod palette;
pub mod pane;
pub mod paths;
pub mod portal;
pub mod preview;
pub mod properties;
pub mod question;
pub mod search;
pub mod settings_ui;
pub mod shell;
pub mod side_panel;
pub mod theme;
pub mod thumbs;
pub mod views;
pub mod widgets;

use std::sync::Arc;

use app::App;

pub use host::{Ask, EguiHost};
pub use inline::{EditKind, RowEdit};
pub use keys::{Intent, KeyMap};
pub use pane::{HANDLE_WIDTH, footer_height, toolbar_height};
pub use shell::{
    APP_ID, DOCK_TILE, DOCK_TILE_PITCH, Dispatched, Request, RickUi, TabUi, WindowUi,
    viewport_builder,
};

/// Opens the first window and runs until the last one closes. `build` is
/// what the About dialog shows and `rick --version` prints: the binary's
/// version, commit, target, build date, toolchain, contributors and the
/// generated component list ([`about::Build`]).
///
/// # Errors
///
/// The error eframe reports when the window or its renderer cannot be set
/// up.
pub fn run(title: &str, build: about::Build, app: App, host: Arc<EguiHost>) -> eframe::Result {
    // Rick draws its own title row. Where the system keeps its controls
    // (macOS), the title bar goes but the window keeps its buttons; elsewhere
    // the decorations go altogether. The restored session's focused window
    // is the first one; its geometry is applied by the first frame.
    let chrome = app.platform().window_chrome();
    let viewport = viewport_builder(title, chrome, None);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let mut app = Some(app);
    let mut build = Some(build);
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            let app = app.take().ok_or("the app was already taken")?;
            let build = build.take().unwrap_or_default();
            Ok(Box::new(
                RickUi::new(app, host, &cc.egui_ctx).with_build(build),
            ))
        }),
    )
}
