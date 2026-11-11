//! Acceptance (issue 149, item 4, the file list), in the running window
//! (SPEC.md *List view*, *Sorting*, *Dates*, *Inline rename*, *Select
//! all*).
//!
//! A folder of 100 files in the List view shows aligned columns of one
//! width, filled top to bottom, and scrolls sideways only; a name of 120
//! characters widens no column beyond a third of the pane and is shown
//! whole on hover. Up and Down move within a column and on into the next,
//! Left and Right jump a column. A second click renames only where the
//! setting says so, while F2 always does. Sorting by *Modified* puts a
//! file from yesterday above one from August, whatever the dates read.
//! The date format is a setting the list follows at once. *Select all*
//! with hidden files not shown says how many it left out.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT as _, Queryable as _};
use ext::Level;
use model::{Column, DateFormat, SettingsChange, SortDir, ViewMode};
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::views::LIST_COLUMN_MIN;
use ui_egui::{EguiHost, RickUi};

/// A window on `start`, 1200 × 700 at one pixel per point.
fn window(start: &Path) -> Harness<'static, RickUi> {
    window_of(start, egui::vec2(1200.0, 700.0))
}

/// A window of `size` on `start`, at one pixel per point.
fn window_of(start: &Path, size: egui::Vec2) -> Harness<'static, RickUi> {
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_step_dt(0.05)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn pane(harness: &Harness<'_, RickUi>) -> model::PaneId {
    harness.state().app().model().focused
}

fn tab<'a>(harness: &'a Harness<'_, RickUi>) -> (model::TabId, &'a model::Tab) {
    harness
        .state()
        .app()
        .model()
        .active_tab(pane(harness))
        .unwrap_or_else(|| panic!("no tab"))
}

fn names(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.rows().map(|e| e.name.clone()).collect()
}

fn selected(harness: &Harness<'_, RickUi>) -> Vec<String> {
    tab(harness).1.selected().map(|e| e.name.clone()).collect()
}

fn command(harness: &mut Harness<'_, RickUi>, command: app::Command) {
    harness
        .state_mut()
        .app_mut()
        .handle(command, &mut Vec::new());
    harness.step();
}

/// Runs frames until the app has nothing in flight and a quiet moment
/// passed.
fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(50));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Runs frames until `done` holds.
fn step_until(
    harness: &mut Harness<'_, RickUi>,
    what: &str,
    mut done: impl FnMut(&Harness<'_, RickUi>) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness) {
        harness.step();
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        harness.step();
    }
}

fn click_at(harness: &mut Harness<'_, RickUi>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.step();
}

fn press(harness: &mut Harness<'_, RickUi>, key: egui::Key) {
    harness.key_press(key);
    harness.step();
    harness.step();
}

/// The focused pane's rectangle.
fn pane_rect(harness: &Harness<'_, RickUi>) -> egui::Rect {
    let pane = pane(harness);
    harness
        .state()
        .pane_rects()
        .into_iter()
        .find(|(p, _)| *p == pane)
        .map_or_else(|| panic!("no pane rect"), |(_, r)| r)
}

/// The rectangle of the list row named `name`, when it is drawn.
fn row(harness: &Harness<'_, RickUi>, name: &str) -> Option<egui::Rect> {
    let pane = pane_rect(harness);
    harness
        .query_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| pane.contains(r.center()))
}

/// The toasts shown, with their level.
fn toasts(harness: &Harness<'_, RickUi>) -> Vec<(Level, String)> {
    harness.state().toasts()
}

/// A name of 120 characters that ends in its extension. Cut at 120 after
/// the extension was added it ended in a dot, and Windows stores a name
/// without its trailing dot: the file was listed, under another name.
fn long_name() -> String {
    format!("{}.txt", &"a-very-long-name-".repeat(7)[..116])
}

/// A folder of 100 files with short names, `file-000.txt` on.
fn hundred() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for i in 0..100 {
        fs::write(dir.path().join(format!("file-{i:03}.txt")), "x")
            .unwrap_or_else(|e| panic!("{e}"));
    }
    dir
}

fn list_view(harness: &mut Harness<'_, RickUi>) -> ui_egui::views::ListGrid {
    let pane = pane(harness);
    command(
        harness,
        app::Command::SetView {
            pane,
            mode: ViewMode::List,
        },
    );
    settle(harness);
    harness
        .state()
        .list_grid(tab(harness).0)
        .unwrap_or_else(|| panic!("the List view was not drawn"))
}

