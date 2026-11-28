use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

pub const CHUNK_CHARS: usize = 900;
pub const CHUNK_OVERLAP: usize = 120;
pub const QUERY_WORDS: usize = 12;
const MIN_SECTION: usize = 20;

const QUERY_SKIP: [&str; 45] = [
    "the", "an", "and", "or", "of", "to", "in", "on", "at", "for", "is", "are", "was", "be", "it", "this", "that", "with", "from", "by", "as", "how", "do", "does", "did", "what", "where", "when", "which", "who", "why", "me", "my", "we", "you", "your", "can", "there", "about", "am", "i", "if", "so", "no", "not",
];

static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]{2,}").expect("word pattern"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub title: String,
    pub text: String,
}

pub fn strip_front_matter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return text };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return &rest[at..];
        }
    }
    text
}

fn heading_of(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=4).contains(&hashes) {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(str::trim).filter(|h| !h.is_empty())
}

fn sections(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 && heading_of(line).is_some() {
            out.push(std::mem::take(&mut current));
        } else if n > 0 {
            current.push('\n');
        }
        current.push_str(line);
    }
    out.push(current);
    out
}

pub fn chunk_markdown(text: &str, title: &str) -> Vec<Part> {
    let mut out = Vec::new();
    for section in sections(strip_front_matter(text)) {
        let body = section.trim();
        let name = match body.lines().next().and_then(heading_of) {
            Some(heading) => format!("{title} › {heading}"),
            None => title.to_string(),
        };
        let chars: Vec<char> = body.chars().collect();
        if chars.len() < MIN_SECTION {
            continue;
        }
        let mut at = 0;
        loop {
            let end = (at + CHUNK_CHARS).min(chars.len());
            out.push(Part { title: name.clone(), text: chars[at..end].iter().collect() });
            if end >= chars.len() {
                break;
            }
            at += CHUNK_CHARS - CHUNK_OVERLAP;
        }
    }
    out
}

pub fn fts_query(query: &str) -> String {
    let lower = query.to_lowercase();
    let mut seen = HashSet::new();
    let words: Vec<&str> = WORD.find_iter(&lower).map(|m| m.as_str()).filter(|w| seen.insert(*w)).collect();
    let kept: Vec<&str> = words.iter().copied().filter(|w| !QUERY_SKIP.contains(w)).collect();
    let pick = if kept.is_empty() { words } else { kept };
    pick.iter().take(QUERY_WORDS).map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" OR ")
}
