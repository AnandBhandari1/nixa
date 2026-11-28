//! Chat brain tool `nixa_drones` and shared action dispatcher.

use super::{DroneStep, broadcast_changed, facts_map, results_json, run_tick, subjects};
use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, schema, spec};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use serde_json::{Value, json};
use std::sync::Arc;

pub const NAME: &str = "nixa_drones";
pub const ACTIONS: [&str; 12] = [
    "read", "facts", "add", "change", "remove", "dry_run", "run", "on", "off", "timing", "undo",
    "history",
];

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        "Read, test or change Nixa's drones (read-only code watchers that walk a JSON decision tree over tasks and terminals and report stalls).",
        schema(
            json!({
                "action": {"type": "string", "enum": ACTIONS},
                "id": {"type": "string", "description": "Step id for add, change, remove, on, off or dry_run."},
                "step": {"description": "Step JSON object (or JSON string) for add, change or dry_run."},
                "every": {"type": "integer", "description": "Minutes between drone checks (1..60) for timing."},
                "remind": {"type": "integer", "description": "Minutes before one reminder (1..240) for timing."}
            }),
            &["action"],
        ),
    )]
}

fn parse_step(raw: Option<&Value>) -> Result<DroneStep, String> {
    let raw = raw.ok_or_else(|| "step is required.".to_string())?;
    let value = match raw {
        Value::String(s) => serde_json::from_str::<Value>(s)
            .map_err(|_| "step must be a JSON object.".to_string())?,
        other => other.clone(),
    };
    serde_json::from_value::<DroneStep>(value).map_err(|e| format!("Invalid step: {e}"))
}

fn need_id(args: &Args, action: &str) -> Result<String, String> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{action} needs id, the step id."))
}

fn opt_minutes(args: &Args, key: &str) -> Result<Option<u64>, String> {
    let Some(v) = args.get(key) else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    if let Some(n) = v.as_u64() {
        return Ok(Some(n));
    }
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        return s
            .parse::<u64>()
            .map(Some)
            .map_err(|_| format!("{key} must be whole minutes."));
    }
    Err(format!("{key} must be whole minutes."))
}

pub async fn run_action(app: &Arc<App>, action: &str, args: &Args, who: &str) -> ToolResult {
    let action = action.trim();
    if !ACTIONS.contains(&action) {
        return Err(format!("Choose one of: {}.", ACTIONS.join(", ")));
    }
    let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    let now = now_ms();
    match action {
        "read" => Ok(pretty(lock(&app.drones.store).read_view())),
        "facts" => Ok(pretty(json!({
            "facts": facts_map(),
            "tests": "A test is { fact, is | in | over | under, not }. Only fact means true when the value is true or not empty.",
            "steps": "if (test fields, then, else), switch (fact, cases, default), all or any (tests, then, else), wait, report (text with {fact} slots). Any step may have label and off."
        }))),
        "history" => Ok(pretty(
            json!({"edits": lock(&app.drones.store).history(20)}),
        )),
        "run" => {
            let walks = run_tick(app, true)?;
            Ok(pretty(results_json(&walks)))
        }
        "dry_run" => {
            let change = if args.contains_key("id") || args.contains_key("step") {
                Some((need_id(args, "dry_run")?, parse_step(args.get("step"))?))
            } else {
                None
            };
            let mut list = subjects::collect(app).map_err(super::subjects_error)?;
            let mut tree = lock(&app.drones.store).tree.clone();
            if let Some((id, step)) = &change {
                tree.steps.insert(id.clone(), step.clone());
            }
            super::asks::fill(app, &tree, &mut list, crate::util::now_ms());
            let walks = match &change {
                Some((id, step)) => {
                    lock(&app.drones.store).dry_run(&list, Some((id.as_str(), step.clone())))?
                }
                None => lock(&app.drones.store).dry_run(&list, None)?,
            };
            Ok(pretty(results_json(&walks)))
        }
        "add" => {
            let id = need_id(args, "add")?;
            let step = parse_step(args.get("step"))?;
            lock(&app.drones.store).add_step(&id, step, who, now)?;
            broadcast_changed(app);
            Ok(format!(
                "Step {id} added to drone-tree.json. Edit saved and logged."
            ))
        }
        "change" => {
            let id = need_id(args, "change")?;
            let step = parse_step(args.get("step"))?;
            lock(&app.drones.store).change_step(&id, step, who, now)?;
            broadcast_changed(app);
            Ok(format!(
                "Step {id} updated in drone-tree.json. Edit saved and logged."
            ))
        }
        "remove" => {
            let id = need_id(args, "remove")?;
            lock(&app.drones.store).remove_step(&id, who, now)?;
            broadcast_changed(app);
            Ok(format!(
                "Step {id} removed from drone-tree.json. Edit saved and logged."
            ))
        }
        "on" | "off" => {
            let on = action == "on";
            let id = args.get("id").map(|_| need_id(args, action)).transpose()?;
            lock(&app.drones.store).set_on(on, who, id.as_deref(), now)?;
            broadcast_changed(app);
            let target = id
                .map(|i| format!("Step {i}"))
                .unwrap_or_else(|| "Drones".into());
            Ok(format!(
                "{target} turned {}. Edit saved and logged.",
                if on { "on" } else { "off" }
            ))
        }
        "timing" => {
            let every = opt_minutes(args, "every")?;
            let remind = opt_minutes(args, "remind")?;
            let tree = lock(&app.drones.store).set_timing(who, every, remind, now)?;
            broadcast_changed(app);
            Ok(format!(
                "Drones now check every {} minute(s) and remind after {} minutes. Edit saved and logged.",
                tree.every.unwrap_or(1),
                tree.remind_after.unwrap_or(super::DRONE_REMIND_MIN)
            ))
        }
        "undo" => {
            let edit = lock(&app.drones.store).undo(who, now)?;
            broadcast_changed(app);
            let step = edit.id.map(|i| format!(" ({i})")).unwrap_or_default();
            Ok(format!("Undid {} edit{step} by {}.", edit.op, edit.who))
        }
        _ => Err(format!("Choose one of: {}.", ACTIONS.join(", "))),
    }
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = match args.get("action") {
        Some(value) => value.as_str().ok_or("action must be text.")?,
        None => "read",
    };
    let who = format!("{} brain", app.name());
    run_action(app, action, args, &who).await
}
