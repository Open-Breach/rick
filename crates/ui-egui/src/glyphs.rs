//! The glyphs the shell draws from egui's bundled fonts.

/// Every symbol a widget label uses; the test below checks the default
/// fonts have them all, so no box ever shows in their place.
pub const ALL: &str = "📁📄🔗☰☷⟳⎘◀▶▲↶↷›✕—…▼";

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn the_default_fonts_have_every_glyph_the_shell_uses() {
        // kittest fails a frame that renders a missing glyph.
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            ui.label(ALL);
        });
        harness.run();
    }
}
