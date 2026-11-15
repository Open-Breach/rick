//! *Edit ZFS properties…*: sizes, the form, the preview, the commands and
//! the permission rule, as `specs/ext-zfs.md` specifies.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Action, ActionOutcome, Availability, FieldKind, FormLaterState, JobRunner, Value, Values,
};

use super::manage::{HELPER, fake, finish, message, shared};
use super::*;
use crate::actions::local;
use crate::edit::{Current, EDITABLE, EditProperties, NotASize, parse_size, read};
use crate::props::{Property, Source};
use crate::provider::{dataset_location, snapshot_location};

const GIB: u64 = 1024 * 1024 * 1024;

fn get_args(dataset: &str) -> Vec<String> {
    let mut names: Vec<&str> = EDITABLE.to_vec();
    names.extend(["used", "referenced"]);
    crate::props::get_args(&names, dataset)
}

fn on_get(fx: &Fixture, dataset: &str, out: &str) {
    let args = get_args(dataset);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    fx.zfs.on(&args, Output::ok(out));
}

/// `tank/home` with `compression=zstd-3` set locally, `atime` inherited,
/// 80 GiB used; `tank` with `lz4` and `atime=off` set locally.
fn script(fx: &Fixture) {
    on_get(
        fx,
        "tank/home",
        &format!(
            "compression\tzstd-3\tlocal\natime\toff\tinherited from tank\nrelatime\ton\tdefault\n\
             copies\t1\tdefault\nrecordsize\t131072\tdefault\nquota\t0\tdefault\n\
             refquota\t0\tdefault\nused\t{}\t-\nreferenced\t{}\t-\n",
            80 * GIB,
            70 * GIB
        ),
    );
    on_get(
        fx,
        "tank",
        "compression\tlz4\tlocal\natime\toff\tlocal\nrelatime\ton\tdefault\ncopies\t1\tdefault\n\
         recordsize\t131072\tdefault\nquota\t0\tdefault\nrefquota\t0\tdefault\nused\t1\t-\n\
         referenced\t1\t-\n",
    );
}

fn current(fx: &Fixture) -> Arc<Current> {
    script(fx);
    Arc::new(read(&*fx.zfs.clone(), "tank/home").unwrap_or_else(|e| panic!("{e}")))
}

fn options(form: &ext::Form, key: &str) -> (Vec<String>, usize) {
    let field = form
        .fields
        .iter()
        .find(|f| f.key == key)
        .unwrap_or_else(|| panic!("no field {key}"));
    match &field.kind {
        FieldKind::Choice { options, selected } => (options.clone(), *selected),
        other => panic!("{key} is {other:?}"),
    }
}

