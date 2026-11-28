use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color as TermColor, CursorShape as TermShape, NamedColor, Processor};
use nixa_protocol::{Cell, Color, Cursor, CursorShape, Modes, Snapshot, TermId, flags};
use std::sync::{Arc, Mutex};

pub const HISTORY_LINES: usize = 10_000;

#[derive(Clone, Default)]
pub struct Listener(Arc<Mutex<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        if let Ok(mut queue) = self.0.lock() {
            queue.push(event);
        }
    }
}

pub struct Screen {
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
}

fn size(rows: u16, cols: u16) -> TermSize {
    TermSize::new(cols.max(1) as usize, rows.max(1) as usize)
}

pub fn new_screen(rows: u16, cols: u16) -> Screen {
    let listener = Listener::default();
    let config = Config {
        scrolling_history: HISTORY_LINES,
        ..Config::default()
    };
    let term = Term::new(config, &size(rows, cols), listener.clone());
    Screen {
        term,
        parser: Processor::new(),
        listener,
    }
}

fn color(c: TermColor) -> Color {
    match c {
        TermColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        TermColor::Indexed(i) => Color::Indexed(i),
        TermColor::Named(n) => {
            let v = n as usize;
            let dim = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
            if v < 16 {
                Color::Indexed(v as u8)
            } else if dim.contains(&v) {
                Color::Indexed((v - NamedColor::DimBlack as usize) as u8)
            } else {
                Color::Default
            }
        }
    }
}

fn cell_flags(f: Flags) -> u16 {
    let pairs = [
        (Flags::BOLD, flags::BOLD),
        (Flags::ITALIC, flags::ITALIC),
        (Flags::UNDERLINE, flags::UNDERLINE),
        (Flags::INVERSE, flags::INVERSE),
        (Flags::DIM, flags::DIM),
        (Flags::STRIKEOUT, flags::STRIKE),
        (Flags::HIDDEN, flags::HIDDEN),
        (Flags::WIDE_CHAR, flags::WIDE),
        (Flags::WIDE_CHAR_SPACER, flags::WIDE_SPACER),
        (Flags::LEADING_WIDE_CHAR_SPACER, flags::WIDE_SPACER),
        (Flags::DOUBLE_UNDERLINE, flags::DOUBLE_UNDERLINE),
        (Flags::UNDERCURL, flags::UNDERCURL),
        (Flags::DOTTED_UNDERLINE, flags::DOTTED_UNDERLINE),
        (Flags::DASHED_UNDERLINE, flags::DASHED_UNDERLINE),
    ];
    pairs
        .iter()
        .filter(|(from, _)| f.contains(*from))
        .fold(0, |acc, (_, to)| acc | to)
}

fn cell(c: &TermCell) -> Cell {
    Cell {
        ch: c.c,
        zerowidth: c.zerowidth().map(|z| z.iter().collect()),
        fg: color(c.fg),
        bg: color(c.bg),
        flags: cell_flags(c.flags),
    }
}

fn shape(s: TermShape) -> CursorShape {
    match s {
        TermShape::Block => CursorShape::Block,
        TermShape::Underline => CursorShape::Underline,
        TermShape::Beam => CursorShape::Bar,
        TermShape::HollowBlock => CursorShape::HollowBlock,
        TermShape::Hidden => CursorShape::Hidden,
    }
}

impl Screen {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn take_events(&self) -> Vec<TermEvent> {
        self.listener
            .0
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    pub fn rows(&self) -> u16 {
        self.term.screen_lines() as u16
    }

    pub fn cols(&self) -> u16 {
        self.term.columns() as u16
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let shrinking_primary = cols < self.cols() && !self.term.mode().contains(TermMode::ALT_SCREEN);
        if !shrinking_primary {
            self.term.resize(size(rows, cols));
            return;
        }
        let history_before = self.history();
        self.term.grid_mut().update_history(HISTORY_LINES + usize::from(rows));
        self.term.resize(size(rows, cols));
        let pushed_up = self.history().saturating_sub(history_before);
        let grid = self.term.grid();
        let last_cursor = grid.cursor.point.line.0.max(grid.saved_cursor.point.line.0);
        let reclaimed = (last_cursor + 1..i32::from(rows))
            .rev()
            .take(pushed_up)
            .take_while(|&row| grid[Line(row)].is_clear())
            .count() as u16;
        if reclaimed > 0 {
            self.term.resize(size(rows - reclaimed, cols));
            self.term.resize(size(rows, cols));
        }
        self.term.grid_mut().update_history(HISTORY_LINES);
    }

    pub fn history(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn focus_events(&self) -> bool {
        self.term.mode().contains(TermMode::FOCUS_IN_OUT)
    }

    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn kitty_keyboard(&self) -> bool {
        self.term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
    }

    pub fn report_all_keys(&self) -> bool {
        self.term.mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC)
    }

    pub fn modes(&self) -> Modes {
        let m = self.term.mode();
        Modes {
            app_cursor: m.contains(TermMode::APP_CURSOR),
            app_keypad: m.contains(TermMode::APP_KEYPAD),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            mouse_click: m.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: m.contains(TermMode::MOUSE_DRAG),
            mouse_motion: m.contains(TermMode::MOUSE_MOTION),
            mouse_sgr: m.contains(TermMode::SGR_MOUSE),
            focus_events: m.contains(TermMode::FOCUS_IN_OUT),
        }
    }

    fn row_text(&self, line: Line) -> (String, bool) {
        let row = &self.term.grid()[line];
        let cols = self.term.columns();
        let mut out = String::new();
        for col in 0..cols {
            let c = &row[Column(col)];
            if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            out.push(c.c);
            if let Some(z) = c.zerowidth() {
                out.extend(z.iter());
            }
        }
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        (out, wrapped)
    }

