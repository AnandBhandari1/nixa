use crate::util::{read_json, write_json};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub const AWAY_MS: u64 = 10 * 60 * 1000;
pub const HELD_MAX: usize = 30;
pub const SAVE_MS: u64 = 60 * 1000;
pub const DEFAULT_QUIET_MS: u64 = 60 * 60 * 1000;
pub const FILE: &str = "presence.json";
pub const QUIET_NOT_ON: &str = "Quiet time is not on. I speak as usual.";
pub const QUIET_PANEL_END: &str = "Quiet time ended from the panel.";
const FIRST_CHARS: usize = 160;
const END_PHRASES: &[&str] = &["quiet off", "end quiet", "speak again", "talk again", "you can talk"];
const NUMBER_WORDS: &[(&str, u64)] = &[
    ("a", 1),
    ("an", 1),
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("fifteen", 15),
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("ninety", 90),
];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("presence pattern")
}

static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[^\p{L}\p{N}.]+"));
static LOOSE_DOT: LazyLock<Regex> = LazyLock::new(|| re(r"\s\.|\.\s|\.$"));
static FILLER_HEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:ok|okay|hey|please|now|and|so)\s+"));
static FILLER_TAIL: LazyLock<Regex> = LazyLock::new(|| re(r"\s+(?:please|now|thanks|thank you)$"));
static QUIET: LazyLock<Regex> = LazyLock::new(|| re(r"^(?:be |stay |go |keep )?quiet(?: mode| time)?(?: for (.+))?$"));
static SPAN: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d+(?:\.\d+)?|[a-z]+)\s?(hours?|hrs?|h|minutes?|mins?|m)$"));
static CALL_HEAD: LazyLock<Regex> = LazyLock::new(|| re(r"^[^,]{1,40},\s*"));

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Held {
    pub at: u64,
    pub text: String,
    pub mac_only: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Saved {
    last_seen: u64,
}

pub struct Presence {
    away_ms: u64,
    last: u64,
    saved: u64,
    until: u64,
    kept: Vec<Held>,
    file: Option<PathBuf>,
}

impl Presence {
    pub fn new(file: Option<PathBuf>, away_ms: u64) -> Presence {
        let last = file.as_deref().and_then(read_json::<Saved>).map(|s| s.last_seen).unwrap_or(0);
        Presence { away_ms: if away_ms > 0 { away_ms } else { AWAY_MS }, last, saved: 0, until: 0, kept: Vec::new(), file }
    }

    pub fn open(dir: &Path) -> Presence {
        Presence::new(Some(dir.join(FILE)), AWAY_MS)
    }

    pub fn touch(&mut self, now: u64) {
        self.last = now;
        if self.saved == 0 || now.saturating_sub(self.saved) >= SAVE_MS {
            self.saved = now;
            if let Some(file) = &self.file
                && let Err(error) = write_json(file, &Saved { last_seen: now })
            {
                crate::util::log("presence", &format!("last seen not saved: {error}"));
            }
        }
    }

    pub fn present(&self, now: u64) -> bool {
        self.last > 0 && now.saturating_sub(self.last) < self.away_ms
    }

    pub fn quiet(&self, now: u64) -> bool {
        self.until > now
    }

    pub fn set_quiet(&mut self, now: u64, ms: u64) -> u64 {
        self.until = if ms > 0 { now + ms } else { 0 };
        self.until
    }

    pub fn quiet_until(&self, now: u64) -> u64 {
        if self.until > now { self.until } else { 0 }
    }

    pub fn hold(&mut self, now: u64, text: &str, mac_only: bool) {
        let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if clean.is_empty() || self.kept.last().is_some_and(|h| h.text == clean) {
            return;
        }
        self.kept.push(Held { at: now, text: clean, mac_only });
        if self.kept.len() > HELD_MAX {
            let extra = self.kept.len() - HELD_MAX;
            self.kept.drain(..extra);
        }
    }

    pub fn take(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.kept)
    }

    pub fn held(&self) -> usize {
        self.kept.len()
    }

    pub fn last_seen(&self) -> u64 {
        self.last
    }

    pub fn snapshot(&self, now: u64) -> Value {
        let mut out = json!({
            "here": self.present(now),
            "away": false,
            "awayMinutes": (self.away_ms as f64 / 60_000.0).round() as u64,
            "held": self.kept.len(),
        });
        if self.quiet(now) {
            out["quietUntil"] = json!(self.until);
        }
        if self.last > 0 {
            out["lastSeen"] = json!(self.last);
        }
        out
    }
}