/// A folder of 100 files in the List view: aligned columns of one width,
/// filled top to bottom and then to the right, scrolling sideways only;
/// one name of 120 characters widens no column beyond a third of the pane
/// and is shown whole on hover.
#[test]
// One folder looked at from every side the acceptance point names.
#[allow(clippy::too_many_lines)]
fn a_hundred_files_fill_aligned_columns_of_one_width_and_scroll_sideways_only() {
    let dir = hundred();
    // A window the hundred do not fit in side by side: there is something
    // to scroll.
    let mut harness = window_of(dir.path(), egui::vec2(1000.0, 480.0));
    let grid = list_view(&mut harness);
    let pane = pane_rect(&harness);
    assert_eq!(names(&harness).len(), 100);
    assert!(grid.rows > 1, "{grid:?}");
    assert!(
        grid.column_width >= LIST_COLUMN_MIN && grid.column_width <= pane.width() / 3.0 + 0.5,
        "{grid:?} in a pane {} wide",
        pane.width()
    );

    // Top to bottom, then the next column to the right.
    let first = row(&harness, "file-000.txt").unwrap_or_else(|| panic!("no first row"));
    let second = row(&harness, "file-001.txt").unwrap_or_else(|| panic!("no second row"));
    assert!(
        (second.left() - first.left()).abs() < 0.5,
        "the same column"
    );
    assert!(
        (second.top() - first.bottom()).abs() < 0.5,
        "directly beneath: {first:?} {second:?}"
    );
    let next_column = format!("file-{:03}.txt", grid.rows);
    let top_of_second = row(&harness, &next_column).unwrap_or_else(|| panic!("no second column"));
    assert!(
        (top_of_second.top() - first.top()).abs() < 0.5,
        "at the top"
    );
    assert!(
        (top_of_second.left() - first.left() - grid.column_width).abs() < 0.5,
        "one column width to the right: {first:?} {top_of_second:?}"
    );
    // The last row of a column is whole, inside the pane.
    let bottom = format!("file-{:03}.txt", grid.rows - 1);
    let bottom = row(&harness, &bottom).unwrap_or_else(|| panic!("no last row of the column"));
    assert!(
        bottom.bottom() <= pane.bottom() + 0.5,
        "{bottom:?} in {pane:?}"
    );

    // Every row drawn has the one width and sits on a column's left edge:
    // no ragged rows, no column of a width of its own.
    let mut drawn = 0;
    for name in names(&harness) {
        let Some(rect) = row(&harness, &name) else {
            continue;
        };
        drawn += 1;
        assert!(
            (rect.width() - grid.column_width).abs() < 0.5,
            "{name}: {rect:?} against {grid:?}"
        );
        let columns = (rect.left() - first.left()) / grid.column_width;
        assert!(
            (columns - columns.round()).abs() < 0.01,
            "{name} is off the columns: {rect:?}"
        );
        assert!(rect.top() >= first.top() - 0.5 && rect.bottom() <= pane.bottom() + 0.5);
    }
    assert!(drawn >= grid.rows * 2, "only {drawn} rows drawn");

    // The wheel moves the columns sideways, and nothing up or down.
    let at = pane.center();
    harness.event(egui::Event::PointerMoved(at));
    harness.step();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..30 {
        harness.step();
    }
    let moved = row(&harness, &next_column).map(|r| (r.left(), r.top()));
    match moved {
        Some((left, top)) => {
            assert!(left < top_of_second.left() - 50.0, "moved left: {left}");
            assert!(
                (top - top_of_second.top()).abs() < 0.5,
                "and not down: {top}"
            );
        }
        // Scrolled out to the left altogether.
        None => assert!(row(&harness, "file-000.txt").is_none()),
    }
    // All the way to the right: the last file, still within the rows.
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -20_000.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    for _ in 0..40 {
        harness.step();
    }
    let last = row(&harness, "file-099.txt").unwrap_or_else(|| panic!("the last file in view"));
    assert!(last.top() >= first.top() - 0.5 && last.bottom() <= pane.bottom() + 0.5);

    // One name of 120 characters: the columns get no wider than a third of
    // the pane, the name is cut to its column, and hovering shows it whole.
    let long = long_name();
    assert_eq!(long.chars().count(), 120);
    fs::write(dir.path().join(&long), "x").unwrap_or_else(|e| panic!("{e}"));
    step_until(&mut harness, "the long name is listed", |h| {
        names(h).len() == 101
    });
    settle(&mut harness);
    let with_long = harness
        .state()
        .list_grid(tab(&harness).0)
        .unwrap_or_else(|| panic!("no grid"));
    assert!(
        with_long.column_width <= pane.width() / 3.0 + 0.5,
        "{with_long:?}"
    );
    assert!(
        with_long.column_width - grid.column_width < 30.0,
        "one long name among a hundred widens the columns by {}",
        with_long.column_width - grid.column_width
    );
    // `a-very-long…` sorts first: back to the left edge to see it.
    press(&mut harness, egui::Key::Home);
    settle(&mut harness);
    let cell = row(&harness, &long).unwrap_or_else(|| {
        panic!(
            "the long name's row; the list starts with {:?}",
            names(&harness).first()
        )
    });
    assert!(
        (cell.width() - with_long.column_width).abs() < 0.5,
        "{cell:?}"
    );
    harness.event(egui::Event::PointerMoved(cell.center()));
    step_until(&mut harness, "the whole name on hover", |h| {
        h.query_all_by_label(&long).count() >= 2
    });
}