fn with(form: &ext::Form, changes: &[(&'static str, Value)]) -> Values {
    let mut values = form.defaults();
    for (key, value) in changes {
        values.set(key, value.clone());
    }
    values
}

#[test]
fn sizes_parse_as_zfs_reads_them() {
    assert_eq!(parse_size("none"), Ok(None));
    assert_eq!(parse_size("NONE"), Ok(None));
    assert_eq!(parse_size("50G"), Ok(Some(50 * GIB)));
    assert_eq!(parse_size("50g"), Ok(Some(50 * GIB)));
    assert_eq!(parse_size("50GiB"), Ok(Some(50 * GIB)));
    assert_eq!(parse_size("50gb"), Ok(Some(50 * GIB)));
    assert_eq!(parse_size("1.5M"), Ok(Some(1_572_864)));
    assert_eq!(parse_size("10K"), Ok(Some(10_240)));
    assert_eq!(parse_size("2T"), Ok(Some(2 * 1024 * GIB)));
    assert_eq!(parse_size("1P"), Ok(Some(1024 * 1024 * GIB)));
    assert_eq!(parse_size("512"), Ok(Some(512)));
    assert_eq!(parse_size("512B"), Ok(Some(512)));
    for bad in ["fifty", "-1G", "1X", "", ".5G", "5.", "G", "1GG", "1 2"] {
        assert_eq!(parse_size(bad), Err(NotASize), "{bad}");
    }
}

#[test]
fn the_form_opens_with_the_current_values_and_inherit_labelled() {
    let fx = Fixture::new();
    let form = current(&fx).form();
    let keys: Vec<&str> = form.fields.iter().map(|f| f.key).collect();
    assert_eq!(keys, EDITABLE);
    for excluded in ["mountpoint", "readonly", "dedup", "encryption"] {
        assert!(!keys.contains(&excluded), "{excluded}");
    }
    let (compression, selected) = options(&form, "compression");
    assert_eq!(compression[0], "Inherit (lz4)");
    assert_eq!(compression[selected], "zstd-3", "{compression:?}");
    for standard in ["off", "on", "lz4", "zstd", "gzip", "zle"] {
        assert!(compression.contains(&standard.to_owned()), "{standard}");
    }
    // Inherited and default values open on Inherit.
    let (atime, selected) = options(&form, "atime");
    assert_eq!((atime[0].as_str(), selected), ("Inherit (off)", 0));
    let (recordsize, selected) = options(&form, "recordsize");
    assert_eq!(selected, 0);
    assert_eq!(recordsize[0], "Inherit (128K)");
    assert!(recordsize.contains(&"1M".to_owned()));
    // A quota not set locally opens empty, which means Inherit, and says
    // what it would inherit.
    assert_eq!(form.defaults().text("quota"), "");
    let quota = form.fields.iter().find(|f| f.key == "quota");
    assert!(
        matches!(quota.map(|f| &f.kind), Some(FieldKind::Text { hint, .. }) if hint.contains("Inherit (none)")),
        "{quota:?}"
    );
}

#[test]
fn an_unusual_current_value_is_kept_selectable() {
    let current = Current {
        dataset: "tank".to_owned(),
        properties: vec![Property {
            name: "recordsize".to_owned(),
            value: "2097152".to_owned(),
            source: Source::Local,
        }],
        inherited: Vec::new(),
        used: 0,
        referenced: 0,
    };
    let form = Arc::new(current).form();
    let (options, selected) = options(&form, "recordsize");
    assert_eq!(options[selected], "2M");
    // A pool's root dataset inherits ZFS's default.
    assert_eq!(options[0], "Inherit (default)");
}

#[test]
fn nothing_changed_keeps_the_form_from_applying() {
    let fx = Fixture::new();
    let current = current(&fx);
    let form = current.form();
    let rows = form.preview_rows(&form.defaults());
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].problem.is_some());
    assert_eq!(current.commands(&form.defaults()), Ok(Vec::new()));
}

#[test]
fn a_change_is_listed_with_its_old_source_and_the_forward_note() {
    let fx = Fixture::new();
    let current = current(&fx);
    let form = current.form();
    let (options, _) = options(&form, "compression");
    let zstd = options.iter().position(|o| o == "zstd").unwrap_or(0);
    let rows = form.preview_rows(&with(&form, &[("compression", Value::Choice(zstd))]));
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].from, "compression: zstd-3 (local)");
    assert_eq!(rows[0].to, "zstd");
    assert!(rows[0].problem.is_none());
    assert!(rows[1].to.contains("from now on"), "{rows:?}");

    // Inheriting atime is no change; inheriting compression is.
    let rows = form.preview_rows(&with(&form, &[("compression", Value::Choice(0))]));
    assert_eq!(rows[0].to, "Inherit (lz4)");
    assert_eq!(rows[0].from, "compression: zstd-3 (local)");
}

#[test]
fn a_quota_is_parsed_and_checked_against_the_space_used() {
    let fx = Fixture::new();
    let current = current(&fx);
    let form = current.form();
    let rows = form.preview_rows(&with(&form, &[("quota", Value::Text("50G".into()))]));
    assert_eq!(rows[0].to, "50.0 GiB");
    assert!(
        rows[0]
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("below")),
        "{rows:?}"
    );
    let rows = form.preview_rows(&with(&form, &[("quota", Value::Text("fifty".into()))]));
    assert!(
        rows[0]
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("not a size")),
        "{rows:?}"
    );
    // refquota is measured against referenced (70 GiB).
    let values = with(&form, &[("refquota", Value::Text("75G".into()))]);
    let rows = form.preview_rows(&values);
    assert!(rows[0].problem.is_none(), "{rows:?}");
    assert_eq!(
        current.commands(&values),
        Ok(vec![vec![
            "set".to_owned(),
            format!("refquota={}", 75 * GIB),
            "tank/home".to_owned()
        ]])
    );
    assert!(
        current
            .commands(&with(&form, &[("quota", Value::Text("50G".into()))]))
            .is_err()
    );
}

