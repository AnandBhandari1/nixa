use super::api::{Role, action};
use super::db;
use crate::app::App;
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use nixa_store::tasks;
use serde_json::{Value, json};
use std::sync::Arc;

pub const START: &str = "nixa_task_start";
pub const SPACE: &str = "nixa_task_space";
pub const PLAN: &str = super::plan::TOOL;
pub const NAMES: [&str; 3] = [START, SPACE, PLAN];
pub fn specs() -> Vec<ToolSpec> {
    vec![
        spec(
            START,
            "Start a task from the task list now: Nixa claims it and gives it to its assigned custom agent, terminal or agent type. Decision is automatic, so start it when the work is ready. A task that waits on a file clash or the space limit is queued and starts later by itself.",
            schema(
                json!({"id": {"type": "integer", "description": "The task number."}, "assign": {"type": "string", "description": "Optional: a custom agent id or a terminal id to assign first."}}),
                &["id"],
            ),
        ),
        spec(
            SPACE,
            "Show the task rules of the spaces, or change one space: auto start on or off, and how many tasks may run at once. Locking a space is the user's own, in the Nixa window.",
            schema(
                json!({
                    "space": {"type": "string", "description": "Space name or id. Leave out to list all spaces."},
                    "autoStart": {"type": "boolean", "description": "Start todo tasks with someone assigned by itself."},
                    "maxDoing": {"type": "integer", "description": "How many tasks may run at once, 1 to 10."}
                }),
                &[],
            ),
        ),
        super::plan::spec_of(),
    ]
}

fn line_of(value: &Value) -> String {
    let task = &value["task"];
    let id = task["id"].as_i64().unwrap_or(0);
    let terminal = task["terminal_id"].as_str().unwrap_or_default();
    if terminal.is_empty() {
        return format!(
            "Not started yet. Task #{id} is queued. Nixa starts it when the wait clears."
        );
    }
    let who = task["claimed_by"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(terminal);
    format!("Started task #{id}: {who} in terminal {terminal} holds it.")
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    Some(match name {
        START => start(app, args).await,
        PLAN => plan(app, args).await,
        SPACE => match text(args, "space") {
            None => action(app, "spaces", args, &Role::Brain)
                .await
                .map(|v| spaces_text(&v)),
            Some(_) => action(app, "space", args, &Role::Brain)
                .await
                .map(|v| v["text"].as_str().unwrap_or("Saved.").to_string()),
        },
        _ => return None,
    })
}

async fn start(app: &Arc<App>, args: &Args) -> ToolResult {
    let id = args
        .get("id")
        .and_then(Value::as_i64)
        .ok_or("id is needed: a task number.")?;
    if let Some(who) = text(args, "assign") {
        let conn = db(app)?;
        let kind = crate::tasks::assign_kind(&conn, &who)?;
        tasks::assign_task(&conn, id, kind, &who).map_err(|e| e.to_string())?;
    }
    let mut call = args.clone();
    call.remove("assign");
    action(app, "start", &call, &Role::Brain)
        .await
        .map(|v| line_of(&v))
}

async fn plan(app: &Arc<App>, args: &Args) -> ToolResult {
    let out = action(app, "plan", args, &Role::Brain).await?;
    Ok(out["text"].as_str().unwrap_or_default().to_string())
}

fn spaces_text(value: &Value) -> String {
    let list = value["spaces"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return "No spaces yet.".into();
    }
    list.iter()
        .map(|s| {
            let on = |key: &str| s[key].as_bool().unwrap_or(false);
            format!(
                "{} ({}): auto start {}, at most {} at once, {} doing{}{}",
                s["name"].as_str().unwrap_or_default(),
                s["id"].as_str().unwrap_or_default(),
                if on("autoStart") { "on" } else { "off" },
                s["maxDoing"].as_i64().unwrap_or(0),
                s["doing"].as_i64().unwrap_or(0),
                if on("locked") { ", locked" } else { "" },
                "",
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
