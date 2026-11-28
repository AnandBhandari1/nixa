use super::actions::{Who, action};
use crate::app::App;
use crate::tools::{Args, text};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::sync::Arc;

pub const WINDOW_WHO: &str = "the user in the window";
pub const WS_KIND: &str = "custom_agents";

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/custom-agents", get(list_get))
        .route("/custom-agents/{action}", post(action_post))
}

fn window() -> Who {
    Who {
        name: WINDOW_WHO.into(),
        by_user: true,
        caller: None,
    }
}

async fn answer(app: &Arc<App>, name: &str, args: &Args, who: &Who) -> Response {
    match action(app, name, args, who).await {
        Ok(out) => (
            StatusCode::OK,
            axum::Json(json!({"ok": true, "text": out.text, "data": out.data})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"ok": false, "error": error})),
        )
            .into_response(),
    }
}

async fn list_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(&app, "list", &Args::new(), &window()).await
}

pub fn deep_seat_who(app: &App, terminal: &str) -> Option<Who> {
    let deep = crate::app::lock(&app.deep);
    let seat = deep.by_terminal(terminal).filter(|s| s.joined)?;
    Some(Who {
        name: seat.name.clone(),
        by_user: false,
        caller: Some(terminal.to_string()),
    })
}

async fn seat_who(app: &App, name: &str, body: Option<&Value>, peer: crate::server::Peer) -> Result<Who, Option<(StatusCode, String)>> {
    let Some(body) = body.filter(|_| name == "use") else { return Err(None) };
    if body.get("terminal").and_then(Value::as_str).is_none_or(|t| t.trim().is_empty()) {
        return Err(None);
    }
    let terminal = crate::server::caller(body, peer).await.map_err(Some)?;
    deep_seat_who(app, &terminal).ok_or(None)
}

async fn action_post(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    peer: crate::server::Peer,
    body: Option<axum::Json<Value>>,
) -> Response {
    let body = body.map(|b| b.0);
    let who = if crate::window_key::allowed(&app, &headers) {
        window()
    } else {
        match seat_who(&app, &name, body.as_ref(), peer).await {
            Ok(who) => who,
            Err(None) => return crate::window_key::refused(),
            Err(Some((code, error))) => return (code, axum::Json(json!({"ok": false, "error": error}))).into_response(),
        }
    };
    let mut body = body;
    if let Some(Value::Object(map)) = body.as_mut() {
        map.remove("terminal");
    }
    let args = match body {
        Some(Value::Object(map)) => map,
        None | Some(Value::Null) => Args::new(),
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"ok": false, "error": "The body must be a JSON object."})),
            )
                .into_response();
        }
    };
    answer(&app, &name, &args, &who).await
}

pub fn is_ws(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WS_KIND)
}

pub fn ws_args(message: &Value) -> Args {
    let mut args = match message.get("args") {
        Some(Value::Object(map)) => map.clone(),
        _ => message.as_object().cloned().unwrap_or_default(),
    };
    args.remove("type");
    args.remove("request");
    args
}

pub async fn ws_request(app: &Arc<App>, message: &Value) -> Option<Value> {
    if !is_ws(message) {
        return None;
    }
    let request = message.get("request").and_then(Value::as_str)?.to_string();
    let args = ws_args(message);
    let name = text(&args, "action").unwrap_or_else(|| "list".into());
    let kind = format!("{WS_KIND}_reply");
    Some(match action(app, &name, &args, &window()).await {
        Ok(out) => {
            json!({"type": kind, "request": request, "ok": true, "data": out.data, "text": out.text})
        }
        Err(error) => json!({"type": kind, "request": request, "ok": false, "error": error}),
    })
}
