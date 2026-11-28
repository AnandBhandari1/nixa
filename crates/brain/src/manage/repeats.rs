use crate::app::App;
use crate::schedules::jobs;
use crate::schedules::jobs::repeats::TEXT_MAX;
use crate::tools::{Args, ToolResult, need, schema, spec};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::sync::Arc;

pub const NAME: &str = "nixa_repeats";
pub const ACTIONS: [&str; 7] = ["list", "add", "change", "pause", "resume", "send", "remove"];
pub const WHY: &str = "A repeat message change asked by the chat brain";
pub const PREVIEW_CHARS: usize = 300;

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Repeat messages: a saved text that Nixa chat gets again and again, every N minutes or hours (1 minute to 7 days), for example a check-in on idle agents every 15 minutes. list shows each one with its id, text, how often, next and last send. add saves a new one (text, every, unit). change changes the text, every, unit or enabled of the repeat id. pause and resume turn one off or on, send sends it now, remove deletes it.",
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "id": {"type": "string", "description": "Repeat id from list, for change, pause, resume, send and remove. Never invent ids."},
                "text": {"type": "string", "description": format!("The message Nixa chat gets, up to {TEXT_MAX} letters.")},
                "every": {"type": "integer", "minimum": 1, "description": "How often, a whole number, with unit."},
                "unit": {"type": "string", "enum": ["minutes", "hours"], "description": "Default minutes."},
                "enabled": {"type": "boolean", "description": "Default true for add."}
            }),
            &["action"],
        ),
    )
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = need(args, "action")?;
    match action.as_str() {
        "list" => Ok(json!({"repeats": jobs::repeats::list_json(app)}).to_string()),
        "add" => save(app, args, None).await,
        "change" => {
            let id = need(args, "id")?;
            save(app, args, Some(id)).await
        }
        "pause" | "resume" => toggle(app, args, action == "resume").await,
        "send" | "remove" => one(app, args, &action).await,
        other => Err(format!("action {other} is not known. Use one of: {}.", ACTIONS.join(", "))),
    }
}

fn list(app: &App) -> Vec<Value> {
    jobs::repeats::list_json(app).as_array().cloned().unwrap_or_default()
}

fn found(app: &App, id: &str) -> Result<Value, String> {
    list(app).into_iter().find(|r| r.get("id").and_then(Value::as_str) == Some(id)).ok_or_else(|| format!("Repeat {id} not found. Use list for the ids."))
}

fn run(app: &Arc<App>, action: &str, payload: Value) -> Result<(), String> {
    jobs::command(app, action, &payload).map_err(|e| format!("{e:#}"))
}

pub fn merged(old: Option<&Value>, args: &Args) -> Value {
    let pick = |key: &str| args.get(key).cloned().or_else(|| old.and_then(|o| o.get(key).cloned()));
    let mut repeat = json!({
        "text": pick("text").unwrap_or(Value::Null),
        "every": pick("every").unwrap_or(Value::Null),
        "unit": pick("unit").unwrap_or_else(|| json!("minutes")),
        "enabled": pick("enabled").unwrap_or(json!(true)),
    });
    if let Some(id) = old.and_then(|o| o.get("id")).cloned() {
        repeat["id"] = id;
    }
    repeat
}

async fn save(app: &Arc<App>, args: &Args, id: Option<String>) -> ToolResult {
    let old = match &id {
        Some(id) => Some(found(app, id)?),
        None => None,
    };
    let repeat = merged(old.as_ref(), args);
    jobs::repeats::read(&repeat).map_err(|e| format!("{e:#}"))?;
    let before: Vec<String> = list(app).iter().filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_string)).collect();
    run(app, "repeat.save", json!({"repeat": repeat}))?;
    let saved = match id {
        Some(id) => found(app, &id)?,
        None => list(app).into_iter().find(|r| r.get("id").and_then(Value::as_str).is_some_and(|i| !before.iter().any(|b| b == i))).unwrap_or(Value::Null),
    };
    Ok(json!({"status": "done", "action": if old.is_some() { "change" } else { "add" }, "repeat": saved}).to_string())
}

async fn toggle(app: &Arc<App>, args: &Args, on: bool) -> ToolResult {
    let id = need(args, "id")?;
    let r = found(app, &id)?;
    if r.get("enabled").and_then(Value::as_bool) == Some(on) {
        return Ok(json!({"status": "unchanged", "reason": format!("Repeat {id} is already {}.", if on { "on" } else { "paused" })}).to_string());
    }
    run(app, "repeat.toggle", json!({"id": id, "enabled": on}))?;
    Ok(json!({"status": "done", "action": if on { "resume" } else { "pause" }, "repeat": found(app, &id)?}).to_string())
}

async fn one(app: &Arc<App>, args: &Args, action: &str) -> ToolResult {
    let id = need(args, "id")?;
    found(app, &id)?;
    let command = if action == "send" { "repeat.send" } else { "repeat.delete" };
    run(app, command, json!({"id": id}))?;
    Ok(json!({"status": "done", "action": action, "id": id}).to_string())
}