/// Up and Down move within a column and go on into the next at its end;
/// Left and Right jump a column and keep the row, landing on the last
/// entry where the column is shorter; Home and End go to the ends.
#[test]
fn the_arrow_keys_move_within_and_between_the_columns() {
    let dir = hundred();
    let mut harness = window(dir.path());
    let grid = list_view(&mut harness);
    let rows = grid.rows;
    let name = |index: usize| format!("file-{index:03}.txt");

    press(&mut harness, egui::Key::Home);
    assert_eq!(selected(&harness), [name(0)]);
    press(&mut harness, egui::Key::ArrowDown);
    press(&mut harness, egui::Key::ArrowDown);
    assert_eq!(selected(&harness), [name(2)]);
    // Right keeps the row; Left comes back.
    press(&mut harness, egui::Key::ArrowRight);
    assert_eq!(selected(&harness), [name(rows + 2)]);
    press(&mut harness, egui::Key::ArrowLeft);
    assert_eq!(selected(&harness), [name(2)]);
    // At the first column Left stays.
    press(&mut harness, egui::Key::ArrowLeft);
    assert_eq!(selected(&harness), [name(2)]);
    // Down at the end of a column goes on into the next one.
    for _ in 2..rows - 1 {
        press(&mut harness, egui::Key::ArrowDown);
    }
    assert_eq!(selected(&harness), [name(rows - 1)]);
    press(&mut harness, egui::Key::ArrowDown);
    assert_eq!(selected(&harness), [name(rows)]);
    press(&mut harness, egui::Key::ArrowUp);
    assert_eq!(selected(&harness), [name(rows - 1)]);

    // End is the last entry; from the last row of the column before it,
    // Right lands on the last entry where the last column is shorter.
    press(&mut harness, egui::Key::End);
    assert_eq!(selected(&harness), [name(99)]);
    let columns = 100usize.div_ceil(rows);
    let in_last = 100 - (columns - 1) * rows;
    if in_last < rows && columns >= 2 {
        press(&mut harness, egui::Key::ArrowLeft);
        let row_kept = (columns - 2) * rows + (in_last - 1);
        assert_eq!(selected(&harness), [name(row_kept)]);
        // The last row of that column has no neighbour to its right.
        for _ in in_last..rows {
            press(&mut harness, egui::Key::ArrowDown);
        }
        assert_eq!(selected(&harness), [name((columns - 1) * rows - 1)]);
        press(&mut harness, egui::Key::ArrowRight);
        assert_eq!(selected(&harness), [name(99)]);
    }
    // The cursor is in view after each move.
    assert!(row(&harness, &name(99)).is_some());
}

/// A second click on the selected row renames only with the setting on,
/// which it is not by default; F2 renames either way.
#[test]
fn a_second_click_renames_only_where_the_setting_says_so() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = window(dir.path());
    let tab_id = tab(&harness).0;
    assert!(!harness.state().app().model().settings.rename_on_click);
    let slow_second_click = |harness: &mut Harness<'static, RickUi>| {
        let at = row(harness, "a.txt")
            .unwrap_or_else(|| panic!("no row"))
            .center();
        click_at(harness, at);
        // Well past the double-click window: a second of frames.
        for _ in 0..20 {
            harness.step();
        }
        click_at(harness, at);
        harness.step();
        harness.step();
    };

    slow_second_click(&mut harness);
    assert_eq!(selected(&harness), ["a.txt"]);
    assert!(
        harness.state().editing(tab_id).is_none(),
        "off by default: a second click renames nothing"
    );
    // F2 always renames.
    press(&mut harness, egui::Key::F2);
    assert!(harness.state().editing(tab_id).is_some());
    press(&mut harness, egui::Key::Escape);
    assert!(harness.state().editing(tab_id).is_none());

    // With the setting on, the second click edits the name in place.
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::RenameOnClick(true)),
    );
    // A click elsewhere first, so the next two clicks are a pair of their
    // own.
    let other = row(&harness, "b.txt")
        .unwrap_or_else(|| panic!("no row"))
        .center();
    click_at(&mut harness, other);
    slow_second_click(&mut harness);
    assert!(
        harness.state().editing(tab_id).is_some(),
        "on: the second click renames"
    );
}

