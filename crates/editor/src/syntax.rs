use crate::lang::{Family, Language};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Str,
    Comment,
    Number,
    Type,
    Func,
    Tag,
    Attr,
    Heading,
    Added,
    Removed,
}

pub type Toks = Vec<(Range<usize>, Tok)>;

fn block_marks(f: Family) -> Option<(&'static str, &'static str)> {
    match f {
        Family::CLike | Family::Sql => Some(("/*", "*/")),
        Family::Markup | Family::Markdown => Some(("<!--", "-->")),
        Family::Lua => Some(("--[[", "]]")),
        Family::Haskell => Some(("{-", "-}")),
        _ => None,
    }
}

fn line_mark(f: Family) -> &'static [&'static str] {
    match f {
        Family::CLike => &["//"],
        Family::Hash => &["#"],
        Family::Sql | Family::Lua | Family::Haskell => &["--"],
        Family::Lisp => &[";"],
        Family::Ini => &[";", "#"],
        _ => &[],
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$' || c == '@'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn string_end(line: &str, from: usize, q: char, short: bool) -> Option<usize> {
    if short {
        let mut chars = line[from + 1..].char_indices();
        let (_, first) = chars.next()?;
        if first != '\\' {
            let (i, second) = chars.next()?;
            return (second == q).then_some(from + 1 + i + 1);
        }
    }
    let mut escaped = false;
    for (i, c) in line[from + 1..].char_indices() {
        if short && i > 8 {
            return None;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == q {
            return Some(from + 1 + i + c.len_utf8());
        }
    }
    if short { None } else { Some(line.len()) }
}

fn markdown_line(line: &str, out: &mut Toks) {
    let t = line.trim_start();
    let lead = line.len() - t.len();
    if t.starts_with('#') {
        out.push((0..line.len(), Tok::Heading));
        return;
    }
    if t.starts_with("```") || t.starts_with("~~~") {
        out.push((0..line.len(), Tok::Comment));
        return;
    }
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") || t.starts_with('>') {
        out.push((lead..lead + 1, Tok::Keyword));
    }
    let mut i = 0;
    while let Some(at) = line[i..].find('`') {
        let s = i + at;
        match line[s + 1..].find('`') {
            Some(e) => {
                out.push((s..s + 1 + e + 1, Tok::Str));
                i = s + e + 2;
            }
            None => break,
        }
    }
}

fn markup_line(line: &str, start: usize, out: &mut Toks) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = start;
    let mut in_tag = false;
    while i < line.len() {
        if line[i..].starts_with("<!--") {
            return Some(i);
        }
        let c = b[i];
        if c == b'<' {
            let name_start = i + 1 + usize::from(b.get(i + 1) == Some(&b'/'));
            let len = line[name_start..].find(|c: char| !(c.is_alphanumeric() || c == '-' || c == ':' || c == '.')).unwrap_or(line.len() - name_start);
            if len > 0 {
                out.push((i..name_start + len, Tok::Tag));
                in_tag = true;
                i = name_start + len;
                continue;
            }
        } else if c == b'>' && in_tag {
            in_tag = false;
        } else if in_tag && (c == b'"' || c == b'\'') {
            let end = string_end(line, i, c as char, false).unwrap_or(line.len());
            out.push((i..end, Tok::Str));
            i = end;
            continue;
        } else if in_tag && (c as char).is_ascii_alphabetic() {
            let len = line[i..].find(|c: char| !(c.is_alphanumeric() || c == '-' || c == ':')).unwrap_or(line.len() - i);
            out.push((i..i + len, Tok::Attr));
            i += len;
            continue;
        }
        i += 1;
    }
    None
}

