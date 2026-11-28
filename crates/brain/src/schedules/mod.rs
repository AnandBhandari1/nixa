pub mod jobs;
pub mod report;
pub mod runner;
pub mod when;
pub mod zone;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use runner::Choice;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TICK_MS: u64 = 2000;
pub const EVENT_KIND: &str = "schedules";
pub const NOTE_KIND: &str = "schedule_event";

pub struct State {
    pub busy: Mutex<HashSet<String>>,
    pub seen_working: Mutex<HashSet<String>>,
    pub quiet: Mutex<HashSet<String>>,
    pub last_check: Mutex<i64>,
    pub zone: Mutex<Option<(zone::Zone, u64)>>,
}

impl State {
    pub fn open(_dir: &Path) -> State {
        State {
            busy: Mutex::new(HashSet::new()),
            seen_working: Mutex::new(HashSet::new()),
            quiet: Mutex::new(HashSet::new()),
            last_check: Mutex::new(0),
            zone: Mutex::new(None),
        }
    }
}

pub fn item(
    zone: &zone::Zone,
    s: &nixa_store::schedules::Schedule,
    lists: &[&[Choice]],
    busy: bool,
) -> Value {
    let mut value = serde_json::to_value(s).unwrap_or(Value::Null);
    let follow = if s.follow_agent.is_empty() {
        String::new()
    } else {
        runner::label_of(&s.follow_agent, lists)
    };
    let run = s.last_run.as_ref();
    value["view"] = json!({
        "when": when::when_text(s),
        "next": when::maybe(zone, s.next_run),
        "last": when::last_text(zone, run, &s.folder),
        "failed": run.is_some_and(|r| r.result == nixa_store::schedules::RunResult::Failed),
        "agent_name": runner::label_of(&s.agent, lists),
        "follow_name": follow,
        "report_done": run.is_some_and(|r| r.done_at.is_some() && !r.report.is_empty()),
        "busy": busy,
    });
    value
}

pub fn snapshot(app: &App) -> Value {
    let zone = jobs::zone_of(app);
    let agents = runner::agents(app);
    let members = runner::members(app);
    let teams = runner::teams(app);
    let lists: [&[Choice]; 3] = [&teams, &members, &agents];
    let busy = lock(&app.schedules.busy).clone();
    let (schedules, error) = match jobs::list(app) {
        Ok(list) => (
            list.iter()
                .map(|s| item(&zone, s, &lists, busy.contains(&s.id)))
                .collect::<Vec<_>>(),
            None,
        ),
        Err(e) => (
            Vec::new(),
            Some(format!("Cannot read the schedules in nixa.db: {e:#}")),
        ),
    };
    let spaces: Vec<Value> = app
        .spaces()
        .iter()
        .map(|s| json!({"id": s.id, "name": s.name, "folder": s.folder}))
        .collect();
    let mut out = json!({
        "enabled": jobs::all_on(app),
        "schedules": schedules,
        "agents": agents,
        "members": members,
        "teams": teams,
        "spaces": spaces,
        "zone": zone.name,
        "repeats": jobs::repeats::list_json(app),
    });
    if let Some(error) = error {
        out["error"] = json!(error);
    }
    out
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn ok(mut body: Value) -> Response {
    body["ok"] = json!(true);
    reply(StatusCode::OK, body)
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/schedules", get(list_get).post(command_post))
        .route("/schedules/picks", post(picks_post))
}

async fn list_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    ok(snapshot(&app))
}

async fn command_post(
    AxState(app): AxState<Arc<App>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let payload = body.get("payload").cloned().unwrap_or_else(|| json!({}));
    match jobs::command(&app, &action, &payload) {
        Ok(()) => ok(snapshot(&app)),
        Err(error) => reply(
            StatusCode::BAD_REQUEST,
            json!({"ok": false, "error": format!("{error:#}")}),
        ),
    }
}

async fn picks_post(
    AxState(app): AxState<Arc<App>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let text = body
        .get("folder")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let folder = match jobs::folder_of(&app, text) {
        Ok(folder) => folder,
        Err(error) => {
            return reply(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": format!("{error:#}")}),
            );
        }
    };
    let repos = runner::repos(&folder).await;
    ok(
        json!({"folder": folder, "repos": repos}),
    )
}

pub fn ws(app: &Arc<App>, kind: &str, _message: &Value) -> bool {
    if kind != EVENT_KIND {
        return false;
    }
    jobs::changed(app);
    true
}

pub async fn run(app: Arc<App>) {
    jobs::plan_missing(&app);
    let mut timer = tokio::time::interval(Duration::from_millis(TICK_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        jobs::tick(&app);
    }
}
