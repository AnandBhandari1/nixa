use crate::review_flow::checks::quoted;
use crate::risk::risky_text;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const LISTED_FILES: usize = 3;
const LISTED_GROUPS: usize = 3;
const STEP_CHARS: usize = 240;

fn re(text: &str) -> Regex {
    Regex::new(text).expect("shadow pattern")
}

static NEXT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:want me to|would you like me to|do you want me to|should I|shall I|I can (?:also |now )?\w+|next steps?\b|next,? (?:I|we|you) (?:can|could|should)|you (?:can|could|should|may) (?:now )?(?:run|use|start|try|restart)|ready to (?:run|start|continue|go on)|run \x60[^\x60]+\x60)"));
static OFFER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:want me to|would you like me to|do you want me to|shall I|should I)\b"));
static SPLIT_OR: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:or|versus|rather than|instead of)\b"));
static WORK_VERB: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:send|start|run|fix|check|test|review|continue|next batch|update|read|open|search|commit|merge|rebase|stash)\b"));
static ASKS_BACK: LazyLock<Regex> = LazyLock::new(|| re(r#"\?["')\]]?$"#));
static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\x60\x60\x60.*?\x60\x60\x60"));
static CLAUSE: LazyLock<Regex> = LazyLock::new(|| re(r"[.!?;]\s+|,\s*"));
static POLITE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:please\s+)?(?:can|could|would|will)\s+you\s+(?:please\s+)?"));
static WANT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:I (?:want|need) you to|I'd like you to)\s+"));
static ALSO: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:please|also)\s+"));
static GO_AHEAD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^go ahead(?:\s+with\b|:)?\s+"));
static WORK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:fix|repair|build|implement|add|remove|delete|refactor|test|check|review|run|update|write|create|configure|install|investigate|debug|find|search|read|open|rename|move|replace|migrate|deploy|publish|commit|push|send|ask|have|start|continue|finish|complete|make|use|set up|work on)\s+\S"));
static CHROME: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [r"Cursor Agent v\S+", r"Tip: [^.\n]*\.?", r"[\u{2584}\u{2580}]{8,}\u{2026}?", r"\u{2192} Add a follow-up", r"ctrl\+c to stop", r"[\u{2800}-\u{28ff}]+ (?:Composing|Thinking|Generating)"].iter().map(|p| re(p)).collect()
});

const YES_WORDS: [&str; 16] = ["yes", "y", "ok", "okay", "sure", "yep", "yeah", "go", "ahead", "please", "do", "it", "fine", "alright", "proceed", "approved"];

pub fn plain_yes(words: &str) -> bool {
    let text = words.trim().to_lowercase();
    let text = text.trim_end_matches(['.', '!']);
    if text.is_empty() || text.contains('?') {
        return false;
    }
    let list: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
    !list.is_empty() && list.len() <= 5 && list.iter().all(|w| YES_WORDS.contains(w))
}

fn clip(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > limit { format!("{}\u{2026}", flat.chars().take(limit).collect::<String>()) } else { flat }
}

pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut start = 0;
        let bytes: Vec<(usize, char)> = line.char_indices().collect();
        for (i, (at, c)) in bytes.iter().enumerate() {
            if matches!(c, '.' | '!' | '?') && bytes.get(i + 1).is_some_and(|(_, n)| n.is_whitespace()) {
                let end = at + c.len_utf8();
                out.push(line[start..end].trim().to_string());
                start = end;
            }
        }
        out.push(line[start..].trim().to_string());
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

pub fn without_chrome(text: &str) -> String {
    CHROME.iter().fold(text.to_string(), |acc, pattern| pattern.replace_all(&acc, " ").into_owned())
}

pub fn next_step(text: &str) -> Option<String> {
    let clean = FENCE.replace_all(text, " ");
    let parts = sentences(&clean);
    for index in (0..parts.len()).rev() {
        let line = &parts[index];
        if NEXT.is_match(line) {
            let picked = if line.ends_with(':') { parts[index..(index + 4).min(parts.len())].join(" ") } else { line.clone() };
            return Some(clip(&picked, STEP_CHARS));
        }
    }
    None
}

pub fn step_note(reply: &str, screen: &str) -> String {
    let reply = without_chrome(reply);
    let screen = without_chrome(screen);
    let tail: Vec<&str> = screen.lines().collect();
    let last = tail[tail.len().saturating_sub(15)..].join("\n");
    let Some(step) = next_step(&reply).or_else(|| next_step(&last)) else { return String::new() };
    if risky_text(&step) {
        format!(" It suggests a next step that is a git write, deploy or production, so it is a decision step: Nixa or the deep brain decides it and records it in the decision journal: {}.", quoted(&step))
    } else {
        format!(" It suggests a next step: {}.", quoted(&step))
    }
}

pub fn is_work_request(text: &str) -> bool {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.is_empty() || plain_yes(&words) {
        return false;
    }
    CLAUSE.split(&words).any(|clause| {
        let a = POLITE.replace(clause, "");
        let b = WANT.replace(&a, "");
        let c = ALSO.replace(&b, "");
        let d = GO_AHEAD.replace(&c, "continue ");
        WORK.is_match(&d)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub safe: String,
    pub gated: Option<String>,
}

pub fn offered_work(reply: &str, gate: &dyn Fn(&str) -> bool) -> Option<Offer> {
    let parts_all = sentences(reply);
    let offers: Vec<&String> = parts_all.iter().filter(|s| OFFER.is_match(s)).collect();
    if offers.is_empty() {
        return None;
    }
    let parts: Vec<String> = offers.iter().flat_map(|s| SPLIT_OR.split(s).map(|p| p.trim().to_string()).collect::<Vec<_>>()).filter(|p| !p.is_empty()).collect();
    let safe: Vec<&String> = parts.iter().filter(|p| !gate(p)).collect();
    if safe.len() != 1 || (parts.len() > 1 && !parts.iter().any(|p| gate(p))) {
        return None;
    }
    if !WORK_VERB.is_match(safe[0]) {
        return None;
    }
    let gated = parts.iter().find(|p| gate(p)).or_else(|| parts_all.iter().find(|s| gate(s))).cloned();
    Some(Offer { safe: safe[0].clone(), gated })
}

pub fn asked_user(reply: &str) -> Option<String> {
    let asks: Vec<String> = sentences(reply).into_iter().filter(|s| ASKS_BACK.is_match(s)).collect();
    if asks.is_empty() { None } else { Some(asks.join(" ")) }
}

pub fn change_summary(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let count = format!("{} {}", paths.len(), if paths.len() == 1 { "file" } else { "files" });
    if paths.len() <= LISTED_FILES {
        return format!("{count}: {}", paths.join(", "));
    }
    let mut groups: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (order, path) in paths.iter().enumerate() {
        let parts: Vec<&str> = path.split('/').collect();
        let group = if parts.len() > 1 { parts[..2.min(parts.len() - 1)].join("/") } else { ".".to_string() };
        let entry = groups.entry(group).or_insert((0, order));
        entry.0 += 1;
    }
    let mut ranked: Vec<(String, usize, usize)> = groups.into_iter().map(|(g, (n, first))| (g, n, first)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    let more = if ranked.len() > LISTED_GROUPS { " and more" } else { "" };
    let top: Vec<String> = ranked.iter().take(LISTED_GROUPS).map(|(g, n, _)| format!("{n} in {g}")).collect();
    format!("{count}, {}{more}", top.join(", "))
}
