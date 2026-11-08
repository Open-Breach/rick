//! The five actions and the jobs behind them.

use ext::t;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ext::{
    Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability, Field, Form, JobCtx,
    JobRunner, PreviewRow, Shortcut, UiHost, Values,
};
use ext_vfs_archive::create::{self, CREATABLE, Source};
use ext_vfs_archive::extract::{self, Extracting, Target};
use ext_vfs_archive::{Codec, Format};
use types::{Entry, EntryKind, Error, Location, Progress, ProviderId, Resolution};

/// Compress the selection into a zip next to it.
pub const COMPRESS: ActionId = ActionId("compress.compress");
/// Compress the selection with a chosen name, format and folder.
pub const COMPRESS_AS: ActionId = ActionId("compress.compress_as");
/// Extract the selected archives into the current folder.
pub const EXTRACT_HERE: ActionId = ActionId("compress.extract_here");
/// Extract each selected archive into a folder named after it.
pub const EXTRACT_FOLDER: ActionId = ActionId("compress.extract_folder");
/// Extract the selected archives into a chosen folder.
pub const EXTRACT_TO: ActionId = ActionId("compress.extract_to");

/// The five actions.
pub fn all() -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(Compress),
        Arc::new(CompressAs),
        Arc::new(Extract::Here),
        Arc::new(Extract::Folder),
        Arc::new(Extract::To),
    ]
}

/// The OS path of a local entry or location.
fn local_path(loc: &Location) -> Result<PathBuf, Error> {
    if loc.provider != ProviderId("local") {
        return Err(Error::Unsupported(
            "archives are made and unpacked on the local disk",
        ));
    }
    ext::platform::paths::to_path(&loc.path)
}

fn location_of(entry: &Entry) -> Location {
    Location::new(entry.id.provider, entry.path.clone())
}

/// Whether every selected entry is a local file or folder.
fn all_local(ctx: &ActionCtx) -> bool {
    ctx.location.provider == ProviderId("local")
        && ctx
            .selection
            .iter()
            .all(|e| e.id.provider == ProviderId("local"))
}

/// Whether a name is an archive the provider reads, by its extension.
#[must_use]
pub fn is_archive(name: &str) -> bool {
    Format::by_extension(name).is_some()
}

/// The archive's name without its archive extension: `a.tar.gz` → `a`,
/// `b.zip` → `b`, `c.7z` → `c`, `notes.txt.gz` → `notes.txt`.
#[must_use]
pub fn archive_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let Some((stem, ext)) = lower.rsplit_once('.') else {
        return name.to_owned();
    };
    let mut cut = name.len() - ext.len() - 1;
    if Codec::by_extension(ext).is_some()
        && let Some((inner_stem, inner_ext)) = stem.rsplit_once('.')
        && inner_ext == "tar"
        && !inner_stem.is_empty()
    {
        cut = inner_stem.len();
    }
    if cut == 0 {
        return name.to_owned();
    }
    name[..cut].to_owned()
}

/// "3 items" or the one entry's name.
fn describe(entries: &[Entry]) -> String {
    match entries {
        [one] => one.name.clone(),
        many => format!("{} items", many.len()),
    }
}

/// The name an archive of the selection takes, without extension: the one
/// item's name without its extension, or the folder's name for several.
fn default_stem(ctx: &ActionCtx) -> String {
    match ctx.selection.as_slice() {
        [one] => {
            let name = &one.name;
            match (one.kind == EntryKind::Dir, name.rsplit_once('.')) {
                (false, Some((stem, _))) if !stem.is_empty() => stem.to_owned(),
                _ => name.clone(),
            }
        }
        _ => ctx
            .location
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Archive".to_owned()),
    }
}

