//! The settings extensions declare ([`SettingsSchema`]): their values,
//! one TOML file per extension in the config directory, `<id>.toml`, as
//! the spec's Configuration section asks. The Settings screen renders the
//! schemas and changes land here; an extension reads and writes its own
//! through the same store, which the registry lends it as the
//! [`SettingsStore`] (issue 50), so a value a plugin wrote back (the path
//! of the library it found) is what the Settings screen shows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use ext::{SettingKind, SettingValue, SettingsSchema, SettingsStore};
use tracing::warn;
use types::{Error, ExtId};

/// The values of every declared setting, by extension id and key; shared
/// between the application and the extensions.
#[derive(Debug, Default)]
pub struct ExtSettings {
    dir: PathBuf,
    values: Mutex<BTreeMap<String, BTreeMap<String, SettingValue>>>,
}

/// The file of an extension's settings.
#[must_use]
pub fn file_of(dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{ext}.toml"))
}

/// The value of `kind` that a TOML value holds, if it fits.
fn value_of(kind: &SettingKind, value: &toml::Value) -> Option<SettingValue> {
    match (kind, value) {
        (SettingKind::Bool { .. }, toml::Value::Boolean(b)) => Some(SettingValue::Bool(*b)),
        (SettingKind::Text { .. }, toml::Value::String(s)) => Some(SettingValue::Text(s.clone())),
        (SettingKind::Integer { min, max, .. }, toml::Value::Integer(i)) => {
            Some(SettingValue::Integer((*i).clamp(*min, *max)))
        }
        (SettingKind::Choice { options, .. }, toml::Value::String(s)) => options
            .iter()
            .any(|o| *o == s)
            .then(|| SettingValue::Choice(s.clone())),
        _ => None,
    }
}

/// A setting's value as TOML writes it.
fn toml_of(value: &SettingValue) -> toml::Value {
    match value {
        SettingValue::Bool(b) => toml::Value::Boolean(*b),
        SettingValue::Text(s) | SettingValue::Choice(s) => toml::Value::String(s.clone()),
        SettingValue::Integer(i) => toml::Value::Integer(*i),
    }
}

fn read_table(path: &Path) -> Result<toml::Table, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(toml::Table::new()),
        Err(error) => return Err(Error::from_io(error)),
    };
    text.parse::<toml::Table>().map_err(|error| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        ))
    })
}

impl ExtSettings {
    /// An empty store over `dir`, before any schema is known: what the
    /// registry lends the extensions while they register; [`load`](Self::load)
    /// fills it once every schema is in.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            values: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, BTreeMap<String, SettingValue>>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads every schema's file under the store's directory; a missing
    /// file, a missing key or a value of the wrong type means the schema's
    /// default. Values set before the load are kept.
    pub fn load(&self, schemas: &[SettingsSchema]) {
        let dir = self.dir.clone();
        let mut values = self.lock();
        for schema in schemas {
            let path = file_of(&dir, schema.ext.0);
            let table = match read_table(&path) {
                Ok(table) => table,
                Err(error) => {
                    warn!(%error, path = %path.display(), "extension settings not read");
                    toml::Table::new()
                }
            };
            let fields = values.entry(schema.ext.0.to_owned()).or_default();
            for field in &schema.fields {
                if fields.contains_key(field.key) {
                    continue;
                }
                let value = table
                    .get(field.key)
                    .and_then(|v| value_of(&field.kind, v))
                    .unwrap_or_else(|| field.kind.default_value());
                fields.insert(field.key.to_owned(), value);
            }
        }
    }

    /// The value of a setting; `None` when no schema declares it.
    #[must_use]
    pub fn get(&self, ext: &str, key: &str) -> Option<SettingValue> {
        self.lock().get(ext)?.get(key).cloned()
    }

    /// Stores a value and writes the extension's file, keeping the keys the
    /// file holds that no schema names.
    ///
    /// # Errors
    ///
    /// The I/O error from reading or writing the file.
    pub fn set(&self, ext: &str, key: &str, value: SettingValue) -> Result<(), Error> {
        let path = file_of(&self.dir, ext);
        let mut table = read_table(&path)?;
        table.insert(key.to_owned(), toml_of(&value));
        let text = toml::to_string_pretty(&table)
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        crate::session::write_atomically(&path, &text)?;
        self.lock()
            .entry(ext.to_owned())
            .or_default()
            .insert(key.to_owned(), value);
        Ok(())
    }
}

impl SettingsStore for ExtSettings {
    fn get(&self, ext: ExtId, key: &str) -> Option<SettingValue> {
        Self::get(self, ext.0, key)
    }

    fn set(&self, ext: ExtId, key: &str, value: SettingValue) -> Result<(), Error> {
        Self::set(self, ext.0, key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ext::SettingField;
    use ext::Text;
    use types::ExtId;

    fn schema() -> SettingsSchema {
        SettingsSchema {
            ext: ExtId("demo"),
            fields: vec![
                SettingField {
                    key: "console",
                    label: Text("Console"),
                    kind: SettingKind::Choice {
                        options: vec!["embedded", "system"],
                        default: 0,
                    },
                },
                SettingField {
                    key: "limit",
                    label: Text("Limit"),
                    kind: SettingKind::Integer {
                        default: 5,
                        min: 1,
                        max: 10,
                    },
                },
            ],
        }
    }

    #[test]
    fn values_default_then_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let settings = ExtSettings::new(dir.path());
        settings.load(&[schema()]);
        assert_eq!(
            settings.get("demo", "console"),
            Some(SettingValue::Choice("embedded".into()))
        );
        assert_eq!(
            settings.get("demo", "limit"),
            Some(SettingValue::Integer(5))
        );
        assert_eq!(settings.get("demo", "missing"), None);
        settings
            .set("demo", "console", SettingValue::Choice("system".into()))
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(
            file_of(dir.path(), "demo"),
            format!(
                "{}limit = 99\nextra = \"kept\"\n",
                std::fs::read_to_string(file_of(dir.path(), "demo")).unwrap_or_default()
            ),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let again = ExtSettings::new(dir.path());
        again.load(&[schema()]);
        assert_eq!(
            again.get("demo", "console"),
            Some(SettingValue::Choice("system".into()))
        );
        // Out of range clamps; an unknown choice falls back to the default.
        assert_eq!(again.get("demo", "limit"), Some(SettingValue::Integer(10)));
        let text = std::fs::read_to_string(file_of(dir.path(), "demo")).unwrap_or_default();
        assert!(text.contains("extra = \"kept\""));
    }
}
