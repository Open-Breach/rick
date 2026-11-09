//! Select by pattern: the entries whose name matches a glob or a regular
//! expression, added to or replacing the selection (Mod+Shift+S).

use ext::t;
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Field, Form, Key,
    Modifiers, Selector, Shortcut, UiHost,
};
use runtime::JobRunner;
use types::{Entry, Error, ExtId};

/// The form's pattern kinds, in the order the choice lists them.
const KINDS: [&str; 2] = ["Glob (* and ?)", "Regular expression"];
/// The form's modes.
const MODES: [&str; 2] = ["Replace the selection", "Add to the selection"];

/// Selects the entries whose name matches a pattern.
pub(crate) struct SelectByPattern;

/// Whether an entry matches, as [`Selector`] carries it.
pub type Matcher = Arc<dyn Fn(&Entry) -> bool + Send + Sync>;

/// A matcher for `pattern`: a glob (case-insensitive, `*` matching across
/// nothing special since names have no separators) or a regular expression
/// matched anywhere in the name.
///
/// # Errors
///
/// `Error::Extension` with the parse error for a pattern that does not
/// parse.
pub fn matcher(pattern: &str, regex: bool) -> Result<Matcher, Error> {
    let error = |e: String| Error::Extension {
        ext: ExtId("core"),
        source: e.into(),
    };
    if regex {
        let re = regex::RegexBuilder::new(pattern)
            .build()
            .map_err(|e| error(e.to_string()))?;
        Ok(Arc::new(move |entry: &Entry| re.is_match(&entry.name)))
    } else {
        let glob = globset::GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(false)
            .build()
            .map_err(|e| error(e.to_string()))?
            .compile_matcher();
        Ok(Arc::new(move |entry: &Entry| glob.is_match(&entry.name)))
    }
}

impl Action for SelectByPattern {
    fn id(&self) -> ActionId {
        ActionId("core.select_pattern")
    }
    fn label(&self) -> String {
        t!("action-select-by-pattern")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        Some(Shortcut {
            modifiers: Modifiers::MOD | Modifiers::SHIFT,
            key: Key::Char('s'),
        })
    }
    fn availability(&self, _ctx: &ActionCtx) -> Availability {
        Availability::Enabled
    }
    fn run(
        &self,
        _ctx: ActionCtx,
        _jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let form = Form::new("Select by pattern", "Select")
            .field(Field::text(
                "pattern",
                "Pattern",
                "",
                "*.txt or ^report.*\\.pdf$",
            ))
            .field(Field::choice(
                "kind",
                "Pattern kind",
                KINDS.iter().map(|k| (*k).to_owned()).collect(),
                0,
            ))
            .field(Field::choice(
                "mode",
                "Selection",
                MODES.iter().map(|m| (*m).to_owned()).collect(),
                0,
            ));
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let pattern = values.text("pattern").trim().to_owned();
        if pattern.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let matches = matcher(&pattern, values.choice("kind") == 1)?;
        Ok(ActionOutcome::Select(Selector {
            matches,
            replace: values.choice("mode") == 0,
        }))
    }
}
