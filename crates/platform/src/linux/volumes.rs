//! The mounted volumes from `/proc/mounts`, with the removable flag from
//! sysfs (`/sys/block/<disk>/removable`, which is what udisks2 reads), the
//! free space from `statvfs`, and ejecting through `udisksctl`: a drive is
//! unmounted and powered off, a mounted disk image (a loop device the
//! *Mount* action set up) is unmounted and its loop device deleted.
//!
//! Pseudo file systems and bind mounts of the system disk are left out; the
//! system disk itself is listed as `System`; network file systems and
//! anything the kernel calls removable are marked as such.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rustix::event::{PollFd, PollFlags, poll};
use rustix::pipe::{PipeFlags, pipe_with};
use tracing::debug;
use types::Error;

use crate::traits::{MountWatch, MountsChanged, Volume, VolumeKind};

/// File system types that are not volumes a user browses.
const PSEUDO: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fuse.gvfsd-fuse",
    "fuse.portal",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nsfs",
    "overlay",
    "proc",
    "pstore",
    "ramfs",
    "rpc_pipefs",
    "securityfs",
    "selinuxfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// File system types that are network mounts.
const NETWORK: &[&str] = &[
    "9p",
    "afs",
    "ceph",
    "cifs",
    "fuse.sshfs",
    "glusterfs",
    "fuse.glusterfs",
    "nfs",
    "nfs4",
    "smb3",
    "smbfs",
];

/// One line of `/proc/mounts`, decoded.
struct MountLine {
    device: String,
    mount_point: PathBuf,
    fs_type: String,
    /// The mount options, `rw,relatime` and the like.
    options: String,
}

impl MountLine {
    /// Whether the mount carries the option `ro`.
    fn read_only(&self) -> bool {
        self.options.split(',').any(|option| option == "ro")
    }
}

/// `/proc/mounts` escapes space, tab, newline and backslash as octal.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&field[i + 1..i + 4], 8)
        {
            out.push(value);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse(text: &str) -> Vec<MountLine> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = unescape(fields.next()?);
            let mount_point = PathBuf::from(unescape(fields.next()?));
            let fs_type = fields.next()?.to_owned();
            let options = fields.next().unwrap_or_default().to_owned();
            Some(MountLine {
                device,
                mount_point,
                fs_type,
                options,
            })
        })
        .collect()
}

/// The block device name without its partition: `sdb1` → `sdb`, `nvme0n1p2`
/// → `nvme0n1`, `mmcblk0p1` → `mmcblk0`.
fn disk_of(device: &str) -> Option<String> {
    let name = device.strip_prefix("/dev/")?;
    if name.contains('/') {
        return None;
    }
    let disk = if let Some((disk, part)) = name.rsplit_once('p')
        && part.bytes().all(|b| b.is_ascii_digit())
        && disk.bytes().last().is_some_and(|b| b.is_ascii_digit())
    {
        disk.to_owned()
    } else {
        name.trim_end_matches(|c: char| c.is_ascii_digit())
            .to_owned()
    };
    (!disk.is_empty()).then_some(disk)
}

/// Whether the kernel flags the disk behind `device` as removable.
fn removable(device: &str) -> bool {
    disk_of(device)
        .and_then(|disk| fs::read_to_string(format!("/sys/block/{disk}/removable")).ok())
        .is_some_and(|flag| flag.trim() == "1")
}

/// The kind of a mount, from its type, device and mount point.
fn kind_of(line: &MountLine) -> VolumeKind {
    let fs_type = line.fs_type.as_str();
    if NETWORK.contains(&fs_type) || fs_type.starts_with("nfs") || line.device.contains("://") {
        return VolumeKind::Network;
    }
    if matches!(fs_type, "iso9660" | "udf") {
        return VolumeKind::Optical;
    }
    let under_media = line.mount_point.starts_with("/media")
        || line.mount_point.starts_with("/run/media")
        || line.mount_point.starts_with("/mnt");
    if removable(&line.device) || (under_media && !line.device.starts_with("/dev/mapper")) {
        return VolumeKind::Removable;
    }
    VolumeKind::Fixed
}

/// The volume's display name: its mount point's last component, `System`
/// for the root mount.
fn name_of(line: &MountLine) -> String {
    if line.mount_point == Path::new("/") {
        return "System".to_owned();
    }
    line.mount_point
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| line.device.clone())
}

