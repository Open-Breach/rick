//! The command line (specs/system-integration.md §4): the core
//! subcommands by `clap` derive, the plugins' added from their manifests,
//! so `--help` lists both.
//!
//! `rick [<path>…]` opens each path in a tab (the caller's working
//! directory without any), `rick reveal <file>…` opens the folders and
//! selects the files, `--new-window` puts them in a window of their own;
//! `rick <command> <args…>` is a plugin's, dispatched to the extension
//! that declared it. Paths are resolved against the caller's working
//! directory before anything is sent to the instance, so a relative path
//! typed in a terminal means the same in the window. An argument the
//! command line does not know is a usage error: the error and the help,
//! exit 2.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{ColorChoice, CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use ext::CommandSpec;

/// The command line, as the launch and the instance both parse it.
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(
    name = "rick",
    about = "Rick, a god-like file manager",
    long_about = "Rick, a god-like file manager.\n\nOne Rick runs per user session and config \
                  directory: a second `rick` hands its command line to the running one and exits \
                  with its status.",
    version = crate::VERSION,
    disable_help_subcommand = true,
    color = ColorChoice::Never
)]
pub struct Cli {
    /// Open the paths in a window of their own
    #[arg(long, global = true)]
    pub new_window: bool,
    /// Use this config directory instead of the platform's; each config directory is a Rick of its own
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// List the plugins found and whether each loaded, then exit
    #[arg(long, conflicts_with = "paths")]
    pub list_plugins: bool,
    #[command(subcommand)]
    pub command: Option<Core>,
    /// Folders to open, one tab each; the current directory without any
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,
}

/// The core subcommands.
#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Core {
    /// Open the folder of each file and select the files
    Reveal {
        /// The files to select, each in its folder's tab
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
    },
}

/// What a command line asks for, paths resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Open each path in a tab; the caller's working directory when none
    /// was given.
    Open {
        /// The folders, resolved against the caller's working directory.
        paths: Vec<PathBuf>,
        /// In a window of their own.
        new_window: bool,
    },
    /// Open each file's folder and select the file.
    Reveal {
        /// The files, resolved against the caller's working directory.
        files: Vec<PathBuf>,
        /// In a window of their own.
        new_window: bool,
    },
    /// A plugin's subcommand, with its arguments as typed.
    Plugin {
        /// The command's name.
        name: String,
        /// Its arguments.
        args: Vec<String>,
    },
    /// `--help`: the text, for the caller's standard output.
    Help(String),
    /// `--version`: the text, for the caller's standard output.
    Version(String),
}

/// A command line that could not be parsed: the message with the usage
/// and, for a command or option the command line does not know, the full
/// help; the caller prints it and exits with
/// [`EXIT_USAGE`](platform::instance::EXIT_USAGE).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub String);

/// The clap command: the core by derive, plus one subcommand per plugin
/// command, each taking its arguments as typed.
#[must_use]
pub fn command(plugins: &[CommandSpec]) -> clap::Command {
    let mut command = Cli::command();
    for spec in plugins {
        command = command.subcommand(
            clap::Command::new(spec.name).about(spec.help).arg(
                clap::Arg::new("args")
                    .value_name("ARGS")
                    .num_args(0..)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true)
                    .value_parser(clap::value_parser!(String)),
            ),
        );
    }
    command
}

/// The help text, core and plugin commands included.
#[must_use]
pub fn help(plugins: &[CommandSpec]) -> String {
    command(plugins).render_long_help().to_string()
}

