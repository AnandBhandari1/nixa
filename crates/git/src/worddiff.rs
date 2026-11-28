use serde::{Deserialize, Serialize};

pub const MAX_LINE: usize = 500;

pub const MAX_HUNK: usize = 400;

pub const MAX_CELLS: usize = 10000;

pub const MIN_SHARED: f64 = 0.3;

pub const MAX_WORK: usize = 400000;

pub type Span = (usize, usize);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineSpans {
    pub a: Vec<Span>,
    pub b: Vec<Span>,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || (c as u32) >= 0x80 && (c as u32) <= 0xffff
}

pub fn tokenize(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut kind = 0u8;
    for (i, c) in text.char_indices() {
        let k = if is_word(c) {
            1
        } else if c.is_whitespace() {
            2
        } else {
            3
        };
        if i > start && (k != kind || k == 3) {
            out.push(&text[start..i]);
            start = i;
        }
        kind = k;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn common_marks(a: &[&str], b: &[&str], same_a: &mut [bool], same_b: &mut [bool], off: usize) {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            same_a[off + i] = true;
            same_b[off + j] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

fn spans_of(tokens: &[&str], same: &[bool]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut pos = 0;
    for (i, token) in tokens.iter().enumerate() {
        let end = pos + token.len();
        if !same[i] {
            match spans.last_mut() {
                Some(last) if last.1 == pos => last.1 = end,
                _ => spans.push((pos, end)),
            }
        }
        pos = end;
    }
    spans
}

pub fn merge_spans(text: &str, spans: &[Span]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for span in spans {
        match out.last_mut() {
            Some(last) if blank(&text[last.1..span.0]) => last.1 = span.1,
            _ => out.push(*span),
        }
    }
    out
}

fn solid(tokens: &[&str], same: &[bool], want: bool) -> usize {
    tokens.iter().enumerate().filter(|(i, t)| same[*i] == want && !blank(t)).map(|(_, t)| t.chars().count()).sum()
}

pub fn line_spans(a: &str, b: &str, budget: Option<&mut usize>) -> Option<LineSpans> {
    if a == b || a.chars().count() > MAX_LINE || b.chars().count() > MAX_LINE {
        return None;
    }
    let ta = tokenize(a);
    let tb = tokenize(b);
    let mut pre = 0;
    while pre < ta.len() && pre < tb.len() && ta[pre] == tb[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < ta.len() - pre && suf < tb.len() - pre && ta[ta.len() - 1 - suf] == tb[tb.len() - 1 - suf] {
        suf += 1;
    }
    let mut same_a: Vec<bool> = (0..ta.len()).map(|i| i < pre || i >= ta.len() - suf).collect();
    let mut same_b: Vec<bool> = (0..tb.len()).map(|i| i < pre || i >= tb.len() - suf).collect();
    let mid_a = &ta[pre..ta.len() - suf];
    let mid_b = &tb[pre..tb.len() - suf];
    let cells = mid_a.len() * mid_b.len();
    let room = match &budget {
        Some(left) => **left >= cells,
        None => true,
    };
    if cells > 0 && cells <= MAX_CELLS && room {
        if let Some(left) = budget {
            *left -= cells;
        }
        common_marks(mid_a, mid_b, &mut same_a, &mut same_b, pre);
    }
    let shared = solid(&ta, &same_a, true);
    let longest = (solid(&ta, &same_a, true) + solid(&ta, &same_a, false)).max(solid(&tb, &same_b, true) + solid(&tb, &same_b, false));
    if longest > 0 && (shared as f64) / (longest as f64) < MIN_SHARED {
        return None;
    }
    let spans_a = merge_spans(a, &spans_of(&ta, &same_a));
    let spans_b = merge_spans(b, &spans_of(&tb, &same_b));
    if spans_a.is_empty() && spans_b.is_empty() {
        return None;
    }
    Some(LineSpans { a: spans_a, b: spans_b })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordRow {
    pub kind: String,
    pub text: String,
}

fn pair_blocks(rows: &[WordRow], from: usize, to: usize, out: &mut [Option<Vec<Span>>], budget: &mut usize) {
    let mut i = from;
    while i < to {
        if rows[i].kind != "-" {
            i += 1;
            continue;
        }
        let mut minus = Vec::new();
        let mut plus = Vec::new();
        while i < to && (rows[i].kind == "-" || rows[i].kind.is_empty()) {
            if rows[i].kind == "-" {
                minus.push(i);
            }
            i += 1;
        }
        while i < to && (rows[i].kind == "+" || rows[i].kind.is_empty()) {
            if rows[i].kind == "+" {
                plus.push(i);
            }
            i += 1;
        }
        for k in 0..minus.len().min(plus.len()) {
            let Some(spans) = line_spans(&rows[minus[k]].text, &rows[plus[k]].text, Some(budget)) else { continue };
            if !spans.a.is_empty() {
                out[minus[k]] = Some(spans.a);
            }
            if !spans.b.is_empty() {
                out[plus[k]] = Some(spans.b);
            }
        }
    }
}

pub fn word_spans(rows: &[WordRow]) -> Vec<Option<Vec<Span>>> {
    let mut out = vec![None; rows.len()];
    let mut budget = MAX_WORK;
    let mut start: Option<usize> = None;
    let close = |start: &mut Option<usize>, end: usize, out: &mut Vec<Option<Vec<Span>>>, budget: &mut usize| {
        if let Some(s) = *start
            && end - s <= MAX_HUNK
        {
            pair_blocks(rows, s, end, out, budget);
        }
        *start = None;
    };
    for (i, row) in rows.iter().enumerate() {
        if row.kind == "@" {
            close(&mut start, i, &mut out, &mut budget);
            start = Some(i + 1);
        } else if !matches!(row.kind.as_str(), " " | "+" | "-" | "") {
            close(&mut start, i, &mut out, &mut budget);
        }
    }
    close(&mut start, rows.len(), &mut out, &mut budget);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, text: &str) -> WordRow {
        WordRow { kind: kind.into(), text: text.into() }
    }

    #[test]
    fn tokens_split_words_spaces_and_signs() {
        assert_eq!(tokenize("let x_1 = a+b;"), vec!["let", " ", "x_1", " ", "=", " ", "a", "+", "b", ";"]);
        assert_eq!(tokenize("  é!"), vec!["  ", "é", "!"]);
    }

    #[test]
    fn changed_words_are_marked() {
        let spans = line_spans("let total = price * 2;", "let total = price * 3;", None).unwrap();
        assert_eq!(spans.a, vec![(20, 21)]);
        assert_eq!(spans.b, vec![(20, 21)]);
        assert!(line_spans("same", "same", None).is_none());
        assert!(line_spans("alpha beta gamma", "one two three", None).is_none());
        assert_eq!(merge_spans("ab cd", &[(0, 2), (3, 5)]), vec![(0, 5)]);
    }

    #[test]
    fn rows_pair_minus_and_plus() {
        let rows = vec![row("@", "@@ -1 +1 @@"), row("-", "value = 10"), row("+", "value = 20"), row(" ", "x")];
        let spans = word_spans(&rows);
        assert_eq!(spans[0], None);
        assert_eq!(spans[1], Some(vec![(8, 10)]));
        assert_eq!(spans[2], Some(vec![(8, 10)]));
        assert_eq!(spans[3], None);
    }
}