/// The mounts of `text` that are user-browsable volumes, the system disk
/// first, `root_dev` being the device id of `/`.
fn volumes_from(
    text: &str,
    root_dev: Option<u64>,
    space: &dyn Fn(&Path) -> Option<(u64, u64)>,
) -> Vec<Volume> {
    let mut out: Vec<Volume> = Vec::new();
    for line in parse(text) {
        if PSEUDO.contains(&line.fs_type.as_str()) {
            continue;
        }
        let is_root = line.mount_point == Path::new("/");
        if line.mount_point.starts_with("/proc")
            || line.mount_point.starts_with("/sys")
            || line.mount_point.starts_with("/dev")
            || line.mount_point.starts_with("/run")
                && !line.mount_point.starts_with("/run/media")
                && !line.mount_point.starts_with("/run/mount")
            || line.mount_point.starts_with("/boot")
            || line.mount_point.starts_with("/snap")
            || line.mount_point.starts_with("/var/lib/docker")
            || line.mount_point.starts_with("/var/snap")
        {
            continue;
        }
        // A bind mount of the system disk shows the root's device id.
        let dev = fs::metadata(&line.mount_point).ok().map(|m| m.dev());
        if !is_root && dev.is_some() && dev == root_dev {
            continue;
        }
        if out.iter().any(|v| v.mount_point == line.mount_point) {
            continue;
        }
        let (total, free) =
            space(&line.mount_point).map_or((None, None), |(t, f)| (Some(t), Some(f)));
        let device = line
            .device
            .starts_with("/dev/")
            .then(|| line.device.clone());
        out.push(Volume {
            name: name_of(&line),
            kind: if is_root {
                VolumeKind::Fixed
            } else {
                kind_of(&line)
            },
            read_only: line.read_only(),
            fs_type: line.fs_type,
            device,
            mount_point: line.mount_point,
            total,
            free,
        });
    }
    out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    out
}

/// Every mounted volume the user can browse, the system disk first.
pub(super) fn list() -> Result<Vec<Volume>, Error> {
    let text = fs::read_to_string("/proc/mounts").map_err(Error::from_io)?;
    let root_dev = fs::metadata("/").ok().map(|m| m.dev());
    Ok(volumes_from(&text, root_dev, &crate::volumes::space))
}

/// Runs a command and turns a failure into an error carrying what it said.
/// Listens for mounts and unmounts and calls `changed` on each, until the
/// returned watch is dropped.
///
/// The kernel's mount table is pollable: on every mount and unmount in this
/// process's namespace a `poll` on `/proc/self/mounts` reports a priority
/// event, once. A thread waits for that and for the end of the watch, which
/// it sees as the hang-up of a pipe whose other end the watch holds.
pub(super) fn watch_mounts(changed: MountsChanged) -> Option<MountWatch> {
    listen(changed).map(|(watch, _)| watch)
}

/// [`watch_mounts`], with the listening thread's handle.
fn listen(changed: MountsChanged) -> Option<(MountWatch, std::thread::JoinHandle<()>)> {
    let table = fs::File::open("/proc/self/mounts")
        .inspect_err(|error| debug!(%error, "the mount table cannot be watched"))
        .ok()?;
    let (ended, keep) = pipe_with(PipeFlags::CLOEXEC)
        .inspect_err(|error| debug!(%error, "the mount table cannot be watched"))
        .ok()?;
    let thread = std::thread::Builder::new()
        .name("rick-mounts".into())
        .spawn(move || {
            loop {
                let mut fds = [
                    PollFd::new(&table, PollFlags::PRI),
                    PollFd::new(&ended, PollFlags::IN),
                ];
                match poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => {
                        debug!(%error, "the watch of the mount table ended");
                        break;
                    }
                }
                let [table_events, ended_events] = [fds[0].revents(), fds[1].revents()];
                if !ended_events.is_empty() {
                    break;
                }
                if table_events.contains(PollFlags::PRI) {
                    changed();
                } else if !table_events.is_empty() {
                    // An error without the event: nothing to wait for.
                    debug!(?table_events, "the watch of the mount table ended");
                    break;
                }
            }
        })
        .inspect_err(|error| debug!(%error, "the mount table cannot be watched"))
        .ok()?;
    Some((MountWatch::new(keep), thread))
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), Error> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(Error::from_io)?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.trim();
    Err(Error::Io(std::io::Error::other(if said.is_empty() {
        format!("{program} exited with {}", output.status)
    } else {
        said.to_owned()
    })))
}

/// Whether `program` is on the path.
fn available(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// The disk utilities *Format…* opens, in the order they are looked for:
/// GNOME Disks, then the desktop's equivalents, KDE Partition Manager and
/// `GParted`.
const FORMAT_TOOLS: &[&str] = &["gnome-disks", "partitionmanager", "gparted"];

/// The disk utility that is installed, if one is.
pub(super) fn format_tool() -> Option<&'static str> {
    FORMAT_TOOLS.iter().copied().find(|tool| available(tool))
}

