use super::model::{self, BoxType, Gate, LinkKind, WhoKind};
use super::{BASE_AGENTS, WINDOW_WHO, changed, driver, open_db};
use crate::app::App;
use crate::tools::Args;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nixa_store::Connection;
use nixa_store::workflows as store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const WS_KIND: &str = "workflows";
pub const RUN_ACTS: [&str; 5] = ["pause", "resume", "cancel", "retry", "answer"];
pub const ACTIONS: &str = "list, options, check, read, put, delete, undo, runs, run, pause, resume, cancel, retry or answer";

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/workflows", get(list_get).post(action_post))
}

fn answer(result: Result<Value, String>) -> Response {
    match result {
        Ok(mut data) => {
            data["ok"] = json!(true);
            (StatusCode::OK, axum::Json(data)).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

async fn list_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(request(&app, &Args::new()).await)
}

async fn action_post(State(app): State<Arc<App>>, headers: HeaderMap, body: Option<axum::Json<Value>>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    match body.map(|b| b.0) {
        Some(Value::Object(args)) => answer(request(&app, &args).await),
        None | Some(Value::Null) => answer(request(&app, &Args::new()).await),
        Some(_) => answer(Err("The body must be a JSON object.".into())),
    }
}

pub fn is_ws(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WS_KIND)
}

pub async fn ws_request(app: &Arc<App>, message: &Value) -> Option<Value> {
    let request_id = message.get("request").and_then(Value::as_str).map(str::to_string);
    let args = message.get("args").and_then(Value::as_object).cloned().unwrap_or_default();
    let result = request(app, &args).await;
    let request_id = request_id?;
    let reply = format!("{WS_KIND}_reply");
    Some(match result {
        Ok(data) => json!({"type": reply, "request": request_id, "ok": true, "data": data}),
        Err(error) => json!({"type": reply, "request": request_id, "ok": false, "error": error}),
    })
}

fn word(args: &Args, key: &str) -> String {
    args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

fn task_number(args: &Args, action: &str) -> Result<i64, String> {
    let n = match args.get("task") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().trim_start_matches('#').parse().ok(),
        _ => None,
    };
    n.filter(|n| *n >= 1).ok_or_else(|| format!("workflows {action} needs task, a task number."))
}

pub fn skills_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".claude").join("skills")
}

