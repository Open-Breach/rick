//! The monitors of an X11 session, from `RandR`, with the working area the
//! window manager publishes (`_NET_WORKAREA`). A Wayland session reports
//! none: there an application is told no position and can set none, so
//! where a window goes is the compositor's decision (SPEC.md *Window
//! geometry*).

use tracing::debug;
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use crate::traits::Monitor;

/// The monitors, in logical pixels for a window of `pixels_per_point`:
/// X11 counts in physical ones.
pub(super) fn list(pixels_per_point: f32) -> Vec<Monitor> {
    // Only where the backend opens X11 windows: a Wayland session has
    // nothing to place a window by.
    if !super::x11_backend(|name| std::env::var(name).ok()) {
        return Vec::new();
    }
    match read() {
        Ok(monitors) => monitors
            .into_iter()
            .map(|monitor| scaled(monitor, pixels_per_point))
            .collect(),
        Err(error) => {
            debug!(%error, "the monitors are not known");
            Vec::new()
        }
    }
}

/// A monitor's rectangles divided by the window's scale.
fn scaled(monitor: Monitor, pixels_per_point: f32) -> Monitor {
    let scale = if pixels_per_point > 0.0 {
        pixels_per_point
    } else {
        1.0
    };
    Monitor {
        bounds: monitor.bounds.map(|v| v / scale),
        work: monitor.work.map(|v| v / scale),
        ..monitor
    }
}

/// The monitors as the X server lists them, in its pixels.
fn read() -> Result<Vec<Monitor>, Box<dyn std::error::Error>> {
    let (conn, screen) = RustConnection::connect(None)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or("the display has no such screen")?
        .root;
    let desktop = work_area(&conn, root);
    let reply = conn.randr_get_monitors(root, true)?.reply()?;
    let mut monitors = Vec::with_capacity(reply.monitors.len());
    for monitor in reply.monitors {
        let name = conn.get_atom_name(monitor.name)?.reply()?.name;
        let bounds = [
            f32::from(monitor.x),
            f32::from(monitor.y),
            f32::from(monitor.width),
            f32::from(monitor.height),
        ];
        monitors.push(Monitor {
            id: String::from_utf8_lossy(&name).into_owned(),
            primary: monitor.primary,
            bounds,
            work: desktop.map_or(bounds, |desktop| intersection(bounds, desktop)),
        });
    }
    Ok(monitors)
}

/// The working area of the current desktop: `_NET_WORKAREA`, one rectangle
/// for the whole desktop with the panels taken off. `None` where the
/// window manager publishes none.
fn work_area(conn: &RustConnection, root: Window) -> Option<[f32; 4]> {
    let cardinals = |name: &str| -> Option<Vec<u32>> {
        let atom = conn
            .intern_atom(true, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        let reply = conn
            .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 4096)
            .ok()?
            .reply()
            .ok()?;
        Some(reply.value32()?.collect())
    };
    let desktop = cardinals("_NET_CURRENT_DESKTOP")
        .and_then(|values| values.first().copied())
        .unwrap_or(0) as usize;
    let areas = cardinals("_NET_WORKAREA")?;
    let area = areas.get(desktop * 4..desktop * 4 + 4)?;
    #[allow(clippy::cast_precision_loss)]
    let area = [
        area[0] as f32,
        area[1] as f32,
        area[2] as f32,
        area[3] as f32,
    ];
    (area[2] > 0.0 && area[3] > 0.0).then_some(area)
}

/// What a monitor and the desktop's working area share; the monitor
/// itself where they share nothing.
fn intersection(monitor: [f32; 4], desktop: [f32; 4]) -> [f32; 4] {
    let left = monitor[0].max(desktop[0]);
    let top = monitor[1].max(desktop[1]);
    let right = (monitor[0] + monitor[2]).min(desktop[0] + desktop[2]);
    let bottom = (monitor[1] + monitor[3]).min(desktop[1] + desktop[3]);
    if right > left && bottom > top {
        [left, top, right - left, bottom - top]
    } else {
        monitor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two rectangles are the same to a hundredth of a pixel.
    fn same(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
    }

    #[test]
    fn a_monitors_working_area_is_its_share_of_the_desktops() {
        // Two monitors side by side, a panel of 32 px across the top.
        let desktop = [0.0, 32.0, 4480.0, 1408.0];
        assert!(same(
            intersection([0.0, 0.0, 1920.0, 1080.0], desktop),
            [0.0, 32.0, 1920.0, 1048.0]
        ));
        assert!(same(
            intersection([1920.0, 0.0, 2560.0, 1440.0], desktop),
            [1920.0, 32.0, 2560.0, 1408.0]
        ));
        // Nothing shared: the monitor as it is.
        assert!(same(
            intersection([5000.0, 0.0, 800.0, 600.0], desktop),
            [5000.0, 0.0, 800.0, 600.0]
        ));
    }

    #[test]
    fn the_rectangles_are_divided_by_the_windows_scale() {
        let monitor = Monitor {
            id: "DP-1".into(),
            primary: true,
            bounds: [0.0, 0.0, 3840.0, 2160.0],
            work: [0.0, 64.0, 3840.0, 2096.0],
        };
        let scaled = scaled(monitor, 2.0);
        assert!(same(scaled.bounds, [0.0, 0.0, 1920.0, 1080.0]));
        assert!(same(scaled.work, [0.0, 32.0, 1920.0, 1048.0]));
    }

    /// Against a real X server, where `DISPLAY` names one (CI's Xvfb, a
    /// desktop): it lists a monitor with a size, and one of them is where
    /// the origin is.
    #[test]
    fn an_x_server_lists_its_monitors() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipped: no X server (DISPLAY is not set)");
            return;
        }
        let monitors = read().unwrap_or_else(|e| panic!("{e}"));
        assert!(!monitors.is_empty(), "an X server has a monitor");
        for monitor in &monitors {
            assert!(!monitor.id.is_empty(), "{monitor:?}");
            assert!(
                monitor.bounds[2] > 0.0 && monitor.bounds[3] > 0.0,
                "{monitor:?}"
            );
            assert!(
                monitor.work[2] > 0.0 && monitor.work[3] > 0.0,
                "{monitor:?}"
            );
        }
        assert!(
            monitors
                .iter()
                .any(|m| m.bounds[0].abs() < 0.01 && m.bounds[1].abs() < 0.01)
        );
    }
}
