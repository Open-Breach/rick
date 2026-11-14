//! *Edit ZFS properties…*: the few dataset properties that are safe to
//! change on a running system, as `specs/ext-zfs.md` specifies.
//!
//! The action reads the dataset's current values fresh in a job, then
//! opens a form through [`ActionOutcome::Later`]: one field per property of
//! [`EDITABLE`], each offering *Inherit* labelled with the value it would
//! inherit. The live preview lists every change as old → new with the old
//! value's source, marks a size that does not parse or a quota below the
//! space already used, and keeps the form from being confirmed while a row
//! has a problem or nothing changed. Applying runs as one ordered job: one
//! `zfs set` for every explicit value and every quota reset (to `none`, as
//! ZFS never inherits a quota), then one `zfs inherit` per other property
//! reset, stopping at the first failure; the permission rule is that of
//! snapshot management ([`crate::manage`]), with the helper `rick-zfs`
//! accepting the same commands ([`crate::helper`]). No undo.

use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Continuation, Field,
    Form, FormLater, JobRunner, PreviewRow, Shortcut, UiHost, Values, t,
};
use types::{Error, Location, Progress};

use crate::actions::{GROUP, Shared, local_path};
use crate::manage::{Batch, spawn};
use crate::props::{Property, Source, format_size, get_args, parse_named};
use crate::provider::{PROVIDER_ID, Target, target};
use crate::zfs::{Zfs, classify};

/// *Edit ZFS properties…*.
pub const EDIT: ActionId = ActionId("zfs.properties");

/// The properties the editor offers, in its order. Nothing else can be
/// set through Rick.
pub const EDITABLE: [&str; 7] = [
    "compression",
    "atime",
    "relatime",
    "copies",
    "recordsize",
    "quota",
    "refquota",
];

/// The properties ZFS never inherits: `zfs inherit` refuses them, and
/// *Inherit* sets them to `none`, which is also what it shows.
const NOT_INHERITED: [&str; 2] = ["quota", "refquota"];

/// The properties whose new value applies only to data written from now on.
const FORWARD_ONLY: [&str; 3] = ["compression", "copies", "recordsize"];

/// The standard values of a property with a list of choices; `None` for
/// the sizes typed as text (`quota`, `refquota`).
fn standard(property: &str) -> Option<&'static [&'static str]> {
    match property {
        "compression" => Some(&["off", "on", "lz4", "zstd", "gzip", "zle"]),
        "atime" | "relatime" => Some(&["on", "off"]),
        "copies" => Some(&["1", "2", "3"]),
        "recordsize" => Some(&[
            "4K", "8K", "16K", "32K", "64K", "128K", "256K", "512K", "1M",
        ]),
        _ => None,
    }
}

/// Text that is no size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a size")]
pub struct NotASize;

/// A size as ZFS reads it: `none`, or a whole or decimal number with an
/// optional unit K, M, G, T or P (powers of 1024, any case, optionally
/// followed by `B` or `iB`). `Ok(None)` for `none`.
///
/// # Errors
///
/// [`NotASize`] for anything else: empty, negative, an unknown unit.
// The bound check compares with u64::MAX as a float on purpose.
#[allow(clippy::cast_precision_loss)]
pub fn parse_size(text: &str) -> Result<Option<u64>, NotASize> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    if number.is_empty() || number.starts_with('.') || number.ends_with('.') {
        return Err(NotASize);
    }
    let unit = unit.trim().to_ascii_uppercase();
    let unit = unit
        .strip_suffix("IB")
        .or_else(|| unit.strip_suffix('B').filter(|u| !u.is_empty()))
        .unwrap_or(&unit);
    let power = match unit {
        "" | "B" => 0,
        "K" => 1,
        "M" => 2,
        "G" => 3,
        "T" => 4,
        "P" => 5,
        _ => return Err(NotASize),
    };
    let factor = 1024_f64.powi(power);
    let value: f64 = number.parse().map_err(|_| NotASize)?;
    let bytes = value * factor;
    if !bytes.is_finite() || bytes < 0.0 || bytes >= u64::MAX as f64 {
        return Err(NotASize);
    }
    // A size is rounded to whole bytes, as ZFS does.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(bytes.round() as u64))
}

/// A record size in bytes as the list writes it (`131072` → `128K`).
fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["K", "M", "G", "T"];
    let mut value = bytes;
    let mut unit = None;
    for u in UNITS {
        if value >= 1024 && value.is_multiple_of(1024) {
            value /= 1024;
            unit = Some(u);
        } else {
            break;
        }
    }
    unit.map_or_else(|| value.to_string(), |u| format!("{value}{u}"))
}

