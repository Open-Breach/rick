//! Snapshot management and the permission rule.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ext::platform::{Elevate, ElevatedOutput, Fake};
use ext::{Action, ActionOutcome, Availability, FormLaterState, JobRunner, Value};
use runtime::JobEvent;
use tokio::sync::mpsc::UnboundedReceiver;
use types::{Error, JobId};

use super::*;
use crate::actions::{Shared, local};
use crate::manage::{Create, Destroy, Rename};
use crate::provider::{dataset_location, snapshot_location};

/// Where the tests say the privileged helper is.
const HELPER: &str = "/opt/rick/rick-zfs";

const DENIED: &str = "cannot create snapshot 'tank/home@x': permission denied\n";

/// An elevation prompt that is dismissed, or missing.
#[derive(Debug)]
enum Prompt {
    Dismissed,
    Missing,
}

impl Elevate for Prompt {
    fn run_elevated(&self, _program: &Path, _args: &[OsString]) -> Result<ElevatedOutput, Error> {
        match self {
            Self::Dismissed => Err(Error::PermissionDenied),
            Self::Missing => Err(Error::Unsupported("no pkexec")),
        }
    }
}

fn shared(fx: &Fixture, elevate: Option<Arc<dyn Elevate>>) -> Arc<Shared> {
    Arc::new(Shared {
        datasets: Arc::clone(&fx.datasets),
        platform: None,
        elevate,
        helper: Some(PathBuf::from(HELPER)),
    })
}

fn fake() -> Arc<Fake> {
    Arc::new(Fake::new().unwrap_or_else(|e| panic!("{e}")))
}

/// Waits for the job `id` to end.
fn finish(events: &mut UnboundedReceiver<JobEvent>, id: Option<JobId>) -> Result<(), Error> {
    runtime::block_on(async {
        loop {
            match tokio::time::timeout(Duration::from_secs(30), events.recv()).await {
                Ok(Some(JobEvent::Finished { id: done, result }))
                    if id.is_none_or(|id| id == done) =>
                {
                    return result;
                }
                Ok(Some(_)) => {}
                other => panic!("no end of the job: {other:?}"),
            }
        }
    })
}

/// What a management action led to: the first job's result, and what the
/// question about elevation became.
struct Ran {
    first: Result<(), Error>,
    later: Option<ext::FormLater>,
    jobs: JobRunner,
    events: UnboundedReceiver<JobEvent>,
}

fn run(
    action: &dyn Action,
    at: types::Location,
    selection: Vec<types::Entry>,
    ui: &Scripted,
) -> Option<Ran> {
    let (jobs, mut events) = JobRunner::with_events();
    match action.run(ctx(at, selection), &jobs, ui) {
        Ok(ActionOutcome::Later(later)) => {
            let first = finish(&mut events, None);
            Some(Ran {
                first,
                later: Some(later),
                jobs,
                events,
            })
        }
        Ok(ActionOutcome::Done) => None,
        Ok(other) => panic!("unexpected outcome {other:?}"),
        Err(error) => Some(Ran {
            first: Err(error),
            later: None,
            jobs,
            events,
        }),
    }
}

impl Ran {
    /// The question about elevation, if the job asked it.
    fn question(&mut self) -> Option<(ext::Form, ext::Continuation)> {
        match self.later.as_mut()?.try_take() {
            FormLaterState::Ready(form, then) => Some((form, then)),
            FormLaterState::Pending => panic!("the job ended without answering"),
            FormLaterState::Gone => None,
        }
    }

    /// Answers the question and waits for the elevated job.
    fn answer(&mut self, yes: bool) -> Result<(), Error> {
        let (form, then) = self.question().unwrap_or_else(|| panic!("no question"));
        let values = yes.then(|| form.defaults());
        match then.run(values)? {
            ActionOutcome::Started(id) => {
                let _ = &self.jobs;
                finish(&mut self.events, Some(id))
            }
            other => panic!("unexpected outcome {other:?}"),
        }
    }
}

fn with_name(name: &str) -> Scripted {
    let ui = Scripted::default();
    *ui.form
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(vec![("name", Value::Text(name.to_owned()))]);
    ui
}

fn message(result: &Result<(), Error>) -> String {
    result
        .as_ref()
        .err()
        .map(ToString::to_string)
        .unwrap_or_default()
}

