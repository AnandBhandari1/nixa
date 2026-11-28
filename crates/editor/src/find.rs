pub const MATCH_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

pub fn matches(text: &str, query: &str) -> Vec<Match> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let hay = text.as_bytes();
    let needle: Vec<u8> = query.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let n = needle.len();
    let mut line = 1usize;
    let mut counted = 0usize;
    let mut i = 0usize;
    while i + n <= hay.len() && out.len() < MATCH_LIMIT {
        let hit = hay[i..i + n].iter().zip(&needle).all(|(a, b)| a.to_ascii_lowercase() == *b);
        if hit && text.is_char_boundary(i) && text.is_char_boundary(i + n) {
            line += hay[counted..i].iter().filter(|b| **b == b'\n').count();
            counted = i;
            out.push(Match { start: i, end: i + n, line });
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

pub fn count_label(query: &str, found: &[Match], index: Option<usize>) -> String {
    if query.is_empty() {
        return String::new();
    }
    match index.and_then(|i| found.get(i).map(|m| (i, m))) {
        _ if found.is_empty() => "No match".into(),
        Some((i, m)) => format!("{}/{} · Ln {}", i + 1, found.len(), m.line),
        None => found.len().to_string(),
    }
}

pub fn step(index: Option<usize>, len: usize, by: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let at = index.map_or(if by < 0 { 0 } else { len - 1 }, |i| i.min(len - 1)) as isize;
    Some((at + by).rem_euclid(len as isize) as usize)
}

pub fn nearest(found: &[Match], offset: usize) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    Some(found.iter().position(|m| m.start >= offset).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_without_case_and_counts_lines() {
        let m = matches("Foo\nbar foo\nFOO", "foo");
        assert_eq!(m.iter().map(|m| (m.start, m.line)).collect::<Vec<_>>(), vec![(0, 1), (8, 2), (12, 3)]);
        assert!(matches("abc", "").is_empty());
        assert!(matches("ab", "abc").is_empty());
    }

    #[test]
    fn keeps_to_char_bounds_and_limit() {
        let m = matches("é é", "é");
        assert_eq!(m.len(), 2);
        let many = "a".repeat(2000);
        assert_eq!(matches(&many, "a").len(), MATCH_LIMIT);
        assert_eq!(matches("aaaa", "aa").len(), 2);
    }

    #[test]
    fn labels_and_steps() {
        let m = matches("x\nx\nx", "x");
        assert_eq!(count_label("x", &m, Some(1)), "2/3 · Ln 2");
        assert_eq!(count_label("y", &[], None), "No match");
        assert_eq!(count_label("", &[], None), "");
        assert_eq!(step(Some(2), 3, 1), Some(0));
        assert_eq!(step(Some(0), 3, -1), Some(2));
        assert_eq!(step(None, 3, 1), Some(0));
        assert_eq!(step(None, 0, 1), None);
        assert_eq!(nearest(&m, 3), Some(2));
        assert_eq!(nearest(&m, 100), Some(0));
    }
}
