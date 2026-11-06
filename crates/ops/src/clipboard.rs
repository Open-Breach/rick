//! The internal clipboard Copy and Cut fill and Paste reads.

use types::Entry;

/// Whether Paste copies or moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    /// Paste copies; the clipboard keeps its items.
    Copy,
    /// Paste moves; the clipboard is emptied afterwards.
    Cut,
}

/// What Copy or Cut put on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipboard {
    /// The entries, with their locations.
    pub items: Vec<Entry>,
    /// Copy or move.
    pub mode: ClipMode,
}