    fn lines_text(&self, from: i32, to: i32, join_wrapped: bool) -> String {
        let mut lines: Vec<String> = Vec::new();
        let mut current = String::new();
        for l in from..to {
            let (text, wrapped) = self.row_text(Line(l));
            current.push_str(&text);
            if join_wrapped && wrapped {
                continue;
            }
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        if !current.is_empty() {
            lines.push(current.trim_end().to_string());
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub fn screen_text(&self) -> String {
        self.lines_text(0, self.rows() as i32, false)
    }

    pub fn history_text(&self) -> String {
        self.lines_text(-(self.history() as i32), self.rows() as i32, true)
    }

    pub fn tail_text(&self, lines: usize) -> String {
        let back = self.history().min(lines.saturating_mul(4).max(64));
        let text = self.lines_text(-(back as i32), self.rows() as i32, true);
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    pub fn snapshot(&self, id: &TermId, seq: u64, scroll: usize, title: Option<String>) -> Snapshot {
        let history = self.history();
        let scroll = scroll.min(history);
        let rows = self.rows();
        let cols = self.cols();
        let grid = self.term.grid();
        let mut cells = Vec::with_capacity(rows as usize * cols as usize);
        for r in 0..rows as i32 {
            let row = &grid[Line(r - scroll as i32)];
            for c in 0..cols as usize {
                cells.push(cell(&row[Column(c)]));
            }
        }
        let point = grid.cursor.point;
        let style = self.term.cursor_style();
        let cursor_row = point.line.0 + scroll as i32;
        let visible = self.term.mode().contains(TermMode::SHOW_CURSOR)
            && style.shape != TermShape::Hidden
            && (0..rows as i32).contains(&cursor_row);
        Snapshot {
            id: id.clone(),
            seq,
            rows,
            cols,
            cells,
            cursor: Cursor {
                row: cursor_row.clamp(0, rows as i32 - 1) as u16,
                col: point.column.0.min(cols as usize - 1) as u16,
                visible,
                shape: shape(style.shape),
            },
            modes: self.modes(),
            title,
            history,
            scroll,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> TermId {
        TermId::from_number(1)
    }

    #[test]
    fn plain_text_of_screen() {
        let mut s = new_screen(4, 20);
        s.feed(b"hello\r\n\x1b[31mworld\x1b[0m");
        assert_eq!(s.screen_text(), "hello\nworld");
        let snap = s.snapshot(&id(), 1, 0, None);
        assert_eq!(snap.cells.len(), 80);
        assert_eq!(snap.row_text(1), "world");
        assert_eq!(snap.row(1)[0].fg, Color::Indexed(1));
        assert_eq!((snap.cursor.row, snap.cursor.col), (1, 5));
    }

    #[test]
    fn history_keeps_scrolled_lines() {
        let mut s = new_screen(3, 10);
        for i in 0..6 {
            s.feed(format!("line{i}\r\n").as_bytes());
        }
        assert_eq!(s.history(), 4);
        assert!(s.history_text().starts_with("line0\nline1"));
        assert!(!s.screen_text().contains("line0"));
        let snap = s.snapshot(&id(), 1, 2, None);
        assert_eq!(snap.row_text(0), "line2");
    }

    #[test]
    fn wrapped_lines_join_in_history() {
        let mut s = new_screen(3, 5);
        s.feed(b"abcdefgh");
        assert_eq!(s.screen_text(), "abcde\nfgh");
        assert_eq!(s.history_text(), "abcdefgh");
    }

    #[test]
    fn tail_is_the_last_lines_with_text() {
        let mut s = new_screen(6, 10);
        for i in 0..30 {
            s.feed(format!("l{i}\r\n").as_bytes());
        }
        s.feed(b"$ ");
        assert_eq!(s.tail_text(3), "l28\nl29\n$");
        let mut fresh = new_screen(24, 10);
        fresh.feed(b"one\r\ntwo\r\n");
        assert_eq!(fresh.tail_text(10), "one\ntwo");
        let mut wrapped = new_screen(4, 5);
        wrapped.feed(b"abcdefghij\r\nend");
        assert_eq!(wrapped.tail_text(2), "abcdefghij\nend");
    }

    #[test]
    fn resize_uses_the_blank_tail_before_moving_output_to_scrollback() {
        for with_history in [false, true] {
            let mut screen = new_screen(12, 44);
            if with_history {
                screen.feed(&b"older\r\n".repeat(HISTORY_LINES + 12));
                screen.feed(b"\x1b[2J\x1b[H");
            }
            let history_before = screen.history();
            screen.feed(b"FIRST\r\nabcdefghijklmnopqrstuvwxyz0123456789\r\nLAST\r\nready> ");
            screen.resize(8, 12);
            let snapshot = screen.snapshot(&id(), 1, 0, None);
            assert_eq!((snapshot.rows, snapshot.cols), (8, 12));
            assert_eq!(
                screen.screen_text(),
                "FIRST\nabcdefghijkl\nmnopqrstuvwx\nyz0123456789\nLAST\nready>"
            );
            assert_eq!(screen.history(), history_before);
        }
    }

    #[test]
    fn wide_chars_and_resize() {
        let mut s = new_screen(3, 10);
        s.feed("日本".as_bytes());
        assert_eq!(s.screen_text(), "日本");
        let snap = s.snapshot(&id(), 1, 0, None);
        assert!(snap.row(0)[0].has(flags::WIDE));
        assert!(snap.row(0)[1].has(flags::WIDE_SPACER));
        s.resize(5, 30);
        assert_eq!((s.rows(), s.cols()), (5, 30));
    }
}
