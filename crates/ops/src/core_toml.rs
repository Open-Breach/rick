//! `core.toml`, the file in the configuration directory where the core
//! actions remember their choices: one table per action (`[open_with]`,
//! `[hash]`), each key written on its own with the rest of the file kept.

use std::path::Path;

use types::Error;

/// The file's name in the configuration directory.
pub const FILE: &str = "core.toml";

/// The whole file; empty when it is missing or does not parse.
fn read(file: &Path) -> toml::Table {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .unwrap_or_default()
}

/// One table of the file; empty when it is absent.
pub(crate) fn table(file: &Path, name: &str) -> toml::Table {
    match read(file).remove(name) {
        Some(toml::Value::Table(inner)) => inner,
        _ => toml::Table::new(),
    }
}

/// Sets `key` of table `name` to `value`, keeping every other key and
/// table of the file.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub(crate) fn set(file: &Path, name: &str, key: &str, value: toml::Value) -> Result<(), Error> {
    let mut whole = read(file);
    let entry = whole
        .entry(name.to_owned())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !entry.is_table() {
        *entry = toml::Value::Table(toml::Table::new());
    }
    if let toml::Value::Table(inner) = entry {
        inner.insert(key.to_owned(), value);
    }
    let text = toml::to_string_pretty(&whole)
        .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(Error::from_io)?;
    }
    std::fs::write(file, text).map_err(Error::from_io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_set_in_its_table_and_the_rest_of_the_file_kept() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = dir.path().join("config").join(FILE);
        assert!(table(&file, "hash").is_empty(), "a missing file is empty");
        std::fs::create_dir_all(dir.path().join("config")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&file, "[other]\nkeep = 1\nhash = \"not a table\"\n")
            .unwrap_or_else(|e| panic!("{e}"));
        set(
            &file,
            "hash",
            "format",
            toml::Value::String("base64".to_owned()),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        set(
            &file,
            "open_with",
            "txt",
            toml::Value::String("editor".to_owned()),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let hash = table(&file, "hash");
        assert_eq!(hash.get("format").and_then(|v| v.as_str()), Some("base64"));
        assert_eq!(
            table(&file, "open_with")
                .get("txt")
                .and_then(|v| v.as_str()),
            Some("editor")
        );
        assert_eq!(
            table(&file, "other")
                .get("keep")
                .and_then(toml::Value::as_integer),
            Some(1)
        );
    }
}
