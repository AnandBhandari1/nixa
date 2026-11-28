use crate::files::normalize;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Style {
    Bold,
    Italic,
    Code,
    Strike,
    Link(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    Bullet,
    Number(String),
    Task(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Inline),
    Para(Inline),
    Quote(Inline),
    Item { depth: usize, mark: Mark, text: Inline },
    Code { info: String, text: String },
    Rule,
    Table { head: Vec<Inline>, rows: Vec<Vec<Inline>> },
    Image { alt: String, src: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Web(String),
    File(String),
    Outside,
    Ignore,
}

const FENCE_TICK: char = '`';

struct Fence {
    ch: char,
    len: usize,
    indent: usize,
    info: String,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn fence_open(line: &str) -> Option<Fence> {
    let indent = indent_of(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next().filter(|c| *c == FENCE_TICK || *c == '~')?;
    let len = rest.len() - rest.trim_start_matches(ch).len();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim().to_string();
    if ch == FENCE_TICK && info.contains(FENCE_TICK) {
        return None;
    }
    Some(Fence { ch, len, indent, info })
}

fn fence_close(line: &str, f: &Fence) -> bool {
    let indent = indent_of(line);
    if indent > 3 {
        return false;
    }
    let rest = line[indent..].trim_end();
    rest.len() >= f.len && rest.chars().all(|c| c == f.ch)
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let indent = indent_of(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.len() - rest.trim_start_matches('#').len();
    if !(1..=6).contains(&level) {
        return None;
    }
    let body = &rest[level..];
    if !body.is_empty() && !body.starts_with([' ', '\t']) {
        return None;
    }
    let body = body.trim();
    let body = body.trim_end_matches('#');
    let body = if body.ends_with(' ') || body.is_empty() { body.trim_end() } else { body.trim() };
    Some((level as u8, body))
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    let Some(c) = t.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    indent_of(line) <= 3 && t.chars().filter(|x| *x == c).count() >= 3 && t.chars().all(|x| x == c || x == ' ')
}

fn list_item(line: &str) -> Option<(usize, Mark, &str)> {
    let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[lead..];
    let (mark, after) = if let Some(r) = rest.strip_prefix(['-', '*', '+']) {
        (Mark::Bullet, r)
    } else {
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 || digits > 9 {
            return None;
        }
        let r = &rest[digits..];
        let r = r.strip_prefix(['.', ')'])?;
        (Mark::Number(rest[..digits].to_string()), r)
    };
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    let body = after.trim_start();
    let (mark, body) = match (&mark, body) {
        (Mark::Bullet, b) if b.starts_with("[ ] ") || b == "[ ]" => (Mark::Task(false), b[3..].trim_start()),
        (Mark::Bullet, b) if b.starts_with("[x] ") || b.starts_with("[X] ") || b == "[x]" || b == "[X]" => {
            (Mark::Task(true), b[3..].trim_start())
        }
        _ => (mark, body),
    };
    Some((lead / 2, mark, body))
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if c == '|' {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn is_table_rule(line: &str) -> bool {
    let cells = table_cells(line);
    line.contains('-')
        && cells.iter().all(|c| {
            let c = c.trim_start_matches(':').trim_end_matches(':');
            !c.is_empty() && c.chars().all(|x| x == '-')
        })
}

fn lone_image(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let rest = t.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let src = rest[close + 2..].strip_suffix(')')?;
    let src = src.split_whitespace().next().unwrap_or("");
    (!src.is_empty()).then(|| (alt.to_string(), src.trim_matches(['<', '>']).to_string()))
}

fn join_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        let hard = l.ends_with("  ") || l.ends_with('\\');
        let t = l.trim();
        let t = if hard { t.trim_end_matches('\\') } else { t };
        out.push_str(t);
        if i + 1 < lines.len() {
            out.push(if hard { '\n' } else { ' ' });
        }
    }
    out
}

pub fn parse(src: &str) -> Vec<Block> {
    let text = crate::doc::to_lf(src);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut i = 0;
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(inline(&join_lines(para))));
            para.clear();
        }
    };
    while i < lines.len() {
        let line = lines[i];
        if let Some(f) = fence_open(line) {
            flush(&mut para, &mut out);
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !fence_close(lines[i], &f) {
                let l = lines[i];
                let strip = indent_of(l).min(f.indent);
                body.push(&l[strip..]);
                i += 1;
            }
            out.push(Block::Code { info: f.info, text: body.join("\n") });
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut para, &mut out);
            i += 1;
            continue;
        }
        if !para.is_empty() {
            let t = line.trim();
            if indent_of(line) <= 3 && !t.is_empty() && t.chars().all(|c| c == '=') {
                out.push(Block::Heading(1, inline(&join_lines(&para))));
                para.clear();
                i += 1;
                continue;
            }
            if indent_of(line) <= 3 && t.len() >= 2 && t.chars().all(|c| c == '-') {
                out.push(Block::Heading(2, inline(&join_lines(&para))));
                para.clear();
                i += 1;
                continue;
            }
        }
        if let Some((level, body)) = heading(line) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(level, inline(body)));
            i += 1;
            continue;
        }
        if is_rule(line) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if para.is_empty() && (line.starts_with("    ") || line.starts_with('\t')) {
            let mut body = Vec::new();
            while i < lines.len() && (lines[i].starts_with("    ") || lines[i].starts_with('\t') || lines[i].trim().is_empty()) {
                let l = lines[i];
                body.push(l.strip_prefix("    ").or_else(|| l.strip_prefix('\t')).unwrap_or(""));
                i += 1;
            }
            while body.last().is_some_and(|l| l.trim().is_empty()) {
                body.pop();
            }
            out.push(Block::Code { info: String::new(), text: body.join("\n") });
            continue;
        }
        if indent_of(line) <= 3 && line.trim_start().starts_with('>') {
            flush(&mut para, &mut out);
            let mut body = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let t = lines[i].trim_start().trim_start_matches('>');
                body.push(t.strip_prefix(' ').unwrap_or(t));
                i += 1;
            }
            out.push(Block::Quote(inline(&join_lines(&body))));
            continue;
        }
        if let Some((depth, mark, body)) = list_item(line) {
            flush(&mut para, &mut out);
            let mut parts = vec![body];
            i += 1;
            while i < lines.len() {
                let l = lines[i];
                if l.trim().is_empty() || list_item(l).is_some() || fence_open(l).is_some() || heading(l).is_some() {
                    break;
                }
                parts.push(l);
                i += 1;
            }
            out.push(Block::Item { depth, mark, text: inline(&join_lines(&parts)) });
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && is_table_rule(lines[i + 1]) {
            flush(&mut para, &mut out);
            let head = table_cells(line).iter().map(|c| inline(c)).collect();
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(table_cells(lines[i]).iter().map(|c| inline(c)).collect());
                i += 1;
            }
            out.push(Block::Table { head, rows });
            continue;
        }
        if para.is_empty() && let Some((alt, src)) = lone_image(line) {
            out.push(Block::Image { alt, src });
            i += 1;
            continue;
        }
        para.push(line);
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

