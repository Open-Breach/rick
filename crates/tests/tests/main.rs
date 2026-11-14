//! The integration tests of the workspace, as one test binary.
//!
//! Each file beside this one is a module of it. One binary instead of one
//! per file: one link instead of sixty-five, and the tests of all files run
//! side by side on the machine's cores, where the binaries ran one after
//! another. A new test file is added here as a module; cargo does not pick
//! it up by itself (`autotests = false` in `Cargo.toml`).
//!
//! One file's tests are run with its module's name as the filter:
//! `cargo test -p tests --test main -- listing::`.
//!
//! The tests share a process, and with it what the application has once
//! per process. Two cases so far. A test that changes such a thing for
//! everybody, the interface language, runs in a process of its own
//! (`tests::alone`). Tests that write the same such thing and read it
//! back, the Tracker connector's attention report, take turns
//! (`#[serial_test::serial(key)]`, one key per thing); a turn keeps them
//! apart from each other only, not from the tests without the attribute.

mod about;
mod actions;
mod app;
mod app_menu;
mod chrome;
mod clipboard_chords;
mod closeout;
mod cloud;
mod copy;
mod desktop_entry;
mod dnd;
mod extensions;
mod file_list;
mod fixture;
mod footer;
mod git;
mod hash;
mod install_dialog;
mod installer;
mod integration;
mod integration_dialog;
mod integration_linux;
mod integration_settings;
mod jobs;
mod launching;
mod list_view;
mod listing;
mod locales;
mod menus;
mod mount_linux;
mod opening;
mod panel_look;
mod panes;
mod paste_conflict;
mod permissions;
mod plugins;
mod preview;
mod preview_3d;
mod preview_icons;
mod preview_pane;
mod previewers;
mod properties;
mod providers;
mod readme;
mod real_chords;
mod remote_cap;
mod remote_providers;
mod removed_media;
mod screenshots;
mod search;
mod session;
mod settings_window;
mod shortcuts;
mod side_panel;
mod small_items;
mod ssh;
mod startup;
mod svn;
mod titlebar;
mod tracker_missing;
mod trash;
mod ui;
mod vpath;
mod window_geometry;
mod workspaces;