/// Whether `value` is one the editor may send for `property`; the helper
/// checks the same before it runs `zfs set`.
#[must_use]
pub fn value_ok(property: &str, value: &str) -> bool {
    match property {
        "atime" | "relatime" => matches!(value, "on" | "off"),
        "copies" => matches!(value, "1" | "2" | "3"),
        "compression" => {
            !value.is_empty()
                && !value.starts_with('-')
                && value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        }
        "recordsize" => matches!(parse_size(value), Ok(Some(b)) if b > 0),
        "quota" | "refquota" => parse_size(value).is_ok(),
        _ => false,
    }
}

/// The current state of a dataset as the editor needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    /// The dataset.
    pub dataset: String,
    /// The editable properties it reports, with their source.
    pub properties: Vec<Property>,
    /// What each property would be on *Inherit*: the parent's value, or
    /// ZFS's default for a pool's root dataset (`None` there).
    pub inherited: Vec<Property>,
    /// The space the dataset uses, for `quota`.
    pub used: u64,
    /// The space the dataset refers to, for `refquota`.
    pub referenced: u64,
}

impl Current {
    fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// The value *Inherit* would give `name`, as shown.
    fn inherit_label(&self, name: &str) -> String {
        let value = if NOT_INHERITED.contains(&name) {
            "none".to_owned()
        } else {
            self.inherited
                .iter()
                .find(|p| p.name == name)
                .map_or_else(|| t!("zfs-edit-default"), shown)
        };
        t!("zfs-edit-inherit", value = value)
    }

    /// The options of a choice field: *Inherit* first, the standard
    /// values, and the current value when it is not among them; with the
    /// one selected.
    fn options(&self, name: &str) -> (Vec<String>, usize) {
        let standard = standard(name).unwrap_or(&[]);
        let mut options = vec![self.inherit_label(name)];
        options.extend(standard.iter().map(|v| (*v).to_owned()));
        let Some(current) = self.property(name) else {
            return (options, 0);
        };
        let value = shown(current);
        let at = if let Some(i) = options.iter().skip(1).position(|o| *o == value) {
            i + 1
        } else {
            options.push(value);
            options.len() - 1
        };
        let selected = if current.source == Source::Local {
            at
        } else {
            0
        };
        (options, selected)
    }

    /// The form for this dataset, with its preview.
    #[must_use]
    pub fn form(self: &Arc<Self>) -> Form {
        let mut form = Form::new(
            t!("zfs-edit-title", dataset = self.dataset.clone()),
            t!("zfs-edit-apply"),
        )
        .preview_title(t!("zfs-edit-changes"));
        for name in EDITABLE {
            if self.property(name).is_none() {
                continue;
            }
            form = form.field(if standard(name).is_some() {
                let (options, selected) = self.options(name);
                Field::choice(name, name, options, selected)
            } else {
                Field::text(
                    name,
                    name,
                    self.text_value(name),
                    t!("zfs-edit-size-hint", inherit = self.inherit_label(name)),
                )
                .mono()
            });
        }
        let current = Arc::clone(self);
        form.preview(Arc::new(move |values: &Values| current.preview(values)))
    }

    /// A quota's text as the field opens with it: empty when not set
    /// locally (empty means *Inherit*), else `none` or the size in units.
    fn text_value(&self, name: &str) -> String {
        match self.property(name) {
            Some(p) if p.source == Source::Local => match p.value.parse::<u64>() {
                Ok(0) => "none".to_owned(),
                Ok(bytes) => size_label(bytes),
                Err(_) => p.value.clone(),
            },
            _ => String::new(),
        }
    }

    /// What the form asks for `name`: `None` when it is unchanged.
    fn wanted(&self, name: &str, values: &Values) -> Option<Wanted> {
        let current = self.property(name)?;
        let local = current.source == Source::Local;
        let wanted = if standard(name).is_some() {
            let (options, _) = self.options(name);
            match values.choice(name) {
                0 => Wanted::Inherit,
                i => Wanted::Value(options.get(i).cloned().unwrap_or_default()),
            }
        } else {
            let text = values.text(name).trim();
            if text.is_empty() {
                Wanted::Inherit
            } else {
                match parse_size(text) {
                    Ok(Some(bytes)) => Wanted::Size(bytes),
                    Ok(None) => Wanted::Size(0),
                    Err(NotASize) => Wanted::Invalid(text.to_owned()),
                }
            }
        };
        let unchanged = match &wanted {
            Wanted::Inherit => {
                !local || (NOT_INHERITED.contains(&name) && current.value.parse() == Ok(0_u64))
            }
            Wanted::Value(v) => local && *v == shown(current),
            Wanted::Size(bytes) => local && current.value.parse::<u64>().ok() == Some(*bytes),
            Wanted::Invalid(_) => false,
        };
        (!unchanged).then_some(wanted)
    }

