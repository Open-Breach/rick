//! The mounted volumes from `/proc/mounts`, with the removable flag from
//! sysfs (`/sys/block/<disk>/removable`, which is what udisks2 reads) and
//! the free space from `statvfs`.
//!
//! Pseudo file systems and the system disk (what is mounted at `/`, and any
//! bind mount of it) are left out; network file systems and anything the
//! kernel calls removable are marked as such.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use types::Error;

use crate::traits::{Volume, VolumeKind};

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
            Some(MountLine {
                device,
                mount_point,
                fs_type,
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

/// The volume's display name: its mount point's last component, or the
/// device for a root mount.
fn name_of(line: &MountLine) -> String {
    line.mount_point
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| line.device.clone())
}

/// The mounts of `text` that are user-browsable volumes other than the
/// system disk, `root_dev` being the device id of `/`.
fn volumes_from(
    text: &str,
    root_dev: Option<u64>,
    space: &dyn Fn(&Path) -> Option<(u64, u64)>,
) -> Vec<Volume> {
    let mut out: Vec<Volume> = Vec::new();
    for line in parse(text) {
        if PSEUDO.contains(&line.fs_type.as_str()) || line.mount_point == Path::new("/") {
            continue;
        }
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
        if dev.is_some() && dev == root_dev {
            continue;
        }
        if out.iter().any(|v| v.mount_point == line.mount_point) {
            continue;
        }
        let (total, free) =
            space(&line.mount_point).map_or((None, None), |(t, f)| (Some(t), Some(f)));
        out.push(Volume {
            name: name_of(&line),
            kind: kind_of(&line),
            mount_point: line.mount_point,
            total,
            free,
        });
    }
    out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    out
}

/// Every mounted volume that is not the system disk.
pub(super) fn list() -> Result<Vec<Volume>, Error> {
    let text = fs::read_to_string("/proc/mounts").map_err(Error::from_io)?;
    let root_dev = fs::metadata("/").ok().map(|m| m.dev());
    Ok(volumes_from(&text, root_dev, &crate::volumes::space))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pseudo_mounts_and_the_system_disk_are_left_out() {
        let volumes = volumes_from(SAMPLE, None, &|p| {
            (p == Path::new("/data")).then_some((100, 40))
        });
        let names: Vec<(&str, &Path, VolumeKind)> = volumes
            .iter()
            .map(|v| (v.name.as_str(), v.mount_point.as_path(), v.kind))
            .collect();
        assert_eq!(
            names,
            [
                ("data", Path::new("/data"), VolumeKind::Fixed),
                ("DISC", Path::new("/media/me/DISC"), VolumeKind::Optical),
                (
                    "USB STICK",
                    Path::new("/media/me/USB STICK"),
                    VolumeKind::Removable
                ),
                ("nfs", Path::new("/mnt/nfs"), VolumeKind::Network),
            ]
        );
        let data = volumes
            .iter()
            .find(|v| v.name == "data")
            .map(|v| (v.total, v.free));
        assert_eq!(data, Some((Some(100), Some(40))));
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
