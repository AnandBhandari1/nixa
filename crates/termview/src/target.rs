use nixa_protocol::{Snapshot, flags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub text: String,
}

const STOP: [char; 6] = ['"', '\'', '`', '<', '>', '|'];
const LEAD: [char; 6] = ['(', '[', '{', ':', ',', ';'];
const TAIL: [char; 8] = ['.', ',', ';', ':', '!', '?', '}', '*'];

fn link_char(c: char) -> bool {
    !c.is_whitespace() && !STOP.contains(&c) && (c.is_ascii_graphic() || c.is_alphanumeric())
}

fn more_closed(body: &[char], open: char, close: char) -> bool {
    let count = |want: char| body.iter().filter(|c| **c == want).count();
    count(close) > count(open)
}

fn trim(chars: &[char]) -> Option<(usize, usize)> {
    let mut from = 0;
    let mut to = chars.len();
    while from < to && LEAD.contains(&chars[from]) {
        from += 1;
    }
    while to > from {
        let body = &chars[from..to];
        let cut = match chars[to - 1] {
            ')' => more_closed(body, '(', ')'),
            ']' => more_closed(body, '[', ']'),
            c => TAIL.contains(&c),
        };
        if !cut {
            break;
        }
        to -= 1;
    }
    (to > from).then_some((from, to))
}

pub fn is_web(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["https://", "http://"]
        .iter()
        .any(|p| lower.starts_with(*p) && lower.len() > p.len())
}

fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub fn split_line(text: &str) -> (&str, Option<u32>, Option<u32>) {
    let Some((head, last)) = text.rsplit_once(':') else {
        return (text, None, None);
    };
    let Some(n) = number(last) else {
        return (text, None, None);
    };
    if let Some((path, mid)) = head.rsplit_once(':')
        && let Some(line) = number(mid)
    {
        return (path, Some(line), Some(n));
    }
    (head, Some(n), None)
}

fn has_ext(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=10).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic())
}

pub fn is_path(text: &str) -> bool {
    let (path, _, _) = split_line(text);
    if !path.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    path.contains('/') || has_ext(path)
}

pub fn looks_like_link(text: &str) -> bool {
    is_web(text) || is_path(text)
}

pub fn hit_at(snap: &Snapshot, row: u16, col: u16) -> Option<Hit> {
    if row >= snap.rows {
        return None;
    }
    let cells: Vec<(u16, u16, char)> = snap
        .row(row)
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.has(flags::WIDE_SPACER))
        .map(|(i, c)| (i as u16, if c.has(flags::WIDE) { 2 } else { 1 }, c.ch))
        .collect();
    let at = cells.iter().rposition(|(c, _, _)| *c <= col)?;
    let (from_col, width, ch) = cells[at];
    if col >= from_col + width || !link_char(ch) {
        return None;
    }
    let mut first = at;
    while first > 0 && link_char(cells[first - 1].2) {
        first -= 1;
    }
    let mut last = at;
    while last + 1 < cells.len() && link_char(cells[last + 1].2) {
        last += 1;
    }
    let chars: Vec<char> = cells[first..=last].iter().map(|c| c.2).collect();
    let (from, to) = trim(&chars)?;
    let (first, last) = (first + from, first + to - 1);
    if at < first || at > last {
        return None;
    }
    let text: String = chars[from..to].iter().collect();
    if !looks_like_link(&text) {
        return None;
    }
    Some(Hit {
        row,
        start: cells[first].0,
        end: cells[last].0 + cells[last].1,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_protocol::{Cell, Cursor, Modes, TermId};

    fn snap(lines: &[&str], cols: u16) -> Snapshot {
        let mut cells = Vec::new();
        for line in lines {
            let mut row: Vec<Cell> = line
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
            modes: Modes::default(),
            title: None,
            history: 0,
            scroll: 0,
        }
    }

    fn text_at(s: &Snapshot, row: u16, col: u16) -> Option<String> {
        hit_at(s, row, col).map(|h| h.text)
    }

    #[test]
    fn finds_web_links_and_drops_trailing_marks() {
        let s = snap(&["see https://example.com/a?b=1. ok", "(https://example.com/x)"], 40);
        let hit = hit_at(&s, 0, 10).unwrap();
        assert_eq!(hit.text, "https://example.com/a?b=1");
        assert_eq!((hit.start, hit.end), (4, 29));
        assert_eq!(text_at(&s, 0, 29), None);
        assert_eq!(text_at(&s, 0, 1), None);
        assert_eq!(text_at(&s, 1, 5).as_deref(), Some("https://example.com/x"));
    }

    #[test]
    fn finds_paths_with_lines() {
        let s = snap(&["error at src/main.rs:12:5: oops", "  --> ./lib.rs:7", "plain words here"], 40);
        assert_eq!(text_at(&s, 0, 12).as_deref(), Some("src/main.rs:12:5"));
        assert_eq!(text_at(&s, 1, 10).as_deref(), Some("./lib.rs:7"));
        assert_eq!(text_at(&s, 2, 2), None);
        assert_eq!(text_at(&s, 0, 2), None);
    }

    #[test]
    fn keeps_balanced_brackets() {
        let s = snap(&["https://en.example.org/wiki/A_(b) x"], 40);
        assert_eq!(
            text_at(&s, 0, 3).as_deref(),
            Some("https://en.example.org/wiki/A_(b)")
        );
    }

    #[test]
    fn splits_line_and_column() {
        assert_eq!(split_line("a/b.rs:12:5"), ("a/b.rs", Some(12), Some(5)));
        assert_eq!(split_line("a/b.rs:12"), ("a/b.rs", Some(12), None));
        assert_eq!(split_line("a/b.rs"), ("a/b.rs", None, None));
        assert_eq!(split_line("a:b"), ("a:b", None, None));
    }

    #[test]
    fn tells_paths_and_links_apart() {
        assert!(is_web("https://x.org"));
        assert!(!is_web("https://"));
        assert!(is_path("Cargo.toml"));
        assert!(is_path("~/notes"));
        assert!(is_path("src/view.rs:3"));
        assert!(!is_path("1.5"));
        assert!(!is_path("hello"));
        assert!(!is_path("//"));
    }
}
