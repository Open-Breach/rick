//! Issue 176, the real thing: the copy, cut and paste chords through a real
//! window. The binary runs on an X display, `xdotool` sends the keys as the
//! X server's own key events, and the files on disk say what happened.
//!
//! This is the path the other tests cannot take. A window does not deliver
//! Mod+C, Mod+X and Mod+V as key presses, so a test that injects key
//! presses passes on chords no keyboard can produce — which is how the
//! chords came to do nothing in a running Rick while the shortcut table's
//! test was green.
//!
//! The test runs where there is an X display, `xdotool` and a built `rick`
//! (tier 2 runs it under Xvfb on Linux x64 and sets
//! `RICK_REQUIRE_REAL_CHORDS`); elsewhere it skips with a note, and
//! `clipboard_chords.rs` covers the events a window delivers.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A pause between two chords: a frame or two of a software renderer.
const BEAT: Duration = Duration::from_millis(400);

/// The running Rick, killed when the test ends either way.
struct Rick(Child);

impl Drop for Rick {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn runs(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The built binary: the bundle's, with its libraries beside it, or the
/// plain one, which finds them through the test's own library path.
fn binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RICK_BIN") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    let profile = exe.parent()?.parent()?;
    [profile.join("bundle/rick"), profile.join("rick")]
        .into_iter()
        .find(|path| path.is_file())
}

/// Why the test cannot run here, if it cannot.
fn missing() -> Option<String> {
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
        return Some("no X display".to_owned());
    }
    if !runs("xdotool", "version") {
        return Some("xdotool is not installed".to_owned());
    }
    if binary().is_none() {
        return Some("rick is not built".to_owned());
    }
    None
}

fn xdotool(args: &[&str]) -> String {
    let output = Command::new("xdotool")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("xdotool {args:?}: {e}"));
    assert!(
        output.status.success(),
        "xdotool {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sends the keys of `sequence` (`key ctrl+c`, `type one`, `keydown ctrl`)
/// and waits a beat.
fn send(sequence: &[&str]) {
    xdotool(sequence);
    std::thread::sleep(BEAT);
}

/// Waits for `path` to exist, or not to.
fn wait_for(path: &Path, there: bool, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while path.exists() != there {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// The window of the process `pid`, once it is on the screen.
fn window_of(pid: u32, rick: &mut Rick) -> String {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let output = Command::new("xdotool")
            .args(["search", "--onlyvisible", "--pid", &pid.to_string()])
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|e| panic!("xdotool search: {e}"));
        let found = String::from_utf8_lossy(&output.stdout);
        if let Some(id) = found.lines().rev().find(|line| !line.trim().is_empty()) {
            return id.trim().to_owned();
        }
        if let Ok(Some(status)) = rick.0.try_wait() {
            panic!("rick ended before it showed a window: {status}");
        }
        assert!(Instant::now() < deadline, "no window of rick appeared");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_real_chords_copy_cut_and_paste_in_a_real_window() {
    if let Some(why) = missing() {
        assert!(
            std::env::var_os("RICK_REQUIRE_REAL_CHORDS").is_none(),
            "RICK_REQUIRE_REAL_CHORDS is set and the test cannot run: {why}"
        );
        eprintln!("skipped: {why}");
        return;
    }
    let binary = binary().unwrap_or_else(|| panic!("rick is built"));

    // top/deep/one.txt, with Rick started in deep; the configuration and
    // the single-instance socket are the test's own.
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let root = dir.path().join("root");
    let top = root.join("top");
    let deep = top.join("deep");
    let config = dir.path().join("config");
    let runtime = dir.path().join("run");
    for folder in [&deep, &config, &runtime] {
        std::fs::create_dir_all(folder).unwrap_or_else(|e| panic!("{e}"));
    }
    std::fs::write(deep.join("one.txt"), "one").unwrap_or_else(|e| panic!("{e}"));
    // No set-up dialogue at the first start: it would take the keys.
    std::fs::write(
        config.join("rick.toml"),
        "[integration]\nask = false\ncheck_at_startup = false\nshell_integration = false\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let child = Command::new(&binary)
        .arg("--config-dir")
        .arg(&config)
        .arg(&deep)
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdin(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("{}: {e}", binary.display()));
    let pid = child.id();
    let mut rick = Rick(child);
    let window = window_of(pid, &mut rick);
    // The keyboard is the window's, whether or not a window manager runs.
    xdotool(&["windowfocus", &window]);
    xdotool(&["mousemove", "--window", &window, "700", "500"]);
    // The first frames, and the listing of the folder.
    std::thread::sleep(Duration::from_secs(3));

    // Copy: select all in deep, Mod+C, up to top, Mod+V. Nothing but the
    // file is on the clipboard — no text — so the paste has no event of
    // its own, only the release of its key.
    let copied = top.join("one.txt");
    send(&["key", "ctrl+a"]);
    send(&["key", "ctrl+c"]);
    send(&["key", "alt+Up"]);
    send(&["key", "ctrl+v"]);
    assert!(
        wait_for(&copied, true, Duration::from_secs(20)),
        "Mod+C then Mod+V copied nothing: top {:?}",
        names(&top)
    );
    assert!(deep.join("one.txt").exists(), "a copy leaves the original");
    assert_eq!(names(&top), ["deep", "one.txt"]);

    // Cut: in top, narrow the list to the file by typing, select it, Mod+X,
    // clear the filter, up to root. This time the clipboard holds text, so
    // the paste event comes too, and Mod is let go before the key: still
    // one paste.
    send(&["type", "--delay", "120", "one"]);
    send(&["key", "ctrl+a"]);
    send(&["key", "ctrl+x"]);
    send(&["key", "Escape"]);
    send(&["key", "alt+Up"]);
    if runs("xclip", "-version") {
        let mut xclip = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("xclip: {e}"));
        {
            use std::io::Write as _;
            let mut stdin = xclip
                .stdin
                .take()
                .unwrap_or_else(|| panic!("xclip's stdin"));
            stdin
                .write_all(b"some text")
                .unwrap_or_else(|e| panic!("xclip: {e}"));
        }
        let _ = xclip.wait();
        std::thread::sleep(BEAT);
    }
    send(&[
        "keydown", "ctrl", "keydown", "v", "keyup", "ctrl", "keyup", "v",
    ]);
    let moved = root.join("one.txt");
    assert!(
        wait_for(&moved, true, Duration::from_secs(20)),
        "Mod+X then Mod+V moved nothing: root {:?}, top {:?}",
        names(&root),
        names(&top)
    );
    assert!(
        wait_for(&copied, false, Duration::from_secs(10)),
        "a cut takes the original away"
    );
    // Once: a second paste would have met the first and asked.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(names(&root), ["one.txt", "top"]);
    assert_eq!(names(&top), ["deep"]);
    assert!(deep.join("one.txt").exists());
}