/// The arguments that take `tool` to `device`: GNOME Disks opens its
/// format dialogue on it, the others open with the device selected.
fn format_args(tool: &str, device: Option<&str>) -> Vec<String> {
    match (tool, device) {
        (_, None) => Vec::new(),
        ("gnome-disks", Some(device)) => vec![
            "--block-device".to_owned(),
            device.to_owned(),
            "--format-device".to_owned(),
        ],
        ("partitionmanager", Some(device)) => vec!["--device".to_owned(), device.to_owned()],
        (_, Some(device)) => vec![device.to_owned()],
    }
}

/// Opens the installed disk utility on the volume's device. Rick formats
/// nothing itself: what happens next is the tool's and the user's.
pub(super) fn open_format_tool(volume: &Volume) -> Result<(), Error> {
    let tool = format_tool().ok_or(Error::Unsupported("no disk utility is installed"))?;
    let args = format_args(tool, volume.device.as_deref());
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    super::spawn_detached(tool, &args)
}

/// The loop device behind a partition or loop device name: `/dev/loop3`
/// for `/dev/loop3p1` and for itself; `None` for any other device.
fn loop_device(device: &str) -> Option<String> {
    let name = device.strip_prefix("/dev/loop")?;
    let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
    let rest = &name[digits.len()..];
    let is_partition = rest
        .strip_prefix('p')
        .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    (!digits.is_empty() && (rest.is_empty() || is_partition)).then(|| format!("/dev/loop{digits}"))
}

