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
    /// The header band under the title (SPEC.md *Dialogues*): an icon, a
    /// mode pill and a secondary line. Without one the title stands alone.
    pub header: Option<Header>,
    /// A note at the footer's left, before the buttons: the archive's
    /// name, a hint.
    pub note: Option<String>,
    /// A caps heading over the preview's rows, which are then drawn as a
    /// card of label and detail rows (*WHAT RICK WILL DO*) rather than as
    /// a from → to table.
    pub preview_title: Option<String>,
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
            .field("header", &self.header)
            .field("note", &self.note)
            .field("preview_title", &self.preview_title)
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
            header: None,
            note: None,
            preview_title: None,
        }
    }

    /// Sets the header band.
    #[must_use]
    pub fn header(mut self, header: Header) -> Self {
        self.header = Some(header);
        self
    }

    /// Sets the footer's note.
    #[must_use]
    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Draws the preview as a titled card of effects.
    #[must_use]
    pub fn preview_title(mut self, title: impl Into<String>) -> Self {
        self.preview_title = Some(title.into());
        self
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
            .map_or_default(|preview| preview(values))
    }
}

/// The header band of a dialogue: the icon at the left of the title, a
/// mode pill (*Update*, *Reinstall*) and a secondary line under the title
/// (*2026.2.3 → 2026.2.4.1*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The icon.
    pub icon: HeaderIcon,
    /// The title shown in the header when it is not the form's: an app's
    /// name over its version change, say.
    pub title: Option<String>,
    /// A short pill before the secondary line, if any.
    pub pill: Option<String>,
    /// The secondary line, if any.
    pub subtitle: Option<String>,
}

impl Header {
    /// A header with `icon` and nothing else yet.
    #[must_use]
    pub fn new(icon: HeaderIcon) -> Self {
        Self {
            icon,
            title: None,
            pill: None,
            subtitle: None,
        }
    }

    /// Shows `title` in the header instead of the form's title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the pill.
    #[must_use]
    pub fn pill(mut self, pill: impl Into<String>) -> Self {
        self.pill = Some(pill.into());
        self
    }

    /// Sets the secondary line.
    #[must_use]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

/// A dialogue header's icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderIcon {
    /// A 16 px glyph of the icon set, by its name in the UI's icon table.
    Glyph(&'static str),
    /// A generated monogram: the letter, white on a colour derived from
    /// `seed`, at 48 px with 12 px corners.
    Monogram {
        /// The letter shown.
        letter: char,
        /// What the colour is derived from, an app id for instance.
        seed: String,
    },
    /// An image at 48 px with 12 px corners: a PNG or an SVG.
    Image {
        /// The file's bytes.
        bytes: Arc<Vec<u8>>,
        /// `true` for an SVG, `false` for a PNG.
        svg: bool,
    },
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
    /// The value is set in the monospace font: a path, an id, a hash.
    pub mono: bool,
    /// The field must not be empty: while it is, this warning shows under
    /// it in `status/warning` and the confirming button is disabled.
    pub required: Option<String>,
}

impl Field {
    /// Sets the value in the monospace font.
    #[must_use]
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// Requires a value, with the warning shown while there is none.
    #[must_use]
    pub fn required(mut self, warning: impl Into<String>) -> Self {
        self.required = Some(warning.into());
        self
    }

    /// A section of the fields that follow, up to the next section: with a
    /// title in small capitals over it, and as a `bg/card` card when
    /// `card`. `key` is unused and the field holds no value.
    #[must_use]
    pub fn section(title: Option<String>, card: bool) -> Self {
        Self::of("", String::new(), FieldKind::Section { title, card })
    }

    /// A disclosure card: `label` in medium weight with `summary` under it
    /// while collapsed, the fields up to the next section inside it when
    /// expanded. Its value is whether it is open.
    #[must_use]
    pub fn disclosure(
        key: &'static str,
        label: impl Into<String>,
        summary: impl Into<String>,
        open: bool,
    ) -> Self {
        Self::of(
            key,
            label,
            FieldKind::Disclosure {
                summary: summary.into(),
                open,
                counted: false,
            },
        )
    }

