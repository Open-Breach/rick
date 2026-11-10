//! Snapshot management: *Create snapshot*, *Rename snapshot* and *Destroy
//! snapshot*, and the permission rule they share.
//!
//! Each command runs first with the user's own rights, as a job. When ZFS
//! refuses it for lack of permission, the job hands the application a
//! question ("Run as administrator?") through [`ActionOutcome::Later`]; on
//! yes the same commands run once more through the platform's elevation
//! prompt, on no or a dismissed prompt the user is told that permission was
//! denied. Without an elevation prompt (no `pkexec`) the failure names the
//! `zfs allow` permissions the user would need. Arguments come only from
//! cached dataset names and validated snapshot names, never from free text.
//! Every job drops its dataset's cached snapshot list when it ends.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use ext::platform::{Elevate, ElevatedOutput};
use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Continuation, Field,
    Form, FormLater, JobRunner, PreviewRow, Shortcut, UiHost, Values, t,
};
use types::{Error, Progress};

use crate::actions::{GROUP, Shared, local_path};
use crate::datasets::Datasets;
use crate::parse::{NameProblem, check_name, default_name};
use crate::provider::{PROVIDER_ID, Target, target};
use crate::zfs::{Output, ZfsError, classify, os_args};

/// *Create snapshot*.
pub const CREATE: ActionId = ActionId("zfs.create");
/// *Rename snapshot*.
pub const RENAME: ActionId = ActionId("zfs.rename");
/// *Destroy snapshot*.
pub const DESTROY: ActionId = ActionId("zfs.destroy");

/// The `zfs allow` permissions a command needs, by its verb.
fn permissions(verb: &str) -> &'static str {
    match verb {
        "snapshot" => "snapshot",
        "rename" => "rename,mount,create",
        "destroy" => "destroy,mount",
        _ => "",
    }
}

/// A failure the user reads, as the job's error.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Refused(pub String);

fn refused(text: String) -> Error {
    Error::Extension {
        ext: crate::ID,
        source: Box::new(Refused(text)),
    }
}

/// What a batch of management commands works on.
#[derive(Debug, Clone)]
pub struct Batch {
    /// The dataset whose snapshot list or properties the commands change.
    pub dataset: String,
    /// The commands, each `zfs` arguments.
    pub commands: Vec<Vec<String>>,
    /// Whether the commands depend on each other's order: the batch stops
    /// at the first failure, and a refusal hands that command and every
    /// one after it to the elevated run (property changes). Otherwise each
    /// command runs on its own (one destroy per snapshot).
    pub ordered: bool,
}

impl Batch {
    /// A batch of independent commands.
    #[must_use]
    pub fn new(dataset: String, commands: Vec<Vec<String>>) -> Self {
        Self {
            dataset,
            commands,
            ordered: false,
        }
    }

    /// A batch whose commands run in order, stopping at the first failure.
    #[must_use]
    pub fn ordered(dataset: String, commands: Vec<Vec<String>>) -> Self {
        Self {
            dataset,
            commands,
            ordered: true,
        }
    }

    fn verb(&self) -> &str {
        self.commands
            .first()
            .and_then(|c| c.first())
            .map_or("", String::as_str)
    }