/// Sorting by *Modified* sorts the timestamp, never the label: descending,
/// a file from yesterday stands above one from August and one from last
/// year, though *Yesterday*, *Aug 24* and *Aug 24 2025* read otherwise;
/// and the date format is a setting the column follows at once.
#[test]
fn modified_sorts_by_the_timestamp_and_the_dates_follow_the_setting() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let day = Duration::from_hours(24);
    let now = SystemTime::now();
    for (name, age) in [
        ("yesterday.txt", day),
        ("august.txt", day * 80),
        ("last-year.txt", day * 400),
        ("today.txt", Duration::ZERO),
    ] {
        let path = dir.path().join(name);
        fs::write(&path, name).unwrap_or_else(|e| panic!("{e}"));
        fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(now - age))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = window(dir.path());
    let pane = pane(&harness);
    command(
        &mut harness,
        app::Command::Sort {
            pane,
            by: Column::Modified,
            dir: SortDir::Descending,
        },
    );
    settle(&mut harness);
    assert_eq!(
        names(&harness),
        ["today.txt", "yesterday.txt", "august.txt", "last-year.txt"]
    );
    command(
        &mut harness,
        app::Command::Sort {
            pane,
            by: Column::Modified,
            dir: SortDir::Ascending,
        },
    );
    settle(&mut harness);
    assert_eq!(
        names(&harness),
        ["last-year.txt", "august.txt", "yesterday.txt", "today.txt"]
    );

    // Smart with the time is the default: the clock stands with the day.
    assert_eq!(
        harness.state().app().model().settings.date_format,
        DateFormat::SmartTime
    );
    let labelled = |h: &Harness<'_, RickUi>, start: &str| {
        h.query_all_by_label_contains(start).next().is_some()
    };
    assert!(labelled(&harness, "Today "), "Today with its time");
    assert!(labelled(&harness, "Yesterday "), "Yesterday with its time");
    // Without the time.
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::DateFormat(DateFormat::Smart)),
    );
    harness.step();
    assert!(harness.query_by_label("Today").is_some());
    assert!(harness.query_by_label("Yesterday").is_some());
    // ISO: every date as `YYYY-MM-DD HH:MM`.
    command(
        &mut harness,
        app::Command::Settings(SettingsChange::DateFormat(DateFormat::Iso)),
    );
    harness.step();
    assert!(harness.query_by_label("Today").is_none());
    let iso = ui_egui::format::date(Some(now - day), now, DateFormat::Iso).0;
    assert_eq!(iso.len(), 16, "{iso}");
    assert!(harness.query_by_label(&iso).is_some(), "{iso}");
}

/// *Select all* with hidden files that are not shown selects the visible
/// entries only, and a toast names how many it left out.
#[test]
fn select_all_says_how_many_hidden_items_it_left_out() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    for name in ["a.txt", "b.txt", ".one", ".two", ".three"] {
        fs::write(dir.path().join(name), name).unwrap_or_else(|e| panic!("{e}"));
    }
    let mut harness = window(dir.path());
    assert_eq!(names(&harness), ["a.txt", "b.txt"]);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.step();
    harness.step();
    assert_eq!(selected(&harness), ["a.txt", "b.txt"]);
    assert_eq!(
        toasts(&harness),
        [(Level::Info, "3 hidden items not selected".to_owned())]
    );
}

/// The two settings of this item are where the spec puts them: *Date
/// format* on the Appearance page, a dropdown of the four formats with
/// *Smart with time* chosen; *Rename on a second click* on the Behaviour
/// page, a switch that is off.
#[test]
fn the_date_format_and_the_rename_switch_are_in_the_settings() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let mut harness = window(dir.path());
    let steps = |harness: &mut Harness<'static, RickUi>| {
        for _ in 0..4 {
            harness.step();
        }
    };
    harness.state_mut().open_settings();
    steps(&mut harness);

    harness
        .get_by_role_and_label(Role::RadioButton, "Appearance")
        .click();
    steps(&mut harness);
    harness.get_by_label("Date format");
    // The dropdown shows the chosen format: smart with the time.
    harness
        .get(By::new().role(Role::ComboBox).value("Smart with time"))
        .click();
    steps(&mut harness);
    // The theme has a *System* of its own on this page: with the list
    // open there are two.
    for (option, shown) in [("Smart without time", 1), ("ISO", 1), ("System", 2)] {
        assert_eq!(
            harness.query_all_by_label(option).count(),
            shown,
            "{option}"
        );
    }
    harness.get_by_label("ISO").click();
    steps(&mut harness);
    assert_eq!(
        harness.state().app().model().settings.date_format,
        DateFormat::Iso
    );

    harness
        .get_by_role_and_label(Role::RadioButton, "Behaviour")
        .click();
    steps(&mut harness);
    let switch = harness.get_by_role_and_label(Role::CheckBox, "Rename on a second click");
    assert_ne!(
        switch.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "off by default"
    );
    switch.click();
    steps(&mut harness);
    assert!(harness.state().app().model().settings.rename_on_click);
}
