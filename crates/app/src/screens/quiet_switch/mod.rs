use crate::screens::phone_chip::chips::{ChipSpec, chip};
use crate::screens::phone_chip::info::clock;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Entity, Global};
use nixa_ui::{Icon, Theme};
use serde_json::{Value, json};

pub const QUIET_ID: &str = "chat-quiet-until";
pub const AWAY_ID: &str = "chat-away";
pub const MINUTE_MS: u64 = 60_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presence {
    pub known: bool,
    pub here: bool,
    pub away: bool,
    pub away_minutes: u64,
    pub held: u64,
    pub quiet_until: Option<u64>,
    pub last_seen: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    Quiet { until: u64 },
    Away { minutes: u64 },
}

#[derive(Default)]
pub struct QuietSwitch {
    pub presence: Presence,
    pub sent: Vec<Value>,
}

struct Hub(Entity<QuietSwitch>);

impl Global for Hub {}

pub fn hub(cx: &mut App) -> Entity<QuietSwitch> {
    if let Some(h) = cx.try_global::<Hub>() {
        return h.0.clone();
    }
    let h = cx.new(|_| QuietSwitch::default());
    cx.set_global(Hub(h.clone()));
    h
}

pub fn current(cx: &App) -> Presence {
    cx.try_global::<Hub>().map(|h| h.0.read(cx).presence.clone()).unwrap_or_default()
}

pub fn ask_message() -> Value {
    json!({"type": "talk_state"})
}

pub fn end_quiet() -> Value {
    json!({"type": "set_quiet", "minutes": 0})
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

pub fn parse(v: &Value) -> Option<Presence> {
    v.as_object()?;
    Some(Presence {
        known: true,
        here: flag(v, "here"),
        away: flag(v, "away"),
        away_minutes: number(v, "awayMinutes").unwrap_or(0),
        held: number(v, "held").unwrap_or(0),
        quiet_until: number(v, "quietUntil").filter(|at| *at > 0),
        last_seen: number(v, "lastSeen").filter(|at| *at > 0),
    })
}

pub fn presence_of(msg: &Value) -> Option<Presence> {
    if msg.get("type").and_then(Value::as_str) != Some("talk") {
        return None;
    }
    parse(msg.get("presence")?)
}

pub fn feed(msg: &Value, cx: &mut App) -> bool {
    let Some(fresh) = presence_of(msg) else {
        return false;
    };
    let h = hub(cx);
    h.update(cx, |h, cx| {
        if h.presence != fresh {
            h.presence = fresh;
            cx.notify();
        }
    });
    true
}

pub fn away_minutes(p: &Presence, now: u64) -> u64 {
    match p.last_seen {
        Some(seen) => p.away_minutes.max((now.saturating_sub(seen) + MINUTE_MS / 2) / MINUTE_MS),
        None => p.away_minutes,
    }
}

pub fn chip_of(p: &Presence, now: u64) -> Option<Chip> {
    if let Some(until) = p.quiet_until.filter(|until| *until > now) {
        return Some(Chip::Quiet { until });
    }
    p.away.then(|| Chip::Away { minutes: away_minutes(p, now) })
}

pub fn spec(which: &Chip, name: &str, offset_min: i32, t: &Theme) -> ChipSpec {
    match which {
        Chip::Quiet { until } => ChipSpec {
            id: QUIET_ID.into(),
            which: Icon::VolumeOff,
            label: format!("Quiet until {}", clock(*until, offset_min)),
            tip: format!("{name} keeps its spoken lines in the chat until then. Click to let {name} speak again."),
            color: t.yellow,
        },
        Chip::Away { minutes } => ChipSpec {
            id: AWAY_ID.into(),
            which: Icon::Ear,
            label: "Away".into(),
            tip: format!("No sign of you for {minutes} minutes. {name} keeps its spoken lines in the chat and speaks a summary when you are back."),
            color: t.dim,
        },
    }
}

pub fn speak_again(cx: &mut App) {
    let value = end_quiet();
    let h = hub(cx);
    h.update(cx, |h, cx| {
        h.sent.push(value.clone());
        cx.notify();
    });
    crate::screens::talk_card::tell_brain(&value, cx);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn offset() -> i32 {
    if cfg!(test) { 0 } else { crate::screens::tabbar::local_offset_min() }
}

pub fn render_at(t: &Theme, name: &str, now: u64, cx: &App) -> Option<AnyElement> {
    let which = chip_of(&current(cx), now)?;
    let quiet = matches!(which, Chip::Quiet { .. });
    let el = chip(t, &spec(&which, name, offset(), t));
    Some(if quiet {
        el.on_click(|_, _, cx| {
            cx.stop_propagation();
            speak_again(cx);
        })
        .into_any_element()
    } else {
        el.cursor_default().into_any_element()
    })
}

pub fn render(t: &Theme, name: &str, cx: &App) -> Option<AnyElement> {
    render_at(t, name, now_ms(), cx)
}

#[cfg(test)]
mod tests;
