//! The console tab: a terminal session an action opened
//! ([`ext::Console`]), drawn cell by cell in the monospace font, typed into
//! while it has the keyboard.
//!
//! The session owns the shell and the terminal emulation; this module only
//! draws the screen it reports, tells it how many cells fit, and turns
//! egui's key and text events into the bytes a terminal sends. Clicking
//! the console gives it the keyboard, and while it has it every key goes
//! to the shell — Tab, Escape and the arrows included, so the shell's
//! shortcuts rest until the mouse moves on. The wheel and Shift with
//! `PageUp` and `PageDown` scroll back into the history; typing returns to
//! the live screen.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, pos2, vec2};
use ext::{Cell, CellStyle, Console, TermColor};

/// What the tab keeps between frames.
#[derive(Default)]
pub struct ConsoleUi {
    /// Lines scrolled back into the history; 0 is the live screen.
    pub scroll_back: usize,
    /// The size last told to the session.
    size: Option<(u16, u16)>,
    /// Take the keyboard on the next frame (a freshly opened console).
    pub focus_next: bool,
}

/// The 16 named colours, as a dark terminal paints them.
const NAMED: [Color32; 16] = [
    Color32::from_rgb(0x2e, 0x34, 0x36),
    Color32::from_rgb(0xcc, 0x00, 0x00),
    Color32::from_rgb(0x4e, 0x9a, 0x06),
    Color32::from_rgb(0xc4, 0xa0, 0x00),
    Color32::from_rgb(0x34, 0x65, 0xa4),
    Color32::from_rgb(0x75, 0x50, 0x7b),
    Color32::from_rgb(0x06, 0x98, 0x9a),
    Color32::from_rgb(0xd3, 0xd7, 0xcf),
    Color32::from_rgb(0x55, 0x57, 0x53),
    Color32::from_rgb(0xef, 0x29, 0x29),
    Color32::from_rgb(0x8a, 0xe2, 0x34),
    Color32::from_rgb(0xfc, 0xe9, 0x4f),
    Color32::from_rgb(0x72, 0x9f, 0xcf),
    Color32::from_rgb(0xad, 0x7f, 0xa8),
    Color32::from_rgb(0x34, 0xe2, 0xe2),
    Color32::from_rgb(0xee, 0xee, 0xec),
];