#[test]
fn explicit_values_go_in_one_set_and_each_reset_in_an_inherit() {
    let fx = Fixture::new();
    let current = current(&fx);
    let form = current.form();
    let (options, _) = options(&form, "atime");
    let on = options.iter().position(|o| o == "on").unwrap_or(0);
    let values = with(
        &form,
        &[
            ("compression", Value::Choice(0)),
            ("atime", Value::Choice(on)),
            ("copies", Value::Choice(2)),
            ("quota", Value::Text("none".into())),
        ],
    );
    assert_eq!(
        current.commands(&values),
        Ok(vec![
            vec![
                "set".to_owned(),
                "atime=on".to_owned(),
                "copies=2".to_owned(),
                "quota=none".to_owned(),
                "tank/home".to_owned()
            ],
            vec![
                "inherit".to_owned(),
                "compression".to_owned(),
                "tank/home".to_owned()
            ],
        ])
    );
    // Every command passes the helper's check.
    for command in current.commands(&values).unwrap_or_default() {
        assert_eq!(crate::helper::check(&command), Ok(()), "{command:?}");
    }
}

/// ZFS refuses `zfs inherit quota`: a quota is never inherited. Emptying
/// a local quota sets it to `none`, and the placeholder says so even when
/// the parent has a quota of its own.
#[test]
fn a_quota_reset_is_set_to_none_not_inherited() {
    let fx = Fixture::new();
    on_get(
        &fx,
        "tank/home",
        &format!(
            "compression\tlz4\tinherited from tank\natime\ton\tdefault\nrelatime\ton\tdefault\n\
             copies\t1\tdefault\nrecordsize\t131072\tdefault\nquota\t{}\tlocal\n\
             refquota\t{}\tlocal\nused\t{}\t-\nreferenced\t{}\t-\n",
            100 * GIB,
            90 * GIB,
            GIB,
            GIB
        ),
    );
    on_get(
        &fx,
        "tank",
        &format!(
            "compression\tlz4\tlocal\natime\ton\tdefault\nrelatime\ton\tdefault\n\
             copies\t1\tdefault\nrecordsize\t131072\tdefault\nquota\t{}\tlocal\n\
             refquota\t{}\tlocal\nused\t1\t-\nreferenced\t1\t-\n",
            500 * GIB,
            400 * GIB
        ),
    );
    let current = Arc::new(read(&*fx.zfs.clone(), "tank/home").unwrap_or_else(|e| panic!("{e}")));
    let form = current.form();
    for key in ["quota", "refquota"] {
        let field = form.fields.iter().find(|f| f.key == key);
        assert!(
            matches!(field.map(|f| &f.kind), Some(FieldKind::Text { hint, .. }) if hint.contains("Inherit (none)")),
            "{field:?}"
        );
    }
    let values = with(
        &form,
        &[
            ("quota", Value::Text(String::new())),
            ("refquota", Value::Text(String::new())),
        ],
    );
    let rows = form.preview_rows(&values);
    assert!(rows.iter().all(|r| r.to == "Inherit (none)"), "{rows:?}");
    assert_eq!(
        current.commands(&values),
        Ok(vec![vec![
            "set".to_owned(),
            "quota=none".to_owned(),
            "refquota=none".to_owned(),
            "tank/home".to_owned()
        ]])
    );
    // A quota set to none locally is what Inherit gives: no change.
    let none = Current {
        properties: current
            .properties
            .iter()
            .map(|p| Property {
                value: if p.name == "quota" {
                    "0".to_owned()
                } else {
                    p.value.clone()
                },
                ..p.clone()
            })
            .collect(),
        ..(*current).clone()
    };
    let rows = none.preview(&with(&form, &[("quota", Value::Text(String::new()))]));
    assert!(
        rows.iter().all(|r| !r.from.starts_with("quota")),
        "{rows:?}"
    );
}