/// Unmounts the volume and powers its drive off: `udisksctl unmount` and
/// `udisksctl power-off` where udisks2 is installed, which needs no
/// privilege for media the user plugged in; a plain `umount` otherwise. A
/// mounted disk image sits on a loop device, which is deleted instead of
/// powered off, so the image file is released.
pub(super) fn eject(volume: &Volume) -> Result<(), Error> {
    if !volume.is_removable() {
        return Err(Error::Unsupported("only removable media can be ejected"));
    }
    let mount_point = volume.mount_point.as_os_str();
    match &volume.device {
        Some(device) if available("udisksctl") => {
            let loop_device = loop_device(device);
            let device = std::ffi::OsStr::new(device);
            run(
                "udisksctl",
                &[
                    std::ffi::OsStr::new("unmount"),
                    std::ffi::OsStr::new("-b"),
                    device,
                ],
            )?;
            if let Some(loop_device) = loop_device {
                // A disk image: give the loop device back.
                if let Err(error) = run(
                    "udisksctl",
                    &[
                        std::ffi::OsStr::new("loop-delete"),
                        std::ffi::OsStr::new("-b"),
                        std::ffi::OsStr::new(&loop_device),
                    ],
                ) {
                    tracing::debug!(%error, "loop-delete refused; unmounted only");
                }
                return Ok(());
            }
            // Powering the drive off is the eject proper; a drive that does
            // not support it (an SD reader) is unmounted all the same.
            if let Err(error) = run(
                "udisksctl",
                &[
                    std::ffi::OsStr::new("power-off"),
                    std::ffi::OsStr::new("-b"),
                    device,
                ],
            ) {
                tracing::debug!(%error, "power-off refused; unmounted only");
            }
            Ok(())
        }
        _ => run("umount", &[mount_point]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 149, *Format…*: Rick opens the system's own tool on the
    /// drive and formats nothing itself. GNOME Disks is taken to its
    /// format dialogue for the device, the others to the device.
    /// Waits up to five seconds for `done`.
    fn within_five_seconds(done: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        done()
    }

    #[test]
    fn the_watch_of_the_mount_table_ends_with_its_watch() {
        let rung = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&rung);
        let Some((watch, thread)) = listen(Box::new(move || {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })) else {
            panic!("the mount table is not watched");
        };
        assert!(!thread.is_finished());
        drop(watch);
        assert!(within_five_seconds(|| thread.is_finished()));
    }

    /// A mount needs the right to mount: the test runs where it has it (as
    /// root, in a container that allows it) and says so where it has not.
    #[test]
    fn a_mount_and_an_unmount_ring_the_watch() {
        let rung = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = std::sync::Arc::clone(&rung);
        let Some((watch, _thread)) = listen(Box::new(move || {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })) else {
            panic!("the mount table is not watched");
        };
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mounted = Command::new("mount")
            .args(["-t", "tmpfs", "none"])
            .arg(dir.path())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !mounted {
            eprintln!("skipped: this user may not mount a file system");
            return;
        }
        let after_mount =
            within_five_seconds(|| rung.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        let before = rung.load(std::sync::atomic::Ordering::SeqCst);
        let unmounted = Command::new("umount")
            .arg(dir.path())
            .status()
            .is_ok_and(|status| status.success());
        assert!(unmounted, "the test's file system stays mounted");
        assert!(after_mount, "the mount did not ring the watch");
        assert!(
            within_five_seconds(|| rung.load(std::sync::atomic::Ordering::SeqCst) > before),
            "the unmount did not ring the watch"
        );
        drop(watch);
    }

    #[test]
    fn the_format_tools_are_taken_to_the_device() {
        assert_eq!(
            format_args("gnome-disks", Some("/dev/sdb1")),
            ["--block-device", "/dev/sdb1", "--format-device"]
        );
        assert_eq!(
            format_args("partitionmanager", Some("/dev/sdb1")),
            ["--device", "/dev/sdb1"]
        );
        assert_eq!(format_args("gparted", Some("/dev/sdb1")), ["/dev/sdb1"]);
        assert_eq!(format_args("gnome-disks", None), Vec::<String>::new());
    }

    const SAMPLE: &str = "\
proc /proc proc rw,relatime 0 0
sysfs /sys sysfs rw,relatime 0 0
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/nvme0n1p1 /boot/efi vfat rw 0 0
tmpfs /run tmpfs rw 0 0
/dev/sdb1 /media/me/USB\\040STICK vfat rw 0 0
server:/export /mnt/nfs nfs4 rw 0 0
//box/share /run/user/1000/gvfs/smb cifs rw 0 0
/dev/sr0 /media/me/DISC iso9660 ro 0 0
/dev/sda1 /data ext4 rw 0 0
";

    #[test]
    fn pseudo_mounts_are_left_out_and_the_system_disk_comes_first() {
        let volumes = volumes_from(SAMPLE, None, &|p| {
            (p == Path::new("/data")).then_some((100, 40))
        });
        let names: Vec<(&str, &Path, VolumeKind, &str)> = volumes
            .iter()
            .map(|v| {
                (
                    v.name.as_str(),
                    v.mount_point.as_path(),
                    v.kind,
                    v.fs_type.as_str(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("System", Path::new("/"), VolumeKind::Fixed, "ext4"),
                ("data", Path::new("/data"), VolumeKind::Fixed, "ext4"),
                (
                    "DISC",
                    Path::new("/media/me/DISC"),
                    VolumeKind::Optical,
                    "iso9660"
                ),
                (
                    "USB STICK",
                    Path::new("/media/me/USB STICK"),
                    VolumeKind::Removable,
                    "vfat"
                ),
                ("nfs", Path::new("/mnt/nfs"), VolumeKind::Network, "nfs4"),
            ]
        );
        let data = volumes
            .iter()
            .find(|v| v.name == "data")
            .map(|v| (v.total, v.free, v.device.clone()));
        assert_eq!(data, Some((Some(100), Some(40), Some("/dev/sda1".into()))));
        // The disc is mounted `ro`, the rest `rw`.
        let read_only: Vec<&str> = volumes
            .iter()
            .filter(|v| v.read_only)
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(read_only, ["DISC"]);
        let nfs = volumes.iter().find(|v| v.name == "nfs");
        assert_eq!(nfs.and_then(|v| v.device.clone()), None);
        assert!(matches!(
            eject(nfs.unwrap_or_else(|| panic!("nfs"))),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn loop_devices_are_told_from_drives() {
        assert_eq!(loop_device("/dev/loop3").as_deref(), Some("/dev/loop3"));
        assert_eq!(loop_device("/dev/loop12p1").as_deref(), Some("/dev/loop12"));
        assert_eq!(loop_device("/dev/loop3x"), None);
        assert_eq!(loop_device("/dev/sdb1"), None);
        assert_eq!(loop_device("/dev/loop"), None);
    }

    #[test]
    fn devices_reduce_to_their_disk() {
        assert_eq!(disk_of("/dev/sdb1").as_deref(), Some("sdb"));
        assert_eq!(disk_of("/dev/nvme0n1p2").as_deref(), Some("nvme0n1"));
        assert_eq!(disk_of("/dev/mmcblk0p1").as_deref(), Some("mmcblk0"));
        assert_eq!(disk_of("/dev/sr0").as_deref(), Some("sr"));
        assert_eq!(disk_of("server:/export"), None);
        assert_eq!(unescape("a\\040b\\134c"), "a b\\c");
    }
}