pub fn options_json(conn: &Connection, skills: &Path, tool_list: &[(String, String)], models: &[nixa_store::models::ModelRow]) -> Value {
    let agents = nixa_store::agents::list_agents(conn).unwrap_or_default();
    let custom_agents: Vec<Value> = agents
        .iter()
        .map(|a| json!({"id": a.id, "title": a.title, "purpose": a.purpose, "role": a.role, "provider": a.provider, "space": a.space, "off": a.off}))
        .collect();
    let rows: Vec<Value> = nixa_store::agent_map::map_rows(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !r.off)
        .map(|r| json!({"id": r.id, "job": r.job, "member": r.member, "agent": r.agent, "model": r.model}))
        .collect();
    let mut kinds: Vec<String> = BASE_AGENTS.iter().map(|k| k.to_string()).collect();
    kinds.extend(models.iter().map(|row| row.agent.clone()));
    kinds.extend(agents.iter().map(|a| a.provider.clone()).filter(|p| !p.is_empty()));
    kinds.sort();
    kinds.dedup();
    let mut skill_names: Vec<String> = std::fs::read_dir(skills)
        .map(|list| list.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.')).collect())
        .unwrap_or_default();
    skill_names.sort();
    let mut tools: Vec<&(String, String)> = tool_list.iter().collect();
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    json!({
        "custom_agents": custom_agents,
        "agent_kinds": kinds,
        "map_rows": rows,
        "models": models.iter().map(|row| json!({"agent": row.agent, "model": row.model, "efforts": row.efforts})).collect::<Vec<_>>(),
        "skills": skill_names.iter().map(|name| json!({"name": name})).collect::<Vec<_>>(),
        "tools": tools.iter().map(|(name, about)| json!({"name": name, "description": crate::workflows::run::clip(about, 160)})).collect::<Vec<_>>(),
        "box_types": BoxType::WORDS,
        "gates": Gate::WORDS,
        "link_kinds": LinkKind::WORDS,
        "who_kinds": WhoKind::WORDS.iter().filter(|w| !w.is_empty()).collect::<Vec<_>>(),
    })
}

pub fn list_json(conn: &Connection) -> Result<Value, String> {
    let saved = store::list_workflows(conn).map_err(|e| format!("{e:#}"))?;
    let infos: Vec<Value> = saved.iter().map(|w| serde_json::to_value(model::info(&w.name, &w.body)).unwrap_or(Value::Null)).collect();
    Ok(json!({"workflows": infos}))
}

fn read_saved(conn: &Connection, name: &str) -> Result<model::Workflow, String> {
    if !store::valid_name(name) {
        return Err(model::NAME_RULE.into());
    }
    let saved = store::get_workflow(conn, name).map_err(|e| format!("{e:#}"))?.ok_or_else(|| format!("There is no workflow named {name}."))?;
    model::parse_saved(&saved.name, &saved.body)
}

fn can_undo(conn: &Connection, name: &str) -> bool {
    store::can_undo(conn, name).unwrap_or(false)
}

pub fn store_request(conn: &Connection, action: &str, args: &Args, extra: &Value) -> Result<(Value, Option<String>), String> {
    let name = word(args, "name");
    let none = None;
    match action {
        "list" => return Ok((list_json(conn)?, none)),
        "options" => return Ok((json!({"options": extra}), none)),
        "check" => {
            let raw = args.get("workflow").cloned().unwrap_or(Value::Null);
            let saved = if name.is_empty() { None } else { Some(name.as_str()) };
            return Ok((json!({"problems": model::problems(&raw, saved)}), none));
        }
        "runs" => return Ok((json!({"runs": driver::run_list(conn)}), none)),
        "run" => {
            let task = task_number(args, action)?;
            return Ok((driver::run_show(conn, task).unwrap_or_else(|| json!({})), none));
        }
        _ => {}
    }
    if !["read", "put", "delete", "undo"].contains(&action) {
        return Err(format!("workflows takes {ACTIONS}."));
    }
    if name.is_empty() {
        return Err(format!("workflows {action} needs a workflow name."));
    }
    match action {
        "read" => {
            let flow = read_saved(conn, &name)?;
            Ok((json!({"workflow": model::to_json(&flow), "can_undo": can_undo(conn, &name)}), none))
        }
        "put" => {
            let raw = args.get("workflow").cloned().unwrap_or(Value::Null);
            let flow = model::check_workflow(&raw, Some(&name))?;
            let problems = model::lint(&flow);
            if !problems.is_empty() {
                return Err(format!("Not saved. {}", problems.join(" ")));
            }
            store::put_workflow(conn, &name, &model::to_json(&flow).to_string()).map_err(|e| format!("{e:#}"))?;
            let text = format!("Saved workflow {name}.");
            Ok((json!({"text": text, "workflow": model::to_json(&flow), "can_undo": can_undo(conn, &name)}), Some(name)))
        }
        "delete" => {
            if store::get_workflow(conn, &name).map_err(|e| format!("{e:#}"))?.is_none() {
                return Err(format!("There is no workflow named {name}."));
            }
            store::remove_workflow(conn, &name).map_err(|e| format!("{e:#}"))?;
            Ok((json!({"text": format!("Deleted workflow {name}. Undo puts it back.")}), Some(name)))
        }
        _ => {
            let Some(saved) = store::undo_workflow(conn, &name).map_err(|e| format!("{e:#}"))? else {
                return Err(format!("Workflow {name} has no earlier copy to put back."));
            };
            let shown = model::parse_saved(&saved.name, &saved.body).map(|f| model::to_json(&f)).unwrap_or(Value::Null);
            Ok((json!({"text": format!("Put back the earlier copy of {name}."), "workflow": shown, "can_undo": can_undo(conn, &name)}), Some(name)))
        }
    }
}

pub async fn request(app: &Arc<App>, args: &Args) -> Result<Value, String> {
    let action = match word(args, "action") {
        a if a.is_empty() => "list".to_string(),
        a => a,
    };
    if RUN_ACTS.contains(&action.as_str()) {
        let task = task_number(args, &action)?;
        let done = super::act(app, task, &action, WINDOW_WHO, &word(args, "text")).await?;
        return Ok(json!({"changed": false, "text": done.text}));
    }
    let extra = if action == "options" {
        let tools: Vec<(String, String)> = crate::tools::specs(app).into_iter().map(|s| (s.name, s.description)).collect();
        let worker = app.clone();
        tokio::task::spawn_blocking(move || open_db(&worker).map(|conn| options_json(&conn, &skills_dir(), &tools, &super::model_rows(&conn))))
            .await
            .map_err(|e| e.to_string())??
    } else {
        Value::Null
    };
    let worker = app.clone();
    let args = args.clone();
    let (mut data, name) = tokio::task::spawn_blocking(move || open_db(&worker).and_then(|conn| store_request(&conn, &action, &args, &extra)))
        .await
        .map_err(|e| e.to_string())??;
    data["changed"] = json!(name.is_some());
    if let Some(name) = name {
        changed(app, &name);
    }
    Ok(data)
}
