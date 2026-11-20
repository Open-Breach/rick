//! The plugin host's test binary: proves a file previews through the
//! plugins as Rick loads them.
//!
//! ```text
//! preview_check <file> [--play]
//! ```
//!
//! It loads the plugins the way `rick` does at start, from `plugins/`
//! beside this executable and from the per-user plugins folder of the
//! config directory (per-user wins), and prints one line per plugin found:
//! its id, what became of it, which of the two folders it came from and
//! its directory. Then it opens the file with the previewer that claims it
//! and renders the first frame. With `--play` it also starts the transport
//! and waits until playback has moved on and a frame has been shown.
//!
//! The release smoke test copies this binary next to the unpacked `rick`,
//! installs the media plugin's package into the per-user folder and plays a
//! video with it: that shows the GStreamer runtime that travels in the
//! plugin's folder is found from there (specs/plugin-distribution.md §4).
//!
//! ```text
//! cargo run -p plugins-native --example preview_check -- clip.mp4 --play
//! ```

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ext::{Frame, RenderRequest};
use platform::ConfigDir as _;
use plugins_native::{Host, Options, PluginStatus};
use types::{CancelToken, Location};
use vfs_local::LocalProvider;

/// How long playback may take to move on.
const PLAY_TIMEOUT: Duration = Duration::from_secs(30);
/// How far playback has to get.
const PLAYED: Duration = Duration::from_millis(300);

/// What the binary was asked for.
struct Args {
    file: PathBuf,
    play: bool,
}

fn args() -> Result<Args, String> {
    let mut file = None;
    let mut play = false;
    for arg in std::env::args_os().skip(1) {
        if arg == "--play" {
            play = true;
        } else if file.is_none() {
            file = Some(PathBuf::from(arg));
        } else {
            return Err("usage: preview_check <file> [--play]".to_owned());
        }
    }
    let file = file.ok_or("usage: preview_check <file> [--play]")?;
    Ok(Args { file, play })
}

/// One line per plugin found, as the plugins table has them.
fn table(host: &Host) -> String {
    let mut out = String::new();
    for plugin in host.plugins() {
        let status = match &plugin.status {
            PluginStatus::Loaded => "loaded".to_owned(),
            PluginStatus::Disabled => "disabled".to_owned(),
            PluginStatus::Refused(reason) => format!("refused ({reason})"),
        };
        // Writing into a String cannot fail.
        let _ = writeln!(
            out,
            "plugin {}: {status}, {}, {}",
            plugin.id,
            plugin.source.label(),
            plugin.dir.display()
        );
    }
    out
}

fn run(args: &Args) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("this executable: {e}"))?;
    let native = Arc::new(platform::Native::new());
    let host = Host::load(&Options {
        dirs: Options::dirs(&exe, &native.config_dir()),
        platform: Some(native.clone()),
        ..Options::default()
    });
    let mut out = table(&host);
    let mut registry = ext::Registry::new();
    for extension in host.extensions() {
        registry
            .load(&*extension)
            .map_err(|e| format!("{out}{}: {e}", extension.manifest().id.0))?;
    }

    let file = std::fs::canonicalize(&args.file)
        .map_err(|e| format!("{out}{}: {e}", args.file.display()))?;
    let head: Vec<u8> = std::fs::read(&file)
        .map_err(|e| format!("{out}{}: {e}", file.display()))?
        .into_iter()
        .take(4096)
        .collect();
    let provider: Arc<dyn ext::Provider> = Arc::new(LocalProvider::new(native));
    let loc = Location::new(vfs_local::PROVIDER_ID, platform::paths::from_path(&file));
    let request = RenderRequest {
        width: 640,
        height: 480,
        scale: 1.0,
        dark: false,
    };

    let shown = runtime::block_on(async {
        let entry = provider
            .stat(&loc)
            .await
            .map_err(|e| format!("stat: {e}"))?;
        let previewer = registry
            .previewers()
            .iter()
            .map(|previewer| (previewer.score(&entry, &head), previewer))
            .filter(|(score, _)| *score > 0)
            .max_by_key(|(score, _)| *score)
            .map(|(_, previewer)| Arc::clone(previewer))
            .ok_or("no loaded plugin previews this file")?;
        let mut doc = previewer
            .open(&loc, Arc::clone(&provider), CancelToken::new())
            .await
            .map_err(|e| format!("open: {e}"))?;
        let size = |frame: &Frame| match frame {
            Frame::Rgba { width, height, .. } => Some((*width, *height)),
            Frame::Text(_) | Frame::External(_) => None,
        };
        let first = doc.render(0, request).map_err(|e| format!("render: {e}"))?;
        let mut shown = match size(&first) {
            Some((width, height)) => format!("first frame {width}x{height}"),
            None => "first frame not a picture".to_owned(),
        };
        if !args.play {
            return Ok::<String, String>(shown);
        }

        // What the previewer says about itself, for the log when playback
        // does not start: a missing runtime or decoder is named there.
        let said = format!("{:?} {:?}", doc.status(), doc.sections());
        match doc.transport() {
            Some(transport) => transport.play(),
            None => return Err(format!("the preview has no transport: {said}")),
        }
        let started = Instant::now();
        let mut pictures = 0_u32;
        loop {
            // The player decodes on threads of its own; this one only
            // looks at what it has.
            std::thread::sleep(Duration::from_millis(40));
            if size(&doc.render(0, request).map_err(|e| format!("render: {e}"))?).is_some() {
                pictures += 1;
            }
            let position = doc.transport().map_or(Duration::ZERO, |t| t.position());
            if position >= PLAYED && pictures > 0 {
                let _ = write!(shown, ", played to {:.2} s", position.as_secs_f32());
                return Ok(shown);
            }
            if started.elapsed() > PLAY_TIMEOUT {
                return Err(format!(
                    "playback did not move on in {} s (position {:.2} s, {pictures} \
                     pictures): {:?} {:?}; at first: {said}",
                    PLAY_TIMEOUT.as_secs(),
                    position.as_secs_f32(),
                    doc.status(),
                    doc.sections()
                ));
            }
        }
    })
    .map_err(|e| format!("{out}{}: {e}", file.display()))?;

    // The host outlives everything that points into the plugins' code.
    drop(registry);
    let _ = write!(out, "preview ok: {}, {shown}", file.display());
    drop(host);
    Ok(out)
}

fn main() -> ExitCode {
    // The binary's output is its purpose.
    let mut stdout = std::io::stdout().lock();
    let (report, code) = match args().and_then(|args| run(&args)) {
        Ok(report) => (report, ExitCode::SUCCESS),
        Err(report) => (format!("preview failed: {report}"), ExitCode::FAILURE),
    };
    let _ = std::io::Write::write_all(&mut stdout, format!("{report}\n").as_bytes());
    code
}
