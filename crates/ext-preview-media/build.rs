//! Gives the plugin library a run path to the GStreamer runtime that
//! travels in its own folder on macOS: `gstreamer/lib` beside the library,
//! wherever `preview-media/` lies, beside the binary or in the per-user
//! plugins folder (`cargo xtask bundle`, specs/plugin-distribution.md §4).
//! The official GStreamer libraries are named `@rpath/<file>`, so the
//! library that links them has to say where they are; from there on each
//! of them, and each element, carries a run path of its own to the `lib`
//! folder it lies in or under. Linux uses the system GStreamer. On Windows
//! the DLLs lie in the plugin's folder and the plugin host puts that
//! folder on the `PATH` before it loads the plugin. This is build
//! configuration, not a `cfg(target_os)` in code.

fn main() {
    // The build script's only channel to cargo is stdout.
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-rpath,@loader_path/gstreamer/lib");
    }
}