    /// The preview rows for `values`.
    #[must_use]
    pub fn preview(&self, values: &Values) -> Vec<PreviewRow> {
        let mut rows = Vec::new();
        let mut forward = false;
        for name in EDITABLE {
            let (Some(current), Some(wanted)) = (self.property(name), self.wanted(name, values))
            else {
                continue;
            };
            forward |= FORWARD_ONLY.contains(&name);
            let source = current.source.text();
            let from = if source.is_empty() {
                format!("{name}: {}", shown(current))
            } else {
                format!("{name}: {} ({source})", shown(current))
            };
            let (to, problem) = match &wanted {
                Wanted::Inherit => (self.inherit_label(name), None),
                Wanted::Value(v) => (v.clone(), None),
                Wanted::Size(0) => ("none".to_owned(), None),
                Wanted::Size(bytes) => (format_size(*bytes), self.too_small(name, *bytes)),
                Wanted::Invalid(text) => (
                    text.clone(),
                    Some(t!("zfs-edit-not-a-size", value = text.clone())),
                ),
            };
            rows.push(PreviewRow { from, to, problem });
        }
        if rows.is_empty() {
            rows.push(PreviewRow {
                from: String::new(),
                to: t!("zfs-edit-nothing-changed"),
                problem: Some(t!("zfs-edit-nothing-changed")),
            });
        } else if forward {
            rows.push(PreviewRow {
                from: String::new(),
                to: t!("zfs-edit-written-from-now-on"),
                problem: None,
            });
        }
        rows
    }

    /// The problem of a quota below what the dataset already uses; `none`
    /// (0) is no limit.
    fn too_small(&self, name: &str, bytes: u64) -> Option<String> {
        if bytes == 0 {
            return None;
        }
        let (limit, used) = match name {
            "quota" => (bytes, self.used),
            "refquota" => (bytes, self.referenced),
            _ => return None,
        };
        (limit < used).then(|| {
            t!(
                "zfs-edit-below-used",
                property = name,
                used = format_size(used)
            )
        })
    }

    /// The commands that apply `values`: one `zfs set` for every explicit
    /// value and every quota reset, then one `zfs inherit` per other
    /// property reset; empty when nothing changed.
    ///
    /// # Errors
    ///
    /// The preview's problem when a value is invalid.
    pub fn commands(&self, values: &Values) -> Result<Vec<Vec<String>>, String> {
        let mut set = vec!["set".to_owned()];
        let mut inherit = Vec::new();
        for name in EDITABLE {
            match self.wanted(name, values) {
                None => {}
                Some(Wanted::Invalid(text)) => {
                    return Err(t!("zfs-edit-not-a-size", value = text));
                }
                Some(Wanted::Size(bytes)) if self.too_small(name, bytes).is_some() => {
                    return Err(self.too_small(name, bytes).unwrap_or_default());
                }
                Some(Wanted::Inherit) if NOT_INHERITED.contains(&name) => {
                    set.push(format!("{name}=none"));
                }
                Some(Wanted::Inherit) => inherit.push(vec![
                    "inherit".to_owned(),
                    name.to_owned(),
                    self.dataset.clone(),
                ]),
                Some(Wanted::Value(v)) => set.push(format!("{name}={v}")),
                Some(Wanted::Size(0)) => set.push(format!("{name}=none")),
                Some(Wanted::Size(bytes)) => set.push(format!("{name}={bytes}")),
            }
        }
        let mut commands = Vec::new();
        if set.len() > 1 {
            set.push(self.dataset.clone());
            commands.push(set);
        }
        commands.extend(inherit);
        Ok(commands)
    }
}

/// What the form asks for one property.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wanted {
    Inherit,
    Value(String),
    Size(u64),
    Invalid(String),
}