#[test]
fn create_offers_a_timestamp_and_creates_the_snapshot() {
    let fx = Fixture::new();
    fx.zfs.on(&["snapshot", "tank/home@before"], Output::ok(""));
    let create = Create(shared(&fx, None));
    let at = local(&fx.home().join("ann"));
    assert_eq!(
        create.availability(&ctx(at.clone(), Vec::new())),
        Availability::Enabled
    );
    let ui = with_name("before");
    let mut ran = run(&create, at, Vec::new(), &ui).unwrap_or_else(|| panic!("nothing ran"));
    assert!(ran.first.is_ok(), "{:?}", ran.first);
    assert!(
        ran.question().is_none(),
        "no elevation for delegated rights"
    );
    let form = ui
        .forms
        .lock()
        .map_or_else(|_| panic!("form"), |f| f[0].defaults());
    assert!(
        form.text("name").starts_with("rick-20"),
        "{}",
        form.text("name")
    );
    assert!(!form.check("recursive"));
    assert!(
        fx.zfs
            .calls()
            .contains(&vec!["snapshot".to_owned(), "tank/home@before".to_owned()])
    );
}

#[test]
fn create_with_children_passes_r() {
    let fx = Fixture::new();
    fx.zfs
        .on(&["snapshot", "-r", "tank/home@all"], Output::ok(""));
    let ui = with_name("all");
    ui.form
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(Vec::new)
        .push(("recursive", Value::Check(true)));
    let ran = run(
        &Create(shared(&fx, None)),
        local(&fx.home()),
        Vec::new(),
        &ui,
    );
    assert!(ran.is_some_and(|r| r.first.is_ok()));
}

