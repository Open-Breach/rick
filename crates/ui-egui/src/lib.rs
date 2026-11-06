//! The egui user interface: the eframe shell around [`app::App`].
//!
//! [`RickUi`] implements [`eframe::App`]. Each frame it polls the app for
//! what listings, watches and jobs delivered, turns key presses into
//! [`Intent`]s through the [`KeyMap`], draws everything from the model, and
//! hands the user's requests back to the app as `Command`s. Nothing here
//! mutates the model.
//!
//! [`run`] opens the window; the binary calls it after building the app with
//! an [`EguiHost`], which is how actions ask the user questions without
//! blocking a frame.

#![forbid(unsafe_code)]
// egui speaks f32 and rows are counted in usize; the conversions between
// counts, offsets and sizes are intended.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]

pub mod dock;
pub mod format;
pub mod glyphs;
pub mod host;
pub mod inspector;
pub mod keys;
pub mod menus;
pub mod palette;
pub mod pane;
pub mod paths;
pub mod preview;
pub mod settings_ui;
pub mod shell;
pub mod side_panel;
pub mod views;

use std::sync::Arc;

use app::App;

pub use host::{Ask, EguiHost};
pub use keys::{Intent, KeyMap};
pub use shell::{Dispatched, Request, RickUi, TabUi};

/// Opens the window and runs until it closes.
///
/// # Errors
///
/// The error eframe reports when the window or its renderer cannot be set
/// up.
pub fn run(title: &str, app: App, host: Arc<EguiHost>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    let mut app = Some(app);
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            let app = app.take().ok_or("the app was already taken")?;
            Ok(Box::new(RickUi::new(app, host, &cc.egui_ctx)))
        }),
    )
}
