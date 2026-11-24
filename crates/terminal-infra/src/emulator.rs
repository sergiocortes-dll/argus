use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{CursorShape, Processor, Rgb};

use crate::keys::InputModes;
use crate::palette::{Palette, dim};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenCell {
    pub ch: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Second half of a double-width character; nothing to draw.
    pub spacer: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub cols: usize,
    pub rows: usize,
    pub background: [u8; 3],
    /// Row-major, `cols * rows` cells.
    pub cells: Vec<ScreenCell>,
}

#[derive(Default)]
struct Outbox {
    replies: Vec<u8>,
    title: Option<String>,
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Outbox>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let Ok(mut outbox) = self.0.lock() else {
            return;
        };
        match event {
            Event::PtyWrite(text) => outbox.replies.extend(text.into_bytes()),
            Event::Title(title) => outbox.title = Some(title),
            _ => {}
        }
    }
}

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Emulator {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    palette: Palette,
}

impl Emulator {
    pub fn new(cols: usize, rows: usize) -> Self {
        let listener = Listener::default();
        let size = Size {
            cols: cols.max(1),
            rows: rows.max(1),
        };
        Self {
            term: Term::new(Config::default(), &size, listener.clone()),
            parser: Processor::new(),
            listener,
            palette: Palette::default(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Bytes the terminal must send back to the program (e.g. cursor position reports).
    pub fn take_replies(&mut self) -> Vec<u8> {
        self.listener
            .0
            .lock()
            .map(|mut o| std::mem::take(&mut o.replies))
            .unwrap_or_default()
    }

    pub fn take_title(&mut self) -> Option<String> {
        self.listener.0.lock().ok().and_then(|mut o| o.title.take())
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.term.resize(Size {
            cols: cols.max(1),
            rows: rows.max(1),
        });
    }

    pub fn size(&self) -> (usize, usize) {
        (self.term.columns(), self.term.screen_lines())
    }

    pub fn input_modes(&self) -> InputModes {
        InputModes {
            app_cursor: self.term.mode().contains(TermMode::APP_CURSOR),
        }
    }

    pub fn screen(&self) -> Screen {
        let content = self.term.renderable_content();
        let cols = self.term.columns();
        let rows = self.term.screen_lines();
        let colors = content.colors;
        let background = self.palette.background;
        let blank = ScreenCell {
            ch: ' ',
            fg: to_array(self.palette.foreground),
            bg: to_array(background),
            bold: false,
            italic: false,
            underline: false,
            spacer: false,
        };
        let mut cells = vec![blank; cols * rows];
        let offset = content.display_offset as i32;

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset;
            let col = indexed.point.column.0;
            if row < 0 || row as usize >= rows || col >= cols {
                continue;
            }
            let cell = &indexed.cell;
            let flags = cell.flags;
            let mut fg = self.palette.resolve(cell.fg, colors);
            let mut bg = self.palette.resolve(cell.bg, colors);
            if flags.contains(Flags::DIM) {
                fg = dim(fg);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            cells[row as usize * cols + col] = ScreenCell {
                ch: cell.c,
                fg: to_array(fg),
                bg: to_array(bg),
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.contains(Flags::UNDERLINE),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
            };
        }

        let cursor = content.cursor;
        let cursor_row = cursor.point.line.0 + offset;
        let cursor_col = cursor.point.column.0;
        if cursor.shape != CursorShape::Hidden
            && cursor_row >= 0
            && (cursor_row as usize) < rows
            && cursor_col < cols
        {
            let cell = &mut cells[cursor_row as usize * cols + cursor_col];
            let cursor_color = colors[alacritty_terminal::vte::ansi::NamedColor::Cursor]
                .unwrap_or(self.palette.cursor);
            cell.fg = cell.bg;
            cell.bg = to_array(cursor_color);
        }

        Screen {
            cols,
            rows,
            background: to_array(background),
            cells,
        }
    }
}

fn to_array(c: Rgb) -> [u8; 3] {
    [c.r, c.g, c.b]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(screen: &Screen, row: usize) -> String {
        screen.cells[row * screen.cols..(row + 1) * screen.cols]
            .iter()
            .filter(|c| !c.spacer)
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn plain_text_lands_on_the_grid() {
        let mut emu = Emulator::new(20, 3);
        emu.feed(b"hello\r\nworld");
        let screen = emu.screen();
        assert_eq!(row_text(&screen, 0), "hello");
        assert_eq!(row_text(&screen, 1), "world");
    }

    #[test]
    fn sgr_colors_and_bold_are_applied() {
        let mut emu = Emulator::new(10, 1);
        emu.feed(b"\x1b[1;31mR\x1b[0m\x1b[38;2;1;2;3mT");
        let screen = emu.screen();
        assert!(screen.cells[0].bold);
        assert_eq!(screen.cells[0].fg, [0xcc, 0x66, 0x66]);
        assert_eq!(screen.cells[1].fg, [1, 2, 3]);
        assert!(!screen.cells[1].bold);
    }

    #[test]
    fn cursor_position_request_produces_a_reply() {
        let mut emu = Emulator::new(10, 5);
        emu.feed(b"ab\x1b[6n");
        assert_eq!(emu.take_replies(), b"\x1b[1;3R");
    }

    #[test]
    fn cursor_cell_is_highlighted() {
        let mut emu = Emulator::new(10, 2);
        emu.feed(b"x");
        let screen = emu.screen();
        let palette = Palette::default();
        assert_eq!(screen.cells[1].bg, to_array(palette.cursor));
    }

    #[test]
    fn resize_changes_grid_size() {
        let mut emu = Emulator::new(10, 2);
        emu.resize(30, 7);
        assert_eq!(emu.size(), (30, 7));
        let screen = emu.screen();
        assert_eq!(screen.cells.len(), 30 * 7);
    }
}
