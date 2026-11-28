use super::{changed, open};
use crate::app::{App, lock};
use nixa_store::models::lists::LIST_TIMEOUT;
use nixa_store::models::{By, ListResult, act, action_name, agent_rows, model_list, parse_args, refresh_list, run_list_command};
use nixa_store::time::now_ms;
use serde_json::{Value, json};
use std::sync::Arc;

const WRITES: [&str; 4] = ["rank", "move", "edit", "refresh"];
const STAMPED: [&str; 3] = ["list", "view", "refresh"];

fn blocking_error(error: tokio::task::JoinError) -> String {
    format!("the model work stopped: {error}")
}

pub async fn action(app: &Arc<App>, args: &Value, by: By) -> Result<Value, String> {
    let parsed = parse_args(args).map_err(|e| format!("{e:#}"))?;
    let name = action_name(&parsed).to_string();
    let value = if name == "refresh" {
        let refreshed = super::refresh(app, true).await;
        let conn = open(app)?;
        let rows = nixa_store::models::model_rows(&nixa_store::models::load(&conn).map_err(|e| format!("{e:#}"))?);
        json!({"refreshed": refreshed, "rows": rows})
    } else {
        let db = app.db_file.clone();
        tokio::task::spawn_blocking(move || {
            let conn = nixa_store::open(&db).map_err(|e| format!("{e:#}"))?;
            act(&conn, &parsed, by).map_err(|e| format!("{e:#}"))
        })
        .await
        .map_err(blocking_error)??
    };
    if WRITES.contains(&name.as_str()) {
        changed(app);
    }
    let mut value = value;
    if STAMPED.contains(&name.as_str()) {
        super::online::stamp(app, &mut value);
    }
    Ok(value)
}

fn run_command(argv: &[&str]) -> anyhow::Result<String> {
    run_list_command(argv, LIST_TIMEOUT)
}

pub fn refresh_later(app: &Arc<App>, agent: &str) {
    if !lock(&app.models.listing).insert(agent.to_string()) {
        return;
    }
    let (app, agent) = (app.clone(), agent.to_string());
    tokio::task::spawn_blocking(move || {
        if let Ok(conn) = nixa_store::open(&app.db_file) {
            let _ = refresh_list(&conn, &agent, now_ms(), &run_command);
        }
        lock(&app.models.listing).remove(&agent);
        changed(&app);
    });
}

pub fn refresh_lists(app: &Arc<App>) -> Value {
    let agents = super::tick::list_agents(&super::programs(app));
    for agent in &agents {
        refresh_later(app, agent);
    }
    json!({"started": agents})
}

pub async fn agent_models(app: &Arc<App>, agent: &str, refresh: bool) -> Result<Value, String> {
    let agent = agent.trim().to_string();
    let db = app.db_file.clone();
    let wanted = agent.clone();
    let (list, rows): (ListResult, _) = tokio::task::spawn_blocking(move || {
        let conn = nixa_store::open(&db).map_err(|e| format!("{e:#}"))?;
        let list = model_list(&conn, &wanted, refresh, now_ms(), &run_command).map_err(|e| format!("{e:#}"))?;
        let rows = agent_rows(&conn, &wanted).map_err(|e| format!("{e:#}"))?;
        Ok::<_, String>((list, rows))
    })
    .await
    .map_err(blocking_error)??;
    if list.stale && !refresh && list.error.is_empty() {
        refresh_later(app, &agent);
    }
    if refresh {
        changed(app);
    }
    let mut value = serde_json::to_value(&list).map_err(|e| e.to_string())?;
    value["leaderboard"] = serde_json::to_value(rows).map_err(|e| e.to_string())?;
    Ok(value)
}

pub fn launch_result(app: &App, terminal: &str, good: bool) -> Result<bool, String> {
    let conn = open(app)?;
    let done = nixa_store::models::result(&conn, terminal, good).map_err(|e| format!("{e:#}"))?;
    if done {
        changed(app);
    }
    Ok(done)
}
