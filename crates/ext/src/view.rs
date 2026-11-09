//! Views: content an action opens in a dock tab of its own, which the UI
//! lays out — a comparison of two files or folders, a terminal session.
//!
//! An action returns [`ActionOutcome::View`](crate::ActionOutcome::View)
//! with one of these; the application opens a tab next to the pane's
//! active one and hands the view to the UI, which draws it. The view is
//! data or a trait object: the extension computes, the UI draws, as the
//! architecture rules ask. A [`Compare`] starts empty and is filled by the
//! job that reads the files, so the tab opens at once and shows the result
//! when it is there; a [`Console`] is live: the UI reads its screen, writes
//! what is typed and sizes it to the tab.
//!
//! # Worked example
//!
//! A comparison that a job finishes with a text diff.
//!
//! ```
//! use ext::{Compare, Diff, Hunk, HunkKind, TextDiff};
//!
//! let compare = Compare::pending("a.txt ↔ b.txt", "a.txt", "b.txt");
//! assert!(compare.result().is_none());
//! compare.finish(Ok(Diff::Text(TextDiff {
//!     left: vec!["same".into(), "old".into()],
//!     right: vec!["same".into(), "new".into()],
//!     hunks: vec![
//!         Hunk { kind: HunkKind::Equal, left: 0..1, right: 0..1 },
//!         Hunk { kind: HunkKind::Replace, left: 1..2, right: 1..2 },
//!     ],
//! })));
//! let diff = compare.result().and_then(Result::ok);
//! assert!(matches!(diff.as_deref(), Some(Diff::Text(d)) if d.changed_lines() == 1));
//! ```

use std::fmt;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use types::Location;

/// What an action opens in a tab.
#[derive(Clone)]
pub enum View {
    /// Two files or two folders compared.
    Compare(Arc<Compare>),
    /// A terminal session.
    Console(Arc<dyn Console>),
}

impl fmt::Debug for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compare(compare) => f.debug_tuple("Compare").field(&compare.title).finish(),
            Self::Console(console) => f.debug_tuple("Console").field(&console.title()).finish(),
        }
    }
}

impl View {
    /// The tab's title.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::Compare(compare) => compare.title.clone(),
            Self::Console(console) => console.title(),
        }
    }
}

/// A comparison: opened empty, filled once by the job that computes it.
pub struct Compare {
    /// The tab's title.
    pub title: String,
    /// What the left side is: a file name or a folder path.
    pub left: String,
    /// What the right side is.
    pub right: String,
    result: Mutex<Option<Result<Arc<Diff>, String>>>,
}

impl fmt::Debug for Compare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Compare")
            .field("title", &self.title)
            .field("left", &self.left)
            .field("right", &self.right)
            .field("result", &self.result().is_some())
            .finish_non_exhaustive()
    }
}

impl Compare {
    /// A comparison with no result yet.
    #[must_use]
    pub fn pending(
        title: impl Into<String>,
        left: impl Into<String>,
        right: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            title: title.into(),
            left: left.into(),
            right: right.into(),
            result: Mutex::new(None),
        })
    }

    /// Stores the result; the UI shows it on its next frame.
    pub fn finish(&self, result: Result<Diff, types::Error>) {
        *self.result.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(result.map(Arc::new).map_err(|e| e.to_string()));
    }

    /// The result, once the job has finished: the diff, or why there is
    /// none.
    #[must_use]
    pub fn result(&self) -> Option<Result<Arc<Diff>, String>> {
        self.result
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What a comparison found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diff {
    /// Two text files, line by line.
    Text(TextDiff),
    /// Two folders, entry by entry.
    Folders(FolderDiff),
    /// Two files that are not text: only whether their bytes are the same.
    Binary {
        /// Whether the two files are byte for byte the same.
        identical: bool,
    },
}

/// Two texts aligned line by line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextDiff {
    /// The left file's lines.
    pub left: Vec<String>,
    /// The right file's lines.
    pub right: Vec<String>,
    /// The alignment, in order, covering every line of both sides.
    pub hunks: Vec<Hunk>,
}

impl TextDiff {
    /// How many lines differ: the larger side of every hunk that is not
    /// equal.
    #[must_use]
    pub fn changed_lines(&self) -> usize {
        self.hunks
            .iter()
            .filter(|h| h.kind != HunkKind::Equal)
            .map(|h| h.left.len().max(h.right.len()))
            .sum()
    }
}

