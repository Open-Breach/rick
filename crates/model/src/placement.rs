//! Where a window goes on the screen (SPEC.md *Window geometry*): the
//! default size, what of a saved geometry is restored, and where a second
//! window opens. Pure rules over the monitors the platform reports, in the
//! logical pixels a window is placed in.
//!
//! ```
//! use model::placement::{Area, Screen, default_size, restore};
//!
//! // An ordinary screen with a panel of 40 px at its top.
//! let screen = Screen {
//!     id: "DP-1".into(),
//!     primary: true,
//!     bounds: Area::new(0.0, 0.0, 1920.0, 1080.0),
//!     work: Area::new(0.0, 40.0, 1920.0, 1040.0),
//! };
//! // 80 % of the working area decides.
//! assert_eq!(default_size(&screen.work), (1479.0, 832.0));
//! // A first launch: centred in the working area, on the primary monitor.
//! let placed = restore(None, None, &[screen], 40.0).unwrap();
//! assert_eq!((placed.geometry.x, placed.geometry.y), (220.5, 144.0));
//! assert_eq!(placed.monitor.as_deref(), Some("DP-1"));
//! ```

use crate::session::Geometry;

/// The tallest a window opens by default, in logical pixels.
const DEFAULT_MAX_HEIGHT: f32 = 1200.0;
/// The widest a window opens by default, in logical pixels.
const DEFAULT_MAX_WIDTH: f32 = 1920.0;
/// The share of the working area a window takes by default.
const DEFAULT_SHARE: f32 = 0.8;
/// The widest a window opens by default for its height: 16:9.
const DEFAULT_RATIO: f32 = 16.0 / 9.0;

/// A rectangle on the desktop, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
}

impl Area {
    /// The rectangle with its top left corner at `x`, `y`.
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The right edge.
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The bottom edge.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether the point lies inside.
    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// A monitor: where it is on the desktop and what of it is free for
/// windows.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// What tells the monitor from the others across restarts: the name
    /// the system gives its output.
    pub id: String,
    /// Whether the system calls it the primary one.
    pub primary: bool,
    /// The whole monitor.
    pub bounds: Area,
    /// The working area: the monitor less the system's panels, dock and
    /// task bar.
    pub work: Area,
}

/// A geometry with the monitor it is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// Where the window goes.
    pub geometry: Geometry,
    /// The monitor's id.
    pub monitor: Option<String>,
}

/// The default size of a window on a monitor with the working area `work`:
/// the height is the smaller of 80 % of the working area's height and
/// 1200 px, the width the smallest of 80 % of its width, 1920 px and the
/// height × 16/9. On an ordinary screen the 80 % decides, on an ultrawide
/// one the ratio, on a 4K one the caps. Whole pixels.
#[must_use]
pub fn default_size(work: &Area) -> (f32, f32) {
    let height = (work.height * DEFAULT_SHARE)
        .min(DEFAULT_MAX_HEIGHT)
        .floor();
    let width = (work.width * DEFAULT_SHARE)
        .min(DEFAULT_MAX_WIDTH)
        .min(height * DEFAULT_RATIO)
        .floor();
    (width, height)
}

/// The primary monitor: the one the system names, else the first.
fn primary(screens: &[Screen]) -> Option<&Screen> {
    screens
        .iter()
        .find(|s| s.primary)
        .or_else(|| screens.first())
}

/// The middle of a window's title row: what has to be on a monitor's
/// working area for the window to count as visible there.
fn title_middle(geometry: &Geometry, title_height: f32) -> (f32, f32) {
    (
        geometry.x + geometry.width / 2.0,
        geometry.y + title_height / 2.0,
    )
}

/// The monitor a window with this geometry is on: the one whose bounds
/// hold the middle of its title row.
#[must_use]
pub fn screen_of<'a>(
    geometry: &Geometry,
    screens: &'a [Screen],
    title_height: f32,
) -> Option<&'a Screen> {
    let (x, y) = title_middle(geometry, title_height);
    screens.iter().find(|s| s.bounds.contains(x, y))
}

