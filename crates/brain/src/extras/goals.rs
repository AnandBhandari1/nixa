use crate::util::{append_line, atomic_write};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "goal-stats.jsonl";
pub const GOAL_DAYS: u64 = 7;
pub const KEEP_DAYS: u64 = 30;
pub const KEEP_MS: u64 = KEEP_DAYS * 86_400_000;
pub const GOAL_WORDS: u64 = 20;
pub const ANSWER_MS: u64 = 2 * 60 * 1000;
pub const BACK_MS: u64 = 60 * 1000;
pub const HARD_KINDS: [&str; 3] = ["check", "assign", "blocker"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "lowercase")]
pub enum Entry {
    Route { kind: String, to: String, #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String> },
    Deep { kind: String, id: String, state: String, ms: u64, #[serde(default, skip_serializing_if = "Option::is_none")] row: Option<i64> },
    Down { brain: String },
    Back { brain: String, ms: u64 },
    Held,
    Said { words: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    #[serde(flatten)]
    pub entry: Entry,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GoalLine {
    pub name: String,
    pub value: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass: Option<bool>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub user: bool,
}

pub struct Goals {
    file: Option<PathBuf>,
    rows: Vec<Row>,
}

fn read(file: &Path) -> Vec<Row> {
    std::fs::read_to_string(file).unwrap_or_default().lines().filter_map(|line| serde_json::from_str::<Row>(line).ok()).collect()
}

impl Goals {
    pub fn open(dir: &Path, now: u64) -> Goals {
        let file = dir.join(FILE);
        let all = read(&file);
        let count = all.len();
        let rows: Vec<Row> = all.into_iter().filter(|r| now.saturating_sub(r.at) < KEEP_MS).collect();
        if rows.len() < count {
            let text: String = rows.iter().filter_map(|r| serde_json::to_string(r).ok()).map(|l| l + "\n").collect();
            let _ = atomic_write(&file, &text);
        }
        Goals { file: Some(file), rows }
    }

    pub fn memory() -> Goals {
        Goals { file: None, rows: Vec::new() }
    }

    pub fn note(&mut self, entry: Entry, now: u64) -> Row {
        let row = Row { entry, at: now };
        self.rows.push(row.clone());
        if let (Some(file), Ok(line)) = (&self.file, serde_json::to_string(&row)) {
            let _ = append_line(file, &line);
        }
        row
    }

    pub fn since(&self, at: u64) -> Vec<Row> {
        self.rows.iter().filter(|r| r.at >= at).cloned().collect()
    }
}

fn seconds(ms: u64) -> String {
    if ms < 60_000 { format!("{} seconds", (ms as f64 / 1000.0).round() as u64) } else { format!("{} min {} s", ms / 60_000, ((ms % 60_000) as f64 / 1000.0).round() as u64) }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 { format!("{count} {word}") } else { format!("{count} {word}s") }
}

pub fn report(rows: &[Row], corrected: &dyn Fn(i64) -> bool, at: u64) -> Vec<GoalLine> {
    let hard = |kind: &str| HARD_KINDS.contains(&kind);
    let held = rows.iter().filter(|r| matches!(r.entry, Entry::Held)).count();
    let routes = rows.iter().filter(|r| matches!(&r.entry, Entry::Route { kind, .. } if hard(kind))).count();
    let deeps: Vec<(&str, &str, u64, Option<i64>)> = rows
        .iter()
        .filter_map(|r| match &r.entry {
            Entry::Deep { kind, state, ms, row, .. } => Some((kind.as_str(), state.as_str(), *ms, *row)),
            _ => None,
        })
        .collect();
    let hard_answered = deeps.iter().filter(|d| hard(d.0) && d.1 == "answered").count();
    let answered: Vec<_> = deeps.iter().filter(|d| d.1 == "answered").collect();
    let overruled = answered.iter().filter(|d| d.3.is_some_and(corrected)).count();
    let timeouts = deeps.iter().filter(|d| d.1 == "timeout").count();
    let slowest = answered.iter().map(|d| d.2).max().unwrap_or(0);
    let downs: Vec<(&str, u64)> = rows.iter().filter_map(|r| match &r.entry { Entry::Down { brain } => Some((brain.as_str(), r.at)), _ => None }).collect();
    let backs: Vec<(&str, u64, u64)> = rows.iter().filter_map(|r| match &r.entry { Entry::Back { brain, ms } => Some((brain.as_str(), r.at, *ms)), _ => None }).collect();
    let missing = downs.iter().filter(|(brain, down)| !backs.iter().any(|(b, back, _)| b == brain && back >= down) && at.saturating_sub(*down) > BACK_MS).count();
    let slow_back = backs.iter().map(|b| b.2).max().unwrap_or(0);
    let said: Vec<u64> = rows.iter().filter_map(|r| match &r.entry { Entry::Said { words } => Some(*words), _ => None }).collect();
    let words = if said.is_empty() { 0 } else { (said.iter().sum::<u64>() as f64 / said.len() as f64).round() as u64 };
    let line = |name: &str, value: String, target: &str, pass: Option<bool>| GoalLine { name: name.into(), value, target: target.into(), pass, user: false };
    let mut duplicate = line("Duplicate work", format!("{} held for the same work", plural(held, "send")), "no real clash", None);
    duplicate.user = true;
    vec![
        duplicate,
        line(
            "Hard decisions answered by the deep brain",
            if routes > 0 { format!("{hard_answered} of {routes}") } else { "no hard decision yet".into() },
            "at least 8 of 10",
            (routes > 0).then_some(hard_answered * 10 >= routes * 8),
        ),
        line(
            "Deep brain answers the user overruled",
            if answered.is_empty() { "no answer yet".into() } else { format!("{overruled} of {}", answered.len()) },
            "at most 1 in 10",
            (!answered.is_empty()).then_some(overruled * 10 <= answered.len()),
        ),
        line(
            "Deep brain answer time",
            if deeps.is_empty() { "no request yet".into() } else { format!("slowest {}, {timeouts} without an answer", seconds(slowest)) },
            "under 2 minutes, none without an answer",
            (!deeps.is_empty()).then_some(slowest < ANSWER_MS && timeouts == 0),
        ),
        line(
            "Deep brain back after a stop",
            if downs.is_empty() { "no stop yet".into() } else { format!("{}, slowest back in {}, {missing} not back", plural(downs.len(), "stop"), seconds(slow_back)) },
            "back within 1 minute every time",
            (!downs.is_empty()).then_some(missing == 0 && slow_back < BACK_MS),
        ),
        line(
            "Spoken reply length",
            if said.is_empty() { "no spoken reply yet".into() } else { format!("{words} words on average over {} replies", said.len()) },
            &format!("about {GOAL_WORDS} words"),
            (!said.is_empty()).then_some(words <= GOAL_WORDS),
        ),
    ]
}

pub fn text(lines: &[GoalLine], days: u64) -> String {
    let mut out = vec![format!("Goals, last {days} days:")];
    for (i, line) in lines.iter().enumerate() {
        let verdict = if line.user {
            "The user checks this one."
        } else {
            match line.pass {
                None => "No data yet.",
                Some(true) => "PASS.",
                Some(false) => "FAIL.",
            }
        };
        out.push(format!("{}. {}: {}. Target: {}. {verdict}", i + 1, line.name, line.value, line.target));
    }
    out.join("\n")
}

pub fn deep_entry(ask: &crate::deep::Ask, row: Option<i64>) -> Option<Entry> {
    let state = match ask.state {
        crate::deep::AskState::Answered => "answered",
        crate::deep::AskState::Timeout => "timeout",
        _ => return None,
    };
    let ms = ask.done_at.unwrap_or(ask.at).saturating_sub(ask.at);
    Some(Entry::Deep { kind: ask.kind.clone().unwrap_or_else(|| "ask".into()), id: ask.id.clone(), state: state.into(), ms, row })
}

pub fn days_of(word: Option<&str>) -> Result<u64, String> {
    match word.map(str::trim).filter(|w| !w.is_empty()) {
        None => Ok(GOAL_DAYS),
        Some(w) => w.parse::<u64>().ok().filter(|d| (1..=KEEP_DAYS).contains(d)).ok_or_else(|| format!("days must be a whole number from 1 to {KEEP_DAYS}. Goal records are kept {KEEP_DAYS} days.")),
    }
}

pub fn word_count(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}
