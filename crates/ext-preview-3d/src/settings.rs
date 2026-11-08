//! The triangle budget: declared as the extension's one setting, which the
//! Settings screen renders and writes to the extension's own TOML file in
//! the config directory, `preview-3d.toml`, and read from that file at
//! every open, so a change takes effect for the next file (QUESTIONS.md
//! 136).

use std::path::Path;

use ext::{SettingField, SettingKind, SettingsSchema};

/// The extension's settings file, in the config directory.
pub const FILE: &str = "preview-3d.toml";
/// The key of the triangle budget in that file.
pub const KEY: &str = "triangle_budget";
/// The budget when the file or the key is missing.
pub const DEFAULT_BUDGET: usize = 2_000_000;
/// The smallest budget the Settings screen accepts.
pub const MIN_BUDGET: i64 = 1_000;
/// The largest budget the Settings screen accepts.
pub const MAX_BUDGET: i64 = 20_000_000;

/// The schema the extension registers.
#[must_use]
pub fn schema() -> SettingsSchema {
    SettingsSchema {
        ext: crate::ID,
        fields: vec![SettingField {
            key: KEY,
            label: "Triangle budget for previews",
            kind: SettingKind::Integer {
                default: i64::try_from(DEFAULT_BUDGET).unwrap_or(i64::MAX),
                min: MIN_BUDGET,
                max: MAX_BUDGET,
            },
        }],
    }
}

/// The budget `text`, the settings file, sets, clamped into the bounds as
/// the application clamps every integer setting; `None` when the file
/// does not set it or is not TOML.
#[must_use]
pub fn parse_budget(text: &str) -> Option<usize> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let value = table.get(KEY)?.as_integer()?;
    usize::try_from(value.clamp(MIN_BUDGET, MAX_BUDGET)).ok()
}

/// The triangle budget: the settings file's when the config directory is
/// known and the file sets it, the default otherwise.
#[must_use]
pub fn triangle_budget(config_dir: Option<&Path>) -> usize {
    let Some(dir) = config_dir else {
        return DEFAULT_BUDGET;
    };
    let path = dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_budget(&text).unwrap_or_else(|| {
            tracing::warn!(path = %path.display(), "no valid {KEY} in the settings file");
            DEFAULT_BUDGET
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DEFAULT_BUDGET,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "settings file not read");
            DEFAULT_BUDGET
        }
    }
}