/// A window of the default size, centred in the working area of the
/// primary monitor.
fn centred(screens: &[Screen]) -> Option<Placed> {
    let screen = primary(screens)?;
    let (width, height) = default_size(&screen.work);
    Some(Placed {
        geometry: Geometry {
            x: screen.work.x + (screen.work.width - width) / 2.0,
            y: screen.work.y + (screen.work.height - height) / 2.0,
            width,
            height,
            maximised: false,
        },
        monitor: Some(screen.id.clone()),
    })
}

/// A size that no longer fits the working area, cut down: each side to the
/// default size's limit for that monitor.
fn fitted(geometry: Geometry, work: &Area) -> Geometry {
    if geometry.width <= work.width && geometry.height <= work.height {
        return geometry;
    }
    let (width, height) = default_size(work);
    Geometry {
        width: geometry.width.min(width),
        height: geometry.height.min(height),
        ..geometry
    }
}

/// `geometry` moved, where it must be, to lie inside `work`.
fn inside(geometry: Geometry, work: &Area) -> Geometry {
    Geometry {
        x: geometry.x.min(work.right() - geometry.width).max(work.x),
        y: geometry.y.min(work.bottom() - geometry.height).max(work.y),
        ..geometry
    }
}

/// Where a window opens at launch, from what the session held for it.
///
/// It comes back exactly — monitor, position, size and maximised state —
/// when its monitor is still connected and the window would be visible on
/// it: the middle of its title row lies inside that monitor's working
/// area. A size that no longer fits that working area is cut down to the
/// default size's limits, and the window moved to lie inside. Otherwise —
/// nothing saved, the monitor gone, the position off-screen — it opens
/// centred in the working area of the primary monitor at the default size.
///
/// `None` where the platform reports no monitors, which is where it gives
/// a window no position either: the saved size and state are the caller's
/// to restore, and the placing is the system's.
#[must_use]
pub fn restore(
    saved: Option<&Geometry>,
    monitor: Option<&str>,
    screens: &[Screen],
    title_height: f32,
) -> Option<Placed> {
    if screens.is_empty() {
        return None;
    }
    let Some(saved) = saved else {
        return centred(screens);
    };
    // The monitor the session names, or for a session that names none the
    // one the saved position lies on.
    let screen = match monitor {
        Some(id) => screens.iter().find(|s| s.id == id),
        None => screen_of(saved, screens, title_height),
    };
    let (x, y) = title_middle(saved, title_height);
    match screen {
        Some(screen) if screen.work.contains(x, y) => {
            let fitted = fitted(*saved, &screen.work);
            let geometry = if fitted == *saved {
                fitted
            } else {
                inside(fitted, &screen.work)
            };
            Some(Placed {
                geometry,
                monitor: Some(screen.id.clone()),
            })
        }
        _ => centred(screens),
    }
}

