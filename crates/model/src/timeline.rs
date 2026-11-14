//! The time slider's state per pane: whether the user switched it on, and
//! what is known about the timeline of the folder the pane shows. The
//! application reads the timeline in a job and records each step here; the
//! UI draws the bar from it.

use std::sync::Arc;

use ext::{PaneId, Timeline};
use types::Location;

use crate::Model;

/// What is known about the timeline of one location.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TimelineState {
    /// No decorator that offers timelines applies to the location: no
    /// bar, no menu entry.
    #[default]
    None,
    /// A source applies, but the timeline was not read: the bar is off.
    Unread,
    /// The read is running.
    Reading,
    /// The read came back without a point.
    Empty,
    /// The merged timeline.
    Points(Arc<Timeline>),
}

/// A pane's timeline: the location it belongs to and its state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaneTimeline {
    /// The location the state was recorded for; `None` before the first.
    pub location: Option<Location>,
    /// The state.
    pub state: TimelineState,
}

impl Model {
    /// Whether the pane's time slider is switched on.
    #[must_use]
    pub fn timeline_shown(&self, pane: PaneId) -> bool {
        self.panes.get(&pane).is_some_and(|p| p.timeline_shown)
    }

    /// Switches the pane's time slider on or off. Returns the new value,
    /// `None` for an unknown pane.
    pub fn toggle_timeline(&mut self, pane: PaneId) -> Option<bool> {
        let pane = self.panes.get_mut(&pane)?;
        pane.timeline_shown = !pane.timeline_shown;
        Some(pane.timeline_shown)
    }

    /// Sets the pane's time slider on or off, as a restored session says.
    pub fn set_timeline_shown(&mut self, pane: PaneId, shown: bool) {
        if let Some(pane) = self.panes.get_mut(&pane) {
            pane.timeline_shown = shown;
        }
    }

    /// Records what is known about the timeline of `location` for the pane.
    /// Returns whether anything changed.
    pub fn set_timeline(&mut self, pane: PaneId, location: Location, state: TimelineState) -> bool {
        let Some(pane) = self.panes.get_mut(&pane) else {
            return false;
        };
        let next = PaneTimeline {
            location: Some(location),
            state,
        };
        if pane.timeline == next {
            return false;
        }
        pane.timeline = next;
        true
    }

    /// The timeline state of the folder the pane shows: what was recorded
    /// for its location, or for a timeline the location is a point of.
    /// [`TimelineState::None`] when nothing was recorded for it.
    #[must_use]
    pub fn timeline(&self, pane: PaneId) -> &TimelineState {
        static NONE: TimelineState = TimelineState::None;
        let Some(p) = self.panes.get(&pane) else {
            return &NONE;
        };
        let Some((_, tab)) = self.active_tab(pane) else {
            return &NONE;
        };
        let here = &tab.location;
        let current = p.timeline.location.as_ref() == Some(here)
            || matches!(&p.timeline.state, TimelineState::Points(t) if t.covers(here));
        if current { &p.timeline.state } else { &NONE }
    }

    /// The pane's merged timeline and the index of the point it shows,
    /// `None` at the live folder (*Now*).
    #[must_use]
    pub fn timeline_points(&self, pane: PaneId) -> Option<(&Arc<Timeline>, Option<usize>)> {
        let TimelineState::Points(timeline) = self.timeline(pane) else {
            return None;
        };
        let (_, tab) = self.active_tab(pane)?;
        Some((timeline, timeline.point_at(&tab.location)))
    }

    /// Whether the pane shows a past point of its timeline.
    #[must_use]
    pub fn in_the_past(&self, pane: PaneId) -> bool {
        matches!(self.timeline_points(pane), Some((_, Some(_))))
    }

