use serde_json::Value;

pub const KIND: &str = "office_voice";
pub const WAIT_MS: u64 = 3000;
pub const LINE_MS: u64 = 12_000;
pub const CHAR_MS: u64 = 60;
pub const MIN_MS: u64 = 1800;
pub const MAX_MS: u64 = 6000;
pub const AFTER_MS: u64 = 300;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub who: String,
    pub text: String,
    pub at: u64,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub skip: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sync {
    pub line: Option<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heard {
    pub who: String,
    pub text: String,
    pub state: String,
    pub chat: bool,
}

pub fn timer_ms(text: &str) -> u64 {
    (CHAR_MS * text.chars().count() as u64).clamp(MIN_MS, MAX_MS)
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn heard(msg: &Value) -> Option<Heard> {
    let text = |k: &str| msg.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let state = text("state");
    if !matches!(state.as_str(), "start" | "end" | "skip") {
        return None;
    }
    let who = text("who");
    (!who.is_empty()).then(|| Heard { who, text: text("text"), state, chat: text("src") == "chat" })
}

pub fn post(sync: &mut Sync, who: &str, text: &str, now: u64, voiced: bool) {
    sync.line = Some(Line {
        who: who.to_string(),
        text: text.to_string(),
        at: now,
        start: None,
        end: None,
        skip: (!voiced).then_some(now),
    });
}

pub fn on_event(sync: &mut Sync, h: &Heard, now: u64) -> bool {
    let Some(line) = sync.line.as_mut() else {
        return false;
    };
    if line.who != h.who || flat(&line.text) != flat(&h.text) {
        return false;
    }
    match h.state.as_str() {
        "start" if line.start.is_none() && line.end.is_none() => {
            line.start = Some(now);
        }
        "end" | "skip" if line.start.is_some() && line.end.is_none() => {
            line.end = Some(now);
        }
        "skip" if line.start.is_none() && line.skip.is_none() => {
            line.skip = Some(now);
        }
        _ => return false,
    }
    true
}

pub fn shown_at(l: &Line) -> u64 {
    let cap = l.at + WAIT_MS;
    [l.start, l.skip].into_iter().flatten().min().unwrap_or(cap).min(cap)
}

pub fn done_at(l: &Line) -> u64 {
    let cap = l.at + LINE_MS;
    let t = match (l.start, l.end) {
        (Some(_), Some(e)) => e + AFTER_MS,
        (Some(_), None) => cap,
        _ => shown_at(l) + timer_ms(&l.text),
    };
    t.min(cap)
}

pub fn open(l: &Line, now: u64) -> bool {
    l.start.is_some() && l.end.is_none() && now < l.at + LINE_MS
}
