//! A small terminal emulator: the screen of cells a shell's output paints,
//! driven by the `vte` parser.
//!
//! Enough of VT100 through xterm for a shell, an editor and a pager:
//! cursor movement, erasing, inserting and deleting lines and characters,
//! a scroll region, the alternate screen, tab stops, the SGR attributes
//! (bold, dim, italic, underline, inverse, strike; the 16 named colours,
//! the 256-colour cube and true colour), the cursor and the application
//! cursor keys modes, the window title, saved and restored cursors, and a
//! scrollback of the lines that left the top. Every character takes one
//! cell. The answers a program asks for (the cursor position, the device
//! attributes) are queued in [`Screen::take_responses`] for the session to
//! send back.

use std::collections::VecDeque;

use ext::{Cell, CellStyle, TermColor};

/// Lines of history kept above the screen.
pub const SCROLLBACK: usize = 10_000;

/// A cursor position and the attributes to save with it (DECSC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Saved {
    col: usize,
    row: usize,
    attrs: Cell,
}

/// The screen of a console.
// The modes are the terminal's switches, one flag each.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Screen {
    cols: usize,
    rows: usize,
    /// The visible cells, `rows × cols`.
    grid: Vec<Cell>,
    /// The primary screen while the alternate one is shown.
    primary: Option<(Vec<Cell>, Saved)>,
    /// The lines that left the top of the primary screen, oldest first.
    scrollback: VecDeque<Vec<Cell>>,
    col: usize,
    row: usize,
    /// The cursor sits past the last column after a print there; the next
    /// print wraps first.
    wrap_pending: bool,
    attrs: Cell,
    saved: Option<Saved>,
    scroll_top: usize,
    scroll_bottom: usize,
    tabs: Vec<bool>,
    cursor_visible: bool,
    application_cursor: bool,
    autowrap: bool,
    origin_mode: bool,
    insert_mode: bool,
    title: Option<String>,
    responses: Vec<u8>,
    generation: u64,
}