/// `stem.ext`, numbered when taken in `dir`.
fn free_name(dir: &Path, stem: &str, ext: &str) -> String {
    let base = format!("{stem}.{ext}");
    if !dir.join(&base).exists() {
        return base;
    }
    for n in 2..10_000u32 {
        let candidate = format!("{stem} ({n}).{ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    base
}

/// Spawns the job that writes `sources` into `out` as `format`.
fn spawn_compress(
    jobs: &JobRunner,
    what: &str,
    format: Format,
    out: PathBuf,
    sources: Vec<Source>,
) -> ActionOutcome {
    let name = format!("Compress {what}");
    let handle = jobs.spawn_pausable(name, move |job| async move {
        let total: u64 = sources.iter().map(Source::size).sum();
        let progress = job.progress.clone();
        progress.report(Progress::Bytes {
            done: 0,
            total: Some(total),
        });
        let cancel = job.cancel.clone();
        let pause = job.pause.clone();
        ext::blocking(move || {
            let span = tracing::info_span!("compress", path = %out.display());
            let _enter = span.enter();
            create::create(format, &out, &sources, &mut |_source, done| {
                if cancel.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                if pause.is_paused() {
                    ext::runtime::handle().block_on(pause.wait());
                }
                progress.report(Progress::Bytes {
                    done,
                    total: Some(total),
                });
                Ok(())
            })?;
            progress.report(Progress::Bytes {
                done: total,
                total: Some(total),
            });
            Ok(())
        })
        .await
    });
    ActionOutcome::Started(handle.id)
}

/// Compress the selection into a zip next to it.
pub struct Compress;

impl Action for Compress {
    fn id(&self) -> ActionId {
        COMPRESS
    }
    fn label(&self) -> String {
        t!("compress-compress")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !all_local(ctx) {
            return Availability::Hidden;
        }
        if ctx.selection.is_empty() {
            Availability::Disabled(t!("compress-select-what-to-compress-first"))
        } else {
            Availability::Enabled
        }
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let dir = local_path(&ctx.location)?;
        let items: Vec<PathBuf> = ctx
            .selection
            .iter()
            .map(|e| local_path(&location_of(e)))
            .collect::<Result<_, _>>()?;
        let out = dir.join(free_name(&dir, &default_stem(&ctx), "zip"));
        let sources = create::collect(&items)?;
        Ok(spawn_compress(
            jobs,
            &describe(&ctx.selection),
            Format::Zip,
            out,
            sources,
        ))
    }
}

/// The creatable formats offered for a selection: every archive format,
/// plus the single-file codecs for exactly one file.
fn formats_for(ctx: &ActionCtx) -> Vec<(&'static str, Format)> {
    let single_file = matches!(ctx.selection.as_slice(), [one] if one.kind == EntryKind::File);
    CREATABLE
        .iter()
        .copied()
        .filter(|(_, format)| single_file || !matches!(format, Format::Single(_)))
        .collect()
}

/// Compress the selection with a chosen name, format and folder.
pub struct CompressAs;

impl Action for CompressAs {
    fn id(&self) -> ActionId {
        COMPRESS_AS
    }
    fn label(&self) -> String {
        t!("compress-compress-to")
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        Compress.availability(ctx)
    }
    fn run(
        &self,
        ctx: ActionCtx,
        jobs: &JobRunner,
        ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let dir = local_path(&ctx.location)?;
        let formats = formats_for(&ctx);
        let what = describe(&ctx.selection);
        let preview_formats: Vec<&'static str> = formats.iter().map(|(ext, _)| *ext).collect();
        let preview_what = what.clone();
        let form = Form::new(format!("Compress {what}"), t!("compress-compress"))
            .field(Field::text(
                "name",
                t!("compress-archive-name"),
                default_stem(&ctx),
                "without the extension",
            ))
            .field(Field::choice(
                "format",
                t!("compress-format"),
                formats.iter().map(|(ext, _)| (*ext).to_owned()).collect(),
                0,
            ))
            .field(Field::folder(
                "into",
                t!("compress-into"),
                dir.display().to_string(),
            ))
            .preview(Arc::new(move |values: &Values| {
                let stem = values.text("name").trim();
                let ext = preview_formats
                    .get(values.choice("format"))
                    .copied()
                    .unwrap_or("zip");
                vec![PreviewRow {
                    from: preview_what.clone(),
                    to: format!("{stem}.{ext}"),
                    problem: (stem.is_empty() || stem.contains(['/', '\\']))
                        .then(|| "name the archive".to_owned()),
                }]
            }));
        let Some(values) = ui.form(form) else {
            return Ok(ActionOutcome::Done);
        };
        let stem = values.text("name").trim().to_owned();
        if stem.is_empty() || stem.contains(['/', '\\']) {
            return Err(Error::Unsupported("name the archive"));
        }
        let (ext, format) = formats
            .get(values.choice("format"))
            .copied()
            .ok_or(Error::Unsupported("no such format"))?;
        let into = values.text("into").trim();
        let into = if into.is_empty() {
            dir
        } else {
            PathBuf::from(into)
        };
        std::fs::create_dir_all(&into).map_err(Error::from_io)?;
        let items: Vec<PathBuf> = ctx
            .selection
            .iter()
            .map(|e| local_path(&location_of(e)))
            .collect::<Result<_, _>>()?;
        let out = into.join(free_name(&into, &stem, ext));
        let sources = create::collect(&items)?;
        Ok(spawn_compress(jobs, &what, format, out, sources))
    }
}

/// Where an extraction unpacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extract {
    /// Into the current folder.
    Here,
    /// Into a folder named after each archive.
    Folder,
    /// Into a chosen folder.
    To,
}

/// What one job remembers from an "apply to all" answer.
#[derive(Default)]
struct Policy {
    all: Option<Resolution>,
}

impl Policy {
    /// Where `entry` of `archive` lands under `into`: a folder over a
    /// folder merges without a word, anything else over an existing entry
    /// asks through the job's conflict queue.
    fn land(
        &mut self,
        job: &JobCtx,
        archive_root: &Location,
        into: &Path,
        entry: &Extracting,
        remaining: u64,
    ) -> Result<Target, Error> {
        let rel = String::from_utf8_lossy(&entry.path).into_owned();
        let target = into.join(&rel);
        let Ok(existing) = std::fs::symlink_metadata(&target) else {
            return Ok(Target::Write(target));
        };
        if existing.is_dir() && entry.kind == EntryKind::Dir {
            return Ok(Target::Write(target));
        }
        let source = Location::new(
            archive_root.provider,
            archive_root.path.join(entry.path.clone()),
        );
        let target_loc = Location::new(
            ProviderId("local"),
            ext::platform::paths::from_path(&target),
        );
        let resolution = if let Some(standing) = &self.all {
            standing.clone()
        } else {
            let answer = ext::runtime::handle().block_on(job.conflicts.ask(
                job,
                &source,
                &target_loc,
                existing.is_dir(),
                remaining,
            ));
            if answer.apply_to_all {
                self.all = Some(answer.resolution.clone());
            }
            answer.resolution
        };
        match resolution {
            Resolution::Skip => Ok(Target::Skip),
            Resolution::Cancel => Err(Error::Cancelled),
            Resolution::Overwrite => Ok(Target::Write(target)),
            Resolution::Rename(name) => {
                let dir = target.parent().unwrap_or(into).to_path_buf();
                let base = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(rel);
                let chosen = if self.all.is_some() {
                    (2..10_000u32)
                        .map(|n| types::numbered_name(&base, n))
                        .find(|c| !dir.join(c).exists())
                        .unwrap_or(base)
                } else {
                    name
                };
                Ok(Target::Write(dir.join(chosen)))
            }
        }
    }
}

/// Spawns the job that unpacks every (archive, folder) pair.
fn spawn_extract(jobs: &JobRunner, what: &str, pairs: Vec<(PathBuf, PathBuf)>) -> ActionOutcome {
    let name = format!("Extract {what}");
    let handle = jobs.spawn_pausable(name, move |job| async move {
        let job = Arc::new(job);
        let worker = Arc::clone(&job);
        ext::blocking(move || {
            let job = worker;
            let mut policy = Policy::default();
            let archives = pairs.len() as u64;
            for (index, (archive, into)) in pairs.iter().enumerate() {
                let span = tracing::info_span!("extract", path = %archive.display());
                let _enter = span.enter();
                let format = Format::detect(archive)?;
                // The sizes are in the index of every format but a tar,
                // which has none and is read once, not twice.
                let total = if matches!(format, Format::Tar(_)) {
                    None
                } else {
                    let index = ext_vfs_archive::index(archive)?;
                    Some(index.items().iter().filter_map(|i| i.size).sum::<u64>())
                };
                let count = if matches!(format, Format::Tar(_)) {
                    None
                } else {
                    ext_vfs_archive::index(archive)
                        .ok()
                        .map(|i| i.items().len() as u64)
                };
                std::fs::create_dir_all(into).map_err(Error::from_io)?;
                let root = Location::new(
                    ProviderId::ARCHIVE,
                    ext::platform::paths::from_path(archive),
                );
                let mut done_bytes = 0u64;
                let mut done_items = 0u64;
                job.progress.report(Progress::Bytes { done: 0, total });
                extract::extract(archive, format, &mut |entry| {
                    if job.cancel.is_cancelled() {
                        return Err(Error::Cancelled);
                    }
                    if job.pause.is_paused() {
                        ext::runtime::handle().block_on(job.pause.wait());
                    }
                    let remaining = count.map_or(0, |c| c.saturating_sub(done_items + 1))
                        + (archives - index as u64 - 1);
                    let target = policy.land(&job, &root, into, entry, remaining)?;
                    done_items += 1;
                    done_bytes += entry.size.unwrap_or(0);
                    job.progress.report(Progress::Bytes {
                        done: done_bytes,
                        total,
                    });
                    Ok(target)
                })?;
                job.progress.report(Progress::Items {
                    done: index as u64 + 1,
                    total: Some(archives),
                });
            }
            Ok(())
        })
        .await
    });
    ActionOutcome::Started(handle.id)
}

impl Action for Extract {
    fn id(&self) -> ActionId {
        match self {
            Self::Here => EXTRACT_HERE,
            Self::Folder => EXTRACT_FOLDER,
            Self::To => EXTRACT_TO,
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Here => t!("compress-extract-here"),
            Self::Folder => t!("compress-extract-to-folder"),
            Self::To => t!("compress-extract-to"),
        }
    }
    fn group(&self) -> ActionGroup {
        ActionGroup::Core
    }
    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }
    fn availability(&self, ctx: &ActionCtx) -> Availability {
        if !all_local(ctx) || ctx.selection.is_empty() {
            return Availability::Hidden;
        }
        if ctx
            .selection
            .iter()
            .all(|e| e.kind == EntryKind::File && is_archive(&e.name))
        {
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
        if ctx.selection.is_empty() {
            return Ok(ActionOutcome::Done);
        }
        let dir = local_path(&ctx.location)?;
        let archives: Vec<PathBuf> = ctx
            .selection
            .iter()
            .map(|e| local_path(&location_of(e)))
            .collect::<Result<_, _>>()?;
        let what = describe(&ctx.selection);
        let pairs: Vec<(PathBuf, PathBuf)> = match self {
            Self::Here => archives.into_iter().map(|a| (a, dir.clone())).collect(),
            Self::Folder => archives
                .into_iter()
                .map(|a| {
                    let stem = a.file_name().map_or_else(
                        || "Archive".to_owned(),
                        |n| archive_stem(&n.to_string_lossy()),
                    );
                    let folder = dir.join(stem);
                    (a, folder)
                })
                .collect(),
            Self::To => {
                let form = Form::new(format!("Extract {what}"), t!("compress-extract")).field(
                    Field::folder("into", t!("compress-into"), dir.display().to_string()),
                );
                let Some(values) = ui.form(form) else {
                    return Ok(ActionOutcome::Done);
                };
                let into = values.text("into").trim().to_owned();
                if into.is_empty() {
                    return Err(Error::Unsupported("name the folder to extract to"));
                }
                let into = PathBuf::from(into);
                archives.into_iter().map(|a| (a, into.clone())).collect()
            }
        };
        Ok(spawn_extract(jobs, &what, pairs))
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn archive_stems_drop_the_archive_extension_only() {
        assert_eq!(archive_stem("a.tar.gz"), "a");
        assert_eq!(archive_stem("a.TAR.ZST"), "a");
        assert_eq!(archive_stem("b.zip"), "b");
        assert_eq!(archive_stem("c.7z"), "c");
        assert_eq!(archive_stem("notes.txt.gz"), "notes.txt");
        assert_eq!(archive_stem("plain"), "plain");
        assert_eq!(archive_stem(".hidden"), ".hidden");
        assert!(is_archive("x.tgz"));
        assert!(!is_archive("x.txt"));
    }
}