fn plain_words(text: &str) -> String {
    let lower = text.to_lowercase();
    let spaced = NOT_WORD.replace_all(&lower, " ");
    LOOSE_DOT.replace_all(&spaced, " ").trim().to_string()
}

fn trim_filler(words: &str) -> String {
    let head = FILLER_HEAD.replace(words, "");
    FILLER_TAIL.replace(&head, "").to_string()
}

fn number_word(word: &str) -> Option<u64> {
    NUMBER_WORDS.iter().find(|(w, _)| *w == word).map(|(_, n)| *n)
}

fn parse_span(text: &str) -> Option<u64> {
    if text == "half an hour" {
        return Some(30 * 60 * 1000);
    }
    let found = SPAN.captures(text)?;
    let amount: f64 = if found[1].starts_with(|c: char| c.is_ascii_digit()) {
        found[1].parse().ok()?
    } else {
        number_word(&found[1])? as f64
    };
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    let unit = if found[2].starts_with('h') { 60.0 * 60.0 * 1000.0 } else { 60.0 * 1000.0 };
    Some((amount * unit).round() as u64)
}

fn parse_words(words: &str) -> Option<u64> {
    if words.is_empty() {
        return None;
    }
    if END_PHRASES.contains(&words) {
        return Some(0);
    }
    let found = QUIET.captures(words)?;
    match found.get(1) {
        Some(span) => parse_span(span.as_str()),
        None => Some(DEFAULT_QUIET_MS),
    }
}

pub fn parse_quiet(text: &str) -> Option<u64> {
    let whole = parse_words(&trim_filler(&trim_filler(&plain_words(text))));
    if whole.is_some() {
        return whole;
    }
    let unnamed = CALL_HEAD.replace(text, "");
    if unnamed == text {
        return None;
    }
    parse_words(&trim_filler(&trim_filler(&plain_words(&unnamed))))
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{}…", head.trim_end())
}

pub fn quiet_start_text(until: u64) -> String {
    format!("Quiet until {}. I keep my words in the chat until then.", crate::prompt::clock(until))
}

pub fn quiet_start_note(until: u64, name: &str) -> String {
    format!(
        "Note: The user asked for quiet until {}. {name} keeps its spoken lines in the chat until then and speaks a summary after; do not repeat them.",
        crate::prompt::clock(until)
    )
}

pub fn quiet_end_note(reply: &str, name: &str) -> String {
    format!("Note: The user ended the quiet time. {name} said: \"{}\"", reply.replace('"', "'"))
}

pub fn quiet_end_text(held: &[Held]) -> String {
    if held.is_empty() {
        return "Quiet time is over. Nothing happened.".into();
    }
    let Some(first) = held.iter().find(|h| !h.mac_only) else {
        return match held.len() {
            1 => "Quiet time is over. 1 thing happened. It is in the chat on the Mac.".into(),
            n => format!("Quiet time is over. {n} things happened. They are in the chat on the Mac."),
        };
    };
    let first = format!("\"{}\"", cut(&first.text, FIRST_CHARS));
    if held.len() == 1 {
        return format!("Quiet time is over. 1 thing happened: {first}.");
    }
    format!("Quiet time is over. {} things happened. The first: {first}. The rest is in the chat.", held.len())
}

