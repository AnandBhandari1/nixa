use super::open_db;
use super::routes::store_request;
use crate::app::App;
use crate::tools::{Args, ToolResult, schema, spec, text};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::sync::Arc;

pub const NAME: &str = "nixa_workflows";
pub const READS: [&str; 5] = ["list", "read", "check", "runs", "run"];
const ABOUT: &str = "Saved workflows are step by step plans (boxes and links) that a task can follow: each stage box goes to an agent, review boxes send work back with revise, gates wait for checks, the deep brain or the user. Nixa or the deep brain answers every gate and run question, and nothing waits for the user. This tool only reads. list shows every workflow with its when line. read shows one workflow (name). check runs the checker on a saved workflow (name). runs lists the active runs. run shows the run of one task (task). The user edits workflows in the Workflows window.";

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        ABOUT,
        schema(
            json!({
                "action": {"type": "string", "enum": READS},
                "name": {"type": "string", "maxLength": 60},
                "task": {"type": "integer", "minimum": 1}
            }),
            &["action"],
        ),
    )]
}

pub fn text_of(action: &str, data: &Value) -> String {
    match action {
        "list" => {
            let list = data["workflows"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                return "No workflows saved.".into();
            }
            list.iter()
                .map(|w| {
                    let broken = w["broken"].as_str().map(|b| format!(" BROKEN: {b}")).unwrap_or_default();
                    format!("{}: {}, {} boxes.{broken} When: {}", w["name"].as_str().unwrap_or(""), w["title"].as_str().unwrap_or(""), w["nodes"], w["when"].as_str().unwrap_or(""))
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "check" => {
            let problems: Vec<String> = data["problems"].as_array().cloned().unwrap_or_default().iter().filter_map(|p| p.as_str().map(str::to_string)).collect();
            if problems.is_empty() {
                "The workflow passes the checker.".into()
            } else {
                format!("The workflow has {} problem{}:\n{}", problems.len(), if problems.len() == 1 { "" } else { "s" }, problems.iter().map(|p| format!("- {p}")).collect::<Vec<_>>().join("\n"))
            }
        }
        "runs" => {
            let list = data["runs"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                return "No workflow run is active.".into();
            }
            list.iter().map(|r| format!("Task #{} ({}): {}", r["task"], r["title"].as_str().unwrap_or(""), r["line"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")
        }
        "run" if data.get("run").is_none() => "That task has no workflow run.".into(),
        "run" => data["run"]["line"].as_str().unwrap_or("").to_string(),
        _ => serde_json::to_string_pretty(&data["workflow"]).unwrap_or_default(),
    }
}

pub async fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    let action = text(args, "action").unwrap_or_else(|| "list".into());
    if !READS.contains(&action.as_str()) {
        return Err(format!("{NAME} only reads: {}.", READS.join(", ")));
    }
    let mut args = args.clone();
    if action == "check" {
        let worker = app.clone();
        let name = text(&args, "name").unwrap_or_default();
        let saved = tokio::task::spawn_blocking(move || open_db(&worker).and_then(|conn| store_request(&conn, "read", &json_args(&name), &Value::Null)))
            .await
            .map_err(|e| e.to_string())?;
        match saved {
            Ok((data, _)) => {
                args.insert("workflow".into(), data["workflow"].clone());
            }
            Err(error) => return Ok(format!("The workflow has 1 problem:\n- {error}")),
        }
    }
    let worker = app.clone();
    let shown = action.clone();
    let (data, _) = tokio::task::spawn_blocking(move || open_db(&worker).and_then(|conn| store_request(&conn, &shown, &args, &Value::Null)))
        .await
        .map_err(|e| e.to_string())??;
    Ok(text_of(&action, &data))
}

fn json_args(name: &str) -> Args {
    let mut args = Args::new();
    args.insert("name".into(), json!(name));
    args
}
