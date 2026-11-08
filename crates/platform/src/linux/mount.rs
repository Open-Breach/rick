//! Mounting a disk image through udisks2: `udisksctl loop-setup` maps the
//! file to a loop device, `udisksctl mount` mounts the device, or its
//! first partition when the image carries a partition table, where the
//! desktop mounts removable media (`/media/<user>` or `/run/media/<user>`).
//! Nothing else mounts anything: no qemu-nbd, no FUSE. Ejecting the volume
//! ([`super::volumes::eject`]) unmounts it and deletes the loop device.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use types::Error;

use crate::traits::{ImageFormat, Volume, VolumeKind};

/// How long the kernel is given to announce the partitions of a freshly
/// set-up loop device.
const PARTITION_WAIT: Duration = Duration::from_secs(2);

/// Runs `udisksctl` with `args` and returns what it printed; a failure
/// becomes an error carrying what it said.
fn udisksctl(args: &[&std::ffi::OsStr]) -> Result<String, Error> {
    let output = Command::new("udisksctl")
        .args(args)
        .arg("--no-user-interaction")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Unsupported(
                    "udisks is not installed: `udisksctl` was not found, and Rick mounts \
                     through nothing else",
                )
            } else {
                Error::from_io(error)
            }
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.trim();
    Err(Error::Io(std::io::Error::other(if said.is_empty() {
        format!("udisksctl exited with {}", output.status)
    } else {
        said.to_owned()
    })))
}

/// The loop device `udisksctl loop-setup` named: `Mapped file x.iso as
/// /dev/loop3.`
fn parse_loop_device(output: &str) -> Option<String> {
    let after = output.rsplit(" as ").next()?;
    let device = after.trim().trim_end_matches('.');
    device.starts_with("/dev/loop").then(|| device.to_owned())
}

/// The mount point `udisksctl mount` named: `Mounted /dev/loop3 at
/// /media/me/DISC` (older releases end the line with a full stop).
fn parse_mount_point(output: &str) -> Option<PathBuf> {
    let after = output.split(" at ").nth(1)?;
    let text = after.trim();
    let candidate = PathBuf::from(text);
    if candidate.is_dir() {
        return Some(candidate);
    }
    let trimmed = PathBuf::from(text.trim_end_matches('.'));
    trimmed.is_dir().then_some(trimmed)
}

/// The mount point of an `AlreadyMounted` error: `Device /dev/loop3 is
/// already mounted at `/media/me/DISC'.`
fn parse_already_mounted(message: &str) -> Option<PathBuf> {
    let start = message.find("already mounted at `")? + "already mounted at `".len();
    let end = message[start..].find('\'')? + start;
    Some(PathBuf::from(&message[start..end]))
}

/// The first partition of a loop device, `/dev/loop3p1`, when the kernel
/// announces one within [`PARTITION_WAIT`]; the device itself otherwise
/// (an ISO or a partition image).
fn mount_target(device: &str) -> String {
    let Some(name) = device.strip_prefix("/dev/") else {
        return device.to_owned();
    };
    let sys = PathBuf::from("/sys/block").join(name);
    let started = Instant::now();
    loop {
        let mut partitions: Vec<String> = std::fs::read_dir(&sys)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|child| child.starts_with(&format!("{name}p")))
            .collect();
        if !partitions.is_empty() {
            partitions.sort();
            return format!("/dev/{}", partitions[0]);
        }
        if started.elapsed() > PARTITION_WAIT {
            return device.to_owned();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Mounts the image at `path` and returns its volume.
pub(super) fn mount(path: &Path) -> Result<Volume, Error> {
    let path = std::path::absolute(path).map_err(Error::from_io)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let format = ImageFormat::for_name(&name)
        .filter(|f| crate::NATIVE_MOUNTABLE.contains(f))
        .ok_or(Error::Unsupported(
            "Linux mounts ISO and raw disk images only",
        ))?;
    let mut args: Vec<&std::ffi::OsStr> = vec![std::ffi::OsStr::new("loop-setup")];
    if format == ImageFormat::Iso {
        args.push(std::ffi::OsStr::new("-r"));
    }
    args.push(std::ffi::OsStr::new("-f"));
    args.push(path.as_os_str());
    let output = udisksctl(&args)?;
    let device = parse_loop_device(&output).ok_or_else(|| {
        Error::Io(std::io::Error::other(format!(
            "udisksctl loop-setup did not name a loop device: {}",
            output.trim()
        )))
    })?;
    let target = mount_target(&device);
    let mount_point = match udisksctl(&[
        std::ffi::OsStr::new("mount"),
        std::ffi::OsStr::new("-b"),
        std::ffi::OsStr::new(&target),
    ]) {
        Ok(output) => parse_mount_point(&output),
        Err(error) => {
            let message = error.to_string();
            match parse_already_mounted(&message) {
                // The desktop mounted the loop device as soon as it appeared.
                Some(mount_point) => Some(mount_point),
                None => {
                    // Leave no loop device behind.
                    let _ = udisksctl(&[
                        std::ffi::OsStr::new("loop-delete"),
                        std::ffi::OsStr::new("-b"),
                        std::ffi::OsStr::new(&device),
                    ]);
                    return Err(error);
                }
            }
        }
    };
    let mount_point = mount_point.ok_or_else(|| {
        Error::Io(std::io::Error::other(
            "udisksctl mount did not name a mount point",
        ))
    })?;
    // The row as the volume list shows it, when the list has it already;
    // the facts the mount gave otherwise.
    if let Ok(volumes) = super::volumes::list()
        && let Some(volume) = volumes.into_iter().find(|v| v.mount_point == mount_point)
    {
        return Ok(volume);
    }
    let (total, free) =
        crate::volumes::space(&mount_point).map_or((None, None), |(t, f)| (Some(t), Some(f)));
    Ok(Volume {
        name: mount_point
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name),
        mount_point,
        kind: if format == ImageFormat::Iso {
            VolumeKind::Optical
        } else {
            VolumeKind::Removable
        },
        fs_type: String::new(),
        device: Some(target),
        total,
        free,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udisksctl_output_is_parsed() {
        assert_eq!(
            parse_loop_device("Mapped file /tmp/a b.iso as /dev/loop3.\n").as_deref(),
            Some("/dev/loop3")
        );
        assert_eq!(parse_loop_device("nothing here"), None);
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let text = format!("Mounted /dev/loop3 at {}.\n", dir.path().display());
        assert_eq!(parse_mount_point(&text).as_deref(), Some(dir.path()));
        let text = format!("Mounted /dev/loop3 at {}\n", dir.path().display());
        assert_eq!(parse_mount_point(&text).as_deref(), Some(dir.path()));
        assert_eq!(
            parse_mount_point("Mounted /dev/loop3 at /nonexistent/x."),
            None
        );
        assert_eq!(
            parse_already_mounted(
                "Error mounting /dev/loop3: GDBus.Error:org.freedesktop.UDisks2.Error.AlreadyMounted: Device /dev/loop3 is already mounted at `/media/me/DISC'.\n"
            ),
            Some(PathBuf::from("/media/me/DISC"))
        );
        assert_eq!(parse_already_mounted("Error mounting: refused"), None);
        assert_eq!(mount_target("/dev/loop999999"), "/dev/loop999999");
        assert_eq!(mount_target("loop0"), "loop0");
    }

    #[test]
    fn only_iso_and_raw_images_are_mountable_here() {
        assert!(matches!(
            mount(Path::new("/tmp/disk.vhdx")),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            mount(Path::new("/tmp/disk.qcow2")),
            Err(Error::Unsupported(_))
        ));
    }
}
