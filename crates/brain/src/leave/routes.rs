use super::{BY_AGENT, Report};
use crate::app::App;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;

type Shared = State<Arc<App>>;
type Peer = Option<Extension<ConnectInfo<SocketAddr>>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/agents/leave", get(leave_get))
        .route("/agents/limit", post(limit_post))
        .route("/agents/handover", post(handover_post))
}

fn reply(result: Result<Value, String>) -> Response {
    match result {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), json!(true));
            (StatusCode::OK, axum::Json(Value::Object(map))).into_response()
        }
        Ok(other) => (StatusCode::OK, axum::Json(json!({"ok": true, "data": other}))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

fn word(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).map(str::trim).unwrap_or("").to_string()
}

async fn who(app: &App, headers: &HeaderMap, body: &Value, peer: Peer) -> Result<nixa_protocol::TermInfo, String> {
    let terminal = word(body, "terminal");
    if terminal.is_empty() {
        return Err("terminal is needed: run it from inside a Nixa terminal (NIXA_TERM_ID).".into());
    }
    if !crate::window_key::allowed(app, headers) {
        crate::queue::caller(&terminal, peer.map(|Extension(ConnectInfo(addr))| addr)).await?;
    }
    let id = crate::engine::term_id(&terminal).map_err(|e| e.to_string())?;
    crate::engine::get(&id).await.map_err(|_| format!("Terminal {terminal} is not a Nixa terminal."))
}

async fn leave_get(State(app): Shared) -> Response {
    reply(Ok(super::view(&app)))
}

async fn limit_post(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let info = match who(&app, &headers, &body, peer).await {
        Ok(info) => info,
        Err(error) => return reply(Err(error)),
    };
    let now = crate::util::now_ms();
    let given = word(&body, "until");
    let until = super::limits::parse_given(&given, now, &crate::schedules::zone::local());
    if !given.is_empty() && until.is_none() {
        return reply(Err(format!("Nixa could not read the time \"{given}\". Use for example 5pm, 17:30, Monday 9am or 3h.")));
    }
    let report = Report {
        terminal: info.id.to_string(),
        name: super::persona(&info),
        engine: crate::prompts::scan::agent_of(&info),
        model: word(&body, "model"),
        line: String::new(),
        before: String::new(),
        until,
        by: BY_AGENT.into(),
        note: crate::util::clip_flat(&word(&body, "note"), 600),
    };
    let to = Some(word(&body, "to")).filter(|t| !t.is_empty());
    reply(super::report_now(&app, report, to).map(|text| json!({"text": text})))
}

async fn handover_post(State(app): Shared, headers: HeaderMap, peer: Peer, axum::Json(body): axum::Json<Value>) -> Response {
    let info = match who(&app, &headers, &body, peer).await {
        Ok(info) => info,
        Err(error) => return reply(Err(error)),
    };
    let note = crate::util::clip_flat(&word(&body, "note"), 600);
    if note.is_empty() {
        return reply(Err("A hand-over needs --note with what is done and what is left.".into()));
    }
    let to = word(&body, "to");
    let engine = crate::prompts::scan::agent_of(&info);
    let to = (!to.is_empty()).then_some(to.as_str());
    reply(super::handover(&app, &info.id.to_string(), &engine, to, &note).await.map(|text| json!({"text": text})))
}