fn push_inner(out: &mut Inline, inner: Inline, style: Option<Style>) {
    let base = out.text.len();
    out.text.push_str(&inner.text);
    for s in inner.spans {
        out.spans.push(Span { range: s.range.start + base..s.range.end + base, style: s.style });
    }
    if let Some(style) = style {
        out.spans.push(Span { range: base..out.text.len(), style });
    }
}

fn find_close(s: &str, from: usize, delim: &str) -> Option<usize> {
    let mut i = from;
    while let Some(at) = s[i..].find(delim) {
        let pos = i + at;
        if pos > from && !s[..pos].ends_with('\\') && !s[..pos].ends_with(' ') {
            return Some(pos);
        }
        i = pos + delim.len();
    }
    None
}

fn bracket_end(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut skip = false;
    for (i, c) in s[open..].char_indices() {
        if skip {
            skip = false;
            continue;
        }
        match c {
            '\\' => skip = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn link_tail(s: &str, close: usize) -> Option<(String, usize)> {
    let rest = s[close + 1..].strip_prefix('(')?;
    let end = rest.find(')')?;
    let inside = rest[..end].trim();
    let url = inside.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']);
    Some((url.to_string(), close + 1 + 1 + end + 1))
}

fn word_char_before(s: &str, i: usize) -> bool {
    s[..i].chars().next_back().is_some_and(char::is_alphanumeric)
}

pub fn inline(s: &str) -> Inline {
    let mut out = Inline::default();
    let mut i = 0;
    let b = s.as_bytes();
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if c == '\\' && let Some(n) = rest[1..].chars().next() && n.is_ascii_punctuation() {
            out.text.push(n);
            i += 1 + n.len_utf8();
            continue;
        }
        if c == FENCE_TICK {
            let run = rest.len() - rest.trim_start_matches(FENCE_TICK).len();
            let ticks = &rest[..run];
            if let Some(end) = rest[run..].find(ticks) {
                let body = &rest[run..run + end];
                let body = if body.len() > 1 && body.starts_with(' ') && body.ends_with(' ') { &body[1..body.len() - 1] } else { body };
                push_inner(&mut out, Inline { text: body.to_string(), spans: vec![] }, Some(Style::Code));
                i += run + end + run;
                continue;
            }
            out.text.push_str(ticks);
            i += run;
            continue;
        }
        let image = rest.starts_with("![");
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = bracket_end(s, open)
                && let Some((url, end)) = link_tail(s, close)
            {
                let label = inline(&s[open + 1..close]);
                push_inner(&mut out, label, Some(Style::Link(url)));
                i = end;
                continue;
            }
        }
        if c == '<' && (rest.starts_with("<http://") || rest.starts_with("<https://")) && let Some(end) = rest.find('>') {
            let url = &rest[1..end];
            push_inner(&mut out, Inline { text: url.to_string(), spans: vec![] }, Some(Style::Link(url.to_string())));
            i += end + 1;
            continue;
        }
        if (rest.starts_with("http://") || rest.starts_with("https://")) && !word_char_before(s, i) {
            let len = rest.find(|c: char| c.is_whitespace() || c == '<').unwrap_or(rest.len());
            let url = rest[..len].trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
            push_inner(&mut out, Inline { text: url.to_string(), spans: vec![] }, Some(Style::Link(url.to_string())));
            i += url.len();
            continue;
        }
        let pair = |d: &str| rest.starts_with(d) && !rest[d.len()..].starts_with(' ');
        let mut done = false;
        for (d, style) in [("**", Style::Bold), ("__", Style::Bold), ("~~", Style::Strike)] {
            if pair(d) && !(d == "__" && word_char_before(s, i)) && let Some(end) = find_close(s, i + 2, d) {
                let inner = inline(&s[i + 2..end]);
                push_inner(&mut out, inner, Some(style));
                i = end + 2;
                done = true;
                break;
            }
        }
        if done {
            continue;
        }
        if (c == '*' || c == '_') && pair(if c == '*' { "*" } else { "_" }) && !(c == '_' && word_char_before(s, i)) {
            let d = if c == '*' { "*" } else { "_" };
            let mut from = i + 1;
            let mut found = None;
            while let Some(end) = find_close(s, from, d) {
                let doubled = b.get(end + 1) == Some(&(c as u8));
                let glued = c == '_' && s[end + 1..].chars().next().is_some_and(char::is_alphanumeric);
                if !doubled && !glued {
                    found = Some(end);
                    break;
                }
                from = end + if doubled { 2 } else { 1 };
            }
            if let Some(end) = found {
                let inner = inline(&s[i + 1..end]);
                push_inner(&mut out, inner, Some(Style::Italic));
                i = end + 1;
                continue;
            }
        }
        out.text.push(c);
        i += c.len_utf8();
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn link_target(url: &str, folder: &str) -> Link {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Link::Web(url.to_string());
    }
    let scheme = url.find(':').is_some_and(|i| {
        i > 0 && url[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) && url.as_bytes()[0].is_ascii_alphabetic()
    });
    if scheme && !lower.starts_with("file:") {
        return Link::Ignore;
    }
    let raw = if lower.starts_with("file://") { &url[7..] } else if lower.starts_with("file:") { &url[5..] } else { url };
    let raw = raw.split(['#', '?']).next().unwrap_or("");
    let Some(raw) = percent_decode(raw) else { return Link::Ignore };
    if raw.is_empty() {
        return Link::Ignore;
    }
    let joined = if raw.starts_with('/') { normalize(&raw) } else { normalize(&format!("{folder}/{raw}")) };
    let folder = normalize(folder);
    if folder != "/" && joined != folder && !joined.starts_with(&format!("{folder}/")) {
        return Link::Outside;
    }
    Link::File(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(i: &Inline) -> Vec<(String, Style)> {
        i.spans.iter().map(|s| (i.text[s.range.clone()].to_string(), s.style.clone())).collect()
    }

    #[test]
    fn blocks() {
        let src = "# Title\n\nSome *text*\nmore.\n\n- one\n- [x] done\n  still done\n1. first\n\n> quoted\n> again\n\n---\n\n```rust\nfn a() {}\n```\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n\n![pic](img/a.png)\n\nSub\n===\n";
        let b = parse(src);
        assert_eq!(b[0], Block::Heading(1, inline("Title")));
        assert!(matches!(&b[1], Block::Para(i) if i.text == "Some text more."));
        assert!(matches!(&b[2], Block::Item { mark: Mark::Bullet, .. }));
        assert!(matches!(&b[3], Block::Item { mark: Mark::Task(true), text, .. } if text.text == "done still done"));
        assert!(matches!(&b[4], Block::Item { mark: Mark::Number(n), .. } if n == "1"));
        assert!(matches!(&b[5], Block::Quote(i) if i.text == "quoted again"));
        assert_eq!(b[6], Block::Rule);
        assert_eq!(b[7], Block::Code { info: "rust".into(), text: "fn a() {}".into() });
        assert!(matches!(&b[8], Block::Table { head, rows } if head.len() == 2 && rows.len() == 1));
        assert_eq!(b[9], Block::Image { alt: "pic".into(), src: "img/a.png".into() });
        assert_eq!(b[10], Block::Heading(1, inline("Sub")));
        assert_eq!(b.len(), 11);
    }

    #[test]
    fn open_fence_runs_to_the_end() {
        let b = parse("```\na\nb");
        assert_eq!(b, vec![Block::Code { info: String::new(), text: "a\nb".into() }]);
    }

    #[test]
    fn inline_styles() {
        let i = inline("a **bold _it_** `co*de` ~~x~~ [link](https://x.y) snake_case_name");
        assert_eq!(i.text, "a bold it co*de x link snake_case_name");
        let s = styles(&i);
        assert!(s.contains(&("it".into(), Style::Italic)));
        assert!(s.contains(&("bold it".into(), Style::Bold)));
        assert!(s.contains(&("co*de".into(), Style::Code)));
        assert!(s.contains(&("x".into(), Style::Strike)));
        assert!(s.contains(&("link".into(), Style::Link("https://x.y".into()))));
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn bare_urls_and_escapes() {
        let i = inline("see https://a.b/c. and \\*not\\*");
        assert_eq!(i.text, "see https://a.b/c. and *not*");
        assert_eq!(styles(&i), vec![("https://a.b/c".into(), Style::Link("https://a.b/c".into()))]);
    }

    #[test]
    fn links_stay_in_the_folder() {
        assert_eq!(link_target("https://x.y", "/d"), Link::Web("https://x.y".into()));
        assert_eq!(link_target("mailto:a@b", "/d"), Link::Ignore);
        assert_eq!(link_target("#part", "/d"), Link::Ignore);
        assert_eq!(link_target("b/c.md#x", "/d"), Link::File("/d/b/c.md".into()));
        assert_eq!(link_target("my%20file.md", "/d"), Link::File("/d/my file.md".into()));
        assert_eq!(link_target("../e.md", "/d"), Link::Outside);
        assert_eq!(link_target("file:///d/x.md", "/d"), Link::File("/d/x.md".into()));
        assert_eq!(link_target("/etc/passwd", "/d"), Link::Outside);
    }
}
