//! Links the Windows import library the vendored unrar sources need. The
//! `unrar_sys` build script compiles RAR's own C++ (`pathfn.cpp` reads the
//! registry, `crypt.cpp` draws random bytes from `CryptGenRandom`) and links
//! `powrprof` and `shell32` but not `advapi32`, so the plugin library fails
//! to link on MSVC without it. This is build configuration, not a
//! `cfg(target_os)` in code.

fn main() {
    // The build script's only channel to cargo is stdout.
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
