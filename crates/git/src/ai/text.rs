pub const MAX_CLIP: usize = 24000;

fn csi_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ';' || chars[i] == '?') {
        i += 1;
    }
    while i < chars.len() && ('\u{20}'..='\u{2f}').contains(&chars[i]) {
        i += 1;
    }
    let last = *chars.get(i)?;
    ('\u{40}'..='\u{7e}').contains(&last).then_some(i + 1)
}

fn osc_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\u{7}' => return Some(i + 1),
            '\u{1b}' => return (chars.get(i + 1) == Some(&'\\')).then_some(i + 2),
            _ => i += 1,
        }
    }
    None
}

fn single(c: char) -> bool {
    ('@'..='Z').contains(&c) || ('\\'..='_').contains(&c)
}

pub fn strip_ansi(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\u{1b}' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        let end = match next {
            Some('[') => csi_end(&chars, i + 2),
            Some(']') => osc_end(&chars, i + 2).or(Some(i + 2)),
            Some(c) if single(c) => Some(i + 2),
            _ => None,
        };
        match end {
            Some(end) => i = end,
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn fence_open(body: &str) -> Option<usize> {
    let rest = body.strip_prefix("```")?;
    let lang = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '+' || c == '-'))
        .unwrap_or(rest.len());
    let after = &rest[lang..];
    let gap = after.find(|c: char| c != ' ' && c != '\t').unwrap_or(after.len());
    after[gap..].starts_with('\n').then_some(3 + lang + gap + 1)
}

pub fn clean_output(text: &str) -> String {
    let plain = strip_ansi(text).replace("\r\n", "\n");
    let lines: Vec<&str> = plain
        .split('\n')
        .map(|line| line.rsplit('\r').next().unwrap_or(""))
        .collect();
    let joined = lines.join("\n");
    let body = joined.trim();
    let Some(open) = fence_open(body) else {
        return body.to_string();
    };
    if body.len() >= open + 3 && body.ends_with("```") {
        let inner = &body[open..body.len() - 3];
        let inner = inner.strip_suffix('\n').unwrap_or(inner);
        return inner.trim().to_string();
    }
    body[open..].trim().to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub summary: String,
    pub body: String,
}

fn unquote(line: &str) -> &str {
    match line.strip_prefix('`').and_then(|l| l.strip_suffix('`')) {
        Some(inner) if !inner.is_empty() && !inner.contains('`') => inner,
        _ => line,
    }
}

pub fn split_message(text: &str) -> Message {
    let cleaned = clean_output(text);
    let lines: Vec<&str> = cleaned.split('\n').collect();
    let Some(at) = lines.iter().position(|l| !l.trim().is_empty()) else {
        return Message::default();
    };
    let summary = unquote(lines[at].trim()).trim().to_string();
    let rest = &lines[at + 1..];
    let skip = rest.iter().take_while(|l| l.trim_matches([' ', '\t']).is_empty()).count();
    let body = rest[skip..].join("\n").trim_end().to_string();
    Message { summary, body }
}

const USEFUL: &[&str] = &[
    "error",
    "fail",
    "denied",
    "invalid",
    "not found",
    "not logged",
    "not authenticated",
    "not allowed",
    "not supported",
    "not installed",
    "unauthorized",
    "unauthorised",
    "forbidden",
    "quota",
    "rate limit",
    "expired",
    "login",
    "log in",
    "signin",
    "sign in",
    "unknown",
    "no such",
    "cannot",
    "can't",
    "could not",
    "refused",
    "unavailable",
];

fn word_start(line: &str, word: &str) -> bool {
    let lower = line.to_lowercase();
    lower.starts_with(word)
        && !lower[word.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn useful(line: &str) -> bool {
    let lower = line.to_lowercase();
    USEFUL.iter().any(|w| lower.contains(w))
}

fn dated(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() >= 11
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && (b[10] == b'T' || b[10] == b' ')
}

fn noise(line: &str) -> bool {
    ["warning", "warn", "info", "debug", "note", "hint"]
        .iter()
        .any(|w| word_start(line, w))
        || (line.len() > 2 && line.starts_with("at") && line[2..].starts_with(char::is_whitespace))
        || dated(line)
}

fn warns(line: &str) -> bool {
    word_start(line, "warn") || word_start(line, "warning")
}

fn drop_error_word(line: &str) -> &str {
    for word in ["error", "fatal"] {
        if line.get(..word.len()).is_some_and(|head| head.eq_ignore_ascii_case(word)) {
            let rest = line[word.len()..].trim_start();
            if let Some(after) = rest.strip_prefix(':') {
                return after.trim_start();
            }
        }
    }
    line
}

pub fn first_useful_line(text: &str) -> String {
    let plain = strip_ansi(text);
    let lines: Vec<&str> = plain
        .split('\n')
        .map(|l| l.trim_end_matches('\r').trim())
        .filter(|l| !l.is_empty())
        .collect();
    let line = lines
        .iter()
        .find(|l| useful(l) && !warns(l))
        .or_else(|| lines.iter().find(|l| !noise(l)))
        .or_else(|| lines.first())
        .copied()
        .unwrap_or("");
    drop_error_word(line).chars().take(300).collect()
}

pub fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}\n…(truncated)", &text[..at]),
        None => text.to_string(),
    }
}

pub fn compose(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_and_title_codes() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}text"), "text");
        assert_eq!(strip_ansi("\u{1b}]8;;x\u{1b}\\link"), "link");
        assert_eq!(strip_ansi("a\u{1b}Mb"), "ab");
        assert_eq!(strip_ansi("a\u{1b}[b"), "a");
        assert_eq!(strip_ansi("a\u{1b}["), "a\u{1b}[");
    }

    #[test]
    fn cleans_fences_and_carriage_returns() {
        assert_eq!(clean_output("```text\nfix: a\n```"), "fix: a");
        assert_eq!(clean_output("  ```\nfix: b\n\nbody"), "fix: b\n\nbody");
        assert_eq!(clean_output("loading\rdone\r\nnext"), "done\nnext");
        assert_eq!(clean_output("plain"), "plain");
        assert_eq!(clean_output("```\n```"), "");
    }

    #[test]
    fn splits_summary_and_body() {
        let m = split_message("\n\n`feat: add x`\n\n\nWhy it was needed.\n");
        assert_eq!(m.summary, "feat: add x");
        assert_eq!(m.body, "Why it was needed.");
        assert_eq!(split_message("  \n "), Message::default());
        assert_eq!(split_message("fix: one").body, "");
    }

    #[test]
    fn finds_the_useful_line() {
        assert_eq!(first_useful_line("warning: old\nError: not logged in\n"), "not logged in");
        assert_eq!(first_useful_line("info start\n2026-11-22T10:00 x\nplain line"), "plain line");
        assert_eq!(first_useful_line("debug only"), "debug only");
        assert_eq!(first_useful_line("fatal : quota used"), "quota used");
        assert_eq!(first_useful_line(""), "");
        assert_eq!(first_useful_line("    at foo (x.js)\nhello"), "hello");
    }

    #[test]
    fn clips_long_text() {
        assert_eq!(clip("abcdef", 3), "abc\n…(truncated)");
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("ééé", 2), "éé\n…(truncated)");
    }

    #[test]
    fn composes_parts() {
        assert_eq!(compose(&["a", "", "b"]), "a\n\nb");
    }
}
