use crate::util::{atomic_write, log};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "usage.json";
pub const KEEP_DAYS: u64 = 30;
const DAY_MS: u64 = 86_400_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DayUsage {
    pub turns: u64,
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TurnUsage {
    pub ms: u64,
    pub tokens: u64,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Debug, Clone, Copy)]
struct Open {
    at: u64,
    counts: Counts,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    days: BTreeMap<String, DayUsage>,
}

pub struct Usage {
    file: Option<PathBuf>,
    days: BTreeMap<String, DayUsage>,
    open: Option<Open>,
    last: Option<TurnUsage>,
    seen: Option<Counts>,
    was_busy: bool,
}

pub fn day_key(at: u64) -> String {
    crate::modes::clock::stamp(at).chars().take(10).collect()
}

fn scaled(value: f64, unit: &str) -> String {
    if value < 10.0 {
        let rounded = (value * 10.0).round() / 10.0;
        if rounded.fract() == 0.0 { format!("{}{unit}", rounded as u64) } else { format!("{rounded:.1}{unit}") }
    } else {
        format!("{}{unit}", value.round() as u64)
    }
}

pub fn format_tokens(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    if count < 999_500 {
        return scaled(count as f64 / 1000.0, "k");
    }
    scaled(count as f64 / 1_000_000.0, "M")
}

pub fn format_turn(turn: &TurnUsage) -> String {
    let seconds = if turn.ms < 10_000 { format!("{:.1}", turn.ms as f64 / 1000.0) } else { (turn.ms as f64 / 1000.0).round().to_string() };
    format!("{seconds} s, {}", format_tokens(turn.tokens))
}

fn sum(counts: Counts) -> u64 {
    counts.input + counts.output + counts.cached
}

fn delta(now: Counts, before: Counts) -> Counts {
    let part = |a: u64, b: u64| if a >= b { a - b } else { a };
    Counts { input: part(now.input, before.input), output: part(now.output, before.output), cached: part(now.cached, before.cached) }
}

fn read_days(file: &Path) -> BTreeMap<String, DayUsage> {
    let saved: Saved = crate::util::read_json(file).unwrap_or_default();
    saved.days.into_iter().filter(|(day, _)| valid_day(day)).collect()
}

fn valid_day(day: &str) -> bool {
    let b = day.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

impl Usage {
    pub fn open(dir: &Path) -> Usage {
        let file = dir.join(FILE);
        let days = read_days(&file);
        Usage::with(Some(file), days)
    }

    pub fn memory() -> Usage {
        Usage::with(None, BTreeMap::new())
    }

    fn with(file: Option<PathBuf>, days: BTreeMap<String, DayUsage>) -> Usage {
        Usage { file, days, open: None, last: None, seen: None, was_busy: false }
    }

    fn add(&mut self, counts: Counts, turns: u64, now: u64) {
        let day = self.days.entry(day_key(now)).or_default();
        day.turns += turns;
        day.tokens += sum(counts);
        day.input += counts.input;
        day.output += counts.output;
        day.cached += counts.cached;
    }

    fn save(&mut self, now: u64) {
        let oldest = day_key(now.saturating_sub(KEEP_DAYS * DAY_MS));
        self.days.retain(|day, _| day.as_str() >= oldest.as_str());
        let Some(file) = &self.file else { return };
        let text = serde_json::to_string_pretty(&json!({"days": self.days})).unwrap_or_default();
        if let Err(e) = atomic_write(file, &format!("{text}\n")) {
            log("extras", &format!("could not save the usage file: {e}"));
        }
    }

    pub fn start(&mut self, now: u64) {
        if self.open.is_none() {
            self.open = Some(Open { at: now, counts: Counts::default() });
        }
    }

    pub fn note(&mut self, counts: Counts, now: u64) {
        if sum(counts) == 0 {
            return;
        }
        match &mut self.open {
            Some(open) => {
                open.counts.input += counts.input;
                open.counts.output += counts.output;
                open.counts.cached += counts.cached;
            }
            None => {
                self.add(counts, 0, now);
                self.save(now);
            }
        }
    }

    pub fn end(&mut self, now: u64) -> Option<TurnUsage> {
        let open = self.open.take()?;
        let c = open.counts;
        let turn = TurnUsage { ms: now.saturating_sub(open.at), tokens: sum(c), input: c.input, output: c.output, cached: c.cached };
        self.last = Some(turn);
        self.add(c, 1, now);
        self.save(now);
        Some(turn)
    }

    pub fn sample(&mut self, busy: bool, counts: Counts, now: u64) -> Option<TurnUsage> {
        let before = self.seen.replace(counts).unwrap_or(counts);
        let more = delta(counts, before);
        if busy && !self.was_busy {
            self.start(now);
        }
        self.note(more, now);
        let ended = if !busy && self.was_busy { self.end(now) } else { None };
        self.was_busy = busy;
        ended
    }

    pub fn today(&self, now: u64) -> DayUsage {
        self.days.get(&day_key(now)).copied().unwrap_or_default()
    }

    pub fn by_day(&self, now: u64) -> Vec<(String, DayUsage)> {
        let oldest = day_key(now.saturating_sub(KEEP_DAYS * DAY_MS));
        self.days.iter().filter(|(day, _)| day.as_str() >= oldest.as_str()).rev().map(|(d, u)| (d.clone(), *u)).collect()
    }

    pub fn snapshot(&self, now: u64) -> Value {
        let mut out = json!({"day": day_key(now), "today": self.today(now)});
        if let Some(turn) = &self.last {
            out["turn"] = json!(turn);
            out["turnText"] = json!(format_turn(turn));
        }
        out
    }

    pub fn view(&self, now: u64) -> Value {
        let days: Vec<Value> = self.by_day(now).into_iter().map(|(day, u)| json!({"day": day, "turns": u.turns, "tokens": u.tokens, "input": u.input, "output": u.output, "cached": u.cached, "text": format_tokens(u.tokens)})).collect();
        let mut out = self.snapshot(now);
        out["days"] = json!(days);
        out
    }

    pub fn text(&self, now: u64) -> String {
        let days = self.by_day(now);
        if days.is_empty() {
            return "No brain usage in the last 30 days.".into();
        }
        let lines: Vec<String> = days.iter().map(|(day, u)| format!("- {day}: {} turns, {} tokens ({} in, {} out, {} cached)", u.turns, format_tokens(u.tokens), format_tokens(u.input), format_tokens(u.output), format_tokens(u.cached))).collect();
        format!("Brain usage by day, newest first:\n{}", lines.join("\n"))
    }
}