impl Screen {
    /// A blank screen of `cols × rows`.
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = usize::from(cols.max(1));
        let rows = usize::from(rows.max(1));
        Self {
            cols,
            rows,
            grid: vec![Cell::default(); cols * rows],
            primary: None,
            scrollback: VecDeque::new(),
            col: 0,
            row: 0,
            wrap_pending: false,
            attrs: Cell::default(),
            saved: None,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            tabs: (0..cols).map(|c| c % 8 == 0).collect(),
            cursor_visible: true,
            application_cursor: false,
            autowrap: true,
            origin_mode: false,
            insert_mode: false,
            title: None,
            responses: Vec::new(),
            generation: 0,
        }
    }

    /// Columns.
    #[must_use]
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Rows.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The title the program set (OSC 0 or 2), if any.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether the application asked for application cursor keys.
    #[must_use]
    pub fn application_cursor(&self) -> bool {
        self.application_cursor
    }

    /// Grows with every change.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The bytes a program asked for (cursor position, device attributes),
    /// to send to it; empties the queue.
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// How many lines of history lie above the screen.
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// The row's text, trimmed on the right.
    #[must_use]
    pub fn line(&self, row: usize) -> String {
        let start = row * self.cols;
        let text: String = self.grid[start..start + self.cols]
            .iter()
            .map(|c| c.ch)
            .collect();
        text.trim_end().to_owned()
    }

    /// Every row's text, joined with newlines, trailing blank rows dropped.
    #[must_use]
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = (0..self.rows).map(|r| self.line(r)).collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// The screen as the UI draws it, scrolled back `scrolled_back` lines
    /// into the history (clamped to what there is).
    #[must_use]
    pub fn snapshot(&self, scrolled_back: usize) -> ext::Screen {
        let back = scrolled_back.min(self.scrollback.len());
        let mut cells = Vec::with_capacity(self.cols * self.rows);
        let history_start = self.scrollback.len() - back;
        let mut taken = 0;
        for line in self.scrollback.iter().skip(history_start) {
            if taken == self.rows {
                break;
            }
            cells.extend(line.iter().copied());
            cells.resize((taken + 1) * self.cols, Cell::default());
            taken += 1;
        }
        let from_grid = self.rows - taken;
        cells.extend_from_slice(&self.grid[..from_grid * self.cols]);
        let cursor = (back == 0 && self.cursor_visible).then(|| {
            (
                u16::try_from(self.col.min(self.cols - 1)).unwrap_or(u16::MAX),
                u16::try_from(self.row).unwrap_or(u16::MAX),
            )
        });
        ext::Screen {
            cols: u16::try_from(self.cols).unwrap_or(u16::MAX),
            rows: u16::try_from(self.rows).unwrap_or(u16::MAX),
            cells,
            cursor,
            scrollback: self.scrollback.len(),
            application_cursor: self.application_cursor,
        }
    }

    /// Resizes the screen, keeping the text from the top left; rows that
    /// no longer fit leave for the history, new rows are blank.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = usize::from(cols.max(1));
        let rows = usize::from(rows.max(1));
        if cols == self.cols && rows == self.rows {
            return;
        }
        let mut lines: Vec<Vec<Cell>> = (0..self.rows)
            .map(|r| {
                let mut line = self.grid[r * self.cols..(r + 1) * self.cols].to_vec();
                line.resize(cols, Cell::default());
                line
            })
            .collect();
        // Rows below the cursor that are blank go first when shrinking.
        while lines.len() > rows {
            let last_used = lines
                .iter()
                .rposition(|l| l.iter().any(|c| c.ch != ' '))
                .unwrap_or(0);
            if lines.len() - 1 > last_used.max(self.row) {
                lines.pop();
            } else {
                let top = lines.remove(0);
                if self.primary.is_none() {
                    self.push_history(top);
                }
                self.row = self.row.saturating_sub(1);
            }
        }
        while lines.len() < rows {
            lines.push(vec![Cell::default(); cols]);
        }
        self.grid = lines.into_iter().flatten().collect();
        self.cols = cols;
        self.rows = rows;
        self.col = self.col.min(cols - 1);
        self.row = self.row.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.tabs = (0..cols).map(|c| c % 8 == 0).collect();
        self.wrap_pending = false;
        self.touch();
    }

    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn push_history(&mut self, line: Vec<Cell>) {
        if self.scrollback.len() == SCROLLBACK {
            self.scrollback.pop_front();
        }
        self.scrollback.push_back(line);
    }

    fn blank(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: TermColor::Default,
            bg: self.attrs.bg,
            style: CellStyle::empty(),
        }
    }

    fn index(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    /// Scrolls the region `top..=bottom` up by `n`, blanking the bottom;
    /// lines leaving the top of the whole primary screen go to history.
    fn scroll_up(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        for _ in 0..n {
            let (from, to, dest) = (
                self.index(0, top + 1),
                self.index(0, bottom + 1),
                self.index(0, top),
            );
            let line = self.grid[dest..dest + self.cols].to_vec();
            if top == 0 && self.primary.is_none() {
                self.push_history(line);
            }
            self.grid.copy_within(from..to, dest);
            let blank = self.blank();
            let start = self.index(0, bottom);
            self.grid[start..start + self.cols].fill(blank);
        }
    }

    /// Scrolls the region down by `n`, blanking the top.
    fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        for _ in 0..n {
            let (from, to, dest) = (
                self.index(0, top),
                self.index(0, bottom),
                self.index(0, top + 1),
            );
            self.grid.copy_within(from..to, dest);
            let blank = self.blank();
            let start = self.index(0, top);
            self.grid[start..start + self.cols].fill(blank);
        }
    }

    fn linefeed(&mut self) {
        if self.row == self.scroll_bottom {
            self.scroll_up(self.scroll_top, self.scroll_bottom, 1);
        } else if self.row < self.rows - 1 {
            self.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.row == self.scroll_top {
            self.scroll_down(self.scroll_top, self.scroll_bottom, 1);
        } else if self.row > 0 {
            self.row -= 1;
        }
    }

    fn put(&mut self, c: char) {
        if self.wrap_pending {
            if self.autowrap {
                self.col = 0;
                self.linefeed();
            }
            self.wrap_pending = false;
        }
        if self.insert_mode {
            let row_start = self.index(0, self.row);
            let at = self.index(self.col, self.row);
            self.grid.copy_within(at..row_start + self.cols - 1, at + 1);
        }
        let index = self.index(self.col, self.row);
        self.grid[index] = Cell {
            ch: c,
            ..self.attrs
        };
        if self.col + 1 < self.cols {
            self.col += 1;
        } else {
            self.wrap_pending = true;
        }
    }

    fn set_cursor(&mut self, col: usize, row: usize) {
        let (top, bottom) = if self.origin_mode {
            (self.scroll_top, self.scroll_bottom)
        } else {
            (0, self.rows - 1)
        };
        self.row = (row + top).clamp(top, bottom);
        self.col = col.min(self.cols - 1);
        self.wrap_pending = false;
    }

    fn erase_display(&mut self, mode: u16) {
        let blank = self.blank();
        match mode {
            0 => {
                let from = self.index(self.col, self.row);
                self.grid[from..].fill(blank);
            }
            1 => {
                let to = self.index(self.col, self.row);
                self.grid[..=to].fill(blank);
            }
            2 | 3 => {
                self.grid.fill(blank);
                if mode == 3 {
                    self.scrollback.clear();
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let blank = self.blank();
        let start = self.index(0, self.row);
        match mode {
            0 => self.grid[start + self.col..start + self.cols].fill(blank),
            1 => self.grid[start..=start + self.col].fill(blank),
            2 => self.grid[start..start + self.cols].fill(blank),
            _ => {}
        }
    }

    fn insert_lines(&mut self, n: usize) {
        if self.row < self.scroll_top || self.row > self.scroll_bottom {
            return;
        }
        self.scroll_down(self.row, self.scroll_bottom, n);
    }

    fn delete_lines(&mut self, n: usize) {
        if self.row < self.scroll_top || self.row > self.scroll_bottom {
            return;
        }
        let bottom = self.scroll_bottom;
        let row = self.row;
        // Lines deleted inside a region never reach the history.
        let n = n.min(bottom + 1 - row);
        for _ in 0..n {
            let (from, to, dest) = (
                self.index(0, row + 1),
                self.index(0, bottom + 1),
                self.index(0, row),
            );
            self.grid.copy_within(from..to, dest);
            let blank = self.blank();
            let start = self.index(0, bottom);
            self.grid[start..start + self.cols].fill(blank);
        }
    }

    fn insert_chars(&mut self, n: usize) {
        let start = self.index(0, self.row);
        let at = start + self.col;
        let end = start + self.cols;
        let n = n.min(end - at);
        self.grid.copy_within(at..end - n, at + n);
        let blank = self.blank();
        self.grid[at..at + n].fill(blank);
    }

    fn delete_chars(&mut self, n: usize) {
        let start = self.index(0, self.row);
        let at = start + self.col;
        let end = start + self.cols;
        let n = n.min(end - at);
        self.grid.copy_within(at + n..end, at);
        let blank = self.blank();
        self.grid[end - n..end].fill(blank);
    }

    fn erase_chars(&mut self, n: usize) {
        let start = self.index(0, self.row);
        let at = start + self.col;
        let end = (at + n).min(start + self.cols);
        let blank = self.blank();
        self.grid[at..end].fill(blank);
    }

    fn tab(&mut self) {
        let mut col = self.col + 1;
        while col < self.cols && !self.tabs[col] {
            col += 1;
        }
        self.col = col.min(self.cols - 1);
        self.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved = Some(Saved {
            col: self.col,
            row: self.row,
            attrs: self.attrs,
        });
    }

    fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved {
            self.col = saved.col.min(self.cols - 1);
            self.row = saved.row.min(self.rows - 1);
            self.attrs = saved.attrs;
        } else {
            self.col = 0;
            self.row = 0;
        }
        self.wrap_pending = false;
    }

    fn enter_alternate(&mut self, save_cursor: bool, clear: bool) {
        if self.primary.is_some() {
            return;
        }
        let saved = Saved {
            col: self.col,
            row: self.row,
            attrs: self.attrs,
        };
        if save_cursor {
            self.saved = Some(saved);
        }
        let grid = std::mem::replace(&mut self.grid, vec![Cell::default(); self.cols * self.rows]);
        self.primary = Some((grid, saved));
        if clear {
            let blank = self.blank();
            self.grid.fill(blank);
        }
    }

    fn leave_alternate(&mut self, restore_cursor: bool) {
        let Some((grid, saved)) = self.primary.take() else {
            return;
        };
        self.grid = grid;
        if restore_cursor {
            self.col = saved.col.min(self.cols - 1);
            self.row = saved.row.min(self.rows - 1);
            self.attrs = saved.attrs;
        }
        self.wrap_pending = false;
    }

    fn set_mode(&mut self, private: bool, mode: u16, on: bool) {
        match (private, mode) {
            (true, 1) => self.application_cursor = on,
            (true, 6) => {
                self.origin_mode = on;
                self.set_cursor(0, 0);
            }
            (true, 7) => self.autowrap = on,
            (true, 25) => self.cursor_visible = on,
            (true, 47) => {
                if on {
                    self.enter_alternate(false, false);
                } else {
                    self.leave_alternate(false);
                }
            }
            (true, 1047) => {
                if on {
                    self.enter_alternate(false, true);
                } else {
                    self.leave_alternate(false);
                }
            }
            (true, 1049) => {
                if on {
                    self.enter_alternate(true, true);
                } else {
                    self.leave_alternate(true);
                }
            }
            (false, 4) => self.insert_mode = on,
            _ => {}
        }
    }

    /// Applies one SGR parameter list.
    fn sgr(&mut self, params: &vte::Params) {
        if params.is_empty() {
            self.attrs = Cell::default();
            return;
        }
        let mut iter = params.iter();
        while let Some(param) = iter.next() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                0 => self.attrs = Cell::default(),
                1 => self.attrs.style |= CellStyle::BOLD,
                2 => self.attrs.style |= CellStyle::DIM,
                3 => self.attrs.style |= CellStyle::ITALIC,
                4 => self.attrs.style |= CellStyle::UNDERLINE,
                7 => self.attrs.style |= CellStyle::INVERSE,
                9 => self.attrs.style |= CellStyle::STRIKE,
                22 => self.attrs.style &= !(CellStyle::BOLD | CellStyle::DIM),
                23 => self.attrs.style &= !CellStyle::ITALIC,
                24 => self.attrs.style &= !CellStyle::UNDERLINE,
                27 => self.attrs.style &= !CellStyle::INVERSE,
                29 => self.attrs.style &= !CellStyle::STRIKE,
                30..=37 => self.attrs.fg = TermColor::Indexed(u8::try_from(code - 30).unwrap_or(0)),
                39 => self.attrs.fg = TermColor::Default,
                40..=47 => self.attrs.bg = TermColor::Indexed(u8::try_from(code - 40).unwrap_or(0)),
                49 => self.attrs.bg = TermColor::Default,
                90..=97 => {
                    self.attrs.fg = TermColor::Indexed(u8::try_from(code - 90 + 8).unwrap_or(8));
                }
                100..=107 => {
                    self.attrs.bg = TermColor::Indexed(u8::try_from(code - 100 + 8).unwrap_or(8));
                }
                38 | 48 => {
                    // `38;5;n`, `38;2;r;g;b`, or the colon form in one
                    // parameter's subparameters.
                    let mut rest: Vec<u16> = param[1..].to_vec();
                    // The kind (5 or 2) may itself be the next parameter.
                    if rest.is_empty()
                        && let Some(p) = iter.next()
                    {
                        rest.push(p.first().copied().unwrap_or(0));
                    }
                    let needed = match rest.first().copied() {
                        Some(5) => 2,
                        Some(2) => 4,
                        _ => 0,
                    };
                    while rest.len() < needed {
                        match iter.next() {
                            Some(p) => rest.push(p.first().copied().unwrap_or(0)),
                            None => break,
                        }
                    }
                    let color = match rest.first().copied() {
                        Some(5) => rest
                            .get(1)
                            .map(|n| TermColor::Indexed(u8::try_from(*n).unwrap_or(255))),
                        Some(2) => match rest.get(1..4) {
                            Some([r, g, b]) => Some(TermColor::Rgb(
                                u8::try_from(*r).unwrap_or(255),
                                u8::try_from(*g).unwrap_or(255),
                                u8::try_from(*b).unwrap_or(255),
                            )),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(color) = color {
                        if code == 38 {
                            self.attrs.fg = color;
                        } else {
                            self.attrs.bg = color;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// The first parameter, or `default` when absent or zero.
fn param(params: &vte::Params, index: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(index)
        .and_then(|p| p.first().copied())
        .filter(|v| *v != 0)
        .unwrap_or(default)
}

// One arm per control sequence.
#[allow(clippy::too_many_lines)]
impl vte::Perform for Screen {
    fn print(&mut self, c: char) {
        self.put(c);
        self.touch();
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.col = self.col.saturating_sub(1);
                self.wrap_pending = false;
            }
            0x09 => self.tab(),
            0x0A..=0x0C => self.linefeed(),
            0x0D => {
                self.col = 0;
                self.wrap_pending = false;
            }
            _ => {}
        }
        self.touch();
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if let [kind, title, ..] = params
            && matches!(*kind, b"0" | b"2")
        {
            self.title = Some(String::from_utf8_lossy(title).into_owned());
            self.touch();
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let private = intermediates.first() == Some(&b'?');
        let n = usize::from(param(params, 0, 1));
        match action {
            'A' => {
                self.row = self
                    .row
                    .saturating_sub(n)
                    .max(if self.row >= self.scroll_top {
                        self.scroll_top
                    } else {
                        0
                    });
                self.wrap_pending = false;
            }
            'B' | 'e' => {
                let limit = if self.row <= self.scroll_bottom {
                    self.scroll_bottom
                } else {
                    self.rows - 1
                };
                self.row = (self.row + n).min(limit);
                self.wrap_pending = false;
            }
            'C' | 'a' => {
                self.col = (self.col + n).min(self.cols - 1);
                self.wrap_pending = false;
            }
            'D' => {
                self.col = self.col.saturating_sub(n);
                self.wrap_pending = false;
            }
            'E' => {
                self.row = (self.row + n).min(self.rows - 1);
                self.col = 0;
                self.wrap_pending = false;
            }
            'F' => {
                self.row = self.row.saturating_sub(n);
                self.col = 0;
                self.wrap_pending = false;
            }
            'G' | '`' => {
                self.col = (n - 1).min(self.cols - 1);
                self.wrap_pending = false;
            }
            'H' | 'f' => {
                let row = usize::from(param(params, 0, 1)) - 1;
                let col = usize::from(param(params, 1, 1)) - 1;
                self.set_cursor(col, row);
            }
            'd' => {
                self.row = (n - 1).min(self.rows - 1);
                self.wrap_pending = false;
            }
            'J' => self.erase_display(param(params, 0, 0)),
            'K' => self.erase_line(param(params, 0, 0)),
            'L' => self.insert_lines(n),
            'M' => self.delete_lines(n),
            'P' => self.delete_chars(n),
            '@' => self.insert_chars(n),
            'X' => self.erase_chars(n),
            'S' => self.scroll_up(self.scroll_top, self.scroll_bottom, n),
            'T' => self.scroll_down(self.scroll_top, self.scroll_bottom, n),
            'g' => {
                if param(params, 0, 0) == 3 {
                    self.tabs.fill(false);
                } else {
                    self.tabs[self.col] = false;
                }
            }
            'r' => {
                let top = usize::from(param(params, 0, 1)) - 1;
                let bottom = usize::from(param(
                    params,
                    1,
                    u16::try_from(self.rows).unwrap_or(u16::MAX),
                )) - 1;
                let bottom = bottom.min(self.rows - 1);
                if top < bottom {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                    self.set_cursor(0, 0);
                }
            }
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'm' => self.sgr(params),
            'h' | 'l' => {
                for p in params {
                    if let Some(mode) = p.first() {
                        self.set_mode(private, *mode, action == 'h');
                    }
                }
            }
            'n' => {
                if param(params, 0, 0) == 6 {
                    let (row, col) = if self.origin_mode {
                        (self.row - self.scroll_top + 1, self.col + 1)
                    } else {
                        (self.row + 1, self.col + 1)
                    };
                    self.responses
                        .extend_from_slice(format!("\x1b[{row};{col}R").as_bytes());
                } else if param(params, 0, 0) == 5 {
                    self.responses.extend_from_slice(b"\x1b[0n");
                }
            }
            // Device attributes: a VT220 with no options.
            'c' if !private => self.responses.extend_from_slice(b"\x1b[?62;22c"),
            _ => {}
        }
        self.touch();
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            // Character set designations and the like.
            return;
        }
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.linefeed(),
            b'E' => {
                self.col = 0;
                self.linefeed();
            }
            b'M' => self.reverse_index(),
            b'H' => self.tabs[self.col] = true,
            b'c' => {
                let (cols, rows) = (self.cols, self.rows);
                let scrollback = std::mem::take(&mut self.scrollback);
                let generation = self.generation;
                *self = Self::new(
                    u16::try_from(cols).unwrap_or(u16::MAX),
                    u16::try_from(rows).unwrap_or(u16::MAX),
                );
                self.scrollback = scrollback;
                self.generation = generation;
            }
            _ => {}
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(screen: &mut Screen, bytes: &[u8]) {
        let mut parser = vte::Parser::new();
        parser.advance(screen, bytes);
    }

    fn fed(cols: u16, rows: u16, bytes: &[u8]) -> Screen {
        let mut screen = Screen::new(cols, rows);
        feed(&mut screen, bytes);
        screen
    }

    #[test]
    fn text_prints_wraps_and_scrolls_into_history() {
        let screen = fed(10, 3, b"hello\r\nworld");
        assert_eq!(screen.text(), "hello\nworld");
        let snapshot = screen.snapshot(0);
        assert_eq!(snapshot.cursor, Some((5, 1)));
        // Wrapping at the margin, then scrolling the first line away.
        let screen = fed(5, 2, b"abcdefg\r\nxyz\r\nlast");
        assert_eq!(screen.text(), "xyz\nlast");
        assert_eq!(screen.scrollback_len(), 2);
        let back = screen.snapshot(2);
        let first: String = back.cells[..5].iter().map(|c| c.ch).collect();
        assert_eq!(first, "abcde");
        assert_eq!(back.cursor, None, "no cursor while scrolled back");
    }

    #[test]
    fn cursor_movement_erasing_and_editing() {
        let mut screen = fed(10, 4, b"line1\r\nline2\r\nline3\r\nline4");
        feed(&mut screen, b"\x1b[2;3H");
        assert_eq!(screen.snapshot(0).cursor, Some((2, 1)));
        feed(&mut screen, b"\x1b[K");
        assert_eq!(screen.line(1), "li");
        // Erase above includes the cursor's own cell.
        feed(&mut screen, b"\x1b[1;1H\x1b[1J");
        assert_eq!(screen.line(0), " ine1");
        feed(&mut screen, b"\x1b[3;1H\x1b[M");
        assert_eq!(screen.line(2), "line4");
        assert_eq!(screen.line(3), "");
        feed(&mut screen, b"\x1b[1;1H\x1b[L");
        assert_eq!(screen.line(0), "");
        assert_eq!(screen.line(1), " ine1");
        feed(&mut screen, b"\x1b[2;1H\x1b[2@");
        assert_eq!(screen.line(1), "   ine1");
        feed(&mut screen, b"\x1b[2P");
        assert_eq!(screen.line(1), " ine1");
        feed(&mut screen, b"\x1b[2X");
        assert_eq!(screen.line(1), "  ne1");
        // Erase all leaves the cursor where it was.
        feed(&mut screen, b"\x1b[2J");
        assert_eq!(screen.text(), "");
        assert_eq!(screen.snapshot(0).cursor, Some((0, 1)));
        feed(&mut screen, b"\x1b[Ha\tb");
        assert_eq!(screen.snapshot(0).cursor, Some((9, 0)));
        assert_eq!(screen.line(0), "a       b");
    }

    #[test]
    fn scroll_region_and_alternate_screen() {
        let mut screen = fed(10, 4, b"a\r\nb\r\nc\r\nd");
        feed(&mut screen, b"\x1b[2;3r\x1b[3;1H\n");
        assert_eq!(screen.line(0), "a");
        assert_eq!(screen.line(1), "c");
        assert_eq!(screen.line(2), "");
        assert_eq!(screen.line(3), "d");
        assert_eq!(
            screen.scrollback_len(),
            0,
            "a region scroll keeps no history"
        );
        feed(&mut screen, b"\x1bM\x1bM");
        assert_eq!(screen.line(1), "");
        assert_eq!(screen.line(2), "c");
        feed(&mut screen, b"\x1b[r");
        feed(&mut screen, b"\x1b[?1049h\x1b[2JALT");
        assert_eq!(screen.text(), "ALT");
        feed(&mut screen, b"\x1b[?1049l");
        assert_eq!(screen.line(0), "a");
        assert_eq!(screen.line(2), "c");
    }

    #[test]
    fn attributes_and_colours_apply_to_cells() {
        let screen = fed(
            20,
            2,
            b"\x1b[1;31mR\x1b[0;44mB\x1b[38;5;200mP\x1b[48;2;1;2;3mT\x1b[7mI\x1b[mN",
        );
        let cells = &screen.snapshot(0).cells;
        assert_eq!(cells[0].fg, TermColor::Indexed(1));
        assert!(cells[0].style.contains(CellStyle::BOLD));
        assert_eq!(cells[1].bg, TermColor::Indexed(4));
        assert!(!cells[1].style.contains(CellStyle::BOLD));
        assert_eq!(cells[2].fg, TermColor::Indexed(200));
        assert_eq!(cells[3].bg, TermColor::Rgb(1, 2, 3));
        assert!(cells[4].style.contains(CellStyle::INVERSE));
        assert_eq!(
            cells[5],
            Cell {
                ch: 'N',
                ..Cell::default()
            }
        );
        let colon = fed(5, 1, b"\x1b[38:2:9:8:7mx");
        assert_eq!(colon.snapshot(0).cells[0].fg, TermColor::Rgb(9, 8, 7));
    }

    #[test]
    fn modes_titles_responses_and_resizing() {
        let mut screen = fed(10, 3, b"\x1b]0;my title\x07\x1b[?1h\x1b[?25l");
        assert_eq!(screen.title(), Some("my title"));
        assert!(screen.application_cursor());
        assert_eq!(screen.snapshot(0).cursor, None);
        feed(&mut screen, b"\x1b[?25h\x1b[2;4H\x1b[6n\x1b[c");
        assert_eq!(screen.take_responses(), b"\x1b[2;4R\x1b[?62;22c");
        assert_eq!(screen.take_responses(), b"");
        let before = screen.generation();
        feed(&mut screen, b"x");
        assert!(screen.generation() > before);

        let mut screen = fed(10, 4, b"one\r\ntwo\r\nthree\r\nfour");
        screen.resize(6, 2);
        assert_eq!(screen.rows(), 2);
        assert_eq!(screen.cols(), 6);
        assert_eq!(screen.text(), "three\nfour");
        assert_eq!(screen.scrollback_len(), 2);
        screen.resize(12, 5);
        assert_eq!(screen.text(), "three\nfour");
        assert_eq!(screen.snapshot(0).cells.len(), 60);
        // Autowrap off: the last column takes every further character.
        let screen = fed(3, 1, b"\x1b[?7labcdef");
        assert_eq!(screen.text(), "abf");
        // Insert mode shifts the line right.
        let screen = fed(6, 1, b"ace\r\x1b[4hb");
        assert_eq!(screen.text(), "bace");
    }
}
