use super::index;
use crate::settings_api::keywords::{LIVE_EFFECTS, Row, phrase};
use regex::Regex;
use std::sync::LazyLock;

const ACT_EFFECTS: [&str; 2] = ["act", "go-on"];
const WHOLE_EFFECTS: [&str; 2] = ["hold", "go-on"];

const LEADS: [&[&str]; 13] = [
    &["i", "want", "to"],
    &["we", "need", "to"],
    &["can", "we"],
    &["let", "us"],
    &["ok"],
    &["okay"],
    &["so"],
    &["please"],
    &["hey"],
    &["and"],
    &["then"],
    &["lets"],
    &["now"],
];

const TAILS: [&str; 8] = ["for a moment", "for a bit", "thank you", "for now", "please", "thanks", "nixa", "now"];

static QUESTION_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:should|shall|can|could|would|will|do|does|did|is|are|was|were|what|why|how|when|who|which|where)\b").expect("pattern"));
static REVIEW_REST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(?:(?:(?:terminal|number|tab|agent)\s+)?(?:\d+|t\d[0-9a-z]*|{})|the agents?|all of them|them)?$", nixa_protocol::id::TERM_ID_PATTERN)).expect("pattern"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub row: Row,
    pub alias: String,
    pub rest: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchState {
    pub held: bool,
    pub hold_on: bool,
}

pub fn is_question(raw: &str) -> bool {
    raw.trim().ends_with('?') || QUESTION_START.is_match(&phrase(raw))
}

fn strip_lead(words: Vec<String>) -> Vec<String> {
    let mut rest = words;
    let mut changed = true;
    while changed {
        changed = false;
        for lead in LEADS {
            if lead.len() < rest.len() && lead.iter().zip(rest.iter()).all(|(a, b)| a == b) {
                rest.drain(..lead.len());
                changed = true;
            }
        }
    }
    rest
}

pub fn strip_tail(text: &str) -> String {
    let mut rest = text.trim().to_string();
    let mut changed = true;
    while changed {
        changed = false;
        for tail in TAILS {
            if rest == tail {
                return String::new();
            }
            let end = format!(" {tail}");
            if rest.ends_with(&end) {
                rest.truncate(rest.len() - end.len());
                rest = rest.trim().to_string();
                changed = true;
            }
        }
    }
    rest
}

pub fn match_line(raw: &str, rows: &[Row], state: MatchState) -> Option<Match> {
    let words = strip_lead(phrase(raw).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect());
    if words.is_empty() {
        return None;
    }
    let text = words.join(" ");
    let question = is_question(raw);
    let mut entries: Vec<(usize, String)> = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        if !row.on || !LIVE_EFFECTS.contains(&row.effect.as_str()) {
            continue;
        }
        for alias in std::iter::once(&row.word).chain(row.aliases.iter()) {
            let alias = phrase(alias);
            if !alias.is_empty() {
                entries.push((at, alias));
            }
        }
    }
    let phrases: Vec<(String, usize)> = entries.iter().enumerate().map(|(n, (_, alias))| (alias.clone(), n)).collect();
    let built = index::build(&phrases);
    let mut found: Vec<&(usize, String)> = index::scan(&built, &text).into_iter().map(|hit| &entries[hit.item]).collect();
    found.sort_by(|a, b| b.1.split(' ').count().cmp(&a.1.split(' ').count()).then(b.1.len().cmp(&a.1.len())));
    for (at, alias) in &found {
        let row = &rows[*at];
        let effect = row.effect.as_str();
        if text != *alias && !text.starts_with(&format!("{alias} ")) {
            continue;
        }
        let rest = strip_tail(text[alias.len()..].trim());
        if ACT_EFFECTS.contains(&effect) && question {
            continue;
        }
        if effect == "act" && !state.held {
            continue;
        }
        if effect == "go-on" && !state.hold_on {
            continue;
        }
        if WHOLE_EFFECTS.contains(&effect) && !rest.is_empty() {
            continue;
        }
        if effect == "review-now" && !REVIEW_REST.is_match(&rest) {
            continue;
        }
        return Some(Match { row: row.clone(), alias: alias.clone(), rest });
    }
    None
}