#[test]
fn an_invalid_name_is_marked_and_refused() {
    let fx = Fixture::new();
    let ui = with_name("a/b");
    let ran = run(
        &Create(shared(&fx, None)),
        local(&fx.home()),
        Vec::new(),
        &ui,
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    assert!(ran.first.is_err());
    let forms = ui
        .forms
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut values = forms[0].defaults();
    values.set("name", Value::Text("a/b".to_owned()));
    assert!(forms[0].preview_rows(&values)[0].problem.is_some());
    values.set("name", Value::Text("fine".to_owned()));
    let rows = forms[0].preview_rows(&values);
    assert_eq!(rows[0].to, "tank/home@fine");
    assert!(rows[0].problem.is_none());
    assert!(!fx.zfs.calls().iter().any(|c| c[0] == "snapshot"));
}

#[test]
fn a_taken_name_fails_naming_the_snapshot() {
    let fx = Fixture::new();
    fx.zfs.on(
        &["snapshot", "tank/home@b"],
        Output::failed(
            1,
            "cannot create snapshot 'tank/home@b': dataset already exists\n",
        ),
    );
    let ran = run(
        &Create(shared(&fx, None)),
        local(&fx.home()),
        Vec::new(),
        &with_name("b"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    assert!(
        message(&ran.first).contains("tank/home@b"),
        "{:?}",
        ran.first
    );
}

#[test]
fn create_is_hidden_outside_a_dataset() {
    let fx = Fixture::new();
    let create = Create(shared(&fx, None));
    let off = ctx(local(Path::new("/definitely/not/zfs")), Vec::new());
    assert_eq!(create.availability(&off), Availability::Hidden);
}

fn snapshot(fx: &Fixture, name: &str) -> types::Entry {
    zfs_entry(
        &fx.provider(),
        &snapshot_location("tank/home", name, Path::new("")),
    )
}

#[test]
fn rename_renames_one_snapshot_and_rereads_the_list() {
    let fx = Fixture::new();
    fx.zfs.on(
        &["rename", "tank/home@a", "tank/home@before-upgrade"],
        Output::ok(""),
    );
    let rename = Rename(shared(&fx, None));
    let at = dataset_location("tank/home");
    let one = vec![snapshot(&fx, "a")];
    assert_eq!(
        rename.availability(&ctx(at.clone(), one.clone())),
        Availability::Enabled
    );
    let two = vec![snapshot(&fx, "a"), snapshot(&fx, "b")];
    assert!(matches!(
        rename.availability(&ctx(at.clone(), two)),
        Availability::Disabled(_)
    ));
    assert_eq!(
        rename.availability(&ctx(local(&fx.home()), one.clone())),
        Availability::Hidden
    );
    let before = fx.zfs.calls().len();
    let ran = run(&rename, at, one, &with_name("before-upgrade"))
        .unwrap_or_else(|| panic!("nothing ran"));
    assert!(ran.first.is_ok(), "{:?}", ran.first);
    // The list is read again after the rename.
    super::on_snapshots(
        &fx.zfs,
        "tank/home",
        "tank/home@before-upgrade\t100\t10\ntank/home@b\t200\t20\n",
    );
    let names: Vec<String> = fx
        .datasets
        .snapshots("tank/home")
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert_eq!(names, ["b", "before-upgrade"]);
    assert!(fx.zfs.calls().len() > before + 1);
}

#[test]
fn destroy_asks_naming_each_snapshot() {
    let fx = Fixture::new();
    fx.zfs.on(&["destroy", "tank/home@a"], Output::ok(""));
    let destroy = Destroy(shared(&fx, None));
    let at = dataset_location("tank/home");
    let ui = Scripted::confirming(true);
    let ran = run(&destroy, at.clone(), vec![snapshot(&fx, "a")], &ui)
        .unwrap_or_else(|| panic!("nothing ran"));
    assert!(ran.first.is_ok(), "{:?}", ran.first);
    let asked = ui.asked();
    assert!(asked[0].contains("tank/home@a"), "{asked:?}");
    assert!(asked[0].contains("cannot be undone"), "{asked:?}");

    let declined = Scripted::confirming(false);
    let b = snapshot(&fx, "b");
    let calls = fx.zfs.calls().len();
    assert!(run(&destroy, at.clone(), vec![b], &declined).is_none());
    assert_eq!(fx.zfs.calls().len(), calls, "nothing ran");
    assert!(matches!(
        destroy.availability(&ctx(at, Vec::new())),
        Availability::Disabled(_)
    ));
}

#[test]
fn a_held_snapshot_is_reported_and_kept() {
    let fx = Fixture::new();
    fx.zfs.on(
        &["destroy", "tank/home@a"],
        Output::failed(1, "cannot destroy snapshot tank/home@a: dataset is busy\n"),
    );
    let ran = run(
        &Destroy(shared(&fx, None)),
        dataset_location("tank/home"),
        vec![snapshot(&fx, "a")],
        &Scripted::confirming(true),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    let text = message(&ran.first);
    assert!(
        text.contains("tank/home@a") && text.contains("held"),
        "{text}"
    );
}

fn denied(fx: &Fixture) {
    fx.zfs
        .on(&["snapshot", "tank/home@x"], Output::failed(1, DENIED));
}

#[test]
fn a_refused_command_runs_elevated_once_accepted() {
    let fx = Fixture::new();
    denied(&fx);
    let platform = fake();
    let mut ran = run(
        &Create(shared(&fx, Some(platform.clone()))),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    assert!(ran.first.is_ok(), "{:?}", ran.first);
    let result = ran.answer(true);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        platform.elevated(),
        [(
            PathBuf::from(HELPER),
            vec![OsString::from("snapshot"), OsString::from("tank/home@x")]
        )]
    );
}

#[test]
fn declining_elevation_reports_permission_denied() {
    let fx = Fixture::new();
    denied(&fx);
    let platform = fake();
    let mut ran = run(
        &Create(shared(&fx, Some(platform.clone()))),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    let result = ran.answer(false);
    assert!(message(&result).contains("Permission denied"), "{result:?}");
    assert_eq!(platform.elevated(), []);
}

#[test]
fn a_dismissed_prompt_reports_permission_denied() {
    let fx = Fixture::new();
    denied(&fx);
    let mut ran = run(
        &Create(shared(&fx, Some(Arc::new(Prompt::Dismissed)))),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    let result = ran.answer(true);
    assert!(message(&result).contains("Permission denied"), "{result:?}");
}

#[test]
fn without_elevation_the_needed_permission_is_named() {
    let fx = Fixture::new();
    denied(&fx);
    let mut ran = run(
        &Create(shared(&fx, None)),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    let text = message(&ran.first);
    assert!(
        text.contains("zfs allow") && text.contains("snapshot tank/home"),
        "{text}"
    );
    assert!(ran.question().is_none(), "no elevation offered");

    // A prompt that turns out to be missing says the same.
    let mut ran = run(
        &Create(shared(&fx, Some(Arc::new(Prompt::Missing)))),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    let result = ran.answer(true);
    assert!(message(&result).contains("zfs allow"), "{result:?}");
}

#[test]
fn without_the_helper_no_elevation_is_offered() {
    let fx = Fixture::new();
    denied(&fx);
    let platform = fake();
    let shared = Arc::new(Shared {
        datasets: Arc::clone(&fx.datasets),
        platform: None,
        elevate: Some(platform.clone()),
        helper: None,
    });
    let mut ran = run(
        &Create(shared),
        local(&fx.home()),
        Vec::new(),
        &with_name("x"),
    )
    .unwrap_or_else(|| panic!("nothing ran"));
    assert!(message(&ran.first).contains("zfs allow"), "{:?}", ran.first);
    assert!(ran.question().is_none(), "no elevation offered");
    assert_eq!(platform.elevated(), []);
}