pub fn line(line: &str, lang: &Language, in_block: bool) -> (Toks, bool) {
    let mut out = Toks::new();
    let f = lang.family;
    let block = block_marks(f);
    let mut i = 0;
    if in_block {
        let (_, close) = block.unwrap_or(("", ""));
        match line.find(close) {
            Some(e) if !close.is_empty() => {
                out.push((0..e + close.len(), Tok::Comment));
                i = e + close.len();
            }
            _ => {
                out.push((0..line.len(), Tok::Comment));
                return (out, true);
            }
        }
    }
    match f {
        Family::Plain => return (out, false),
        Family::Diff => {
            let tok = match line.as_bytes().first() {
                Some(b'+') => Some(Tok::Added),
                Some(b'-') => Some(Tok::Removed),
                Some(b'@') => Some(Tok::Func),
                _ => None,
            };
            if let Some(t) = tok {
                out.push((0..line.len(), t));
            }
            return (out, false);
        }
        Family::Markdown => {
            markdown_line(line, &mut out);
            return (out, false);
        }
        Family::Markup => {
            let mut from = i;
            loop {
                match markup_line(line, from, &mut out) {
                    None => return (out, false),
                    Some(s) => match line[s + 4..].find("-->") {
                        Some(e) => {
                            out.push((s..s + 4 + e + 3, Tok::Comment));
                            from = s + 4 + e + 3;
                        }
                        None => {
                            out.push((s..line.len(), Tok::Comment));
                            return (out, true);
                        }
                    },
                }
            }
        }
        Family::Ini => {
            let t = line.trim_start();
            let lead = line.len() - t.len();
            if t.starts_with(';') || t.starts_with('#') {
                out.push((lead..line.len(), Tok::Comment));
            } else if t.starts_with('[') {
                out.push((lead..line.len(), Tok::Keyword));
            } else if let Some(eq) = line.find(['=', ':']) {
                out.push((lead..eq, Tok::Attr));
            }
            return (out, false);
        }
        _ => {}
    }
    let ci = f == Family::Sql;
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if let Some((open, close)) = block && rest.starts_with(open) {
            match rest[open.len()..].find(close) {
                Some(e) => {
                    let end = i + open.len() + e + close.len();
                    out.push((i..end, Tok::Comment));
                    i = end;
                    continue;
                }
                None => {
                    out.push((i..line.len(), Tok::Comment));
                    return (out, true);
                }
            }
        }
        if line_mark(f).iter().any(|m| rest.starts_with(m)) {
            out.push((i..line.len(), Tok::Comment));
            break;
        }
        if c == '"' || c == '`' || (c == '\'' && f != Family::Lisp) {
            let short = c == '\'' && f == Family::CLike;
            if let Some(end) = string_end(line, i, c, short) {
                let tok = if f == Family::Json && line[end..].trim_start().starts_with(':') { Tok::Attr } else { Tok::Str };
                out.push((i..end, tok));
                i = end;
                continue;
            }
        }
        if c.is_ascii_digit() && !line[..i].chars().next_back().is_some_and(is_ident) {
            let len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_')).unwrap_or(rest.len());
            out.push((i..i + len, Tok::Number));
            i += len;
            continue;
        }
        if is_ident_start(c) {
            let len = rest.find(|c: char| !is_ident(c) && c != '@').unwrap_or(rest.len());
            let word = &rest[..len];
            let kw = if ci {
                lang.keywords.iter().any(|k| k.eq_ignore_ascii_case(word))
            } else {
                lang.keywords.contains(&word)
            };
            let tok = if kw {
                Some(Tok::Keyword)
            } else if rest[len..].starts_with('(') || rest[len..].starts_with("!(") {
                Some(Tok::Func)
            } else if f == Family::CLike && word.chars().next().is_some_and(char::is_uppercase) {
                Some(Tok::Type)
            } else {
                None
            };
            if let Some(t) = tok {
                out.push((i..i + len, t));
            }
            i += len;
            continue;
        }
        i += c.len_utf8();
    }
    (out, false)
}

pub fn block_states<'a>(lines: impl Iterator<Item = &'a str>, lang: &Language) -> Vec<bool> {
    let mut out = Vec::new();
    let mut state = false;
    if block_marks(lang.family).is_none() {
        return out;
    }
    for l in lines {
        out.push(state);
        state = line(l, lang, state).1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::language;

    fn names(l: &str, path: &str) -> Vec<(String, Tok)> {
        let lang = language(path);
        line(l, &lang, false).0.into_iter().map(|(r, t)| (l[r].to_string(), t)).collect()
    }

    #[test]
    fn rust_line() {
        let got = names("pub fn go(x: Foo) -> u8 { 42 } // done", "/a.rs");
        assert_eq!(
            got,
            vec![
                ("pub".into(), Tok::Keyword),
                ("fn".into(), Tok::Keyword),
                ("go".into(), Tok::Func),
                ("Foo".into(), Tok::Type),
                ("42".into(), Tok::Number),
                ("// done".into(), Tok::Comment),
            ]
        );
        assert_eq!(names("fn a<'a>(x: &'a str)", "/a.rs").iter().filter(|t| t.1 == Tok::Str).count(), 0);
        assert_eq!(names("let c = 'x';", "/a.rs")[1], ("'x'".into(), Tok::Str));
    }

    #[test]
    fn block_comments_cross_lines() {
        let lang = language("/a.c");
        let text = "a /* one\ntwo\nthree */ b\nc";
        let states = block_states(text.split('\n'), &lang);
        assert_eq!(states, vec![false, true, true, false]);
        let (t, open) = line("three */ b", &lang, true);
        assert_eq!(t[0], (0..8, Tok::Comment));
        assert!(!open);
    }

    #[test]
    fn json_keys_and_python_comments() {
        let got = names(r#""name": "nixa", "n": 1, "ok": true"#, "/a.json");
        assert_eq!(got[0], ("\"name\"".into(), Tok::Attr));
        assert_eq!(got[1], ("\"nixa\"".into(), Tok::Str));
        assert!(got.contains(&("true".into(), Tok::Keyword)));
        assert_eq!(names("x = 1  # note", "/a.py").last().unwrap(), &("# note".into(), Tok::Comment));
    }

    #[test]
    fn markup_and_markdown() {
        let got = names(r#"<a href="x">hi</a>"#, "/a.html");
        assert_eq!(got[0], ("<a".into(), Tok::Tag));
        assert_eq!(got[1], ("href".into(), Tok::Attr));
        assert_eq!(got[2], ("\"x\"".into(), Tok::Str));
        assert_eq!(names("# Title", "/a.md")[0].1, Tok::Heading);
        assert_eq!(names("+ added", "/a.diff")[0].1, Tok::Added);
        assert!(names("plain words", "/a.txt").is_empty());
    }
}
