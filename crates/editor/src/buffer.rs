use std::ops::Range;

const UNDO_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
}

impl Sel {
    pub fn caret(at: usize) -> Sel {
        Sel { anchor: at, head: at }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Type,
    Erase,
    Other,
}

#[derive(Clone, Debug)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    before: Sel,
    after: Sel,
    kind: Kind,
}

#[derive(Clone, Debug, Default)]
pub struct Buffer {
    text: String,
    starts: Vec<usize>,
    pub sel: Sel,
    goal: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    pub rev: u64,
    pub readonly: bool,
}

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn prev_char(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].char_indices().next_back().map_or(0, |(i, _)| i)
}

pub fn next_char(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text[at..].chars().next().map_or(text.len(), |c| at + c.len_utf8())
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    out.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1));
    out
}

fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

impl Buffer {
    pub fn new(text: &str) -> Buffer {
        let mut b = Buffer::default();
        b.set_text(text);
        b
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.starts = line_starts(text);
        self.sel = Sel::caret(0);
        self.goal = None;
        self.undo.clear();
        self.redo.clear();
        self.rev += 1;
    }

    pub fn replace_all_keep_place(&mut self, text: &str) {
        let line = self.line_of(self.sel.head);
        let col = self.col_of(self.sel.head);
        self.set_text(text);
        let line = line.min(self.line_count() - 1);
        let at = self.offset_at(line, col);
        self.sel = Sel::caret(at);
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line.min(self.starts.len() - 1)]
    }

    pub fn line_end(&self, line: usize) -> usize {
        match self.starts.get(line + 1) {
            Some(next) => next - 1,
            None => self.text.len(),
        }
    }

    pub fn line_text(&self, line: usize) -> &str {
        &self.text[self.line_start(line)..self.line_end(line)]
    }

    pub fn line_of(&self, at: usize) -> usize {
        self.starts.partition_point(|s| *s <= at).saturating_sub(1)
    }

    pub fn col_of(&self, at: usize) -> usize {
        let start = self.line_start(self.line_of(at));
        self.text[start..at.min(self.text.len())].chars().count()
    }

    pub fn offset_at(&self, line: usize, col: usize) -> usize {
        let start = self.line_start(line);
        let text = self.line_text(line);
        start + text.char_indices().nth(col).map_or(text.len(), |(i, _)| i)
    }

    pub fn selected(&self) -> &str {
        &self.text[self.sel.range()]
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn apply(&mut self, at: usize, remove: usize, insert: &str) {
        self.text.replace_range(at..at + remove, insert);
        self.starts = line_starts(&self.text);
        self.rev += 1;
    }

    fn record(&mut self, edit: Edit) {
        self.redo.clear();
        if let Some(last) = self.undo.last_mut() {
            let typing = edit.kind == Kind::Type
                && last.kind == Kind::Type
                && edit.removed.is_empty()
                && last.removed.is_empty()
                && last.at + last.inserted.len() == edit.at;
            let erasing = edit.kind == Kind::Erase
                && last.kind == Kind::Erase
                && edit.inserted.is_empty()
                && last.inserted.is_empty()
                && edit.at + edit.removed.len() == last.at;
            if typing {
                last.inserted.push_str(&edit.inserted);
                last.after = edit.after;
                return;
            }
            if erasing {
                last.removed.insert_str(0, &edit.removed);
                last.at = edit.at;
                last.after = edit.after;
                return;
            }
        }
        self.undo.push(edit);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    fn edit(&mut self, range: Range<usize>, insert: &str, kind: Kind, after: Option<Sel>) -> bool {
        if self.readonly {
            return false;
        }
        let start = range.start.min(self.text.len());
        let end = range.end.clamp(start, self.text.len());
        if start == end && insert.is_empty() {
            return false;
        }
        let removed = self.text[start..end].to_string();
        let before = self.sel;
        self.apply(start, end - start, insert);
        self.sel = after.unwrap_or(Sel::caret(start + insert.len()));
        self.goal = None;
        self.record(Edit { at: start, removed, inserted: insert.to_string(), before, after: self.sel, kind });
        true
    }

    pub fn replace(&mut self, range: Range<usize>, insert: &str) -> bool {
        let kind = if insert.chars().count() == 1 && insert != "\n" { Kind::Type } else { Kind::Other };
        self.edit(range, insert, kind, None)
    }

    pub fn insert(&mut self, text: &str) -> bool {
        self.replace(self.sel.range(), text)
    }

    pub fn paste(&mut self, text: &str) -> bool {
        let clean = crate::doc::to_lf(text);
        self.edit(self.sel.range(), &clean, Kind::Other, None)
    }

    pub fn newline(&mut self) -> bool {
        let head = self.sel.range().start;
        let line = self.line_of(head);
        let start = self.line_start(line);
        let ws = leading_ws(&self.text[start..head]).to_string();
        self.edit(self.sel.range(), &format!("\n{ws}"), Kind::Other, None)
    }

    pub fn tab(&mut self, unit: &str) -> bool {
        let r = self.sel.range();
        if self.line_of(r.start) != self.line_of(r.end) {
            return self.indent_lines(unit);
        }
        let insert = if unit == "\t" {
            unit.to_string()
        } else {
            let w = unit.len().max(1);
            " ".repeat(w - self.col_of(r.start) % w)
        };
        self.edit(r, &insert, Kind::Other, None)
    }

    fn lines_of_sel(&self) -> (usize, usize) {
        let r = self.sel.range();
        let first = self.line_of(r.start);
        let mut last = self.line_of(r.end);
        if last > first && r.end == self.line_start(last) {
            last -= 1;
        }
        (first, last)
    }

    fn rewrite_lines(&mut self, first: usize, last: usize, f: impl Fn(&str) -> String) -> bool {
        let start = self.line_start(first);
        let end = self.line_end(last);
        let old: Vec<&str> = self.text[start..end].split('\n').collect();
        let new: Vec<String> = old.iter().map(|l| f(l)).collect();
        let joined = new.join("\n");
        if joined == self.text[start..end] {
            return false;
        }
        let after = Sel { anchor: start, head: start + joined.len() };
        let after = if self.sel.is_empty() {
            let delta = new[0].len() as isize - old[0].len() as isize;
            Sel::caret((self.sel.head as isize + delta).max(start as isize) as usize)
        } else {
            after
        };
        self.edit(start..end, &joined, Kind::Other, Some(after))
    }

    pub fn indent_lines(&mut self, unit: &str) -> bool {
        let (first, last) = self.lines_of_sel();
        let unit = unit.to_string();
        self.rewrite_lines(first, last, |l| if l.is_empty() { String::new() } else { format!("{unit}{l}") })
    }

    pub fn outdent(&mut self, unit: &str) -> bool {
        let (first, last) = self.lines_of_sel();
        let width = if unit == "\t" { 1 } else { unit.len().max(1) };
        self.rewrite_lines(first, last, |l| {
            if let Some(rest) = l.strip_prefix('\t') {
                return rest.to_string();
            }
            let n = l.len() - l.trim_start_matches(' ').len();
            l[n.min(width)..].to_string()
        })
    }

    pub fn backspace(&mut self) -> bool {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.edit(r, "", Kind::Other, None);
        }
        let from = prev_char(&self.text, r.start);
        self.edit(from..r.start, "", Kind::Erase, None)
    }

    pub fn delete_forward(&mut self) -> bool {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.edit(r, "", Kind::Other, None);
        }
        let to = next_char(&self.text, r.start);
        self.edit(r.start..to, "", Kind::Other, None)
    }

    pub fn delete_word_back(&mut self) -> bool {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.edit(r, "", Kind::Other, None);
        }
        let from = self.word_left_of(r.start);
        self.edit(from..r.start, "", Kind::Other, None)
    }

    pub fn delete_line_back(&mut self) -> bool {
        let r = self.sel.range();
        if !r.is_empty() {
            return self.edit(r, "", Kind::Other, None);
        }
        let start = self.line_start(self.line_of(r.start));
        let from = if start == r.start { prev_char(&self.text, start) } else { start };
        self.edit(from..r.start, "", Kind::Other, None)
    }

    pub fn cut(&mut self) -> Option<String> {
        if self.sel.is_empty() {
            return None;
        }
        let out = self.selected().to_string();
        self.edit(self.sel.range(), "", Kind::Other, None).then_some(out)
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else { return false };
        self.apply(e.at, e.inserted.len(), &e.removed);
        self.sel = e.before;
        self.goal = None;
        self.redo.push(e);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else { return false };
        self.apply(e.at, e.removed.len(), &e.inserted);
        self.sel = e.after;
        self.goal = None;
        self.undo.push(e);
        true
    }

    pub fn move_to(&mut self, at: usize, extend: bool) {
        let at = at.min(self.text.len());
        self.sel = if extend { Sel { anchor: self.sel.anchor, head: at } } else { Sel::caret(at) };
        self.goal = None;
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.sel = Sel { anchor: range.start.min(self.text.len()), head: range.end.min(self.text.len()) };
        self.goal = None;
    }

    pub fn left(&mut self, extend: bool) {
        let r = self.sel.range();
        let to = if !extend && !r.is_empty() { r.start } else { prev_char(&self.text, self.sel.head) };
        self.move_to(to, extend);
    }

    pub fn right(&mut self, extend: bool) {
        let r = self.sel.range();
        let to = if !extend && !r.is_empty() { r.end } else { next_char(&self.text, self.sel.head) };
        self.move_to(to, extend);
    }

    fn word_left_of(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let mut chars = before.char_indices().rev().peekable();
        let mut pos = at;
        while let Some((i, c)) = chars.peek().copied() {
            if is_word(c) {
                break;
            }
            pos = i;
            chars.next();
        }
        while let Some((i, c)) = chars.peek().copied() {
            if !is_word(c) {
                break;
            }
            pos = i;
            chars.next();
        }
        pos
    }

    fn word_right_of(&self, at: usize) -> usize {
        let mut pos = at;
        let mut chars = self.text[at..].chars().peekable();
        while let Some(c) = chars.peek().copied() {
            if is_word(c) {
                break;
            }
            pos += c.len_utf8();
            chars.next();
        }
        while let Some(c) = chars.peek().copied() {
            if !is_word(c) {
                break;
            }
            pos += c.len_utf8();
            chars.next();
        }
        pos
    }

    pub fn word_left(&mut self, extend: bool) {
        let to = self.word_left_of(self.sel.head);
        self.move_to(to, extend);
    }

    pub fn word_right(&mut self, extend: bool) {
        let to = self.word_right_of(self.sel.head);
        self.move_to(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        let line = self.line_of(self.sel.head);
        let start = self.line_start(line);
        let first = start + leading_ws(self.line_text(line)).len();
        let to = if self.sel.head == first { start } else { first };
        self.move_to(to, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let to = self.line_end(self.line_of(self.sel.head));
        self.move_to(to, extend);
    }

    pub fn vertical(&mut self, lines: isize, extend: bool) {
        let line = self.line_of(self.sel.head) as isize;
        let goal = self.goal.unwrap_or_else(|| self.col_of(self.sel.head));
        let target = line + lines;
        let to = if target < 0 {
            0
        } else if target as usize >= self.line_count() {
            self.text.len()
        } else {
            self.offset_at(target as usize, goal)
        };
        if !extend && !self.sel.is_empty() && lines.abs() == 1 {
            let r = self.sel.range();
            let edge = if lines < 0 { r.start } else { r.end };
            self.sel = Sel::caret(edge);
            let line = self.line_of(edge) as isize + lines;
            let goal = self.col_of(edge);
            let to = if line < 0 {
                0
            } else if line as usize >= self.line_count() {
                self.text.len()
            } else {
                self.offset_at(line as usize, goal)
            };
            self.sel = Sel::caret(to);
            self.goal = Some(goal);
            return;
        }
        self.sel = if extend { Sel { anchor: self.sel.anchor, head: to } } else { Sel::caret(to) };
        self.goal = Some(goal);
    }

    pub fn doc_start(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn doc_end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.sel = Sel { anchor: 0, head: self.text.len() };
    }

    pub fn word_at(&self, at: usize) -> Range<usize> {
        let at = at.min(self.text.len());
        let c = self.text[at..].chars().next();
        if !c.is_some_and(is_word) {
            let prev = self.text[..at].chars().next_back();
            if prev.is_some_and(is_word) {
                return self.word_left_of(at)..at;
            }
            return at..next_char(&self.text, at);
        }
        let mut start = at;
        for (i, c) in self.text[..at].char_indices().rev() {
            if !is_word(c) {
                break;
            }
            start = i;
        }
        let mut end = at;
        for c in self.text[at..].chars() {
            if !is_word(c) {
                break;
            }
            end += c.len_utf8();
        }
        start..end
    }

    pub fn line_range_at(&self, at: usize) -> Range<usize> {
        let line = self.line_of(at);
        let end = self.starts.get(line + 1).copied().unwrap_or(self.text.len());
        self.line_start(line)..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_columns() {
        let b = Buffer::new("ab\ncdé\n\nx");
        assert_eq!(b.line_count(), 4);
        assert_eq!(b.line_text(1), "cdé");
        assert_eq!(b.line_of(3), 1);
        assert_eq!(b.line_of(2), 0);
        assert_eq!(b.col_of(7), 3);
        assert_eq!(b.offset_at(1, 3), 7);
        assert_eq!(b.offset_at(1, 99), 7);
        assert_eq!(b.line_text(2), "");
    }

    #[test]
    fn typing_groups_for_undo() {
        let mut b = Buffer::new("");
        for c in ["h", "e", "y"] {
            b.insert(c);
        }
        b.insert(" ");
        assert_eq!(b.text(), "hey ");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        assert_eq!(b.text(), "hey ");
        b.backspace();
        b.backspace();
        assert_eq!(b.text(), "he");
        b.undo();
        assert_eq!(b.text(), "hey ");
    }

    #[test]
    fn newline_keeps_indent() {
        let mut b = Buffer::new("    let x = 1;");
        b.doc_end(false);
        b.newline();
        b.insert("y");
        assert_eq!(b.text(), "    let x = 1;\n    y");
    }

    #[test]
    fn tab_and_outdent() {
        let mut b = Buffer::new("a\nb\nc");
        b.select(0..3);
        b.tab("    ");
        assert_eq!(b.text(), "    a\n    b\nc");
        b.outdent("    ");
        assert_eq!(b.text(), "a\nb\nc");
        let mut b = Buffer::new("ab");
        b.move_to(1, false);
        b.tab("    ");
        assert_eq!(b.text(), "a   b");
        let mut b = Buffer::new("x");
        b.tab("\t");
        assert_eq!(b.text(), "\tx");
        b.outdent("\t");
        assert_eq!(b.text(), "x");
    }

    #[test]
    fn moves() {
        let mut b = Buffer::new("  foo bar\nbaz");
        b.end(false);
        b.word_left(false);
        assert_eq!(b.sel.head, 6);
        b.home(false);
        assert_eq!(b.sel.head, 2);
        b.home(false);
        assert_eq!(b.sel.head, 0);
        b.move_to(8, false);
        b.vertical(1, false);
        assert_eq!(b.sel.head, b.len());
        b.vertical(-1, false);
        assert_eq!(b.sel.head, 8);
        b.vertical(-5, true);
        assert_eq!(b.sel.range(), 0..8);
        assert_eq!(b.word_at(3), 2..5);
        assert_eq!(b.line_range_at(3), 0..10);
    }

    #[test]
    fn readonly_blocks_edits() {
        let mut b = Buffer::new("x");
        b.readonly = true;
        assert!(!b.insert("y"));
        assert!(!b.backspace());
        assert_eq!(b.text(), "x");
    }

    #[test]
    fn paste_makes_lf_and_cut_returns_text() {
        let mut b = Buffer::new("");
        b.paste("a\r\nb");
        assert_eq!(b.text(), "a\nb");
        b.select(0..1);
        assert_eq!(b.cut().as_deref(), Some("a"));
        assert_eq!(b.text(), "\nb");
    }

    #[test]
    fn delete_words_and_lines() {
        let mut b = Buffer::new("one two");
        b.doc_end(false);
        b.delete_word_back();
        assert_eq!(b.text(), "one ");
        b.delete_line_back();
        assert_eq!(b.text(), "");
    }
}