    /// The `zfs allow` permissions the batch needs: by its verb, and for
    /// property changes the names of the properties it sets or resets.
    fn permissions(&self) -> String {
        match self.verb() {
            "set" | "inherit" => {
                let mut names: Vec<&str> = Vec::new();
                for command in &self.commands {
                    for name in changed_properties(command) {
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                names.join(",")
            }
            verb => permissions(verb).to_owned(),
        }
    }

    /// The message for a refusal nobody can lift here: the permissions
    /// `zfs allow` would have to grant.
    fn needs(&self) -> String {
        let user = std::env::var("USER").unwrap_or_else(|_| "<user>".to_owned());
        t!(
            "zfs-needs-permission",
            dataset = self.dataset.clone(),
            command = format!("zfs allow {user} {} {}", self.permissions(), self.dataset)
        )
    }
}

/// How the first, unprivileged run ended.
#[derive(Debug)]
pub enum FirstRun {
    /// Everything succeeded.
    Done,
    /// These commands were refused for lack of permission; the rest
    /// succeeded or failed with the errors in `failed`.
    Denied {
        /// The refused commands, to run elevated.
        commands: Vec<Vec<String>>,
        /// Messages of the other failures.
        failed: String,
    },
    /// Some commands failed for other reasons; their messages.
    Failed(String),
}

/// The properties a `zfs set p=v … ds` or `zfs inherit p ds` changes;
/// empty for other commands.
fn changed_properties(command: &[String]) -> Vec<&str> {
    match command.first().map(String::as_str) {
        Some("set") => command[1..command.len().saturating_sub(1)]
            .iter()
            .filter_map(|a| a.split_once('=').map(|(p, _)| p))
            .collect(),
        Some("inherit") => command.get(1).map(String::as_str).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The message for one failure, by its class.
fn describe(command: &[String], error: &ZfsError) -> String {
    let name = command.last().cloned().unwrap_or_default();
    let properties = changed_properties(command);
    if !properties.is_empty() {
        return t!(
            "zfs-property-failed",
            properties = properties.join(", "),
            reason = error.message().to_owned()
        );
    }
    match error {
        ZfsError::AlreadyExists(_) if command.first().is_some_and(|v| v == "snapshot") => {
            t!("zfs-snapshot-exists", name = name)
        }
        ZfsError::Busy(message) => t!("zfs-snapshot-busy", name = name, reason = message.clone()),
        other => other.message().to_owned(),
    }
}

/// Runs `batch` with the user's own rights and drops the dataset's cached
/// list.
#[must_use]
pub fn run_first(datasets: &Datasets, batch: &Batch) -> FirstRun {
    let mut denied = Vec::new();
    let mut failed = String::new();
    for (i, command) in batch.commands.iter().enumerate() {
        let outcome = datasets
            .zfs()
            .run(command)
            .map_err(|e| ZfsError::Other(e.to_string()))
            .and_then(classify);
        match outcome {
            Ok(_) => {}
            Err(ZfsError::PermissionDenied(_)) if batch.ordered => {
                denied.extend(batch.commands[i..].iter().cloned());
                break;
            }
            Err(ZfsError::PermissionDenied(_)) => denied.push(command.clone()),
            Err(error) => {
                let _ = writeln!(failed, "{}", describe(command, &error));
                if batch.ordered {
                    break;
                }
            }
        }
    }
    datasets.invalidate(&batch.dataset);
    match (denied.is_empty(), failed.is_empty()) {
        (true, true) => FirstRun::Done,
        (true, false) => FirstRun::Failed(failed.trim_end().to_owned()),
        (false, _) => FirstRun::Denied {
            commands: denied,
            failed: failed.trim_end().to_owned(),
        },
    }
}

/// Runs the refused commands of `batch` once more through the elevation
/// prompt, each as one call of the privileged helper at `program`
/// ([`crate::helper`]), which takes the same arguments as `zfs`.
///
/// # Errors
///
/// Permission denied when the prompt was dismissed or ZFS refuses again;
/// the `zfs allow` message when there is no prompt; ZFS's messages for
/// other failures.
pub fn run_elevated(
    datasets: &Datasets,
    elevate: &dyn Elevate,
    program: &std::path::Path,
    batch: &Batch,
) -> Result<(), Error> {
    let mut failed = String::new();
    let mut denied = false;
    for command in &batch.commands {
        match elevate.run_elevated(program, &os_args(command)) {
            Ok(ElevatedOutput {
                code,
                stdout,
                stderr,
            }) => {
                let output = Output {
                    code,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                };
                match classify(output) {
                    Ok(_) => {}
                    Err(ZfsError::PermissionDenied(_)) => denied = true,
                    Err(error) => {
                        let _ = writeln!(failed, "{}", describe(command, &error));
                    }
                }
                if batch.ordered && (denied || !failed.is_empty()) {
                    break;
                }
            }
            Err(Error::PermissionDenied) => {
                denied = true;
                break;
            }
            Err(Error::Unsupported(_)) => {
                datasets.invalidate(&batch.dataset);
                return Err(refused(batch.needs()));
            }
            Err(error) => {
                let _ = writeln!(failed, "{error}");
            }
        }
    }
    datasets.invalidate(&batch.dataset);
    if denied {
        return Err(refused(t!("zfs-permission-denied")));
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(refused(failed.trim_end().to_owned()))
    }
}

/// Spawns `batch` as a job named `name`. Refused commands lead to the
/// question whether to run them as administrator, through the returned
/// [`ActionOutcome::Later`]; everything else ends in the job's result.
#[must_use]
pub fn spawn(shared: &Arc<Shared>, jobs: &JobRunner, name: String, batch: Batch) -> ActionOutcome {
    let (sender, later) = FormLater::channel();
    let shared = Arc::clone(shared);
    let runner = jobs.clone();
    let job_name = name.clone();
    jobs.spawn_named(name, move |job| async move {
        job.progress.report(Progress::Message(job_name.clone()));
        let first = {
            let datasets = Arc::clone(&shared.datasets);
            let batch = batch.clone();
            ext::blocking(move || Ok(run_first(&datasets, &batch))).await?
        };
        match first {
            FirstRun::Done => Ok(()),
            FirstRun::Failed(message) => Err(refused(message)),
            FirstRun::Denied { commands, failed } => {
                let denied = Batch {
                    dataset: batch.dataset.clone(),
                    commands,
                    ordered: batch.ordered,
                };
                let (Some(elevate), Some(program)) =
                    (shared.elevate.clone(), shared.helper.clone())
                else {
                    let mut message = denied.needs();
                    if !failed.is_empty() {
                        message = format!("{failed}\n{message}");
                    }
                    return Err(refused(message));
                };
                let form = Form::new(
                    t!("zfs-run-as-administrator"),
                    t!("zfs-run-as-administrator-ok"),
                )
                .note(t!("zfs-permission-denied-ask", name = job_name.clone()));
                let then = Continuation::new(job_name.clone(), move |values: Option<Values>| {
                    if values.is_none() {
                        return Err(refused(t!("zfs-permission-denied")));
                    }
                    Ok(spawn_elevated(
                        &shared, &runner, job_name, elevate, program, denied,
                    ))
                });
                let _ = sender.send((form, then));
                if failed.is_empty() {
                    Ok(())
                } else {
                    Err(refused(failed))
                }
            }
        }
    });
    ActionOutcome::Later(later)
}

fn spawn_elevated(
    shared: &Arc<Shared>,
    jobs: &JobRunner,
    name: String,
    elevate: Arc<dyn Elevate>,
    program: PathBuf,
    batch: Batch,
) -> ActionOutcome {
    let datasets = Arc::clone(&shared.datasets);
    let handle = jobs.spawn_named(
        t!("zfs-as-administrator", name = name),
        move |_job| async move {
            ext::blocking(move || run_elevated(&datasets, &*elevate, &program, &batch)).await
        },
    );
    ActionOutcome::Started(handle.id)
}

/// The form asking for a snapshot name for `dataset`, starting at
/// `name`; the preview row shows `dataset@name` and marks a name ZFS would
/// refuse, which keeps the dialogue from being confirmed.
fn name_form(title: String, ok: String, dataset: &str, name: String, extra: Option<Field>) -> Form {
    let mut form = Form::new(title, ok).field(
        Field::text("name", t!("zfs-snapshot-name"), name, "")
            .mono()
            .required(t!("zfs-name-empty")),
    );
    if let Some(field) = extra {
        form = form.field(field);
    }
    let dataset = dataset.to_owned();
    form.preview(Arc::new(move |values: &Values| {
        let name = values.text("name");
        vec![PreviewRow {
            from: dataset.clone(),
            to: format!("{dataset}@{name}"),
            problem: check_name(&dataset, name).err().map(NameProblem::text),
        }]
    }))
}

/// The dataset of the pane's live location.
fn live_dataset(shared: &Shared, ctx: &ActionCtx) -> Option<String> {
    let dir = local_path(&ctx.location)?;
    shared.datasets.dataset_of(&dir).map(|fs| fs.name.clone())
}

/// The snapshots selected at the dataset level of the `zfs` provider, with
/// the dataset; `None` when the pane is not at a dataset level.
fn selected_snapshots(ctx: &ActionCtx) -> Option<(String, Vec<String>)> {
    let Ok(Target::Dataset(dataset)) = target(&ctx.location) else {
        return None;
    };
    let names = ctx
        .selection
        .iter()
        .filter_map(
            |e| match target(&types::Location::new(PROVIDER_ID, e.path.clone())) {
                Ok(Target::Snapshot {
                    dataset: of,
                    snapshot,
                    rel,
                }) if of == dataset && rel.as_os_str().is_empty() => Some(snapshot),
                _ => None,
            },
        )
        .collect();
    Some((dataset, names))
}

/// Creates a snapshot of the dataset the pane's live folder lies in.
#[derive(Debug)]
pub struct Create(pub Arc<Shared>);

impl Action for Create {
    fn id(&self) -> ActionId {
        CREATE
    }

    fn label(&self) -> String {
        t!("zfs-create-snapshot")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if live_dataset(&self.0, ctx).is_some() {
            Availability::Enabled
        } else {
            Availability::Hidden
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let dataset = live_dataset(&self.0, &ctx)
            .ok_or(Error::Unsupported("the folder is in no ZFS dataset"))?;
        let form = name_form(
            t!("zfs-create-snapshot-of", dataset = dataset.clone()),
            t!("zfs-create"),
            &dataset,
            default_name(),
            Some(Field::check("recursive", t!("zfs-include-children"), false)),
        );
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let name = values.text("name").to_owned();
        check_name(&dataset, &name).map_err(|p| refused(p.text()))?;
        let mut command = vec!["snapshot".to_owned()];
        if values.check("recursive") {
            command.push("-r".to_owned());
        }
        let full = format!("{dataset}@{name}");
        command.push(full.clone());
        Ok(spawn(
            &self.0,
            jobs,
            t!("zfs-creating", name = full),
            Batch::new(dataset, vec![command]),
        ))
    }
}

/// Renames one snapshot selected in its dataset's snapshot list.
#[derive(Debug)]
pub struct Rename(pub Arc<Shared>);

impl Action for Rename {
    fn id(&self) -> ActionId {
        RENAME
    }

    fn label(&self) -> String {
        t!("zfs-rename-snapshot")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match selected_snapshots(ctx) {
            None => Availability::Hidden,
            Some((_, names)) if names.len() == 1 && ctx.selection.len() == 1 => {
                Availability::Enabled
            }
            Some(_) => Availability::Disabled(t!("zfs-select-one-snapshot")),
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some((dataset, names)) = selected_snapshots(&ctx) else {
            return Err(Error::Unsupported("not in a snapshot list"));
        };
        let [old] = names.as_slice() else {
            return Err(Error::Unsupported("select one snapshot"));
        };
        let form = name_form(
            t!("zfs-rename-snapshot"),
            t!("zfs-rename"),
            &dataset,
            old.clone(),
            None,
        );
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let new = values.text("name").to_owned();
        if new == *old {
            return Ok(ActionOutcome::Done);
        }
        check_name(&dataset, &new).map_err(|p| refused(p.text()))?;
        let command = vec![
            "rename".to_owned(),
            format!("{dataset}@{old}"),
            format!("{dataset}@{new}"),
        ];
        Ok(spawn(
            &self.0,
            jobs,
            t!("zfs-renaming", name = format!("{dataset}@{old}")),
            Batch::new(dataset, vec![command]),
        ))
    }
}

/// Destroys the snapshots selected in their dataset's snapshot list, after
/// a confirmation naming each one.
#[derive(Debug)]
pub struct Destroy(pub Arc<Shared>);

impl Action for Destroy {
    fn id(&self) -> ActionId {
        DESTROY
    }

    fn label(&self) -> String {
        t!("zfs-destroy-snapshot")
    }

    fn group(&self) -> ActionGroup {
        GROUP
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, ctx: &ActionCtx) -> Availability {
        match selected_snapshots(ctx) {
            None => Availability::Hidden,
            Some((_, names)) if !names.is_empty() => Availability::Enabled,
            Some(_) => Availability::Disabled(t!("zfs-select-snapshots")),
        }
    }

    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        let Some((dataset, names)) = selected_snapshots(&ctx) else {
            return Err(Error::Unsupported("not in a snapshot list"));
        };
        if names.is_empty() {
            return Err(Error::Unsupported("select snapshots"));
        }
        let full: Vec<String> = names.iter().map(|n| format!("{dataset}@{n}")).collect();
        if !ui.confirm(
            &t!("zfs-destroy-snapshot"),
            &t!("zfs-destroy-confirm", names = full.join("\n")),
        ) {
            return Ok(ActionOutcome::Done);
        }
        let commands = full
            .iter()
            .map(|name| vec!["destroy".to_owned(), name.clone()])
            .collect();
        Ok(spawn(
            &self.0,
            jobs,
            t!("zfs-destroying", count = full.len()),
            Batch::new(dataset, commands),
        ))
    }
}