/// One stretch of a text diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Equal, or how the stretch differs.
    pub kind: HunkKind,
    /// The lines on the left; empty for an insertion.
    pub left: Range<usize>,
    /// The lines on the right; empty for a deletion.
    pub right: Range<usize>,
}

/// How a hunk's two sides relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    /// The same lines on both sides.
    Equal,
    /// Lines on the left only.
    Delete,
    /// Lines on the right only.
    Insert,
    /// Different lines on each side.
    Replace,
}

/// Two folders compared by name, size and modification time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderDiff {
    /// The left folder.
    pub left: Location,
    /// The right folder.
    pub right: Location,
    /// One row per name on either side, sorted by name.
    pub rows: Vec<FolderRow>,
}

/// One name of a folder diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    /// The entry's name.
    pub name: String,
    /// Same on both sides, or how the sides differ.
    pub status: RowStatus,
    /// The left entry, when there is one.
    pub left: Option<Side>,
    /// The right entry, when there is one.
    pub right: Option<Side>,
}

/// How a folder diff's row differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    /// The same name, size and time on both sides.
    Same,
    /// On the right only.
    Added,
    /// On the left only.
    Removed,
    /// On both sides, with another size or time.
    Changed,
}

/// One side of a folder diff's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    /// Where the entry is.
    pub location: Location,
    /// Whether it is a folder.
    pub is_dir: bool,
    /// Its size in bytes, for a file.
    pub size: Option<u64>,
    /// Its modification time.
    pub modified: Option<SystemTime>,
}

/// Rings when a console has new output, so the UI repaints.
pub type ConsoleWaker = Arc<dyn Fn() + Send + Sync>;

/// A terminal session the UI draws: a shell in a pseudo-terminal whose
/// output a small terminal emulator keeps as a screen of cells.
pub trait Console: Send + Sync {
    /// The tab's title: what the shell set, or the folder it started in.
    fn title(&self) -> String;

    /// The screen as it is now, scrolled back `scrolled_back` lines into
    /// the history; 0 is the live screen.
    fn screen(&self, scrolled_back: usize) -> Screen;

    /// Sends bytes to the shell, as typed.
    fn write(&self, bytes: &[u8]);

    /// Tells the session how many columns and rows the tab shows.
    fn resize(&self, cols: u16, rows: u16);

    /// The shell's exit status once it has ended; `None` while it runs.
    fn exit_status(&self) -> Option<i32>;

    /// Sets what to call after every change to the screen.
    fn set_waker(&self, waker: ConsoleWaker);

    /// A counter that grows with every change to the screen, so the UI
    /// knows when to read it again.
    fn generation(&self) -> u64;
}

/// A console's screen: `cols × rows` cells, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
    /// The cells, `rows` rows of `cols` cells each.
    pub cells: Vec<Cell>,
    /// The cursor as (column, row), when it is shown and on the screen.
    pub cursor: Option<(u16, u16)>,
    /// How many lines of history lie above the screen.
    pub scrollback: usize,
    /// Whether the application asked for application cursor keys, which
    /// changes how the arrow keys are encoded.
    pub application_cursor: bool,
}

/// One cell of a console screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The character; a space when empty.
    pub ch: char,
    /// Text colour.
    pub fg: TermColor,
    /// Background colour.
    pub bg: TermColor,
    /// Bold, italic and the rest.
    pub style: CellStyle,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: TermColor::Default,
            bg: TermColor::Default,
            style: CellStyle::empty(),
        }
    }
}

/// A terminal colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TermColor {
    /// The theme's default text or background colour.
    #[default]
    Default,
    /// One of the 256 indexed colours; 0 to 15 are the named ones.
    Indexed(u8),
    /// A true colour.
    Rgb(u8, u8, u8),
}

bitflags::bitflags! {
    /// The attributes of a cell.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct CellStyle: u8 {
        /// Bold.
        const BOLD = 1;
        /// Faint.
        const DIM = 2;
        /// Italic.
        const ITALIC = 4;
        /// Underlined.
        const UNDERLINE = 8;
        /// Colours swapped.
        const INVERSE = 16;
        /// Struck through.
        const STRIKE = 32;
    }
}
