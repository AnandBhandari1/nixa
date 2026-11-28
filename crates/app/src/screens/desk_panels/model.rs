use serde_json::Value;
use std::path::{Path, PathBuf};

pub const BROWSER_PREFIX: &str = "Browser ";
pub const AUDIT_DIR: &str = "desktop";
pub const MAX_BODY: usize = 6000;
pub const CARD_KIND: &str = "desktop_card";
pub const TALK_KIND: &str = "talk";
pub const TALK_DONE: &str = "confirm_done";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Eyes,
    Browser,
    Hands,
}

pub const PANELS: [Panel; 3] = [Panel::Eyes, Panel::Browser, Panel::Hands];

pub fn key(panel: Panel) -> &'static str {
    match panel {
        Panel::Eyes => "nixa-eyes",
        Panel::Browser => "nixa-browser",
        Panel::Hands => "nixa-hands",
    }
}

pub fn label(panel: Panel) -> &'static str {
    match panel {
        Panel::Eyes => "Eyes",
        Panel::Browser => "Browser",
        Panel::Hands => "Hands",
    }
}

pub fn title(panel: Panel) -> &'static str {
    match panel {
        Panel::Eyes => "Nixa's eyes",
        Panel::Browser => "Nixa's browser",
        Panel::Hands => "Nixa's hands",
    }
}

pub fn help(panel: Panel) -> &'static str {
    match panel {
        Panel::Eyes => "Ask Nixa to list windows, read a window ID, read the whole screen, or read Nixa's own window. Captures return visible text and accessible controls.",
        Panel::Browser => "Ask Nixa to open an HTTP(S) page in the browser, read a browser window, click an observed control or fill an observed text field. Each step returns what is visible.",
        Panel::Hands => "Ask Nixa to click or fill a text field using the observation and element IDs returned by Eyes or Browser. Each input uses its observation once; changed targets are refused.",
    }
}

pub fn panel_of(name: &str) -> Option<Panel> {
    match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "nixa-eyes" | "eyes" => Some(Panel::Eyes),
        "nixa-browser" | "browser" => Some(Panel::Browser),
        "nixa-hands" | "hands" => Some(Panel::Hands),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardKey {
    Desk(String),
    Talk(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub key: CardKey,
    pub message: String,
    pub note: String,
    pub stamp: u64,
}

pub fn card_tag(card: &Card) -> String {
    match &card.key {
        CardKey::Desk(id) => format!("desk:{id}@{}", card.stamp),
        CardKey::Talk(id) => format!("talk:{id}@{}", card.stamp),
    }
}

fn stamp(v: &Value, name: &str) -> u64 {
    v.get(name).and_then(Value::as_u64).unwrap_or(0)
}

fn text(v: &Value, name: &str) -> Option<String> {
    v.get(name).and_then(Value::as_str).map(str::to_string)
}

pub fn desk_card_of(card: &Value) -> Option<Card> {
    let id = text(card, "id").filter(|id| !id.trim().is_empty())?;
    let message = text(card, "message").filter(|m| !m.trim().is_empty())?;
    let note = match (text(card, "tool"), text(card, "action")) {
        (Some(tool), Some(action)) => format!("{tool} {action}"),
        _ => String::new(),
    };
    Some(Card { key: CardKey::Desk(id), message, note, stamp: stamp(card, "at") })
}

pub fn talk_card_of(ask: &Value) -> Option<Card> {
    let id = ask.get("id").and_then(Value::as_u64)?;
    let message = text(ask, "message").filter(|m| m.starts_with(BROWSER_PREFIX))?;
    Some(Card { key: CardKey::Talk(id), message, note: text(ask, "why").unwrap_or_default(), stamp: stamp(ask, "asked_at") })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Desk(Option<Card>),
    Talk(Option<Card>),
    TalkDone(Option<u64>),
    Other,
}

pub fn event_of(msg: &Value) -> Event {
    match msg.get("type").and_then(Value::as_str) {
        Some(CARD_KIND) => Event::Desk(msg.get("card").and_then(desk_card_of)),
        Some(TALK_KIND) => match msg.get("confirm") {
            Some(ask) => Event::Talk(talk_card_of(ask)),
            None => Event::Other,
        },
        Some(TALK_DONE) => Event::TalkDone(msg.get("id").and_then(Value::as_u64)),
        _ => Event::Other,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub panel: Panel,
    pub head: String,
    pub body: String,
}

pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n…");
    out
}

fn body_of(v: &Value) -> String {
    clip(&serde_json::to_string_pretty(v).unwrap_or_default(), MAX_BODY)
}

pub fn desk_row(v: &Value) -> Option<Row> {
    let step = v.get("step").or_else(|| v.get("attempt"))?;
    let name = text(step, "tool").or_else(|| text(step, "mod"))?;
    let panel = panel_of(&name).filter(|p| *p != Panel::Browser)?;
    let action = text(step, "action").or_else(|| text(step, "what")).unwrap_or_else(|| "step".into());
    let phase = text(v, "phase").unwrap_or_else(|| "unknown".into());
    let at = text(v, "at").unwrap_or_default();
    Some(Row { panel, head: format!("{action} · {phase} · {at}"), body: body_of(v) })
}

pub fn desk_rows(recent: &Value) -> Vec<Row> {
    recent.as_array().map(|list| list.iter().rev().filter_map(desk_row).collect()).unwrap_or_default()
}

pub fn browser_row(v: &Value) -> Option<Row> {
    let action = text(v, "action")?;
    let verdict = text(v, "verdict").unwrap_or_else(|| "unknown".into());
    let at = v.get("at").and_then(Value::as_u64).map(iso_time).unwrap_or_default();
    Some(Row { panel: Panel::Browser, head: format!("{action} · {verdict} · {at}"), body: body_of(v) })
}

pub fn browser_rows(calls: &Value) -> Vec<Row> {
    calls.as_array().map(|list| list.iter().filter_map(browser_row).collect()).unwrap_or_default()
}

pub fn for_panel(rows: &[Row], panel: Panel) -> Vec<&Row> {
    rows.iter().filter(|r| r.panel == panel).collect()
}

pub fn iso_time(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    pub desk: Option<Card>,
    pub talk: Option<Card>,
    pub rows: Vec<Row>,
    pub allow_reads: bool,
    pub browser: Option<Value>,
}

pub fn loaded_of(desktop: &Value, talk: Option<&Value>, browser: Option<&Value>) -> Loaded {
    let mut rows = desk_rows(desktop.get("recent").unwrap_or(&Value::Null));
    if let Some(calls) = browser.and_then(|b| b.get("browser")).and_then(|b| b.get("calls")) {
        rows.extend(browser_rows(calls));
    }
    Loaded {
        desk: desktop.get("card").and_then(desk_card_of),
        talk: talk.and_then(|t| t.get("talk")).and_then(|t| t.get("confirm")).and_then(talk_card_of),
        rows,
        browser: browser.and_then(|value| value.get("browser")).cloned(),
        allow_reads: desktop.get("allowReads").and_then(Value::as_bool).unwrap_or(false),
    }
}

pub fn audit_dir(data: &Path) -> PathBuf {
    data.join(AUDIT_DIR)
}