/// A terminal colour as a pixel colour; `default` for the theme's own.
fn color_of(color: TermColor, default: Color32) -> Color32 {
    match color {
        TermColor::Default => default,
        TermColor::Indexed(i) => indexed(i),
        TermColor::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// One of the 256 indexed colours: the 16 named, the 6×6×6 cube, the
/// greys.
fn indexed(index: u8) -> Color32 {
    match index {
        0..=15 => NAMED[usize::from(index)],
        16..=231 => {
            let i = index - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let grey = 8 + (index - 232) * 10;
            Color32::from_rgb(grey, grey, grey)
        }
    }
}

/// The colours a cell paints with, inverse applied.
fn colors(cell: &Cell, fg_default: Color32, bg_default: Color32) -> (Color32, Color32) {
    let mut fg = color_of(cell.fg, fg_default);
    let mut bg = color_of(cell.bg, bg_default);
    if cell.style.contains(CellStyle::BOLD)
        && let TermColor::Indexed(i @ 0..=7) = cell.fg
    {
        fg = NAMED[usize::from(i) + 8];
    }
    if cell.style.contains(CellStyle::DIM) {
        fg = fg.gamma_multiply(0.6);
    }
    if cell.style.contains(CellStyle::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

/// The bytes a key press sends to the shell, if it sends any: the control
/// characters, the escape sequences of the cursor and function keys
/// (`application` picks the application form for the arrows), and the
/// control-letter combinations egui gives no text event for.
#[must_use]
pub fn encode_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application: bool,
) -> Option<Vec<u8>> {
    use egui::Key as K;
    let ctrl = modifiers.ctrl || modifiers.command;
    let alt = modifiers.alt;
    let csi = |rest: &str| format!("\x1b[{rest}").into_bytes();
    let arrow = |letter: char| {
        if modifiers.shift || ctrl || alt {
            let modifier = 1 + u8::from(modifiers.shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
            format!("\x1b[1;{modifier}{letter}").into_bytes()
        } else if application {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| {
        if modifiers.shift || ctrl || alt {
            let modifier = 1 + u8::from(modifiers.shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let bytes = match key {
        K::Enter => vec![b'\r'],
        K::Backspace => {
            if ctrl {
                vec![0x08]
            } else {
                vec![0x7f]
            }
        }
        K::Tab => {
            if modifiers.shift {
                csi("Z")
            } else {
                vec![b'\t']
            }
        }
        K::Escape => vec![0x1b],
        K::ArrowUp => arrow('A'),
        K::ArrowDown => arrow('B'),
        K::ArrowRight => arrow('C'),
        K::ArrowLeft => arrow('D'),
        K::Home => arrow('H'),
        K::End => arrow('F'),
        K::Insert => tilde(2),
        K::Delete => tilde(3),
        K::PageUp => tilde(5),
        K::PageDown => tilde(6),
        K::F1 => b"\x1bOP".to_vec(),
        K::F2 => b"\x1bOQ".to_vec(),
        K::F3 => b"\x1bOR".to_vec(),
        K::F4 => b"\x1bOS".to_vec(),
        K::F5 => tilde(15),
        K::F6 => tilde(17),
        K::F7 => tilde(18),
        K::F8 => tilde(19),
        K::F9 => tilde(20),
        K::F10 => tilde(21),
        K::F11 => tilde(23),
        K::F12 => tilde(24),
        K::Space if ctrl => vec![0],
        _ if ctrl => {
            // Control-letter: the letter's byte with the high bits cleared.
            let name = key.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => {
                    vec![c.to_ascii_uppercase() as u8 & 0x1f]
                }
                (Some('['), None) => vec![0x1b],
                (Some('\\'), None) => vec![0x1c],
                (Some(']'), None) => vec![0x1d],
                _ => return None,
            }
        }
        _ => return None,
    };
    if alt
        && !matches!(
            key,
            K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight
        )
    {
        let mut out = vec![0x1b];
        out.extend(bytes);
        return Some(out);
    }
    Some(bytes)
}

/// Draws the console into `ui` and feeds it the keyboard while it has it.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut ConsoleUi,
    console: &Arc<dyn Console>,
    tab: model::TabId,
) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let (cell_w, cell_h) =
        ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    let cell_w = cell_w.max(1.0);
    let cell_h = cell_h.max(1.0);
    let available = ui.available_rect_before_wrap();
    let rect = available.shrink(4.0);
    // Columns and rows that fit; one at least so the shell has a screen.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cols = ((rect.width() / cell_w).floor() as u16).clamp(1, 1000);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rows = ((rect.height() / cell_h).floor() as u16).clamp(1, 1000);
    if state.size != Some((cols, rows)) {
        console.resize(cols, rows);
        state.size = Some((cols, rows));
    }

    let id = egui::Id::new(("console", tab.0));
    let response = ui.interact(available, id, Sense::click_and_drag());
    if response.clicked() || state.focus_next {
        response.request_focus();
        state.focus_next = false;
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }

    // Scrolling back: the wheel over the console, Shift+PageUp and
    // Shift+PageDown.
    let scroll = ui.input(|i| {
        if response.hovered() {
            i.smooth_scroll_delta.y
        } else {
            0.0
        }
    });
    if scroll != 0.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let lines = ((scroll.abs() / cell_h).ceil() as usize).max(1);
        if scroll > 0.0 {
            state.scroll_back = state.scroll_back.saturating_add(lines);
        } else {
            state.scroll_back = state.scroll_back.saturating_sub(lines);
        }
    }

    let screen = console.screen(state.scroll_back);
    state.scroll_back = state.scroll_back.min(screen.scrollback);
    let screen = if state.scroll_back == 0 && screen.scrollback > 0 && scroll == 0.0 {
        screen
    } else {
        console.screen(state.scroll_back)
    };

    if focused {
        let events = ui.input(|i| i.events.clone());
        let pasted = events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut
            )
        });
        let mut typed = false;
        for event in &events {
            match event {
                egui::Event::Text(text) | egui::Event::Paste(text) => {
                    console.write(text.as_bytes());
                    typed = true;
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let ctrl = modifiers.ctrl || modifiers.command;
                    if modifiers.shift && matches!(key, egui::Key::PageUp | egui::Key::PageDown) {
                        let page = usize::from(rows.saturating_sub(1)).max(1);
                        if *key == egui::Key::PageUp {
                            state.scroll_back = state.scroll_back.saturating_add(page);
                        } else {
                            state.scroll_back = state.scroll_back.saturating_sub(page);
                        }
                        continue;
                    }
                    // Ctrl+V and Ctrl+C arrive as Paste and Copy as well;
                    // the paste goes to the shell as text, the copy is kept
                    // as the interrupt it is in a terminal.
                    if pasted && ctrl && *key == egui::Key::V {
                        continue;
                    }
                    if let Some(bytes) = encode_key(*key, *modifiers, screen.application_cursor) {
                        console.write(&bytes);
                        typed = true;
                    }
                }
                _ => {}
            }
        }
        if typed {
            state.scroll_back = 0;
        }
    }

    // The screen: the background, then every row's cells, then the cursor.
    let visuals = ui.visuals();
    let bg_default = if visuals.dark_mode {
        Color32::from_rgb(0x1e, 0x1e, 0x1e)
    } else {
        Color32::from_rgb(0xfa, 0xfa, 0xfa)
    };
    let fg_default = visuals.text_color();
    let painter = ui.painter_at(available);
    painter.rect_filled(available, 0.0, bg_default);
    let origin = rect.left_top();
    let cols_usize = usize::from(screen.cols);
    for row in 0..usize::from(screen.rows) {
        let Some(cells) = screen.cells.get(row * cols_usize..(row + 1) * cols_usize) else {
            break;
        };
        #[allow(clippy::cast_precision_loss)]
        let y = origin.y + row as f32 * cell_h;
        let mut job = egui::text::LayoutJob::default();
        let mut run_start = 0;
        while run_start < cells.len() {
            let first = cells[run_start];
            let (fg, bg) = colors(&first, fg_default, bg_default);
            let mut run_end = run_start + 1;
            while run_end < cells.len()
                && cells[run_end].fg == first.fg
                && cells[run_end].bg == first.bg
                && cells[run_end].style == first.style
            {
                run_end += 1;
            }
            if bg != bg_default {
                #[allow(clippy::cast_precision_loss)]
                let x0 = origin.x + run_start as f32 * cell_w;
                #[allow(clippy::cast_precision_loss)]
                let x1 = origin.x + run_end as f32 * cell_w;
                painter.rect_filled(
                    Rect::from_min_max(pos2(x0, y), pos2(x1, y + cell_h)),
                    0.0,
                    bg,
                );
            }
            let text: String = cells[run_start..run_end]
                .iter()
                .map(|c| if c.ch == '\0' { ' ' } else { c.ch })
                .collect();
            let mut format = egui::TextFormat {
                font_id: font.clone(),
                color: fg,
                ..Default::default()
            };
            if first.style.contains(CellStyle::UNDERLINE) {
                format.underline = egui::Stroke::new(1.0, fg);
            }
            if first.style.contains(CellStyle::STRIKE) {
                format.strikethrough = egui::Stroke::new(1.0, fg);
            }
            if first.style.contains(CellStyle::ITALIC) {
                format.italics = true;
            }
            job.append(&text, 0.0, format);
            run_start = run_end;
        }
        job.wrap.max_width = f32::INFINITY;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        painter.galley(pos2(origin.x, y), galley, fg_default);
    }
    if state.scroll_back == 0
        && let Some((col, row)) = screen.cursor
    {
        let cursor = Rect::from_min_size(
            pos2(
                origin.x + f32::from(col) * cell_w,
                origin.y + f32::from(row) * cell_h,
            ),
            vec2(cell_w, cell_h),
        );
        let color = visuals.strong_text_color();
        if focused {
            painter.rect_filled(cursor, 0.0, color.gamma_multiply(0.7));
            if let Some(cell) = screen
                .cells
                .get(usize::from(row) * cols_usize + usize::from(col))
                && cell.ch != ' '
                && cell.ch != '\0'
            {
                painter.text(
                    cursor.left_top(),
                    egui::Align2::LEFT_TOP,
                    cell.ch,
                    font.clone(),
                    bg_default,
                );
            }
        } else {
            painter.rect_stroke(
                cursor,
                0.0,
                egui::Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
        }
    }
    if state.scroll_back > 0 {
        painter.text(
            available.right_top() + vec2(-8.0, 4.0),
            egui::Align2::RIGHT_TOP,
            format!("{} lines back", state.scroll_back),
            egui::TextStyle::Small.resolve(ui.style()),
            visuals.weak_text_color(),
        );
    }
    if let Some(code) = console.exit_status() {
        let text = format!("[the shell exited with code {code}]");
        painter.text(
            available.left_bottom() + vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            font,
            visuals.weak_text_color(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_encode_as_a_terminal_sends_them() {
        let none = egui::Modifiers::NONE;
        assert_eq!(encode_key(egui::Key::Enter, none, false), Some(vec![b'\r']));
        assert_eq!(
            encode_key(egui::Key::Backspace, none, false),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode_key(egui::Key::ArrowUp, none, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::ArrowUp, none, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::ArrowRight, egui::Modifiers::SHIFT, false),
            Some(b"\x1b[1;2C".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::Delete, none, false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::F5, none, false),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::C, egui::Modifiers::CTRL, false),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(egui::Key::D, egui::Modifiers::CTRL, false),
            Some(vec![0x04])
        );
        assert_eq!(encode_key(egui::Key::A, none, false), None);
        assert_eq!(encode_key(egui::Key::Escape, none, false), Some(vec![0x1b]));
        assert_eq!(
            encode_key(egui::Key::Tab, egui::Modifiers::SHIFT, false),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode_key(egui::Key::Backspace, egui::Modifiers::ALT, false),
            Some(vec![0x1b, 0x7f])
        );
    }

    #[test]
    fn indexed_colours_cover_the_cube_and_the_greys() {
        assert_eq!(indexed(1), NAMED[1]);
        assert_eq!(indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232), Color32::from_rgb(8, 8, 8));
        assert_eq!(indexed(255), Color32::from_rgb(238, 238, 238));
        let bold_red = Cell {
            ch: 'x',
            fg: TermColor::Indexed(1),
            bg: TermColor::Default,
            style: CellStyle::BOLD,
        };
        let (fg, _) = colors(&bold_red, Color32::WHITE, Color32::BLACK);
        assert_eq!(fg, NAMED[9]);
        let inverse = Cell {
            ch: 'x',
            fg: TermColor::Default,
            bg: TermColor::Default,
            style: CellStyle::INVERSE,
        };
        assert_eq!(
            colors(&inverse, Color32::WHITE, Color32::BLACK),
            (Color32::BLACK, Color32::WHITE)
        );
    }
}
