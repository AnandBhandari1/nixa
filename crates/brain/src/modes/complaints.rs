use super::clock::stamp;
use crate::settings_api::keywords::phrase;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::LazyLock;

pub const FILE: &str = "complaints.jsonl";
pub const RECENT: usize = 20;
pub const RECENT_MAX: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complaint {
    pub at: u64,
    pub text: String,
    pub typed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nixa: Option<String>,
}

static STRONG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:dumb|stupid|useless|idiot|rubbish|nonsense|annoying|irritating|frustrat\w*|waste of (?:my )?time|be smart|not smart|how many times|i told you|again and again|so confusing|very confusing|too confusing)\b").expect("pattern")
});

static AT_NIXA: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\byou (?:keep|keeps|always keep) (?:asking|saying|telling|doing|sending|stopping|repeating|interrupting|missing)\b",
        r"\byou (?:are|re) (?:not|never) (?:doing|listening|understanding|helping|working|hearing|deciding|reporting|telling|answering|smart)\b",
        r"\byou (?:are|re) (?:behaving|acting) like\b",
        r"\byou (?:dont|do not|didnt|did not|never|cannot|cant) (?:listen|understand|do|tell|answer|reply|hear|report|decide|follow|remember)\b",
        r"\b(?:cannot|cant|can not|could not|couldnt|did not|didnt) hear (?:you|nixa|anything)\b",
        r"\b(?:thats|that is|this is|thats is) (?:the|my) problem\b",
        r"\b(?:ask|asks|asked|asking) (?:me )?too much\b|\btoo much (?:asking|questions)\b",
        r"\b(?:is|are|its|it is) (?:confusing|not working|broken again)\b",
        r"\bwhy (?:do|did|are|is|dont|do not) you (?:keep|always|not|never|ask|stop)\b",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("pattern"))
    .collect()
});

static RULE_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:from now on|whenever|every time .* say|next time .* say)\b").expect("pattern"));

pub fn is_complaint(text: &str) -> bool {
    let plain = phrase(text);
    if plain.split(' ').filter(|w| !w.is_empty()).count() < 3 || RULE_WORDS.is_match(&plain) {
        return false;
    }
    STRONG.is_match(&plain) || AT_NIXA.iter().any(|p| p.is_match(&plain))
}

pub fn read(file: &Path) -> Vec<Complaint> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str::<Complaint>(l).ok()).collect()
}

pub fn add(file: &Path, text: &str, typed: bool, nixa: Option<&str>, now: u64) -> std::io::Result<Complaint> {
    let row = Complaint {
        at: now,
        text: text.trim().to_string(),
        typed,
        nixa: nixa.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string),
    };
    if let Some(dir) = file.parent() {
        nixa_protocol::paths::ensure_private_dir(dir)?;
    }
    crate::util::append_line(file, &serde_json::to_string(&row).unwrap_or_default())?;
    Ok(row)
}

pub fn recent(file: &Path, limit: usize) -> Vec<Complaint> {
    let mut rows = read(file);
    let skip = rows.len().saturating_sub(limit);
    rows.drain(..skip);
    rows.reverse();
    rows
}

pub fn since(file: &Path, at: u64) -> Vec<Complaint> {
    read(file).into_iter().filter(|r| r.at >= at).collect()
}

pub fn line(row: &Complaint) -> String {
    let said = row
        .nixa
        .as_deref()
        .map(|m| format!(". Nixa had just said: {}", serde_json::to_string(&crate::util::clip_flat(m, 200)).unwrap_or_default()))
        .unwrap_or_default();
    format!(
        "{}, {}: {}{said}",
        stamp(row.at),
        if row.typed { "typed" } else { "said" },
        serde_json::to_string(&crate::util::clip_flat(&row.text, 300)).unwrap_or_default()
    )
}

pub fn text_of(rows: &[Complaint]) -> String {
    if rows.is_empty() {
        return "No complaints saved.".into();
    }
    rows.iter().map(|r| format!("- {}", line(r))).collect::<Vec<_>>().join("\n")
}
