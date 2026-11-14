//! `rick-zfs`, the privileged helper of the ZFS plugin: the one program the
//! plugin runs as the administrator, through the platform's prompt, when
//! ZFS refuses a snapshot change for lack of permission. It accepts the
//! `zfs` arguments of one `snapshot`, `rename` or `destroy` of a snapshot
//! and nothing else, and runs the system's `zfs` with them
//! (`ext_zfs::helper`).

#![forbid(unsafe_code)]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ext_zfs::helper::main(&args));
}
