use crate::tools::{schema, spec};
use crate::turns::ToolSpec;
use serde_json::json;
use std::collections::HashSet;

pub const PROGRESS_MS: u64 = 20_000;
pub const PROGRESS_CHARS: usize = 240;
pub const NAME: &str = "nixa_progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    Published,
    Queued,
    Duplicate,
    Inactive,
    Empty,
}

impl Offer {
    pub fn word(self) -> &'static str {
        match self {
            Offer::Published => "published",
            Offer::Queued => "queued",
            Offer::Duplicate => "duplicate",
            Offer::Inactive => "inactive",
            Offer::Empty => "empty",
        }
    }
}

pub fn key(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    for word in lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

pub fn first_sentence(text: &str) -> String {
    let clean = nixa_voice::scrub_for_speech(text);
    let mut chars = clean.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && let Some((_, next)) = chars.peek()
            && next.is_whitespace()
        {
            return clean[..i + c.len_utf8()].to_string();
        }
    }
    clean
}

#[derive(Default)]
pub struct Progress {
    pending: String,
    last_at: Option<u64>,
    seen: HashSet<String>,
    last: Option<String>,
}

impl Progress {
    pub fn reset(&mut self) {
        self.pending.clear();
        self.last_at = None;
        self.seen.clear();
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn flush(&mut self, now: u64, active: bool, blocked: bool) -> Option<String> {
        if !active {
            self.reset();
            return None;
        }
        if self.pending.is_empty() || blocked || self.last_at.is_some_and(|at| now.saturating_sub(at) < PROGRESS_MS) {
            return None;
        }
        let text = std::mem::take(&mut self.pending);
        self.seen.insert(key(&text));
        self.last_at = Some(now);
        self.last = Some(text.clone());
        Some(text)
    }

    pub fn offer(&mut self, now: u64, active: bool, blocked: bool, clean: &str) -> (Offer, Option<String>) {
        if !active {
            return (Offer::Inactive, None);
        }
        let clean: String = clean.trim().chars().take(PROGRESS_CHARS).collect();
        let k = key(&clean);
        if k.is_empty() {
            return (Offer::Empty, None);
        }
        if self.seen.contains(&k) || key(&self.pending) == k {
            return (Offer::Duplicate, None);
        }
        self.pending = clean;
        match self.flush(now, active, blocked) {
            Some(text) => (Offer::Published, Some(text)),
            None => (Offer::Queued, None),
        }
    }
}

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Give the user one short factual progress update while continuing the current work. Use after adding a task when more work was requested, and at meaningful milestones during a longer investigation. This is not a final answer: continue the requested tools or hand the work to an agent. Repeated lines are suppressed; updates are at least 20 seconds apart and wait while the user speaks, a confirmation is open, or speech is busy. Published means the line reached chat; speech follows the user's voice settings. A queued update is not yet published and is dropped when the turn ends. Never claim unverified success or use this to ask for approval.",
        schema(json!({"text": {"type": "string", "minLength": 1, "maxLength": PROGRESS_CHARS}}), &["text"]),
    )
}
