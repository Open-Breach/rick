//! Acceptance (issue 139, item 10, SPEC.md *Title bar colour*): each of
//! the six presets shows its band in Light and in Dark, the choice
//! survives a restart, and the tab titles and icons on the band meet
//! 4.5:1 and 3:1 in every preset.
//!
//! With it: the setting's card on the Appearance page — a tile per
//! preset named after it, then *Custom…* with its two colour fields,
//! where a value that is not a colour is not taken — and what the title
//! row draws on a band that does not agree with the scheme.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use model::{Rgb, SettingsChange, Theme, TitlebarColour};
use platform::{ConfigDir, Fake};
use tests::local;
use ui_egui::theme::{self, Band, Scheme};
use ui_egui::{EguiHost, RickUi};

/// A window on `start`, sharing `fake` — and so `rick.toml` — with the
/// windows before it.
fn window(fake: &Arc<Fake>, start: &Path) -> Harness<'static, RickUi> {
    let host = Arc::new(EguiHost::new());
    let app = app::App::new(app::Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(start)),
    });
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| RickUi::new(app, host, &cc.egui_ctx));
    settle(&mut harness);
    harness
}

fn settle(harness: &mut Harness<'_, RickUi>) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for _ in 0..3 {
            harness.step();
        }
        if !harness.state().app().is_busy() {
            std::thread::sleep(Duration::from_millis(30));
            harness.step();
            if !harness.state().app().is_busy() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn steps(harness: &mut Harness<'_, RickUi>) {
    for _ in 0..4 {
        harness.step();
    }
}

fn change(harness: &mut Harness<'_, RickUi>, change: SettingsChange) {
    harness
        .state_mut()
        .app_mut()
        .handle(app::Command::Settings(change), &mut Vec::new());
    steps(harness);
}

/// The band the window draws its title row on, with what sits on it.
fn band(harness: &Harness<'_, RickUi>) -> Band {
    theme::band_of(&harness.ctx)
}

fn click(harness: &mut Harness<'_, RickUi>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).click();
    steps(harness);
}

/// The presets with their names and their light and dark value.
const PRESETS: [(TitlebarColour, &str, &str, &str); 6] = [
    (
        TitlebarColour::DustyRose,
        "Dusty rose",
        "#ece0db",
        "#332c2c",
    ),
    (TitlebarColour::WarmGrey, "Warm grey", "#e6e2dc", "#2c2a28"),
    (
        TitlebarColour::SlateTeal,
        "Slate teal",
        "#d9e4e4",
        "#263233",
    ),
    (TitlebarColour::Sage, "Sage", "#dfe6d8", "#283027"),
    (TitlebarColour::Plum, "Plum", "#e4dcea", "#3a2e4a"),
    (TitlebarColour::DarkBand, "Dark band", "#2f3237", "#2f3237"),
];

fn colour(hex: &str) -> egui::Color32 {
    theme::rgb(Rgb::parse(hex).unwrap_or_else(|| panic!("{hex}")))
}

#[test]
fn every_preset_shows_its_band_in_both_schemes_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());

    // Out of the box: Dusty rose, the token's own pair.
    change(&mut harness, SettingsChange::Theme(Theme::Light));
    assert_eq!(band(&harness).fill, Scheme::LIGHT.bg_titlebar);

    for (preset, name, light, dark) in PRESETS {
        change(&mut harness, SettingsChange::TitlebarColour(preset));
        for (scheme, hex) in [(Theme::Light, light), (Theme::Dark, dark)] {
            change(&mut harness, SettingsChange::Theme(scheme));
            let band = band(&harness);
            let at = format!("{name} in {scheme:?}");
            assert_eq!(band.fill, colour(hex), "{at}");
            // The titles of the inactive tabs at 4.5:1 and the icons at
            // 3:1, at rest and under the pointer.
            for (what, fg, bg, floor) in [
                ("title", band.title, band.fill, 4.5),
                ("title on a hovered tab", band.title, band.tab_hover, 4.5),
                ("icon", band.icon, band.fill, 3.0),
                ("icon on a hovered tab", band.icon, band.tab_hover, 3.0),
                ("icon on a hovered button", band.icon, band.hover, 3.0),
                ("search text", band.search_text, band.search_fill, 4.5),
            ] {
                let ratio = theme::contrast(fg, bg);
                assert!(ratio >= floor, "{at}: {what} {ratio:.2}");
            }
            // Only the Dark band disagrees with a scheme: with Light.
            let contrary = preset == TitlebarColour::DarkBand && scheme == Theme::Light;
            assert_eq!(band.contrary, contrary, "{at}");
        }
    }

    // The choice is in rick.toml, and the next start shows it.
    change(
        &mut harness,
        SettingsChange::TitlebarColour(TitlebarColour::Sage),
    );
    change(&mut harness, SettingsChange::Theme(Theme::Light));
    let settings = harness.state().app().settings_path().to_path_buf();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        settle(&mut harness);
        let text = std::fs::read_to_string(&settings).unwrap_or_default();
        if text.contains("titlebar_colour = \"sage\"") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the choice was not saved: {text}"
        );
    }
    drop(harness);
    let harness = window(&fake, dir.path());
    assert_eq!(
        harness.state().app().model().settings.titlebar_colour,
        TitlebarColour::Sage
    );
    assert_eq!(band(&harness).fill, colour("#dfe6d8"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_setting_is_a_tile_per_preset_and_custom_takes_two_colours() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let mut harness = window(&fake, dir.path());
    change(&mut harness, SettingsChange::Theme(Theme::Light));
    harness.state_mut().open_settings();
    steps(&mut harness);
    click(&mut harness, Role::RadioButton, "Appearance");
    harness.get_by_label("Title bar colour");
    harness.get_by_label_contains("The band behind the workspace tabs.");

    // A tile per preset, in the order of the table, then Custom; each is
    // named after its preset, and the default is the chosen one.
    let mut lefts = Vec::new();
    for name in PRESETS.map(|preset| preset.1) {
        let tile = harness.get_by_role_and_label(Role::RadioButton, name);
        let chosen = tile.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
        assert_eq!(chosen, name == "Dusty rose", "{name}");
        lefts.push((tile.rect().top(), tile.rect().left()));
    }
    let custom = harness.get_by_role_and_label(Role::RadioButton, "Custom…");
    lefts.push((custom.rect().top(), custom.rect().left()));
    assert!(
        lefts.windows(2).all(|pair| pair[0] < pair[1]),
        "in reading order: {lefts:?}"
    );

    // A click is the choice, and the title row has it at once.
    click(&mut harness, Role::RadioButton, "Plum");
    assert_eq!(
        harness.state().app().model().settings.titlebar_colour,
        TitlebarColour::Plum
    );
    assert_eq!(band(&harness).fill, colour("#e4dcea"));
    let plum = harness.get_by_role_and_label(Role::RadioButton, "Plum");
    assert_eq!(
        plum.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );

    // Custom: two fields appear, one per scheme, starting from the default
    // pair; the band is theirs.
    assert_eq!(
        harness
            .query_all_by_label_contains("Title bar colour, ")
            .count(),
        0
    );
    click(&mut harness, Role::RadioButton, "Custom…");
    assert_eq!(band(&harness).fill, colour("#ece0db"));
    let field = |harness: &Harness<'_, RickUi>, scheme: &str| {
        harness
            .get_by_label(&format!("Title bar colour, {scheme}"))
            .rect()
    };
    let light = field(&harness, "Light");
    let dark = field(&harness, "Dark");
    assert!(light.left() < dark.left() && (light.top() - dark.top()).abs() < 1.0);

    // What is typed is taken once it is a colour, and not before.
    let type_into = |harness: &mut Harness<'_, RickUi>, scheme: &str, text: &str| {
        let label = format!("Title bar colour, {scheme}");
        harness.get_by_label(&label).click();
        steps(harness);
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.step();
        harness.get_by_label(&label).type_text(text);
        steps(harness);
    };
    type_into(&mut harness, "Light", "#12");
    assert_eq!(
        harness.state().app().model().settings.titlebar_custom_light,
        Rgb([0xec, 0xe0, 0xdb]),
        "half a colour is not taken"
    );
    assert_eq!(band(&harness).fill, colour("#ece0db"));
    type_into(&mut harness, "Light", "#1d3a5f");
    assert_eq!(
        harness.state().app().model().settings.titlebar_custom_light,
        Rgb([0x1d, 0x3a, 0x5f])
    );
    // A dark colour of the user's in Light: the row turns to light text.
    let band_now = band(&harness);
    assert_eq!(band_now.fill, colour("#1d3a5f"));
    assert!(band_now.contrary);
    assert_eq!(
        band_now.icon,
        theme::over(egui::Color32::WHITE, 0.90, band_now.fill)
    );
    type_into(&mut harness, "Dark", "#e6e2dc");
    assert_eq!(
        harness.state().app().model().settings.titlebar_custom_dark,
        Rgb([0xe6, 0xe2, 0xdc])
    );
    // And a light one in Dark is the mirror image.
    change(&mut harness, SettingsChange::Theme(Theme::Dark));
    let band_now = band(&harness);
    assert_eq!(band_now.fill, colour("#e6e2dc"));
    assert!(band_now.contrary);
    assert_eq!(band_now.title, theme::over(Band::INK, 0.78, band_now.fill));

    // A preset again: the fields go, the custom pair is kept for later.
    click(&mut harness, Role::RadioButton, "Warm grey");
    assert_eq!(
        harness
            .query_all_by_label_contains("Title bar colour, ")
            .count(),
        0
    );
    assert_eq!(band(&harness).fill, colour("#2c2a28"));
    assert_eq!(
        harness.state().app().model().settings.titlebar_custom_light,
        Rgb([0x1d, 0x3a, 0x5f])
    );
}