/// A property's value as the editor shows it: record sizes in units
/// (`128K`), quotas as `none` or in binary units, the rest as ZFS printed
/// it.
fn shown(p: &Property) -> String {
    match p.name.as_str() {
        "recordsize" => p
            .value
            .parse::<u64>()
            .map_or_else(|_| p.value.clone(), size_label),
        "quota" | "refquota" => p.shown(),
        _ => p.value.clone(),
    }
}

/// The parent of `dataset`; `None` for a pool's root dataset.
fn parent(dataset: &str) -> Option<&str> {
    dataset.rsplit_once('/').map(|(parent, _)| parent)
}

/// Reads what the editor shows for `dataset`, fresh: the editable
/// properties with `used` and `referenced`, and the parent's values for
/// the *Inherit* labels.
///
/// # Errors
///
/// `Error::Unsupported` without `zfs`; ZFS's error otherwise.
pub fn read(zfs: &dyn Zfs, dataset: &str) -> Result<Current, Error> {
    let mut names: Vec<&str> = EDITABLE.to_vec();
    names.extend(["used", "referenced"]);
    let get = |of: &str| -> Result<Vec<Property>, Error> {
        let output = zfs.run(&get_args(&names, of))?;
        let out = classify(output).map_err(|e| Error::Extension {
            ext: crate::ID,
            source: Box::new(crate::manage::Refused(e.message().to_owned())),
        })?;
        Ok(parse_named(&out, &names))
    };
    let all = get(dataset)?;
    let number = |name: &str| {
        all.iter()
            .find(|p| p.name == name)
            .and_then(|p| p.value.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let used = number("used");
    let referenced = number("referenced");
    let inherited = match parent(dataset) {
        Some(parent) => get(parent)?
            .into_iter()
            .filter(|p| EDITABLE.contains(&p.name.as_str()))
            .collect(),
        None => Vec::new(),
    };
    Ok(Current {
        dataset: dataset.to_owned(),
        properties: all
            .into_iter()
            .filter(|p| EDITABLE.contains(&p.name.as_str()))
            .collect(),
        inherited,
        used,
        referenced,
    })
}

/// The dataset the action edits in `ctx`: the live folder's, or the one
/// dataset folder selected at the root of the `zfs` location.
fn dataset_in(shared: &Shared, ctx: &ActionCtx) -> Option<String> {
    if let Some(dir) = local_path(&ctx.location) {
        return shared.datasets.dataset_of(&dir).map(|fs| fs.name.clone());
    }
    if ctx.location.provider != PROVIDER_ID || !matches!(target(&ctx.location), Ok(Target::Root)) {
        return None;
    }
    let [one] = ctx.selection.as_slice() else {
        return None;
    };
    match target(&Location::new(PROVIDER_ID, one.path.clone())) {
        Ok(Target::Dataset(name)) => Some(name),
        _ => None,
    }
}

/// *Edit ZFS properties…*.
#[derive(Debug)]
pub struct EditProperties(pub Arc<Shared>);

impl Action for EditProperties {
    fn id(&self) -> ActionId {
        EDIT
    }

    fn label(&self) -> String {
        t!("zfs-edit-properties")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if dataset_in(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let dataset = dataset_in(&self.0, &ctx)
            .ok_or(Error::Unsupported("the folder is in no ZFS dataset"))?;
        let (sender, later) = FormLater::channel();
        let shared = Arc::clone(&self.0);
        let runner = jobs.clone();
        let name = t!("zfs-edit-reading", dataset = dataset.clone());
        jobs.spawn_named(name.clone(), move |job| async move {
            job.progress.report(Progress::Message(name));
            let current = {
                let zfs = Arc::clone(shared.datasets.zfs());
                let dataset = dataset.clone();
                ext::blocking(move || read(&*zfs, &dataset)).await?
            };
            let current = Arc::new(current);
            let form = current.form();
            let label = t!("zfs-edit-properties");
            let then = Continuation::new(label, move |values: Option<Values>| {
                let Some(values) = values else {
                    return Ok(ActionOutcome::Done);
                };
                let commands = current
                    .commands(&values)
                    .map_err(|problem| Error::Extension {
                        ext: crate::ID,
                        source: Box::new(crate::manage::Refused(problem)),
                    })?;
                if commands.is_empty() {
                    return Ok(ActionOutcome::Done);
                }
                Ok(spawn(
                    &shared,
                    &runner,
                    t!("zfs-edit-applying", dataset = current.dataset.clone()),
                    Batch::ordered(current.dataset.clone(), commands),
                ))
            });
            let _ = sender.send((form, then));
            Ok(())
        });
        Ok(ActionOutcome::Later(later))
    }
}
