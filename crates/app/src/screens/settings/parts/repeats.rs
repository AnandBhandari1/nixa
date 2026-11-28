mod view;

use crate::links::brain::{BrainMsg, Link, NOT_CONNECTED};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Window};
use nixa_ui::text_input::TextInput;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub const PART: &str = "Repeat messages";
pub const KINDS: [&str; 1] = ["schedules"];
pub const UNITS: [(&str, &str); 2] = [("minutes", "Minutes"), ("hours", "Hours")];
pub const MOST_MINUTES: i64 = 7 * 24 * 60;
pub const CLIP: usize = 240;
const POLL: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Shown {
    pub every: String,
    pub next: String,
    pub last: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub text: String,
    pub every: i64,
    pub unit: String,
    pub enabled: bool,
    pub sent: u64,
    pub refused: u64,
    pub next_at: Option<i64>,
    pub last_note: Option<String>,
    pub view: Shown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub repeats: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub id: Option<String>,
    pub unit: String,
    pub enabled: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub ok: bool,
    pub text: String,
}

pub struct Inputs {
    pub text: Entity<TextInput>,
    pub every: Entity<TextInput>,
}

pub struct Repeats {
    pub snap: Option<Snapshot>,
    pub load_error: Option<String>,
    pub retry_at: Option<std::time::Instant>,
    pub form: Option<Form>,
    pub removing: Option<String>,
    pub notice: Option<Notice>,
    pub pending: Option<String>,
    pub inputs: Inputs,
    pub link: Option<Link>,
    pub focus: FocusHandle,
    rx: Option<Receiver<BrainMsg>>,
}

impl Focusable for Repeats {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Repeats {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        view::render(self, cx)
    }
}

pub fn new(link: Option<Link>, cx: &mut App) -> Entity<Repeats> {
    let view = cx.new(|cx| build(link, cx));
    view.update(cx, |part, cx| {
        reload(part, cx);
        listen(cx);
    });
    view
}

pub fn build(link: Option<Link>, cx: &mut Context<Repeats>) -> Repeats {
    let inputs = Inputs {
        text: cx.new(|cx| TextInput::new(cx, "What Nixa gets, like: check the open tasks")),
        every: cx.new(|cx| TextInput::new(cx, "15")),
    };
    let rx = link.as_ref().map(|l| l.subscribe(&KINDS));
    Repeats {
        snap: None,
        load_error: None,
        retry_at: None,
        form: None,
        removing: None,
        notice: None,
        pending: None,
        inputs,
        link,
        focus: cx.focus_handle(),
        rx,
    }
}

fn listen(cx: &mut Context<Repeats>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            if this.update(cx, drain).is_err() {
                break;
            }
        }
    })
    .detach();
}

pub fn drain(part: &mut Repeats, cx: &mut Context<Repeats>) {
    let now = std::time::Instant::now();
    if part.snap.is_none() && part.load_error.is_some() && part.retry_at.is_none_or(|at| now >= at) {
        part.retry_at = Some(now + Duration::from_secs(3));
        reload(part, cx);
    }
    let mut msgs = Vec::new();
    if let Some(rx) = &part.rx {
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
    }
    for msg in msgs {
        match msg {
            BrainMsg::Up(_) => reload(part, cx),
            BrainMsg::Down(why) => {
                part.load_error = Some(why);
                cx.notify();
            }
            BrainMsg::Event(value) => {
                if value.get("type").and_then(Value::as_str) == Some("schedules")
                    && let Some(snap) = snapshot(&value)
                {
                    take(part, snap);
                    cx.notify();
                }
            }
        }
    }
}

pub fn snapshot(value: &Value) -> Option<Snapshot> {
    value.get("repeats")?;
    serde_json::from_value::<Snapshot>(value.clone()).ok()
}

pub fn take(part: &mut Repeats, snap: Snapshot) {
    if let Some(id) = &part.removing
        && !snap.repeats.iter().any(|r| &r.id == id)
    {
        part.removing = None;
    }
    part.snap = Some(snap);
    part.load_error = None;
}

pub fn reload(part: &mut Repeats, cx: &mut Context<Repeats>) {
    let Some(link) = part.link.clone() else {
        part.load_error = Some(NOT_CONNECTED.into());
        cx.notify();
        return;
    };
    let task = cx.background_executor().spawn(async move { link.get("/schedules") });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |part, cx| {
            match result {
                Ok(value) => match snapshot(&value) {
                    Some(snap) => take(part, snap),
                    None => part.load_error = Some("Nixa sent no repeat messages list. Update Nixa.".into()),
                },
                Err(error) => part.load_error = Some(format!("{error:#}")),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn send(part: &mut Repeats, action: &str, payload: Value, label: &str, cx: &mut Context<Repeats>) {
    let Some(link) = part.link.clone() else {
        part.notice = Some(Notice { ok: false, text: "Nixa is not connected. Nothing was sent.".into() });
        cx.notify();
        return;
    };
    if part.pending.is_some() {
        return;
    }
    part.pending = Some(label.to_string());
    part.notice = None;
    cx.notify();
    let body = json!({"action": action, "payload": payload});
    let task = cx.background_executor().spawn(async move { link.post("/schedules", &body) });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |part, cx| done(part, result.map_err(|e| format!("{e:#}")), cx)).ok();
    })
    .detach();
}