/// Where a second window opens, from the geometry of the window it was
/// opened from: at that window's size, a title row's height down and to
/// the right of it, so that both can be grabbed; where that would take it
/// past the edge of the working area it starts again at the working area's
/// top left corner. Without monitors the offset alone.
#[must_use]
pub fn beside(opener: &Geometry, screens: &[Screen], title_height: f32) -> Placed {
    let stepped = Geometry {
        x: opener.x + title_height,
        y: opener.y + title_height,
        maximised: false,
        ..*opener
    };
    let Some(screen) = screen_of(opener, screens, title_height).or_else(|| primary(screens)) else {
        return Placed {
            geometry: stepped,
            monitor: None,
        };
    };
    let work = &screen.work;
    let past =
        stepped.x + stepped.width > work.right() || stepped.y + stepped.height > work.bottom();
    let geometry = if past {
        Geometry {
            x: work.x,
            y: work.y,
            ..stepped
        }
    } else {
        stepped
    };
    Placed {
        geometry,
        monitor: Some(screen.id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: f32 = 40.0;

    fn screen(id: &str, primary: bool, bounds: Area, panel: f32) -> Screen {
        Screen {
            id: id.to_owned(),
            primary,
            bounds,
            work: Area::new(
                bounds.x,
                bounds.y + panel,
                bounds.width,
                bounds.height - panel,
            ),
        }
    }

    fn laptop() -> Screen {
        screen("eDP-1", true, Area::new(0.0, 0.0, 1920.0, 1080.0), 32.0)
    }

    fn second() -> Screen {
        screen("DP-2", false, Area::new(1920.0, 0.0, 2560.0, 1440.0), 0.0)
    }

    fn window(x: f32, y: f32, width: f32, height: f32) -> Geometry {
        Geometry {
            x,
            y,
            width,
            height,
            maximised: false,
        }
    }

    /// Issue 149: the default size by the three limits.
    #[test]
    fn the_default_size_is_eighty_per_cent_capped_and_no_wider_than_sixteen_to_nine() {
        // An ordinary screen: the 80 % decides.
        assert_eq!(
            default_size(&Area::new(0.0, 0.0, 1920.0, 1080.0)),
            (1536.0, 864.0)
        );
        // An ultrawide one: the ratio keeps the window from spanning it.
        assert_eq!(
            default_size(&Area::new(0.0, 0.0, 3440.0, 1440.0)),
            (1920.0, 1152.0)
        );
        assert_eq!(
            default_size(&Area::new(0.0, 0.0, 5120.0, 1080.0)),
            (1536.0, 864.0)
        );
        // A 4K screen: the pixel caps.
        assert_eq!(
            default_size(&Area::new(0.0, 0.0, 3840.0, 2160.0)),
            (1920.0, 1200.0)
        );
        // Never more than 1920 × 1200, never wider than 16:9 of the height.
        for (w, h) in [
            (800.0, 600.0),
            (1366.0, 768.0),
            (7680.0, 4320.0),
            (1080.0, 1920.0),
        ] {
            let (width, height) = default_size(&Area::new(0.0, 0.0, w, h));
            assert!(width <= 1920.0 && height <= 1200.0, "{w}×{h}");
            assert!(width <= height * 16.0 / 9.0 + 1.0, "{w}×{h}");
            assert!(width <= w * 0.8 + 1.0 && height <= h * 0.8 + 1.0, "{w}×{h}");
        }
    }

    #[test]
    fn a_first_launch_is_centred_on_the_primary_monitor_at_the_default_size() {
        let placed =
            restore(None, None, &[second(), laptop()], TITLE).unwrap_or_else(|| panic!("placed"));
        let work = laptop().work;
        let (width, height) = default_size(&work);
        assert_eq!(placed.monitor.as_deref(), Some("eDP-1"));
        assert_eq!(
            (placed.geometry.width, placed.geometry.height),
            (width, height)
        );
        assert!((placed.geometry.x - (work.x + (work.width - width) / 2.0)).abs() < 0.01);
        assert!((placed.geometry.y - (work.y + (work.height - height) / 2.0)).abs() < 0.01);
        assert!(!placed.geometry.maximised);
    }

    /// Acceptance (issue 149): quit with the window on a second monitor,
    /// unplug it, start: centred on the primary monitor, at most
    /// 1920 × 1200 and no wider than 16:9 of its height.
    #[test]
    fn a_window_whose_monitor_is_gone_opens_centred_on_the_primary_one() {
        let saved = Geometry {
            maximised: true,
            ..window(2200.0, 100.0, 2000.0, 1200.0)
        };
        // With the monitor there it comes back exactly, maximised and all.
        let both = [laptop(), second()];
        let back =
            restore(Some(&saved), Some("DP-2"), &both, TITLE).unwrap_or_else(|| panic!("placed"));
        assert_eq!(back.geometry, saved);
        assert_eq!(back.monitor.as_deref(), Some("DP-2"));
        // Unplugged.
        let placed = restore(Some(&saved), Some("DP-2"), &[laptop()], TITLE)
            .unwrap_or_else(|| panic!("placed"));
        let work = laptop().work;
        assert_eq!(placed.monitor.as_deref(), Some("eDP-1"));
        let g = placed.geometry;
        assert!(g.width <= 1920.0 && g.height <= 1200.0);
        assert!(g.width <= g.height * 16.0 / 9.0 + 1.0);
        assert!(((g.x + g.width / 2.0) - (work.x + work.width / 2.0)).abs() < 0.01);
        assert!(((g.y + g.height / 2.0) - (work.y + work.height / 2.0)).abs() < 0.01);
        assert!(!g.maximised);
    }

    #[test]
    fn a_window_is_restored_exactly_while_its_title_row_is_on_its_monitor() {
        let screens = [laptop(), second()];
        // Half off the right edge of its monitor, the title row's middle
        // still on it: exactly as it was.
        let saved = window(1100.0, 300.0, 1200.0, 700.0);
        let placed = restore(Some(&saved), Some("eDP-1"), &screens, TITLE)
            .unwrap_or_else(|| panic!("placed"));
        assert_eq!(placed.geometry, saved);
        // The title row under the panel, or off the monitor's side: not
        // visible, so centred at the default size.
        for off in [
            window(100.0, 0.0, 800.0, 600.0),
            window(-900.0, 300.0, 800.0, 600.0),
            window(400.0, 1070.0, 800.0, 600.0),
        ] {
            let placed = restore(Some(&off), Some("eDP-1"), &screens, TITLE)
                .unwrap_or_else(|| panic!("placed"));
            assert_ne!(placed.geometry, off, "{off:?}");
            assert!((placed.geometry.width - default_size(&laptop().work).0).abs() < 0.01);
        }
        // A session that names no monitor: the one the position lies on.
        let placed =
            restore(Some(&saved), None, &screens, TITLE).unwrap_or_else(|| panic!("placed"));
        assert_eq!(placed.geometry, saved);
        assert_eq!(placed.monitor.as_deref(), Some("eDP-1"));
        // The named monitor decides: a position that lies on another one
        // is not on it.
        let elsewhere = window(2200.0, 200.0, 800.0, 600.0);
        let placed = restore(Some(&elsewhere), Some("eDP-1"), &screens, TITLE)
            .unwrap_or_else(|| panic!("placed"));
        assert_ne!(placed.geometry, elsewhere);
    }

    #[test]
    fn a_size_that_no_longer_fits_is_cut_to_the_default_limits_and_moved_inside() {
        // Saved on a large monitor that now has a smaller mode.
        let saved = window(100.0, 100.0, 2400.0, 1300.0);
        let placed = restore(Some(&saved), Some("eDP-1"), &[laptop()], TITLE)
            .unwrap_or_else(|| panic!("placed"));
        let work = laptop().work;
        let (width, height) = default_size(&work);
        assert_eq!(
            (placed.geometry.width, placed.geometry.height),
            (width, height)
        );
        let g = placed.geometry;
        assert!(g.x >= work.x && g.x + g.width <= work.right());
        assert!(g.y >= work.y && g.y + g.height <= work.bottom());
        // Only the side that does not fit is cut.
        let tall = window(100.0, 100.0, 900.0, 1300.0);
        let placed = restore(Some(&tall), Some("eDP-1"), &[laptop()], TITLE)
            .unwrap_or_else(|| panic!("placed"));
        assert_eq!(
            (placed.geometry.width, placed.geometry.height),
            (900.0, height)
        );
    }

    #[test]
    fn a_second_window_is_offset_by_a_title_row_and_starts_again_at_the_corner() {
        let screens = [laptop()];
        let first = window(200.0, 150.0, 1000.0, 600.0);
        let placed = beside(&first, &screens, TITLE);
        assert_eq!(placed.geometry, window(240.0, 190.0, 1000.0, 600.0));
        assert_eq!(placed.monitor.as_deref(), Some("eDP-1"));
        // Never maximised, at the opener's restored size.
        let maximised = Geometry {
            maximised: true,
            ..first
        };
        assert!(!beside(&maximised, &screens, TITLE).geometry.maximised);
        // Past the working area's edge: the working area's top left corner.
        let low = window(700.0, 460.0, 1000.0, 600.0);
        let placed = beside(&low, &screens, TITLE);
        let work = laptop().work;
        assert_eq!((placed.geometry.x, placed.geometry.y), (work.x, work.y));
        assert_eq!(
            (placed.geometry.width, placed.geometry.height),
            (1000.0, 600.0)
        );
        // Without monitors, the offset alone.
        let blind = beside(&first, &[], TITLE);
        assert_eq!(blind.geometry, window(240.0, 190.0, 1000.0, 600.0));
        assert_eq!(blind.monitor, None);
    }

    #[test]
    fn without_monitors_the_placing_is_the_systems() {
        let saved = window(10.0, 10.0, 800.0, 600.0);
        assert_eq!(restore(Some(&saved), Some("DP-1"), &[], TITLE), None);
        assert_eq!(restore(None, None, &[], TITLE), None);
    }
}