#[test]
fn offered_on_live_folders_and_dataset_folders_only() {
    let fx = Fixture::new();
    let edit = EditProperties(shared(&fx, None));
    let enabled =
        |at: types::Location, selection: Vec<types::Entry>| edit.availability(&ctx(at, selection));
    assert_eq!(
        enabled(local(&fx.home().join("ann")), Vec::new()),
        Availability::Enabled
    );
    assert_eq!(
        enabled(
            snapshot_location("tank/home", "b", Path::new("ann")),
            Vec::new()
        ),
        Availability::Hidden
    );
    assert_eq!(
        enabled(dataset_location("tank/home"), Vec::new()),
        Availability::Hidden
    );
    let root = types::Location::new(crate::provider::PROVIDER_ID, types::VPath::from("/"));
    let dataset = zfs_entry(&fx.provider(), &dataset_location("tank/home"));
    assert_eq!(enabled(root.clone(), vec![dataset]), Availability::Enabled);
    assert_eq!(enabled(root, Vec::new()), Availability::Hidden);
    assert_eq!(
        enabled(local(Path::new("/elsewhere")), Vec::new()),
        Availability::Hidden
    );
}

/// Runs the action in the live `home` folder and answers its form with
/// `changes`; the outcome of the apply step.
struct Applied {
    read: Result<(), types::Error>,
    form: ext::Form,
    then: Result<ActionOutcome, types::Error>,
    jobs: JobRunner,
    events: tokio::sync::mpsc::UnboundedReceiver<runtime::JobEvent>,
}

fn apply(
    fx: &Fixture,
    edit: &EditProperties,
    changes: impl FnOnce(&ext::Form) -> Option<Values>,
) -> Applied {
    let (jobs, mut events) = JobRunner::with_events();
    let ui = Scripted::default();
    let outcome = edit
        .run(ctx(local(&fx.home()), Vec::new()), &jobs, &ui)
        .unwrap_or_else(|e| panic!("{e}"));
    let ActionOutcome::Later(mut later) = outcome else {
        panic!("no form: {outcome:?}");
    };
    let read = finish(&mut events, None);
    let FormLaterState::Ready(form, then) = later.try_take() else {
        panic!("the reading job handed over no form ({read:?})");
    };
    let values = changes(&form);
    let then = then.run(values);
    Applied {
        read,
        form,
        then,
        jobs,
        events,
    }
}

impl Applied {
    /// Waits for the apply job; the elevation question if it asked one.
    fn finish(
        &mut self,
    ) -> (
        Result<(), types::Error>,
        Option<(ext::Form, ext::Continuation)>,
    ) {
        let _ = &self.jobs;
        let then = std::mem::replace(&mut self.then, Ok(ActionOutcome::Done));
        match then {
            Ok(ActionOutcome::Later(mut later)) => {
                let result = finish(&mut self.events, None);
                let question = match later.try_take() {
                    FormLaterState::Ready(form, then) => Some((form, then)),
                    _ => None,
                };
                (result, question)
            }
            Ok(ActionOutcome::Done) => (Ok(()), None),
            Ok(other) => panic!("unexpected outcome {other:?}"),
            Err(error) => (Err(error), None),
        }
    }
}

fn zstd(form: &ext::Form) -> Values {
    let (compression, _) = options(form, "compression");
    let zstd = compression.iter().position(|o| o == "zstd").unwrap_or(0);
    let (atime, _) = options(form, "atime");
    let on = atime.iter().position(|o| o == "on").unwrap_or(0);
    let mut values = with(form, &[("compression", Value::Choice(zstd))]);
    values.set("atime", Value::Choice(on));
    values.set("relatime", Value::Choice(0));
    values
}

#[test]
fn applying_sets_then_inherits_and_refreshes_the_cache() {
    let fx = Fixture::new();
    script(&fx);
    // relatime is default: Inherit is no change; make it local to reset it.
    on_get(
        &fx,
        "tank/home",
        &format!(
            "compression\tzstd-3\tlocal\natime\toff\tinherited from tank\nrelatime\toff\tlocal\n\
             copies\t1\tdefault\nrecordsize\t131072\tdefault\nquota\t0\tdefault\n\
             refquota\t0\tdefault\nused\t{GIB}\t-\nreferenced\t{GIB}\t-\n"
        ),
    );
    fx.zfs.on(
        &["set", "compression=zstd", "atime=on", "tank/home"],
        Output::ok(""),
    );
    fx.zfs
        .on(&["inherit", "relatime", "tank/home"], Output::ok(""));
    let edit = EditProperties(shared(&fx, None));
    let mut applied = apply(&fx, &edit, |form| Some(zstd(form)));
    assert!(applied.read.is_ok(), "{:?}", applied.read);
    assert!(applied.form.title.contains("tank/home"));
    let (result, question) = applied.finish();
    assert!(result.is_ok(), "{result:?}");
    assert!(question.is_none());
    let calls = fx.zfs.calls();
    let set = calls.iter().position(|c| c[0] == "set");
    let inherit = calls.iter().position(|c| c[0] == "inherit");
    assert!(set.is_some() && set < inherit, "{calls:?}");
}

