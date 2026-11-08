//! The user's terminal emulator: `$TERMINAL`, then the usual ones.

use std::path::Path;
use std::process::{Command, Stdio};

use types::Error;

use super::apps::on_path;

/// A known emulator: its program, the flag that sets the working
/// directory, and the flag that precedes a command to run.
struct Known {
    program: &'static str,
    cwd: &'static [&'static str],
    exec: &'static [&'static str],
}

/// The emulators tried in order when `$TERMINAL` is unset: the Debian
/// alternative first, then the desktop ones.
const KNOWN: &[Known] = &[
    Known {
        program: "x-terminal-emulator",
        cwd: &[],
        exec: &["-e"],
    },
    Known {
        program: "gnome-terminal",
        cwd: &["--working-directory"],
        exec: &["--"],
    },
    Known {
        program: "konsole",
        cwd: &["--workdir"],
        exec: &["-e"],
    },
    Known {
        program: "xfce4-terminal",
        cwd: &["--working-directory"],
        exec: &["-x"],
    },
    Known {
        program: "kitty",
        cwd: &["--directory"],
        exec: &[],
    },
    Known {
        program: "alacritty",
        cwd: &["--working-directory"],
        exec: &["-e"],
    },
    Known {
        program: "foot",
        cwd: &["--working-directory"],
        exec: &[],
    },
    Known {
        program: "wezterm",
        cwd: &["start", "--cwd"],
        exec: &[],
    },
    Known {
        program: "xterm",
        cwd: &[],
        exec: &["-e"],
    },
];

/// Opens a terminal in `dir`, running `command` there when given.
pub(crate) fn open(dir: &Path, command: Option<&[String]>) -> Result<(), Error> {
    let dir = std::path::absolute(dir).map_err(Error::from_io)?;
    let chosen = std::env::var("TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| {
            let program = t.split_whitespace().next()?.to_owned();
            on_path(&program).map(|_| (t, None))
        })
        .or_else(|| {
            KNOWN
                .iter()
                .find(|k| on_path(k.program).is_some())
                .map(|k| (k.program.to_owned(), Some(k)))
        });
    let Some((program, known)) = chosen else {
        return Err(Error::Unsupported(
            "no terminal emulator found; set $TERMINAL",
        ));
    };
    let mut words: Vec<String> = program.split_whitespace().map(str::to_owned).collect();
    if let Some(known) = known {
        let is_wezterm = known.program == "wezterm";
        if !known.cwd.is_empty() {
            words.extend(known.cwd.iter().map(|s| (*s).to_owned()));
            words.push(dir.to_string_lossy().into_owned());
        }
        if let Some(command) = command.filter(|c| !c.is_empty()) {
            if is_wezterm {
                words.push("--".to_owned());
            } else {
                words.extend(known.exec.iter().map(|s| (*s).to_owned()));
            }
            words.extend(command.iter().cloned());
        }
    } else if let Some(command) = command.filter(|c| !c.is_empty()) {
        words.push("-e".to_owned());
        words.extend(command.iter().cloned());
    }
    let (program, args) = words
        .split_first()
        .ok_or(Error::Unsupported("no terminal emulator named"))?;
    Command::new(program)
        .args(args)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(Error::from_io)
}