/// `path` against `cwd`: canonical where it names something, so a
/// launcher's relative path and a symlink both reach the thing itself; as
/// joined otherwise, for the pane to say what it cannot list.
#[must_use]
pub fn resolve(cwd: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// Parses `argv` (the arguments after the program name) typed from
/// `cwd`, with the plugin commands `plugins` known.
///
/// # Errors
///
/// [`Usage`] for a command line clap refuses.
pub fn parse(argv: &[String], cwd: &Path, plugins: &[CommandSpec]) -> Result<Parsed, Usage> {
    let mut command = command(plugins);
    let full: Vec<OsString> = std::iter::once(OsString::from("rick"))
        .chain(argv.iter().map(OsString::from))
        .collect();
    let matches = match command.try_get_matches_from_mut(full) {
        Ok(matches) => matches,
        Err(error) => {
            return match error.kind() {
                clap::error::ErrorKind::DisplayHelp => Ok(Parsed::Help(error.to_string())),
                clap::error::ErrorKind::DisplayVersion => {
                    Ok(Parsed::Version(error.to_string().trim_end().to_owned()))
                }
                clap::error::ErrorKind::InvalidSubcommand
                | clap::error::ErrorKind::UnknownArgument => Err(Usage(format!(
                    "{}\n{}",
                    error.render(),
                    command.render_long_help()
                ))),
                _ => Err(Usage(error.render().to_string())),
            };
        }
    };
    // A plugin's subcommand comes first: the derived `Cli` knows only the
    // core ones.
    if let Some((name, sub)) = matches.subcommand()
        && plugins.iter().any(|spec| spec.name == name)
    {
        let rest = sub
            .get_many::<String>("args")
            .map(|values| values.cloned().collect())
            .unwrap_or_default();
        return Ok(Parsed::Plugin {
            name: name.to_owned(),
            args: rest,
        });
    }
    let cli = Cli::from_arg_matches(&matches).map_err(|error| Usage(error.render().to_string()))?;
    Ok(match cli.command {
        Some(Core::Reveal { files }) => Parsed::Reveal {
            files: files.iter().map(|f| resolve(cwd, f)).collect(),
            new_window: cli.new_window,
        },
        None => {
            // A bare word that names nothing in the working directory is
            // a command the command line does not know (a typo of one, a
            // plugin that is not loaded), not a folder to open: the help
            // says what the commands are. A path with a separator, or
            // one that exists, is a folder whatever it is called.
            if let Some(word) = cli.paths.iter().find(|p| is_unknown_command(cwd, p)) {
                return Err(Usage(format!(
                    "rick: unknown command `{}`\n\n{}",
                    word.display(),
                    command.render_long_help()
                )));
            }
            let mut paths: Vec<PathBuf> = cli.paths.iter().map(|p| resolve(cwd, p)).collect();
            if paths.is_empty() {
                paths.push(resolve(cwd, Path::new(".")));
            }
            Parsed::Open {
                paths,
                new_window: cli.new_window,
            }
        }
    })
}

/// Whether a positional argument reads as a command rather than a path:
/// one component, no separator, not `.` or `..`, and nothing of that name
/// in `cwd`.
fn is_unknown_command(cwd: &Path, arg: &Path) -> bool {
    let Some(text) = arg.to_str() else {
        return false;
    };
    if text.is_empty() || text == "." || text == ".." || text.contains(['/', '\\']) {
        return false;
    }
    std::fs::symlink_metadata(cwd.join(arg)).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALL: CommandSpec = CommandSpec {
        name: "install",
        help: "Install an archive",
    };

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn paths_open_and_none_is_the_working_directory() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap_or_else(|e| panic!("{e}"));
        let canonical = dir.path().canonicalize().unwrap_or_else(|e| panic!("{e}"));
        let parsed = parse(&args(&["sub", "./nowhere"]), dir.path(), &[])
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Open {
                paths: vec![canonical.join("sub"), dir.path().join("nowhere")],
                new_window: false
            }
        );
        // A bare word that names nothing is a command the command line
        // does not know, not a folder; with a separator it is a path.
        let Err(Usage(text)) = parse(&args(&["nowhere"]), dir.path(), &[]) else {
            panic!("a bare unknown word is an unknown command");
        };
        assert!(
            text.contains("unknown command `nowhere`") && text.contains("reveal"),
            "{text}"
        );
        let parsed = parse(&[], dir.path(), &[]).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Open {
                paths: vec![canonical.clone()],
                new_window: false
            }
        );
        let parsed = parse(&args(&["--new-window", "sub"]), dir.path(), &[])
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Open {
                paths: vec![canonical.join("sub")],
                new_window: true
            }
        );
    }

    #[test]
    fn reveal_takes_files_and_a_plugin_command_takes_its_arguments_as_typed() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let parsed = parse(
            &args(&["reveal", "--new-window", "a.txt", "/b/c.txt"]),
            dir.path(),
            &[],
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Reveal {
                files: vec![dir.path().join("a.txt"), PathBuf::from("/b/c.txt")],
                new_window: true
            }
        );
        assert!(
            matches!(parse(&args(&["reveal"]), dir.path(), &[]), Err(Usage(text)) if text.contains("FILE")),
            "reveal needs a file"
        );
        let parsed = parse(
            &args(&["install", "--scope", "x.tar.gz"]),
            dir.path(),
            &[INSTALL],
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Plugin {
                name: "install".to_owned(),
                args: args(&["--scope", "x.tar.gz"])
            }
        );
        let parsed =
            parse(&args(&["install"]), dir.path(), &[INSTALL]).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Plugin {
                name: "install".to_owned(),
                args: Vec::new()
            }
        );
        // Without the plugin, `install` is a folder to open when there is
        // one of that name, and an unknown command otherwise.
        assert!(matches!(
            parse(&args(&["install"]), dir.path(), &[]),
            Err(Usage(text)) if text.contains("unknown command `install`")
        ));
        std::fs::create_dir(dir.path().join("install")).unwrap_or_else(|e| panic!("{e}"));
        let parsed =
            parse(&args(&["install"]), dir.path(), &[]).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            parsed,
            Parsed::Open {
                paths: vec![
                    dir.path()
                        .join("install")
                        .canonicalize()
                        .unwrap_or_else(|e| panic!("{e}"))
                ],
                new_window: false
            }
        );
    }

    #[test]
    fn an_unknown_option_is_a_usage_error_with_the_help_and_help_lists_the_plugins() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let Err(Usage(text)) = parse(&args(&["--frobnicate"]), dir.path(), &[INSTALL]) else {
            panic!("an unknown option is refused");
        };
        assert!(text.contains("--frobnicate"), "{text}");
        assert!(
            text.contains("install") && text.contains("Install an archive"),
            "the help follows: {text}"
        );
        assert!(text.contains("reveal"), "{text}");

        let Ok(Parsed::Help(text)) = parse(&args(&["--help"]), dir.path(), &[INSTALL]) else {
            panic!("--help is the help");
        };
        assert!(
            text.contains("reveal") && text.contains("install"),
            "{text}"
        );
        assert!(
            text.contains("--new-window")
                && text.contains("--config-dir")
                && text.contains("--list-plugins"),
            "{text}"
        );
        assert_eq!(text, help(&[INSTALL]));
        let Ok(Parsed::Version(text)) = parse(&args(&["--version"]), dir.path(), &[]) else {
            panic!("--version is the version");
        };
        assert_eq!(text, format!("rick {}", crate::VERSION));
        // No colour codes: the text travels over the socket.
        assert!(!text.contains('\u{1b}'));
        assert!(!help(&[INSTALL]).contains('\u{1b}'));
    }
}
