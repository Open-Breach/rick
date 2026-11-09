//! The system's reduce-motion setting (SPEC.md *Loading animations*): with
//! it on, Rick's animations stand still. Read once per process from the
//! desktop's own setting: GNOME's `enable-animations` on Linux, the
//! Accessibility pane's *Reduce motion* on macOS, *Show animations in
//! Windows* on Windows. A desktop without the setting reads as motion on.

use std::sync::OnceLock;

/// Whether the system asks for reduced motion, read once.
#[must_use]
pub fn system() -> bool {
    static REDUCE: OnceLock<bool> = OnceLock::new();
    *REDUCE.get_or_init(probe)
}

#[cfg(target_os = "linux")]
fn probe() -> bool {
    // GNOME, and the desktops that read its settings: animations off means
    // reduce motion.
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
}

#[cfg(target_os = "macos")]
fn probe() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

#[cfg(windows)]
fn probe() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut animate: windows::Win32::Foundation::BOOL = windows::Win32::Foundation::TRUE;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL into the buffer it
    // is given, which outlives the call.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut animate).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_ok() && !animate.as_bool()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn probe() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_answers_and_the_answer_is_cached() {
        let first = system();
        assert_eq!(system(), first);
    }
}