    /// A disclosure that counts the ticked checkboxes inside it in a pill
    /// (*3 of 7*) and offers *Select all*.
    #[must_use]
    pub fn counted(mut self) -> Self {
        if let FieldKind::Disclosure { counted, .. } = &mut self.kind {
            *counted = true;
        }
        self
    }

    /// Where the preview's card sits among the fields.
    #[must_use]
    pub fn preview_here() -> Self {
        Self::of("", String::new(), FieldKind::Preview)
    }

    /// One of a few options as a segmented control.
    #[must_use]
    pub fn segmented(
        key: &'static str,
        label: impl Into<String>,
        options: Vec<String>,
        selected: usize,
    ) -> Self {
        Self::of(key, label, FieldKind::Segmented { options, selected })
    }

    /// A file path, typed or picked with the platform's dialogue, limited
    /// to `filters`: each a name and its extensions.
    #[must_use]
    pub fn file(
        key: &'static str,
        label: impl Into<String>,
        value: impl Into<String>,
        filters: Vec<(&'static str, &'static [&'static str])>,
    ) -> Self {
        Self::of(
            key,
            label,
            FieldKind::File {
                value: value.into(),
                filters,
            },
        )
    }

    fn of(key: &'static str, label: impl Into<String>, kind: FieldKind) -> Self {
        Self {
            key,
            label: label.into(),
            kind,
            mono: false,
            required: None,
        }
    }
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
        Self::of(
            key,
            label,
            FieldKind::Text {
                value: value.into(),
                hint: hint.into(),
            },
        )
    }

    /// A checkbox.
    #[must_use]
    pub fn check(key: &'static str, label: impl Into<String>, on: bool) -> Self {
        Self::of(key, label, FieldKind::Check { on })
    }

    /// One of a list.
    #[must_use]
    pub fn choice(
        key: &'static str,
        label: impl Into<String>,
        options: Vec<String>,
        selected: usize,
    ) -> Self {
        Self::of(key, label, FieldKind::Choice { options, selected })
    }

    /// A folder path, typed or picked with the platform's dialogue.
    #[must_use]
    pub fn folder(key: &'static str, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::of(
            key,
            label,
            FieldKind::Folder {
                value: value.into(),
            },
        )
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
    /// A file path, with a *Choose…* button that opens the platform's file
    /// dialogue limited to `filters`.
    File {
        /// The initial path.
        value: String,
        /// The file dialogue's filters: a name and its extensions.
        filters: Vec<(&'static str, &'static [&'static str])>,
    },
    /// A section of the fields that follow (SPEC.md *Dialogues*): a caps
    /// title over them, a `bg/card` card around them when `card`. Holds no
    /// value.
    Section {
        /// The title in small capitals, if any.
        title: Option<String>,
        /// Whether the fields sit in a card with hairlines between rows.
        card: bool,
    },
    /// A disclosure card holding the fields up to the next section: the
    /// label with `summary` under it while collapsed. Its value is whether
    /// it is open.
    Disclosure {
        /// One line of what the card holds, shown while collapsed.
        summary: String,
        /// Whether it opens expanded.
        open: bool,
        /// Whether a pill counts the ticked checkboxes inside (*3 of 7*)
        /// with a *Select all* link.
        counted: bool,
    },
    /// One of a few options as a segmented control.
    Segmented {
        /// The options, in display order.
        options: Vec<String>,
        /// The index selected at first.
        selected: usize,
    },
    /// Where the preview's card is drawn among the fields; without this
    /// marker it follows the last field. Holds no value.
    Preview,
}

impl FieldKind {
    /// The value the field opens with.
    #[must_use]
    pub fn default(&self) -> Value {
        match self {
            Self::Text { value, .. } | Self::Folder { value } | Self::File { value, .. } => {
                Value::Text(value.clone())
            }
            Self::Check { on } => Value::Check(*on),
            Self::Choice { selected, .. } | Self::Segmented { selected, .. } => {
                Value::Choice(*selected)
            }
            Self::Section { .. } | Self::Preview => Value::Check(false),
            Self::Disclosure { open, .. } => Value::Check(*open),
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