    /// Whether the pane draws its time slider: switched on with a timeline
    /// that has points or is being read, or at a past point whether
    /// switched on or not. Never without points otherwise.
    #[must_use]
    pub fn timeline_visible(&self, pane: PaneId) -> bool {
        match self.timeline(pane) {
            TimelineState::Points(_) => self.timeline_shown(pane) || self.in_the_past(pane),
            TimelineState::Reading => self.timeline_shown(pane),
            TimelineState::None | TimelineState::Unread | TimelineState::Empty => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use ext::{TimePoint, Timeline};
    use types::{Location, ProviderId};

    use super::TimelineState;
    use crate::{Model, Settings};

    fn loc(path: &str) -> Location {
        Location::new(ProviderId("t"), path)
    }

    fn timeline() -> Arc<Timeline> {
        Arc::new(Timeline {
            live: loc("/docs"),
            points: ["a", "b"]
                .iter()
                .enumerate()
                .map(|(i, name)| TimePoint {
                    time: SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64 * 60),
                    label: (*name).into(),
                    source: "ZFS".into(),
                    location: loc(&format!("/@{name}/docs")),
                })
                .collect(),
        })
    }

    #[test]
    fn the_toggle_flips_per_pane() {
        let mut model = Model::new(Settings::default(), loc("/docs"));
        let pane = model.focused;
        assert!(!model.timeline_shown(pane));
        assert_eq!(model.toggle_timeline(pane), Some(true));
        assert!(model.timeline_shown(pane));
        assert_eq!(model.toggle_timeline(pane), Some(false));
    }

    #[test]
    fn the_bar_shows_only_with_points_or_while_reading() {
        let mut model = Model::new(Settings::default(), loc("/docs"));
        let pane = model.focused;
        assert_eq!(*model.timeline(pane), TimelineState::None);
        model.set_timeline(pane, loc("/docs"), TimelineState::Reading);
        assert!(!model.timeline_visible(pane), "switched off");
        model.toggle_timeline(pane);
        assert!(model.timeline_visible(pane), "reading");
        model.set_timeline(pane, loc("/docs"), TimelineState::Empty);
        assert!(!model.timeline_visible(pane), "never an empty bar");
        model.set_timeline(pane, loc("/docs"), TimelineState::Points(timeline()));
        assert!(model.timeline_visible(pane));
        assert_eq!(model.timeline_points(pane).map(|(_, at)| at), Some(None));
    }

    #[test]
    fn a_past_point_keeps_the_timeline_and_shows_the_bar() {
        let mut model = Model::new(Settings::default(), loc("/docs"));
        let pane = model.focused;
        model.set_timeline(pane, loc("/docs"), TimelineState::Points(timeline()));
        model.spring_load(pane, loc("/@b/docs"), true);
        assert!(model.in_the_past(pane));
        assert_eq!(model.timeline_points(pane).map(|(_, at)| at), Some(Some(1)));
        assert!(model.timeline_visible(pane), "shown although switched off");

        model.spring_load(pane, loc("/elsewhere"), false);
        assert_eq!(*model.timeline(pane), TimelineState::None);
        assert!(!model.timeline_visible(pane));
    }
}

#[cfg(test)]
mod session_tests {
    use types::{Location, ProviderId};

    use crate::{Model, PaneSession, Settings};

    #[test]
    fn the_toggle_comes_back_with_the_session() {
        let home = Location::new(ProviderId("local"), "/home");
        let mut model = Model::new(Settings::default(), home.clone());
        let pane = model.focused;
        model.toggle_timeline(pane);
        let session = model.session();
        let text = toml::to_string(&session).unwrap_or_else(|e| panic!("{e}"));
        assert!(text.contains("timeline = true"), "{text}");

        let restored = Model::from_session(Settings::default(), &session, &home);
        assert!(restored.timeline_shown(restored.focused));
    }

    #[test]
    fn an_older_file_has_the_toggle_off() {
        let pane: PaneSession =
            toml::from_str("active = 0\ntabs = []\n").unwrap_or_else(|e| panic!("{e}"));
        assert!(!pane.timeline);
        let text = toml::to_string(&PaneSession::default()).unwrap_or_else(|e| panic!("{e}"));
        assert!(!text.contains("timeline"), "off is not written: {text}");
    }
}
