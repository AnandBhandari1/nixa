use regex::Regex;
use std::sync::LazyLock;

const DROP_TAGS: [&str; 7] = ["script", "style", "noscript", "svg", "template", "iframe", "head"];

static DROPS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DROP_TAGS.iter().map(|tag| Regex::new(&format!(r"(?is)<{tag}\b.*?</{tag}\s*>")).expect("drop pattern")).collect()
});
static NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").expect("note pattern"));
static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:p|div|br|li|tr|h[1-6]|section|article|header|footer|nav|main|aside|ul|ol|table|blockquote|pre|hr|dt|dd|figure|figcaption)\b[^>]*>").expect("block pattern")
});
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("tag pattern"));
static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title\s*>").expect("title pattern"));
static ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z]{2,8});").expect("entity pattern"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}\r\f\v]+").expect("space pattern"));
static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("gap pattern"));

fn entity(name: &str) -> Option<String> {
    let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        u32::from_str_radix(hex, 16).ok()
    } else if let Some(dec) = name.strip_prefix('#') {
        dec.parse().ok()
    } else {
        return match name {
            "amp" => Some("&".into()),
            "lt" => Some("<".into()),
            "gt" => Some(">".into()),
            "quot" => Some("\"".into()),
            "apos" => Some("'".into()),
            "nbsp" => Some(" ".into()),
            "ndash" => Some("–".into()),
            "mdash" => Some("—".into()),
            "hellip" => Some("…".into()),
            "copy" => Some("©".into()),
            _ => None,
        };
    };
    code.and_then(char::from_u32).filter(|c| !c.is_control() || *c == '\n').map(String::from)
}

pub fn decode(text: &str) -> String {
    ENTITY.replace_all(text, |caps: &regex::Captures| entity(&caps[1]).unwrap_or_else(|| caps[0].to_string())).into_owned()
}

pub fn tidy(text: &str) -> String {
    let lines: Vec<String> = text.split('\n').map(|line| SPACES.replace_all(line, " ").trim().to_string()).collect();
    GAPS.replace_all(&lines.join("\n"), "\n\n").trim().to_string()
}

pub fn title(html: &str) -> Option<String> {
    TITLE.captures(html).map(|caps| tidy(&decode(&TAG.replace_all(&caps[1], " ")))).filter(|t| !t.is_empty())
}

pub fn html_text(html: &str) -> String {
    let mut text = NOTE.replace_all(html, " ").into_owned();
    for drop in DROPS.iter() {
        text = drop.replace_all(&text, " ").into_owned();
    }
    text = BLOCK.replace_all(&text, "\n").into_owned();
    text = TAG.replace_all(&text, " ").into_owned();
    tidy(&decode(&text))
}

pub fn is_html(content_type: &str, body: &str) -> bool {
    let kind = content_type.to_ascii_lowercase();
    kind.contains("html") || (kind.is_empty() && body.trim_start().get(..15).is_some_and(|head| head.to_ascii_lowercase().starts_with("<!doctype html") || head.to_ascii_lowercase().starts_with("<html")))
}

pub fn readable(content_type: &str) -> bool {
    let kind = content_type.to_ascii_lowercase();
    kind.is_empty() || kind.starts_with("text/") || kind.contains("html") || kind.contains("json") || kind.contains("xml")
}

pub fn window(text: &str, start: usize, length: usize) -> (String, Option<usize>) {
    let total = text.chars().count();
    let part: String = text.chars().skip(start).take(length).collect();
    let end = start.saturating_add(length);
    (part, (end < total).then_some(end))
}
