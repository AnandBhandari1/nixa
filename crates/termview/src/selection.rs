use nixa_protocol::{Snapshot, flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub line: i64,
    pub col: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Simple,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Spot,
    pub head: Spot,
    pub mode: Mode,
}

pub fn line_of(row: u16, scroll: usize) -> i64 {
    row as i64 - scroll as i64
}

pub fn row_of(line: i64, scroll: usize) -> i64 {
    line + scroll as i64
}

fn is_word(c: char) -> bool {
    !(c.is_whitespace() || "()[]{}<>'\"`|,;:".contains(c))
}

fn word_bounds(snap: &Snapshot, row: u16, col: u16) -> (u16, u16) {
    let cells = snap.row(row);
    if cells.is_empty() {
        return (col, col);
    }
    let col = col.min(cells.len() as u16 - 1);
    let word = |i: usize| is_word(cells[i].ch);
    if !word(col as usize) {
        return (col, col);
    }
    let mut start = col as usize;
    while start > 0 && word(start - 1) {
        start -= 1;
    }
    let mut end = col as usize;
    while end + 1 < cells.len() && word(end + 1) {
        end += 1;
    }
    (start as u16, end as u16)
}

impl Selection {
    pub fn start(at: Spot, mode: Mode) -> Selection {
        Selection {
            anchor: at,
            head: at,
            mode,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mode == Mode::Simple && self.anchor == self.head
    }

    pub fn range(&self, snap: &Snapshot) -> (Spot, Spot) {
        let (mut a, mut b) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let last = snap.cols.saturating_sub(1);
        match self.mode {
            Mode::Simple => {}
            Mode::Line => {
                a.col = 0;
                b.col = last;
            }
            Mode::Word => {
                let ra = row_of(a.line, snap.scroll);
                if (0..snap.rows as i64).contains(&ra) {
                    a.col = word_bounds(snap, ra as u16, a.col).0;
                }
                let rb = row_of(b.line, snap.scroll);
                if (0..snap.rows as i64).contains(&rb) {
                    b.col = word_bounds(snap, rb as u16, b.col).1;
                }
            }
        }
        (a, b)
    }

    pub fn row_span(&self, snap: &Snapshot, row: u16) -> Option<(u16, u16)> {
        if self.is_empty() {
            return None;
        }
        let (a, b) = self.range(snap);
        let line = line_of(row, snap.scroll);
        if line < a.line || line > b.line {
            return None;
        }
        let last = snap.cols.saturating_sub(1);
        let from = if line == a.line { a.col } else { 0 };
        let to = if line == b.line { b.col } else { last };
        if from > to {
            None
        } else {
            Some((from, to.min(last)))
        }
    }

    pub fn text(&self, snap: &Snapshot) -> String {
        let mut lines = Vec::new();
        for row in 0..snap.rows {
            let Some((from, to)) = self.row_span(snap, row) else {
                continue;
            };
            let mut s = String::new();
            for cell in &snap.row(row)[from as usize..=to as usize] {
                if cell.has(flags::WIDE_SPACER) {
                    continue;
                }
                s.push(cell.ch);
                if let Some(z) = &cell.zerowidth {
                    s.push_str(z);
                }
            }
            lines.push(s.trim_end().to_string());
        }
        lines.join("\n")
    }
}

pub fn all(snap: &Snapshot) -> Selection {
    Selection {
        anchor: Spot {
            line: line_of(0, snap.scroll),
            col: 0,
        },
        head: Spot {
            line: line_of(snap.rows.saturating_sub(1), snap.scroll),
            col: snap.cols.saturating_sub(1),
        },
        mode: Mode::Simple,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::{Cell, Cursor, TermId};

    fn snap(lines: &[&str], cols: u16) -> Snapshot {
        let mut cells = Vec::new();
        for l in lines {
            let mut row: Vec<Cell> = l
                .chars()
                .map(|ch| Cell {
                    ch,
                    ..Cell::default()
                })
                .collect();
            row.resize(cols as usize, Cell::default());
            cells.extend(row);
        }
        Snapshot {
            id: TermId::from_number(1),
            seq: 1,
            rows: lines.len() as u16,
            cols,
            cells,
            cursor: Cursor::default(),
            modes: Default::default(),
            title: None,
            history: 0,
            scroll: 0,
        }
    }

    #[test]
    fn simple_selection_spans_lines() {
        let s = snap(&["hello world", "second line"], 12);
        let sel = Selection {
            anchor: Spot { line: 0, col: 6 },
            head: Spot { line: 1, col: 5 },
            mode: Mode::Simple,
        };
        assert_eq!(sel.text(&s), "world\nsecond");
    }

    #[test]
    fn word_and_line_modes() {
        let s = snap(&["foo bar-baz qux"], 16);
        let at = Spot { line: 0, col: 6 };
        assert_eq!(Selection::start(at, Mode::Word).text(&s), "bar-baz");
        assert_eq!(Selection::start(at, Mode::Line).text(&s), "foo bar-baz qux");
        assert!(Selection::start(at, Mode::Simple).is_empty());
    }
}
