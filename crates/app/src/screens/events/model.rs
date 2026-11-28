use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_EVENTS: usize = 50;
pub const MERGE_MS: u64 = 5000;
pub const UNDO_MS: u64 = 15_000;
pub const TOAST_MS: u64 = 6000;
pub const SKIP_WORDS: &str = "permission checks off";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Agent,
    Notification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

impl Level {
    pub fn parse(word: &str) -> Level {
        match word.trim() {
            "success" => Level::Success,
            "warning" => Level::Warning,
            "error" => Level::Error,
            _ => Level::Info,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Level::Error => "blocked",
            Level::Warning => "working",
            Level::Success => "done",
            Level::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub terminal: String,
    pub title: String,
    pub body: String,
    pub command: String,
    #[serde(default)]
    pub space: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resume(Offer),
    Focus(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub time: u64,
    pub kind: String,
    pub title: String,
    pub text: String,
    pub body: String,
    pub terminal: String,
    pub space: String,
    pub job: String,
    pub agent_number: Option<u32>,
    pub source: Source,
    pub action: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub level: Level,
    pub kind: Option<String>,
    pub terminal: String,
    pub action: Option<Action>,
    pub timeout_ms: Option<u64>,
}

impl Notice {
    pub fn new(level: Level, title: impl Into<String>) -> Notice {
        Notice {
            title: title.into(),
            body: String::new(),
            level,
            kind: None,
            terminal: String::new(),
            action: None,
            timeout_ms: None,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Notice {
        self.body = body.into();
        self
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn ago(time: u64, now: u64) -> String {
    let seconds = now.saturating_sub(time) / 1000;
    if seconds < 45 {
        return "just now".into();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", hours / 24)
}

pub fn state_words(kind: &str) -> Option<&'static str> {
    match kind {
        "blocked" => Some("needs attention"),
        "error" => Some("reported an error"),
        "done" => Some("finished"),
        "resume" => Some("can be resumed"),
        _ => None,
    }
}

pub fn terminal_of(item: &Item) -> String {
    if !item.terminal.is_empty() {
        return item.terminal.clone();
    }
    match &item.action {
        Some(Action::Focus(t)) => t.clone(),
        Some(Action::Resume(offer)) => offer.terminal.clone(),
        None => String::new(),
    }
}

pub fn agent_label(item: &Item, numbers: &HashMap<String, u32>) -> String {
    let terminal = terminal_of(item);
    match item.agent_number.or_else(|| numbers.get(&terminal).copied()) {
        Some(n) => format!("Agent #{n}"),
        None if !item.job.is_empty() => item.job.clone(),
        None => format!("Agent {terminal}"),
    }
}

pub fn row_text(item: &Item, numbers: &HashMap<String, u32>) -> String {
    let terminal = terminal_of(item);
    match state_words(&item.kind) {
        Some(state) if !terminal.is_empty() => {
            let label = agent_label(item, numbers);
            let job = if item.job.is_empty() || label == item.job {
                String::new()
            } else {
                format!(": {}", item.job)
            };
            format!("{label} {state}{job}")
        }
        _ => item.text.clone(),
    }
}

pub fn row_body(item: &Item) -> String {
    let agent_state = !terminal_of(item).is_empty() && state_words(&item.kind).is_some();
    if agent_state && item.source == Source::Notification {
        return item.body.split(" / ").next().unwrap_or_default().to_string();
    }
    item.body.clone()
}

pub fn row_meta(item: &Item, numbers: &HashMap<String, u32>) -> String {
    let terminal = terminal_of(item);
    let label = if terminal.is_empty() {
        String::new()
    } else {
        agent_label(item, numbers)
    };
    [item.space.clone(), label]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn row_tip(item: &Item, numbers: &HashMap<String, u32>) -> String {
    match &item.action {
        Some(Action::Resume(_)) => "Resume this agent in a new tab".into(),
        _ if !terminal_of(item).is_empty() => {
            format!("Show {}'s pane", agent_label(item, numbers).to_lowercase())
        }
        _ => row_text(item, numbers),
    }
}

fn key(item: &Item) -> String {
    format!("{}|{}|{}", item.time, item.terminal, item.text)
}

#[derive(Debug, Clone, Default)]
pub struct Events {
    pub items: Vec<Item>,
    pub undo: Option<Vec<Item>>,
    pub undo_token: u64,
    pub seen: u64,
}

impl Events {
    pub fn add_agent(&mut self, item: Item) -> bool {
        let close = |old: &Item| {
            old.source == Source::Notification
                && old.terminal == item.terminal
                && old.kind == item.kind
                && old.time.abs_diff(item.time) < MERGE_MS
        };
        if self.items.iter().any(close) {
            return false;
        }
        let same = self.items.iter().position(|old| {
            old.source == Source::Agent
                && !item.terminal.is_empty()
                && old.terminal == item.terminal
                && old.kind == item.kind
                && old.time.abs_diff(item.time) < MERGE_MS
        });
        if let Some(at) = same {
            self.items.remove(at);
        }
        self.items.insert(0, item);
        self.items.truncate(MAX_EVENTS);
        true
    }

    pub fn post(&mut self, notice: &Notice, now: u64) -> Option<Item> {
        let title = notice.title.trim().to_string();
        let body = notice.body.trim().to_string();
        if title.is_empty() && body.is_empty() {
            return None;
        }
        let terminal = if notice.terminal.is_empty() {
            match &notice.action {
                Some(Action::Focus(t)) => t.clone(),
                Some(Action::Resume(o)) => o.terminal.clone(),
                None => String::new(),
            }
        } else {
            notice.terminal.clone()
        };
        let finished = !terminal.is_empty()
            && title
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == "finished");
        let kind = notice.kind.clone().unwrap_or_else(|| {
            if finished {
                "done".into()
            } else {
                notice.level.kind().into()
            }
        });
        let at = if terminal.is_empty() {
            None
        } else {
            self.items.iter().position(|e| {
                e.terminal == terminal && e.kind == kind && now.abs_diff(e.time) < MERGE_MS
            })
        };
        let prev = at.map(|i| self.items.remove(i));
        let pick = |old: Option<&String>, new: &str, fallback: &str| {
            old.filter(|s| !s.is_empty())
                .cloned()
                .or_else(|| (!new.is_empty()).then(|| new.to_string()))
                .unwrap_or_else(|| fallback.to_string())
        };
        let text_new = if title.is_empty() { body.clone() } else { title.clone() };
        let item = Item {
            time: now,
            title: pick(prev.as_ref().map(|p| &p.title), &title, "Notice"),
            text: pick(prev.as_ref().map(|p| &p.text), &text_new, "Notice"),
            body: pick(prev.as_ref().map(|p| &p.body), &body, ""),
            kind,
            space: prev.as_ref().map(|p| p.space.clone()).unwrap_or_default(),
            job: prev.as_ref().map(|p| p.job.clone()).unwrap_or_default(),
            agent_number: prev.as_ref().and_then(|p| p.agent_number),
            action: notice
                .action
                .clone()
                .or_else(|| prev.as_ref().and_then(|p| p.action.clone())),
            terminal,
            source: Source::Notification,
        };
        self.items.insert(0, item.clone());
        self.items.truncate(MAX_EVENTS);
        Some(item)
    }

    pub fn clear(&mut self) -> Option<u64> {
        if self.items.is_empty() {
            self.undo = None;
            return None;
        }
        self.undo = Some(std::mem::take(&mut self.items));
        self.undo_token += 1;
        Some(self.undo_token)
    }

    pub fn expire_undo(&mut self, token: u64) -> bool {
        if self.undo_token == token && self.undo.is_some() {
            self.undo = None;
            return true;
        }
        false
    }

    pub fn undo_clear(&mut self) -> bool {
        let Some(old) = self.undo.take() else {
            return false;
        };
        let keys: Vec<String> = old.iter().map(key).collect();
        let mut list: Vec<Item> = self
            .items
            .drain(..)
            .filter(|e| !keys.contains(&key(e)))
            .collect();
        list.extend(old);
        list.truncate(MAX_EVENTS);
        self.items = list;
        true
    }

    pub fn load(&mut self, fresh: Vec<Item>) {
        let keys: Vec<String> = self.items.iter().map(key).collect();
        for item in fresh {
            if !keys.contains(&key(&item)) {
                self.items.push(item);
            }
        }
        self.items.sort_by_key(|e| std::cmp::Reverse(e.time));
        self.items.truncate(MAX_EVENTS);
    }

    pub fn unseen(&self) -> usize {
        self.items.iter().filter(|e| e.time > self.seen).count()
    }
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn offer_of(value: &Value) -> Option<Offer> {
    let offer: Offer = serde_json::from_value(value.clone()).ok()?;
    if !nixa_store::agents::safe_resume_command(&offer.command) {
        return None;
    }
    (!offer.terminal.is_empty()).then_some(offer)
}

pub fn item_of(value: &Value) -> Option<Item> {
    let time = value.get("time").and_then(Value::as_u64)?;
    let kind = text(value, "kind");
    let event_text = text(value, "text");
    if kind.is_empty() || event_text.is_empty() {
        return None;
    }
    let terminal = text(value, "terminal");
    let action = match value.get("offer").and_then(offer_of) {
        Some(offer) => Some(Action::Resume(offer)),
        None if !terminal.is_empty() => Some(Action::Focus(terminal.clone())),
        None => None,
    };
    Some(Item {
        time,
        kind,
        title: event_text.clone(),
        text: event_text,
        body: String::new(),
        terminal,
        space: text(value, "space"),
        job: text(value, "job"),
        agent_number: value
            .get("agent_number")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        source: Source::Agent,
        action,
    })
}

pub fn skips_checks(command: &str) -> bool {
    nixa_store::agents::resume_skips_checks(command)
}

pub fn resume_argv(command: &str) -> Option<Vec<String>> {
    let command = nixa_store::agents::configured_resume_in(
        &nixa_protocol::paths::data_dir(),
        command,
    )?;
    Some(vec!["sh".into(), "-lc".into(), command])
}
