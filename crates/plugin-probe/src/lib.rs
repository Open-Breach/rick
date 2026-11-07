//! The probe plugin: the fixture that proves the plugin boundary
//! (QUESTIONS.md 81) and stands in for a real plugin in the host's tests.
//!
//! It registers one [`Previewer`] for files named `*.probe`, whose `open`
//! spawns a job through a [`JobRunner`], reads the file through the host's
//! provider inside that job and renders the text together with the name of
//! the thread the job ran on, which is the host runtime's; and one
//! [`Action`], `probe.wait`, that starts a job which runs until it is
//! cancelled, the job the acceptance tests cancel from the job table. Both
//! log through `tracing`, so a test can see the plugin's events arrive at
//! the host's subscriber.

#![deny(unsafe_code)]

use std::sync::Arc;

use ext::{
    API_VERSION, Action, ActionCtx, ActionGroup, ActionId, ActionOutcome, Availability,
    Capabilities, ControlId, ControlValue, Extension, Frame, Icon, InfoSection, JobRunner,
    Manifest, PreviewControl, PreviewDoc, Previewer, Provider, Registry, RenderRequest, Shortcut,
    TextBlock, TextLayer, Transport, UiHost, Version, async_trait,
};
use types::{CancelToken, Entry, Error, ExtId, Location, Progress};

/// The extension id.
pub const ID: ExtId = ExtId("probe");

/// The waiting action's id.
pub const WAIT: ActionId = ActionId("probe.wait");

/// The extension: one previewer, one action.
#[derive(Debug)]
pub struct ProbeExtension {
    manifest: Manifest,
}

impl Default for ProbeExtension {
    fn default() -> Self {
        Self::new()
    }
}

impl ProbeExtension {
    /// The extension, with the crate's version in its manifest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            manifest: Manifest {
                id: ID,
                name: "Probe",
                version: Version::parse(env!("CARGO_PKG_VERSION"))
                    .unwrap_or_else(|_| Version::new(0, 0, 0)),
                api_version: API_VERSION,
                requires: Capabilities::empty(),
            },
        }
    }
}

impl Extension for ProbeExtension {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn register(&self, reg: &mut Registry) -> Result<(), Error> {
        tracing::info!("probe: registering");
        reg.previewer(Arc::new(ProbePreviewer {
            jobs: JobRunner::new(),
        }));
        reg.action(Arc::new(Wait));
        Ok(())
    }
}

ext::plugin!(|| Arc::new(ProbeExtension::new()) as Arc<dyn Extension>);

/// Previews `*.probe` files through a job.
#[derive(Debug)]
pub struct ProbePreviewer {
    jobs: JobRunner,
}

#[async_trait]
impl Previewer for ProbePreviewer {
    fn id(&self) -> ExtId {
        ID
    }

    fn score(&self, entry: &Entry, _sniff: &[u8]) -> u8 {
        let probe = std::path::Path::new(&entry.name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("probe"));
        if probe { 255 } else { 0 }
    }

    async fn open(
        &self,
        loc: &Location,
        provider: Arc<dyn Provider>,
        cancel: CancelToken,
    ) -> Result<Box<dyn PreviewDoc>, Error> {
        tracing::info!(path = %loc.path.to_string_lossy(), "probe: opening");
        let loc = loc.clone();
        let handle = self.jobs.spawn(move |ctx| async move {
            let bytes = ext::read_bytes(&*provider, &loc, None, &ctx.cancel).await?;
            let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            ctx.progress.report(Progress::Bytes {
                done: len,
                total: Some(len),
            });
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned();
            tracing::info!(%thread, "probe: read in a job");
            Ok::<_, Error>((bytes, thread))
        });
        if cancel.is_cancelled() {
            handle.cancel.cancel();
        }
        let (bytes, thread) = handle.result.await.map_err(|_| Error::Cancelled)??;
        Ok(Box::new(ProbeDoc {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            thread,
        }))
    }
}

/// The document: the file's text and the job's thread.
#[derive(Debug)]
pub struct ProbeDoc {
    text: String,
    thread: String,
}

impl PreviewDoc for ProbeDoc {
    fn pages(&self) -> Option<usize> {
        None
    }

    fn render(&mut self, _page: usize, _req: RenderRequest) -> Result<Frame, Error> {
        Ok(Frame::Text(TextBlock {
            text: self.text.clone(),
            language: None,
            spans: Vec::new(),
        }))
    }

    fn text_layer(&self) -> Option<&TextLayer> {
        None
    }

    fn sections(&self) -> Vec<InfoSection> {
        vec![InfoSection {
            id: "file",
            title: "File info",
            icon: Icon::Info,
            rows: vec![("Job thread".to_owned(), self.thread.clone())],
        }]
    }

    fn status(&self) -> Vec<String> {
        vec![format!("job ran on {}", self.thread)]
    }

    fn controls(&self) -> Vec<PreviewControl> {
        Vec::new()
    }

    fn set_control(&mut self, _id: ControlId, _value: ControlValue) -> Result<(), Error> {
        Err(Error::Unsupported("the probe has no controls"))
    }

    fn transport(&mut self) -> Option<&mut dyn Transport> {
        None
    }
}

/// Starts a job that runs until it is cancelled.
#[derive(Debug)]
pub struct Wait;

impl Action for Wait {
    fn id(&self) -> ActionId {
        WAIT
    }

    fn label(&self) -> &'static str {
        "Wait until cancelled"
    }

    fn group(&self) -> ActionGroup {
        ActionGroup::Extension("Probe")
    }

    fn default_shortcut(&self) -> Option<Shortcut> {
        None
    }

    fn availability(&self, _ctx: &ActionCtx) -> Availability {
        Availability::Enabled
    }

    fn run(
        &self,
        _ctx: ActionCtx,
        jobs: &JobRunner,
        _ui: &dyn UiHost,
    ) -> Result<ActionOutcome, Error> {
        tracing::info!("probe: waiting");
        let handle = jobs.spawn_named("Probe: wait until cancelled", |ctx| async move {
            ctx.cancel.cancelled().await;
            Err::<(), Error>(Error::Cancelled)
        });
        Ok(ActionOutcome::Started(handle.id))
    }
}
