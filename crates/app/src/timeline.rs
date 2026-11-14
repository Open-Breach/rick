//! The time slider: a pane's timeline is read from
//! the decorators that offer timelines, in a job and only while the pane's
//! bar is shown; the answers of several sources are merged and kept per
//! location for [`TIMELINE_FOR`]. Moving along the bar is a
//! [`Command::TimeTravel`](crate::Command::TimeTravel), recorded as one
//! history step per run, and a point that does not hold the folder falls
//! back to its nearest parent with a notice.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ext::{Decorator, Level, Timeline};
use model::TimelineState;
use types::{CancelToken, Error, Location};

use crate::{App, Event, Msg, PaneId};

/// How long a timeline read for a location is used again before the
/// sources are asked anew; the ZFS source caches its snapshots as long.
pub(crate) const TIMELINE_FOR: Duration = Duration::from_secs(30);

/// The application's timeline bookkeeping.
#[derive(Default)]
pub(crate) struct Timelines {
    /// Merged answers by location, with when they arrived; `None` is an
    /// answer without a point.
    cache: HashMap<Location, (Instant, Option<Arc<Timeline>>)>,
    /// The read running per pane: its number and its cancel token.
    reading: HashMap<PaneId, (u64, CancelToken)>,
    /// Numbers the reads, so a late answer is told from the current.
    generation: u64,
    /// Where a pane's time travel aimed, so a point that does not hold the
    /// folder falls back to its nearest parent.
    travel: HashMap<PaneId, Location>,
}

impl App {
    /// The decorators that offer a timeline for `location`.
    fn timeline_sources(&self, location: &Location) -> Vec<Arc<dyn Decorator>> {
        self.registry
            .decorators()
            .iter()
            .filter(|d| d.offers_timelines() && d.applies_to(location))
            .cloned()
            .collect()
    }

    /// Brings the pane's timeline in line with the folder it shows: no
    /// source, unread while the bar is off, a fresh cached answer, or a
    /// read started in a job. A location that is a point of the timeline
    /// the pane already holds keeps it, so moving along the bar asks
    /// nobody.
    pub(crate) fn sync_timeline(&mut self, pane: PaneId) {
        let Some(location) = self
            .model
            .active_tab(pane)
            .filter(|(_, tab)| tab.kind.lists())
            .map(|(_, tab)| tab.location.clone())
        else {
            return;
        };
        if matches!(self.model.timeline(pane), TimelineState::Points(t) if t.covers(&location)) {
            return;
        }
        self.cancel_timeline_read(pane);
        let sources = self.timeline_sources(&location);
        let state = if sources.is_empty() {
            TimelineState::None
        } else if let Some(cached) = self.cached_timeline(&location) {
            cached
        } else if self.model.timeline_shown(pane) {
            self.read_timeline(pane, location.clone(), sources);
            TimelineState::Reading
        } else {
            TimelineState::Unread
        };
        if self.model.set_timeline(pane, location, state) {
            self.pending.push(Event::Timeline { pane });
        }
    }

    /// The merged answer for `location` while it is fresh, as the pane's
    /// state.
    fn cached_timeline(&self, location: &Location) -> Option<TimelineState> {
        self.timelines
            .cache
            .get(location)
            .filter(|(at, _)| at.elapsed() < TIMELINE_FOR)
            .map(|(_, timeline)| {
                timeline
                    .clone()
                    .map_or(TimelineState::Empty, TimelineState::Points)
            })
    }

    /// Cancels the pane's running read, if any.
    fn cancel_timeline_read(&mut self, pane: PaneId) {
        if let Some((_, cancel)) = self.timelines.reading.remove(&pane) {
            cancel.cancel();
        }
    }

    /// Asks every source for the timeline of `location` in a job, off the
    /// UI thread, and sends the merged answer to the loop.
    fn read_timeline(
        &mut self,
        pane: PaneId,
        location: Location,
        sources: Vec<Arc<dyn Decorator>>,
    ) {
        self.timelines.generation += 1;
        let generation = self.timelines.generation;
        let tx = self.mailbox.clone();
        let handle = self.decor_jobs.spawn_named(
            format!("Timeline {location}"),
            move |job| async move {
                let asked = location.clone();
                let read = tokio::task::spawn_blocking(move || {
                    Timeline::merge(sources.iter().filter_map(|source| source.timeline(&asked)))
                });
                let timeline = tokio::select! {
                    () = job.cancel.cancelled() => return Err(Error::Cancelled),
                    read = read => read.map_err(|error| Error::Io(std::io::Error::other(error)))?,
                };
                tx.send(Msg::Timeline {
                    pane,
                    generation,
                    location,
                    timeline,
                });
                Ok(())
            },
        );
        self.timelines
            .reading
            .insert(pane, (generation, handle.cancel));
    }

    /// A read finished: keep the answer and show it where the pane still
    /// is.
    pub(crate) fn timeline_read(
        &mut self,
        pane: PaneId,
        generation: u64,
        location: Location,
        timeline: Option<Timeline>,
        events: &mut Vec<Event>,
    ) {
        let timeline = timeline.map(Arc::new);
        self.timelines
            .cache
            .insert(location.clone(), (Instant::now(), timeline.clone()));
        if self.timelines.reading.get(&pane).map(|(g, _)| *g) != Some(generation) {
            return;
        }
        self.timelines.reading.remove(&pane);
        let state = timeline.map_or(TimelineState::Empty, TimelineState::Points);
        if self.model.set_timeline(pane, location, state) {
            events.push(Event::Timeline { pane });
        }
    }

    /// Flips the pane's time slider and reads the timeline when it came on.
    pub(crate) fn toggle_timeline(&mut self, pane: PaneId, events: &mut Vec<Event>) {
        if self.model.toggle_timeline(pane).is_none() {
            return;
        }
        self.sync_timeline(pane);
        events.push(Event::Timeline { pane });
    }

    /// Takes the pane to a point of its timeline, or back to *Now*; one
    /// history entry per run (`first`).
    pub(crate) fn time_travel(&mut self, pane: PaneId, to: Location, first: bool) {
        self.timelines.travel.insert(pane, to.clone());
        self.model.spring_load(pane, to, first);
    }

    /// The listing of `location` failed: when the pane travelled there and
    /// the folder did not exist at that time, go to its parent instead and
    /// say so once. Returns whether it did.
    pub(crate) fn travel_missed(
        &mut self,
        pane: PaneId,
        location: &Location,
        error: &Error,
        events: &mut Vec<Event>,
    ) -> bool {
        if !matches!(error, Error::NotFound) {
            self.timelines.travel.remove(&pane);
            return false;
        }
        let Some(aimed) = self.timelines.travel.get(&pane) else {
            return false;
        };
        let Some(parent) = location.path.parent() else {
            return false;
        };
        if *aimed == *location {
            events.push(Event::Notify {
                level: Level::Info,
                text: ext::t!("timeline-missing", folder = location.path.to_string()),
            });
        }
        let parent = Location::new(location.provider, parent);
        self.timelines.travel.insert(pane, parent.clone());
        self.model.spring_load(pane, parent, false);
        true
    }

    /// A listing worked: a travel there is over.
    pub(crate) fn travel_arrived(&mut self, pane: PaneId) {
        self.timelines.travel.remove(&pane);
    }

    /// Forgets the answer for `location`, as after a refresh or a change
    /// the decorators watch.
    pub(crate) fn forget_timeline(&mut self, location: &Location) {
        self.timelines.cache.remove(location);
    }
}
