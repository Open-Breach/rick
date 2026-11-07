//! Gives the plugin library a run path to the GStreamer runtime a bundle
//! carries on macOS: `<bundle>/gstreamer/lib`, two directories up from
//! `plugins/preview-media/` (`cargo xtask bundle`, QUESTIONS.md 89). The
//! official GStreamer libraries name each other through `@rpath`, so the
//! library that loads them has to say where they are. Linux uses the
//! system GStreamer and Windows searches the binary's directory, where the
//! bundle puts the DLLs. This is build configuration, not a
//! `cfg(target_os)` in code.

fn main() {
    // The build script's only channel to cargo is stdout.
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-rpath,@loader_path/../../gstreamer/lib");
    }
}
