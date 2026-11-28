use super::crew;
use super::model::By;
use super::prompt::run_brief;
use super::runs::{self, RunStart, briefs};
use super::sources::find_space;
use crate::app::{App, lock};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

pub const WS_KINDS: [&str; 3] = ["team_run", "team_run_stop", "team_run_resume"];

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn bad(error: &str) -> Response {
    reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error}))
}

pub fn snapshot(app: &App) -> Value {
    let agents = runs::agents(app);
    let agent_list: Vec<Value> = agents
        .iter()
        .map(|a| json!({"id": a.id, "title": crew::title_of(a), "job": a.job, "off": a.off, "can_start": crew::can_start(a)}))
        .collect();
    let mut out = match super::tool::teams_view(app) {
        Ok(view) => view,
        Err(error) => json!({"teams": [], "spaces": [], "error": error}),
    };
    let file = runs::teams_file(app).unwrap_or_default();
    out["roles"] = json!(crew::roles_of(&file));
    out["agents"] = json!(agent_list);
    out["runs"] = json!(briefs(&lock(&app.teams.runs), 20));
    out["ok"] = json!(true);
    out
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/teams", get(list_get).post(command_post))
}

async fn list_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(StatusCode::OK, snapshot(&app))
}

fn text_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

fn list_of(v: &Value, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Value::Array(l)) => crew::names(&l.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()),
        Some(Value::String(s)) => crew::names(std::slice::from_ref(s)),
        _ => Vec::new(),
    }
}

pub fn start_input(app: &App, v: &Value, by: By) -> Result<RunStart, String> {
    let spaces = app.spaces();
    let wanted = text_of(v, "space");
    let space = if wanted.is_empty() {
        None
    } else {
        Some(find_space(&spaces, &wanted).cloned().ok_or_else(|| format!("No space \"{wanted}\"."))?)
    };
    let project = text_of(v, "project");
    if space.is_none() && project.is_empty() {
        return Err("Pick a space or a project folder.".into());
    }
    Ok(RunStart {
        team: text_of(v, "team"),
        task: text_of(v, "task"),
        project,
        by,
        space,
        repo: text_of(v, "repo"),
        branch: text_of(v, "branch"),
        skills: list_of(v, "skills"),
        mcps: list_of(v, "mcps"),
        ..RunStart::default()
    })
}

pub fn command(app: &Arc<App>, action: &str, v: &Value) -> Result<Value, String> {
    match action {
        "save" => {
            let agents = runs::agents(app);
            let team = crew::read_team(v.get("team").unwrap_or(v), &agents).map_err(|e| format!("{e:#}"))?;
            let mut file = runs::teams_file(app)?;
            crew::put_team(&mut file, team.clone());
            crew::save(&app.teams.teams_file, &file).map_err(|e| format!("{e:#}"))?;
            super::changed(app);
            Ok(json!({"team": team}))
        }
        "remove" => {
            let id = text_of(v, "id");
            if lock(&app.teams.runs).iter().any(|r| r.team == id && r.state == super::model::RunState::Running) {
                return Err("This team has a running run. Stop it first.".into());
            }
            let mut file = runs::teams_file(app)?;
            let gone = crew::remove_team(&mut file, &id).map_err(|e| format!("{e:#}"))?;
            crew::save(&app.teams.teams_file, &file).map_err(|e| format!("{e:#}"))?;
            super::changed(app);
            Ok(json!({"removed": gone.id}))
        }
        "start" => {
            let input = start_input(app, v, By::Button)?;
            Ok(json!({"run": run_brief(&runs::start(app, input)?)}))
        }
        "stop" => Ok(json!({"run": run_brief(&runs::stop(app, &text_of(v, "run"))?)})),
        "resume" => Ok(json!({"run": run_brief(&runs::resume(app, &text_of(v, "run"))?)})),
        "migrate" => {
            let from = text_of(v, "from");
            if from.is_empty() {
                return Err("Give from: the path of a copy of the old team file.".into());
            }
            let write = v.get("write").and_then(Value::as_bool) == Some(true);
            let text = super::migrate::run_migration(&app.dir, &app.db_file, &app.teams.teams_file, Path::new(&from), write, crate::util::now_ms()).map_err(|e| format!("{e:#}"))?;
            if write {
                super::changed(app);
                crate::custom_agents::changed(app);
            }
            Ok(json!({"text": text, "written": write}))
        }
        _ => Err("Use one of: save, remove, start, stop, resume, migrate.".into()),
    }
}

async fn command_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let action = text_of(&body, "action");
    let payload = body.get("payload").cloned().unwrap_or_else(|| body.clone());
    match command(&app, &action, &payload) {
        Ok(mut out) => {
            out["ok"] = json!(true);
            out["teams"] = snapshot(&app);
            reply(StatusCode::OK, out)
        }
        Err(error) => bad(&error),
    }
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    let action = match kind {
        "team_run" => {
            let given = text_of(message, "action");
            if given.is_empty() { "start".to_string() } else { given }
        }
        "team_run_stop" => "stop".into(),
        "team_run_resume" => "resume".into(),
        _ => return false,
    };
    if let Err(error) = command(app, &action, message) {
        app.info(&format!("Team run: {error}"));
        app.send_out(json!({"type": "team_run_error", "action": action, "error": error}));
    }
    true
}
