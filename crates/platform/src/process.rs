//! What the operating system knows about a process Rick started: whether
//! the shell in a console has handed its terminal to a job
//! (specs/system-integration.md §4), so the application types `cd` into
//! the console only at the prompt.
//!
//! Linux reads `/proc/<pid>/stat`: the shell's process group against the
//! terminal's foreground process group. macOS asks `proc_pidinfo` for the
//! same two numbers. Windows has no foreground process group on a pseudo
//! console, so it cannot tell.

/// Whether a program other than the shell with process id `shell` has the
/// terminal (`Some(true)`), the shell itself is in the foreground
/// (`Some(false)`), or the system cannot say (`None`: no such process, no
/// controlling terminal, Windows).
#[must_use]
pub fn foreground_job(shell: u32) -> Option<bool> {
    imp::foreground_job(shell)
}

#[cfg(target_os = "linux")]
mod imp {
    /// The process group and the terminal's foreground process group of a
    /// process, from `/proc/<pid>/stat`: the fields after the command
    /// name, which is in parentheses and may hold anything, so the split
    /// is at the last closing one.
    pub(super) fn foreground_job(shell: u32) -> Option<bool> {
        let stat = std::fs::read_to_string(format!("/proc/{shell}/stat")).ok()?;
        let rest = &stat[stat.rfind(')')? + 1..];
        let mut fields = rest.split_whitespace();
        // state, ppid, pgrp, session, tty_nr, tpgid
        let _state = fields.next()?;
        let _ppid = fields.next()?;
        let pgrp: i64 = fields.next()?.parse().ok()?;
        let _session = fields.next()?;
        let _tty = fields.next()?;
        let tpgid: i64 = fields.next()?.parse().ok()?;
        if tpgid <= 0 {
            // No controlling terminal.
            return None;
        }
        Some(tpgid != pgrp)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    /// `proc_pidinfo` with `PROC_PIDTBSDINFO`: the process group and the
    /// terminal's foreground process group, and the terminal's device,
    /// which is `NODEV` without a controlling terminal.
    pub(super) fn foreground_job(shell: u32) -> Option<bool> {
        let pid = i32::try_from(shell).ok()?;
        // SAFETY: `proc_bsdinfo` is plain data the call fills in full or
        // not at all; the buffer size is the struct's own, and a short
        // answer is refused below.
        let info = unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = i32::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
            let got = libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                std::ptr::from_mut(&mut info).cast::<std::ffi::c_void>(),
                size,
            );
            if got != size {
                return None;
            }
            info
        };
        if info.e_tdev == u32::MAX {
            // No controlling terminal.
            return None;
        }
        Some(info.e_tpgid != info.pbi_pgid)
    }
}

#[cfg(windows)]
mod imp {
    /// A pseudo console has no foreground process group to ask about.
    pub(super) fn foreground_job(_shell: u32) -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that is not there has no answer; this one, when it has
    /// a terminal, is either at the prompt or not, never a panic.
    #[test]
    fn a_missing_process_has_no_answer_and_this_one_does_not_panic() {
        assert_eq!(foreground_job(u32::MAX - 1), None);
        let _ = foreground_job(std::process::id());
    }
}
