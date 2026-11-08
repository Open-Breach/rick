//! Forms: the small dialogues an action asks the UI to show before it runs.
//!
//! An action that needs more than a line of text or a yes/no — an archive
//! format and a name, a rename pattern with a live preview, an application
//! to open with — describes a [`Form`] and hands it to
//! [`UiHost::form`](crate::UiHost::form). The UI lays the fields out,
//! shows the preview the form computes from the values as they are typed,
//! and returns the [`Values`] when the user confirms. The form is data: the
//! action owns what the fields mean, the UI owns how they look.
//!
//! # Worked example
//!
//! A form with a text field, a choice and a preview, answered by a host
//! that fills in the defaults.
//!
//! ```
//! use std::sync::Arc;
//!
//! use ext::{Field, FieldKind, Form, PreviewRow, Values};
//!
//! let form = Form::new("Compress to…", "Compress")
//!     .field(Field::text("name", "Archive name", "photos.zip", ""))
//!     .field(Field::choice("format", "Format", vec!["zip".into(), "tar.gz".into()], 0))
//!     .preview(Arc::new(|values: &Values| {
//!         vec![PreviewRow {
//!             from: "3 items".into(),
//!             to: values.text("name").to_owned(),
//!             problem: None,
//!         }]
//!     }));
//!
//! // What a host does when it answers with the fields as they are.
//! let values = form.defaults();
//! assert_eq!(values.text("name"), "photos.zip");
//! assert_eq!(values.choice("format"), 0);
//! let rows = form.preview_rows(&values);
//! assert_eq!(rows[0].to, "photos.zip");
//! ```

use std::fmt;
use std::sync::Arc;

/// Computes the preview rows for the values typed so far.
pub type Preview = Arc<dyn Fn(&Values) -> Vec<PreviewRow> + Send + Sync>;

/// A dialogue an action asks the UI to show.
#[derive(Clone)]
pub struct Form {
    /// The dialogue's title.
    pub title: String,
    /// The label of the confirming button.
    pub ok: String,
    /// The fields, in display order.
    pub fields: Vec<Field>,
    /// The live preview, if the form has one: computed again whenever a
    /// value changes; a row with a problem disables the confirming button.
    pub preview: Option<Preview>,
}

impl fmt::Debug for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Form")
            .field("title", &self.title)
            .field("ok", &self.ok)
            .field("fields", &self.fields)
            .field("preview", &self.preview.is_some())
            .finish()
    }
}

impl Form {
    /// A form with no fields yet.
    #[must_use]
    pub fn new(title: impl Into<String>, ok: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ok: ok.into(),
            fields: Vec::new(),
            preview: None,
        }
    }

    /// Adds a field.
    #[must_use]
    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Sets the live preview.
    #[must_use]
    pub fn preview(mut self, preview: Preview) -> Self {
        self.preview = Some(preview);
        self
    }

    /// The values the form opens with.
    #[must_use]
    pub fn defaults(&self) -> Values {
        Values(
            self.fields
                .iter()
                .map(|field| (field.key, field.kind.default()))
                .collect(),
        )
    }

    /// The preview rows for `values`; empty when the form has no preview.
    #[must_use]
    pub fn preview_rows(&self, values: &Values) -> Vec<PreviewRow> {
        self.preview
            .as_ref()
            .map(|preview| preview(values))
            .unwrap_or_default()
    }
}

/// One field of a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The key the value is read back by.
    pub key: &'static str,
    /// The label beside the field.
    pub label: String,
    /// What kind of field, with its initial value.
    pub kind: FieldKind,
}

