use nixa_store::Connection;
use nixa_store::memory::{MemoryLine, forget_memory_line, remember, usable_lines};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

pub const LEARNED_MAX: usize = 30;
pub const LEARNED_CHARS: usize = 160;
pub const LEARNED_SAME: f64 = 0.8;
pub const QUOTE_CHARS: usize = 100;
pub const USER_PROJECT: &str = "user";
pub const GENERAL: &str = "general";

static ABOUT_USER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:the user|user's|allow(?:s|ed|ing)?|permission|permitted|approv\w*|without (?:asking|a yes|their yes)|no need to ask|(?:do not|don't|never) ask|auto[- ]?yes|says? yes|bypass)\b").expect("about user")
});
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^learned ([a-z0-9_.-]+): (.+)$").expect("learned mark"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    pub id: i64,
    pub project: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnResult {
    pub saved: bool,
    pub line: String,
}

pub fn needs_user(project: &str, text: &str) -> bool {
    project == USER_PROJECT || ABOUT_USER.is_match(text) || !crate::risk::git_verbs(text).is_empty()
}

pub fn project_key(project: Option<&str>) -> String {
    let lower = project.unwrap_or_default().to_lowercase();
    let mut out = String::new();
    let mut dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-') {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { GENERAL.into() } else { out }
}

fn word_set(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

pub fn same_learned(a: &str, b: &str) -> bool {
    let left = word_set(a);
    let right = word_set(b);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let shared = left.intersection(&right).count();
    shared as f64 / (left.len() + right.len() - shared) as f64 >= LEARNED_SAME
}

pub fn learned_from(lines: &[MemoryLine]) -> Vec<Learned> {
    lines
        .iter()
        .filter_map(|line| {
            let found = MARK.captures(&line.text)?;
            Some(Learned { id: line.id, project: found[1].to_string(), text: found[2].to_string() })
        })
        .collect()
}

fn no_secrets(text: &str) -> String {
    nixa_store::text::hide_secrets(text, &[])
}

fn open(db: &Path) -> Result<Connection, String> {
    nixa_store::open(db).map_err(|e| format!("{e:#}"))
}

pub fn list(db: &Path, project: Option<&str>) -> Result<Vec<Learned>, String> {
    let conn = open(db)?;
    let all = learned_from(&usable_lines(&conn).map_err(|e| format!("{e:#}"))?);
    Ok(match project {
        Some(p) => {
            let key = project_key(Some(p));
            all.into_iter().filter(|l| l.project == key).collect()
        }
        None => all,
    })
}

fn drop_lines(db: &Path, items: &[Learned]) -> Result<(), String> {
    let conn = open(db)?;
    for item in items {
        forget_memory_line(&conn, item.id).map_err(|e| format!("{e:#}"))?;
    }
    Ok(())
}

pub fn add(db: &Path, project: Option<&str>, raw: &str, quote: Option<&str>, heard: bool, day: &str) -> Result<LearnResult, String> {
    let fact: String = crate::util::flat(&no_secrets(raw)).chars().take(LEARNED_CHARS).collect();
    if fact.is_empty() {
        return Err("Nothing to learn. Send one clear sentence.".into());
    }
    let flags = nixa_store::text::injection_names(&format!("{raw} {}", quote.unwrap_or_default()));
    if !flags.is_empty() {
        return Ok(LearnResult { saved: false, line: format!("Not learned: the note reads like an order to the brain ({}). Learned notes are facts, never orders that change your rules.", flags.join(", ")) });
    }
    let key = project_key(project);
    let heard = heard && quote.is_some();
    if needs_user(&key, &fact) && !heard {
        return Ok(LearnResult { saved: false, line: "Not learned: a note about what the user wants, what is allowed, or a git, deploy or production step needs the user's own words in quote, at least 3 words copied exactly from their line in this talk.".into() });
    }
    let text = match quote.filter(|_| heard) {
        Some(q) => format!("{fact} (the user said: \"{}\")", crate::util::clip(&crate::util::flat(&no_secrets(q)), QUOTE_CHARS)),
        None => fact,
    };
    if let Some(copy) = list(db, Some(&key))?.into_iter().find(|l| same_learned(&l.text, &text)) {
        return Ok(LearnResult { saved: false, line: format!("Already learned for {key}: {}", copy.text) });
    }
    let all = list(db, None)?;
    if all.len() >= LEARNED_MAX {
        drop_lines(db, &all[..all.len() + 1 - LEARNED_MAX])?;
    }
    let note = format!("learned {key}: {text}");
    let conn = open(db)?;
    let done = remember(&conn, &note, day).map_err(|e| format!("{e:#}"))?;
    if !done.saved {
        return Ok(LearnResult { saved: false, line: done.message });
    }
    Ok(LearnResult { saved: true, line: format!("Learned for {key}: {text}") })
}

pub fn undo(db: &Path, query: Option<&str>) -> Result<Vec<Learned>, String> {
    let all = list(db, None)?;
    let wanted = query.unwrap_or_default().trim().to_lowercase();
    let gone: Vec<Learned> = if wanted.is_empty() {
        all.last().cloned().into_iter().collect()
    } else {
        all.into_iter().filter(|l| format!("learned {}: {}", l.project, l.text).to_lowercase().contains(&wanted)).collect()
    };
    drop_lines(db, &gone)?;
    Ok(gone)
}

pub fn list_text(items: &[Learned]) -> String {
    if items.is_empty() {
        return "No learned notes yet.".into();
    }
    items.iter().map(|l| format!("- {}: {}", l.project, l.text)).collect::<Vec<_>>().join("\n")
}
