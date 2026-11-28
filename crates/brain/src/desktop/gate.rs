use super::check::Step;
use super::{CARD_KIND, State};
use crate::app::{App, lock};
use crate::util::{atomic_write, now_ms, read_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub const STEPS_FILE: &str = "steps.jsonl";
pub const RECENT_FILE: &str = "recent.json";
pub const PERMISSION_FILE: &str = "permission.json";
pub const RECENT_MAX: usize = 30;
pub const CARD_MS: u64 = crate::decision::YES_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Card,
    ReadSetting,
    AppWindow,
    Automatic,
}

impl Approval {
    pub fn word(self) -> &'static str {
        match self {
            Approval::Card => "one-step card",
            Approval::ReadSetting => "the user's read setting",
            Approval::AppWindow => "Nixa window rule",
            Approval::Automatic => "Nixa decides",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Held {
    pub id: String,
    #[serde(skip)]
    pub key: String,
    pub tool: String,
    pub action: String,
    pub message: String,
    pub step: Step,
    pub at: u64,
    #[serde(skip)]
    pub yes: Option<u64>,
    pub hard: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    Go(String),
    Held(Value),
    No(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Permission {
    pub allow_reads: bool,
}

pub fn new_id() -> String {
    let hex = crate::cli_brains::new_token();
    let hex = if hex.len() < 32 { format!("{hex:0<32}") } else { hex };
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

pub fn key_of(tool: &str, args: &crate::tools::Args) -> String {
    crate::util::short_hash(&format!("{tool} {}", serde_json::to_string(args).unwrap_or_default()))
}

pub fn audit(state: &State, row: Value) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(&state.dir).map_err(|e| format!("The desktop log folder cannot be made: {e}"))?;
    let mut event = json!({"at": iso_time(now_ms())});
    if let (Some(out), Value::Object(more)) = (event.as_object_mut(), row) {
        out.extend(more);
    }
    let line = nixa_store::text::hide_secrets(&serde_json::to_string(&event).map_err(|e| e.to_string())?, &[]);
    crate::util::append_line(&state.dir.join(STEPS_FILE), &line).map_err(|e| format!("The desktop log could not be written, so nothing ran: {e}"))?;
    let mirror = {
        let mut recent = lock(&state.recent);
        let shown = match event.get("result") {
            Some(result) => {
                let mut shown = event.clone();
                shown["result"] = super::format::result(result);
                shown
            }
            None => event,
        };
        recent.push(shown);
        let extra = recent.len().saturating_sub(RECENT_MAX);
        recent.drain(..extra);
        nixa_store::text::hide_secrets(&serde_json::to_string(&*recent).unwrap_or_default(), &[])
    };
    if let Err(e) = atomic_write(&state.dir.join(RECENT_FILE), &mirror) {
        crate::util::log("desktop", &format!("the recent list was not saved: {e}"));
    }
    Ok(())
}

pub fn iso_time(ms: u64) -> String {
    let days = (ms / 86_400_000) as i64;
    let (year, month, day) = nixa_store::time::civil_from_days(days);
    let rest = ms % 86_400_000;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000)
}

pub fn load_recent(dir: &std::path::Path) -> Vec<Value> {
    let list: Vec<Value> = read_json(&dir.join(RECENT_FILE)).unwrap_or_default();
    let extra = list.len().saturating_sub(RECENT_MAX);
    list.into_iter().skip(extra).collect()
}

pub fn permission(state: &State) -> Permission {
    read_json(&state.dir.join(PERMISSION_FILE)).unwrap_or_default()
}

pub fn save_permission(state: &State, allow_reads: bool) -> Result<Permission, String> {
    nixa_protocol::paths::ensure_private_dir(&state.dir).map_err(|e| e.to_string())?;
    let saved = Permission { allow_reads };
    crate::util::write_json(&state.dir.join(PERMISSION_FILE), &saved).map_err(|e| e.to_string())?;
    Ok(saved)
}

pub fn hard(step: &Step) -> bool {
    let text = serde_json::to_string(step).unwrap_or_default();
    crate::risk::hard_stop_text(&text) || regex::Regex::new(r"(?i)\b(?:pay|buy|purchase|checkout|transfer|money|subscribe|payment)\b").is_ok_and(|r| r.is_match(&text))
}

pub fn message(id: &str, step: &Step, note: Option<&str>, hard: bool) -> String {
    let details = serde_json::to_string(step).unwrap_or_default();
    let mut lines = vec![format!("Desktop step {id}"), format!("{}: {}", step.tool, step.action), details];
    if let Some(note) = note {
        lines.push(note.to_string());
    }
    if hard {
        lines.push("Hard stop: this exact step needs the user's own yes.".into());
    }
    lines.push(if step.action == "vision" {
        "The picture goes to an image reading model through an API brain the user saved; only a text answer comes back to Nixa's brain. Allow this step?".into()
    } else {
        "Visible text goes back to Nixa's brain. Allow this step and the screen read after it?".into()
    });
    lines.join("\n")
}

pub fn card_view(held: &Held) -> Value {
    json!({"id": held.id, "tool": held.tool, "action": held.action, "message": held.message, "step": held.step, "at": held.at})
}

fn chat_yes(app: &App, held: &Held, now: u64) -> Option<u64> {
    if held.hard {
        return None;
    }
    crate::yes_once::chat_yes(app, held.at, now, crate::decision::YES_MS, &step_names(&held.action))
}

fn step_names(action: &str) -> Vec<&'static str> {
    let mut names = step_words(action).to_vec();
    names.push("desktop");
    names
}

pub fn step_words(action: &str) -> &'static [&'static str] {
    match action {
        "click" => &["click", "press"],
        "type" => &["type", "fill"],
        "open" => &["open", "safari", "browser"],
        "vision" => &["vision", "picture", "look"],
        "shot" => &["shot", "screenshot", "capture"],
        "windows" => &["windows", "list"],
        _ => &["read", "screen"],
    }
}

pub fn say_yes(action: &str) -> String {
    format!("yes {}", step_words(action)[0])
}

pub fn names_step(line: &str, action: &str) -> bool {
    crate::yes_once::says_yes_to(line, &step_names(action))
}

fn held_reply(held: &Held) -> Value {
    json!({
        "id": held.id,
        "outcome": "held",
        "note": if held.hard {
            "Held for the user's yes on the desktop card in the Eyes or Hands panel. This is a hard stop, so a yes in chat does not count. Tell the user in one sentence what waits. After the card yes, call this tool again with the same input. Nothing ran.".to_string()
        } else {
            format!("Held for the user's yes. Tell the user in one sentence what waits. A plain yes does not count: the user says \"{}\" (or \"yes desktop\") in chat, or answers the desktop card. Then call this tool again with the same input. Nothing ran.", say_yes(&held.action))
        },
    })
}

pub fn ask(app: &Arc<App>, key: &str, step: &Step, hard: bool, message: impl Fn(&str) -> String) -> Ask {
    let state = &app.desktop;
    let now = now_ms();
    let mut held = lock(&state.held);
    if let Some(h) = held.as_ref().filter(|h| h.key == key && now.saturating_sub(h.at) <= CARD_MS) {
        if let Some(at) = h.yes {
            let id = h.id.clone();
            *held = None;
            crate::yes_once::use_yes(app, at, format!("desktop {id}"));
            return Ask::Go(id);
        }
        if let Some(at) = chat_yes(app, h, now) {
            let id = h.id.clone();
            *held = None;
            crate::yes_once::use_yes(app, at, format!("desktop {id}"));
            return Ask::Go(id);
        }
        return Ask::Held(held_reply(h));
    }
    if let Some(old) = held.take() {
        let _ = audit(state, json!({"id": old.id, "phase": "cancelled", "step": old.step, "result": "A newer desktop step replaced this card. Nothing ran."}));
    }
    let id = new_id();
    let card = Held { id: id.clone(), key: key.to_string(), tool: step.tool.clone(), action: step.action.clone(), message: message(&id), step: step.clone(), at: now, yes: None, hard };
    let reply = held_reply(&card);
    if let Err(e) = audit(state, json!({"id": id, "phase": "held", "step": step})) {
        return Ask::No(e);
    }
    app.send_out(json!({"type": CARD_KIND, "card": card_view(&card)}));
    let how = if hard { "Answer the card in the Eyes or Hands panel.".to_string() } else { format!("Say \"{}\", or answer the card in the Eyes or Hands panel.", say_yes(&step.action)) };
    app.info(&format!("Held for you: desktop step {} {}. {how}", step.tool, step.action));
    *held = Some(card);
    Ask::Held(reply)
}

pub fn answer(app: &Arc<App>, id: &str, yes: bool) -> Result<Value, String> {
    let state = &app.desktop;
    let card = {
        let mut held = lock(&state.held);
        let Some(card) = held.as_mut().filter(|h| h.id == id) else { return Err("This desktop card is not waiting any more.".into()) };
        if now_ms().saturating_sub(card.at) > CARD_MS {
            *held = None;
            return Err("This desktop card is too old. Ask Nixa again.".into());
        }
        if yes {
            card.yes = Some(now_ms());
            card.clone()
        } else {
            let card = card.clone();
            *held = None;
            card
        }
    };
    if yes {
        audit(state, json!({"id": id, "phase": "answered", "step": card.step, "answer": "yes"}))?;
        crate::agent::tell(app, format!("The user said yes on the desktop card {id} ({} {}). Call {} again with the same input now. The yes covers only this one step.", card.tool, card.action, card.tool));
    } else {
        audit(state, json!({"id": id, "phase": "cancelled", "step": card.step, "result": "The user said no. Nothing ran."}))?;
        crate::agent::tell(app, format!("The user said no on the desktop card {id} ({} {}). Do not run it or ask again unless the user asks.", card.tool, card.action));
    }
    app.send_out(json!({"type": CARD_KIND, "card": Value::Null}));
    Ok(json!({"id": id, "answer": if yes { "yes" } else { "no" }}))
}