pub fn done(part: &mut Repeats, result: Result<Value, String>, cx: &mut Context<Repeats>) {
    let label = part.pending.take().unwrap_or_else(|| "Change".into());
    match result {
        Ok(value) => {
            if let Some(snap) = snapshot(&value) {
                take(part, snap);
            }
            part.form = None;
            part.removing = None;
            part.notice = Some(Notice { ok: true, text: format!("{label}: done.") });
        }
        Err(error) => {
            let text = if error.is_empty() { format!("{label} failed.") } else { error };
            if let Some(form) = part.form.as_mut() {
                form.error = Some(text);
            } else {
                part.notice = Some(Notice { ok: false, text });
            }
        }
    }
    cx.notify();
}

pub fn open_form(part: &mut Repeats, item: Option<&Item>, cx: &mut Context<Repeats>) {
    let (text, every) = item.map(|i| (i.text.clone(), i.every.to_string())).unwrap_or_else(|| (String::new(), "15".into()));
    part.inputs.text.update(cx, |i, cx| i.set_value(text, cx));
    part.inputs.every.update(cx, |i, cx| i.set_value(every, cx));
    part.form = Some(Form {
        id: item.map(|i| i.id.clone()),
        unit: item.map(|i| i.unit.clone()).filter(|u| UNITS.iter().any(|(k, _)| k == u)).unwrap_or_else(|| "minutes".into()),
        enabled: item.is_none_or(|i| i.enabled),
        error: None,
    });
    part.removing = None;
    part.notice = None;
    cx.notify();
}

pub fn payload(form: &Form, text: &str, every: &str) -> Result<Value, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Write the message to send.".into());
    }
    let every: i64 = every.trim().parse().map_err(|_| "How often must be a whole number, like 15.".to_string())?;
    let most = if form.unit == "hours" { MOST_MINUTES / 60 } else { MOST_MINUTES };
    if every < 1 || every > most {
        return Err("How often must be from 1 minute to 7 days.".into());
    }
    let mut out = json!({"text": text, "every": every, "unit": form.unit, "enabled": form.enabled});
    if let Some(id) = &form.id {
        out["id"] = json!(id);
    }
    Ok(out)
}

pub fn save_form(part: &mut Repeats, cx: &mut Context<Repeats>) {
    let Some(form) = part.form.clone() else { return };
    let text = part.inputs.text.read(cx).value().to_string();
    let every = part.inputs.every.read(cx).value().to_string();
    match payload(&form, &text, &every) {
        Ok(repeat) => {
            let label = if form.id.is_some() { "Save repeat message" } else { "Add repeat message" };
            send(part, "repeat.save", json!({"repeat": repeat}), label, cx);
        }
        Err(error) => {
            if let Some(f) = part.form.as_mut() {
                f.error = Some(error);
            }
            cx.notify();
        }
    }
}

pub fn status(item: &Item) -> String {
    let sent = match item.sent {
        0 => "not sent yet".to_string(),
        1 => "sent 1 time".to_string(),
        n => format!("sent {n} times"),
    };
    let next = if item.enabled { format!("next {}", item.view.next) } else { "paused".to_string() };
    let refusal = if item.refused > 0 { format!(" · {} refused", item.refused) } else { String::new() };
    format!("{} · {next} · {sent}{refusal}", item.view.every)
}

fn groups(items: &[Item]) -> BTreeMap<i64, Vec<&Item>> {
    let mut out: BTreeMap<i64, Vec<&Item>> = BTreeMap::new();
    for item in items {
        let minutes = if item.unit == "hours" { item.every.saturating_mul(60) } else { item.every };
        out.entry(minutes).or_default().push(item);
    }
    for group in out.values_mut() {
        group.sort_by_key(|item| (item.next_at.unwrap_or(i64::MAX), &item.id));
    }
    out
}

fn frequency(minutes: i64) -> String {
    let (n, unit) = if minutes >= 60 && minutes % 60 == 0 { (minutes / 60, "hour") } else { (minutes, "minute") };
    if n == 1 { format!("Every {unit}") } else { format!("Every {n} {unit}s") }
}

pub fn clip(text: &str) -> String {
    if text.chars().count() <= CLIP {
        return text.to_string();
    }
    let mut out: String = text.chars().take(CLIP).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