#[test]
fn a_failed_reset_keeps_the_values_set_and_names_the_property() {
    let fx = Fixture::new();
    script(&fx);
    on_get(
        &fx,
        "tank/home",
        &format!(
            "compression\tzstd-3\tlocal\natime\toff\tinherited from tank\nrelatime\toff\tlocal\n\
             copies\t1\tdefault\nrecordsize\t131072\tdefault\nquota\t0\tdefault\n\
             refquota\t0\tdefault\nused\t{GIB}\t-\nreferenced\t{GIB}\t-\n"
        ),
    );
    fx.zfs.on(
        &["set", "compression=zstd", "atime=on", "tank/home"],
        Output::ok(""),
    );
    fx.zfs.on(
        &["inherit", "relatime", "tank/home"],
        Output::failed(1, "cannot inherit relatime for 'tank/home': pool is busy\n"),
    );
    let edit = EditProperties(shared(&fx, None));
    let mut applied = apply(&fx, &edit, |form| Some(zstd(form)));
    let (result, _) = applied.finish();
    let text = message(&result);
    assert!(text.contains("relatime") && text.contains("busy"), "{text}");
    assert!(
        fx.zfs.calls().iter().any(|c| c[0] == "set"),
        "the values were set"
    );
}

#[test]
fn a_refused_change_runs_the_rest_through_the_helper() {
    let fx = Fixture::new();
    script(&fx);
    fx.zfs.on(
        &["set", "compression=zstd", "atime=on", "tank/home"],
        Output::failed(
            1,
            "cannot set property for 'tank/home': permission denied\n",
        ),
    );
    let platform = fake();
    let edit = EditProperties(shared(&fx, Some(platform.clone())));
    let mut applied = apply(&fx, &edit, |form| Some(zstd(form)));
    let (result, question) = applied.finish();
    assert!(result.is_ok(), "{result:?}");
    let (form, then) = question.unwrap_or_else(|| panic!("no elevation offered"));
    match then.run(Some(form.defaults())) {
        Ok(ActionOutcome::Started(id)) => {
            let done = finish(&mut applied.events, Some(id));
            assert!(done.is_ok(), "{done:?}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        platform.elevated(),
        [(
            PathBuf::from(HELPER),
            vec![
                OsString::from("set"),
                OsString::from("compression=zstd"),
                OsString::from("atime=on"),
                OsString::from("tank/home"),
            ]
        )]
    );
}

#[test]
fn without_elevation_the_properties_to_delegate_are_named() {
    let fx = Fixture::new();
    script(&fx);
    fx.zfs.on(
        &["set", "compression=zstd", "atime=on", "tank/home"],
        Output::failed(
            1,
            "cannot set property for 'tank/home': permission denied\n",
        ),
    );
    let edit = EditProperties(shared(&fx, None));
    let mut applied = apply(&fx, &edit, |form| Some(zstd(form)));
    let (result, question) = applied.finish();
    assert!(question.is_none());
    let text = message(&result);
    assert!(
        text.contains("zfs allow") && text.contains("compression,atime tank/home"),
        "{text}"
    );
}

#[test]
fn a_dismissed_form_changes_nothing() {
    let fx = Fixture::new();
    script(&fx);
    let edit = EditProperties(shared(&fx, None));
    let mut applied = apply(&fx, &edit, |_| None);
    assert!(matches!(applied.then, Ok(ActionOutcome::Done)));
    let (result, _) = applied.finish();
    assert!(result.is_ok());
    assert!(
        !fx.zfs
            .calls()
            .iter()
            .any(|c| c[0] == "set" || c[0] == "inherit")
    );
}
