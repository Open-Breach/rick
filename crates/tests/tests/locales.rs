//! Acceptance (issue 24, localisation): every key of every German file
//! exists in the English one beside it, for the kernel's `locales/` and
//! every plugin's; the string-literal check passes over the egui UI; and
//! switching the language setting re-labels the Settings screen and the
//! context menu's actions without a restart, English coming back the same
//! way.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{App, Config};
use ext::platform::{ConfigDir as _, Fake};
use model::SettingsChange;
use tests::local;

/// The repository root: two directories above this crate.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The message keys of a Fluent file: the identifiers before `=` at the
/// start of a line, comments and blank lines skipped.
fn keys(path: &Path) -> BTreeSet<String> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with(' '))
        .filter_map(|line| {
            let (key, _) = line.split_once('=')?;
            let key = key.trim();
            (!key.is_empty()).then(|| key.to_owned())
        })
        .collect()
}

/// Every `locales/` directory of the repository: the kernel's and the
/// plugins'.
fn locale_dirs() -> Vec<PathBuf> {
    let root = repo();
    let mut dirs = vec![root.join("locales")];
    for entry in fs::read_dir(root.join("crates")).unwrap_or_else(|e| panic!("{e}")) {
        let dir = entry
            .unwrap_or_else(|e| panic!("{e}"))
            .path()
            .join("locales");
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
    dirs
}

#[test]
fn every_key_of_every_german_file_exists_in_its_english_one() {
    let mut checked = 0;
    for dir in locale_dirs() {
        let en = dir.join("en.ftl");
        assert!(en.is_file(), "{} has no en.ftl", dir.display());
        let english = keys(&en);
        assert!(!english.is_empty(), "{} is empty", en.display());
        for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("{e}")) {
            let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
            if path.extension().is_none_or(|e| e != "ftl") || path == en {
                continue;
            }
            let other = keys(&path);
            let missing: Vec<&String> = other.difference(&english).collect();
            assert!(
                missing.is_empty(),
                "{} has keys {missing:?} that {} lacks",
                path.display(),
                en.display()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 2,
        "the kernel and at least one plugin have a German file"
    );
}

#[test]
fn the_string_literal_check_passes_over_the_egui_ui() {
    let root = repo();
    let output = Command::new("python3")
        .arg(root.join(".github/scripts/check-strings.py"))
        .arg("--root")
        .arg(&root)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("python3 is not available ({error}); the check is not run here");
            return;
        }
    };
    assert!(
        output.status.success(),
        "check-strings failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn switching_the_language_relabels_the_settings_screen_and_the_menus_live() {
    use egui_kittest::kittest::Queryable as _;

    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fake = Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")));
    let host = Arc::new(ui_egui::EguiHost::new());
    let app = App::new(Config {
        platform: fake.clone(),
        ui: host.clone(),
        extensions: Vec::new(),
        config_dir: Some(fake.config_dir()),
        start: Some(local(dir.path())),
    });
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_step_dt(0.05)
        .build_eframe(move |cc| ui_egui::RickUi::new(app, host, &cc.egui_ctx));
    let deadline = Instant::now() + Duration::from_secs(30);
    while harness.state().app().is_busy() {
        harness.step();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    // English to begin with, whatever the machine speaks: the Settings
    // window and a core action.
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Language(Some("en".to_owned()))),
        &mut Vec::new(),
    );
    harness.state_mut().open_settings();
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label("Appearance").is_some(),
        "the Settings window in English"
    );
    let copy = harness
        .state()
        .app()
        .registry()
        .actions()
        .iter()
        .find(|a| a.id().0 == "core.copy")
        .map(|a| a.label());
    assert_eq!(copy.as_deref(), Some("Copy"));

    // German, through the setting, without a restart.
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Language(Some("de".to_owned()))),
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label("Erscheinungsbild").is_some(),
        "the Settings window in German"
    );
    assert!(harness.query_by_label("Appearance").is_none());
    assert!(
        harness.query_by_label("Sprache").is_some(),
        "the language row itself"
    );
    let copy = harness
        .state()
        .app()
        .registry()
        .actions()
        .iter()
        .find(|a| a.id().0 == "core.copy")
        .map(|a| a.label());
    assert_eq!(
        copy.as_deref(),
        Some("Kopieren"),
        "the context menu's actions follow"
    );
    assert_eq!(
        harness.state().app().model().settings.language.as_deref(),
        Some("de")
    );

    // And back.
    harness.state_mut().app_mut().handle(
        app::Command::Settings(SettingsChange::Language(Some("en".to_owned()))),
        &mut Vec::new(),
    );
    harness.step();
    harness.step();
    assert!(harness.query_by_label("Appearance").is_some());
}
