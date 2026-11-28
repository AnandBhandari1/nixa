mod show;
mod snapshot;
mod watch;

#[cfg(test)]
mod tests;

use crate::app::App;
use serde_json::{Value, json};
use std::sync::Arc;

pub use show::{ANSWERED, agent_word, show_on};
pub use snapshot::{send_snapshot, snapshot};
pub use watch::{MAX_MINUTES, dry_run, minutes_of, set_watch, timer_line};

pub const KINDS: [&str; 5] = ["set_watch", "watch_dry_run", "automation", "show", "snapshot"];

pub fn kind_of(message: &Value) -> Option<&str> {
    message.get("type").and_then(Value::as_str).filter(|kind| KINDS.contains(kind))
}

pub fn is_ws(message: &Value) -> bool {
    kind_of(message).is_some()
}

pub async fn ws_request(app: &Arc<App>, message: &Value) -> Option<Value> {
    let kind = kind_of(message)?.to_string();
    if kind != "snapshot" {
        crate::talk::flow::touch_later(app);
    }
    let done = run(app, &kind, message).await;
    let request = request_of(message)?;
    Some(answer(&kind, request, done))
}

pub async fn run(app: &Arc<App>, kind: &str, message: &Value) -> Result<Value, String> {
    match kind {
        "set_watch" => set_watch(app, message),
        "watch_dry_run" => dry_run(app, message),
        "automation" => automation(app, message),
        "show" => show_on(app, message, nixa_protocol::control::app_socket_in(&app.dir)).await,
        "snapshot" => Ok(send_snapshot(app)),
        other => Err(format!("The brain does not know the message {other}.")),
    }
}

pub fn request_of(message: &Value) -> Option<Value> {
    match message.get("request")? {
        Value::String(text) if !text.trim().is_empty() => Some(json!(text)),
        Value::Number(number) => Some(json!(number)),
        _ => None,
    }
}

pub fn answer(kind: &str, request: Value, done: Result<Value, String>) -> Value {
    let kind = format!("{kind}_reply");
    match done {
        Ok(data) => json!({"type": kind, "request": request, "ok": true, "data": data}),
        Err(error) => json!({"type": kind, "request": request, "ok": false, "error": error}),
    }
}

pub fn automation(app: &Arc<App>, message: &Value) -> Result<Value, String> {
    let action = message
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|action| !action.is_empty())
        .ok_or("automation needs an action, for example schedule.run.")?;
    let payload = message.get("payload").cloned().unwrap_or_else(|| json!({}));
    if !payload.is_object() {
        return Err("automation needs payload as an object.".into());
    }
    crate::schedules::jobs::command(app, action, &payload).map_err(|error| format!("{error:#}"))?;
    crate::util::log("automation", action);
    Ok(crate::schedules::snapshot(app))
}
