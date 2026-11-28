use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const FILE: &str = "agent-leave.json";
pub const UNKNOWN_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Leave {
    pub terminal: String,
    pub name: String,
    pub engine: String,
    pub model: String,
    pub line: String,
    pub before: String,
    pub note: String,
    pub by: String,
    pub since: u64,
    pub until: u64,
    pub known: bool,
    pub task: i64,
    pub title: String,
    pub moved_to: String,
    pub to_engine: String,
    pub waiting: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub items: Vec<Leave>,
    pub handled: BTreeMap<String, String>,
}

pub fn load(path: &Path) -> Book {
    crate::util::read_json(path).unwrap_or_default()
}

pub fn save(path: &Path, book: &Book) -> std::io::Result<()> {
    crate::util::write_json(path, book)
}

pub fn until_of(given: Option<u64>, now: u64) -> (u64, bool) {
    match given {
        Some(at) if at > now => (at, true),
        _ => (now + UNKNOWN_MS, false),
    }
}

pub fn engine_key(raw: &str) -> String {
    let word = raw.trim().to_lowercase();
    let word = word.trim_start_matches("member:").trim_start_matches("custom:");
    let word = word.split([':', ' ', '/']).next().unwrap_or("");
    match word {
        "agy" | "antigravity" => "antigravity".into(),
        "cursor-agent" | "cursor" => "cursor".into(),
        "dima" | "dim" => "dim".into(),
        "claude-code" | "claude" => "claude".into(),
        other => other.to_string(),
    }
}

pub fn same_engine(a: &str, b: &str) -> bool {
    let a = engine_key(a);
    !a.is_empty() && a == engine_key(b)
}

pub fn on_leave<'a>(book: &'a Book, terminal: &str) -> Option<&'a Leave> {
    book.items.iter().find(|l| l.terminal == terminal)
}

pub fn add(book: &mut Book, leave: Leave) -> bool {
    if on_leave(book, &leave.terminal).is_some() {
        return false;
    }
    if !leave.line.is_empty() {
        if book.handled.get(&leave.terminal) == Some(&leave.line) {
            return false;
        }
        book.handled.insert(leave.terminal.clone(), leave.line.clone());
    }
    book.items.push(leave);
    true
}

pub fn engine_away<'a>(book: &'a Book, engine: &str, now: u64) -> Option<&'a Leave> {
    book.items.iter().filter(|l| l.until > now && same_engine(&l.engine, engine)).max_by_key(|l| l.until)
}

pub fn away_engines(book: &Book, now: u64) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in book.items.iter().filter(|l| l.until > now) {
        let key = engine_key(&l.engine);
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

pub fn due(book: &Book, now: u64) -> Vec<Leave> {
    book.items.iter().filter(|l| l.until <= now).cloned().collect()
}

pub fn why_not(installed: &[String], away: &[String], old: &str, to: &str) -> Option<&'static str> {
    if same_engine(to, old) {
        return Some("it is the same engine that hit the limit");
    }
    if away.iter().any(|a| same_engine(a, to)) {
        return Some("it is on leave too");
    }
    if !installed.iter().any(|i| same_engine(i, to)) {
        return Some("it is not installed");
    }
    None
}

pub fn remove(book: &mut Book, terminal: &str) -> Option<Leave> {
    let at = book.items.iter().position(|l| l.terminal == terminal)?;
    Some(book.items.remove(at))
}

pub fn update(book: &mut Book, terminal: &str, change: impl FnOnce(&mut Leave)) {
    if let Some(l) = book.items.iter_mut().find(|l| l.terminal == terminal) {
        change(l);
    }
}

pub fn forget(book: &mut Book, live: &[String]) -> bool {
    let before = book.handled.len();
    book.handled.retain(|t, _| live.contains(t));
    before != book.handled.len()
}

pub fn pick_engine(installed: &[String], order: &[String], away: &[String], old: &str) -> Option<String> {
    let usable = |key: &String| {
        let k = engine_key(key);
        !k.is_empty() && k != engine_key(old) && !away.iter().any(|a| engine_key(a) == k)
    };
    let find = |want: &str| installed.iter().find(|i| same_engine(i, want)).cloned();
    let start = order.iter().find(|o| engine_key(o) == engine_key(old)).cloned().unwrap_or_default();
    nixa_store::agent_map::after(order, &start).iter().filter_map(|o| find(o)).chain(installed.iter().cloned()).find(usable)
}