impl Field {
    /// A line of text with a placeholder shown while it is empty.
    #[must_use]
    pub fn text(
        key: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            key,
            label: label.into(),
            kind: FieldKind::Text {
                value: value.into(),
                hint: hint.into(),
            },
        }
    }

    /// A checkbox.
    #[must_use]
    pub fn check(key: &'static str, label: impl Into<String>, on: bool) -> Self {
        Self {
            key,
            label: label.into(),
            kind: FieldKind::Check { on },
        }
    }

    /// One of a list.
    #[must_use]
    pub fn choice(
        key: &'static str,
        label: impl Into<String>,
        options: Vec<String>,
        selected: usize,
    ) -> Self {
        Self {
            key,
            label: label.into(),
            kind: FieldKind::Choice { options, selected },
        }
    }

    /// A folder path, typed or picked with the platform's dialogue.
    #[must_use]
    pub fn folder(key: &'static str, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            kind: FieldKind::Folder {
                value: value.into(),
            },
        }
    }
}

/// What kind of field, with its initial value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    /// A line of text.
    Text {
        /// The initial text.
        value: String,
        /// Shown greyed while the field is empty.
        hint: String,
    },
    /// A checkbox.
    Check {
        /// The initial state.
        on: bool,
    },
    /// One of a list.
    Choice {
        /// The options, in display order.
        options: Vec<String>,
        /// The index selected at first.
        selected: usize,
    },
    /// A folder path, with a button that opens the platform's folder
    /// dialogue beside the text.
    Folder {
        /// The initial path.
        value: String,
    },
}

impl FieldKind {
    /// The value the field opens with.
    #[must_use]
    pub fn default(&self) -> Value {
        match self {
            Self::Text { value, .. } | Self::Folder { value } => Value::Text(value.clone()),
            Self::Check { on } => Value::Check(*on),
            Self::Choice { selected, .. } => Value::Choice(*selected),
        }
    }
}

/// The value of one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A text or folder field.
    Text(String),
    /// A checkbox.
    Check(bool),
    /// The index of the chosen option.
    Choice(usize),
}

/// The values of a form, by field key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Values(pub Vec<(&'static str, Value)>);

impl Values {
    /// The value of the field `key`, if the form has one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    /// Replaces the value of the field `key`.
    pub fn set(&mut self, key: &'static str, value: Value) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => *slot = value,
            None => self.0.push((key, value)),
        }
    }

    /// The text of a text or folder field; empty for any other.
    #[must_use]
    pub fn text(&self, key: &str) -> &str {
        match self.get(key) {
            Some(Value::Text(text)) => text,
            _ => "",
        }
    }

    /// Whether a checkbox is on; `false` for any other field.
    #[must_use]
    pub fn check(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Value::Check(true)))
    }

    /// The chosen index of a choice field; 0 for any other.
    #[must_use]
    pub fn choice(&self, key: &str) -> usize {
        match self.get(key) {
            Some(Value::Choice(index)) => *index,
            _ => 0,
        }
    }
}

/// One row of a form's live preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRow {
    /// What is there now.
    pub from: String,
    /// What it becomes.
    pub to: String,
    /// Why this row cannot be applied, if it cannot: a clash with another
    /// row, an existing name, an empty result.
    pub problem: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_back_by_key_with_defaults_for_the_rest() {
        let form = Form::new("Test", "OK")
            .field(Field::text("a", "A", "hello", ""))
            .field(Field::check("b", "B", true))
            .field(Field::choice("c", "C", vec!["x".into(), "y".into()], 1))
            .field(Field::folder("d", "D", "/tmp"));
        let mut values = form.defaults();
        assert_eq!(values.text("a"), "hello");
        assert!(values.check("b"));
        assert_eq!(values.choice("c"), 1);
        assert_eq!(values.text("d"), "/tmp");
        assert_eq!(values.text("missing"), "");
        assert!(!values.check("a"));
        assert_eq!(values.choice("a"), 0);
        values.set("a", Value::Text("bye".into()));
        values.set("e", Value::Check(true));
        assert_eq!(values.text("a"), "bye");
        assert!(values.check("e"));
        assert_eq!(form.preview_rows(&values), Vec::<PreviewRow>::new());
        assert!(format!("{form:?}").contains("preview: false"));
    }
}
