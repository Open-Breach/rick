//! Records the toolchain this host is built with, the exact `rustc -V`
//! line, which a plugin's manifest must repeat for the host to load it: a
//! plugin shares the kernel dylibs with the host, and only one compiler
//! build lays them out one way (QUESTIONS.md 81).

use std::process::Command;

fn main() {
    // The build script's only channel to cargo is stdout.
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=build.rs");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let toolchain = Command::new(rustc)
        .arg("-V")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=RICK_TOOLCHAIN={toolchain}");
}
