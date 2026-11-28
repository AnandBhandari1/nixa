use super::{action, agent_models, refresh_lists};
use crate::app::App;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nixa_store::models::By;
use serde_json::{Value, json};
use std::sync::Arc;

pub const WS_KIND: &str = "models";
pub const AGENT_MODELS_ACTION: &str = "agent_models";
pub const REFRESH_LISTS_ACTION: &str = "refresh_lists";

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/models", get(list_get))
        .route("/models/view", get(view_get))
        .route("/models/{action}", post(action_post))
}

fn reply(result: Result<Value, String>) -> Response {
    match result {
        Ok(data) => (StatusCode::OK, axum::Json(json!({"ok": true, "data": data}))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

fn text_of(args: &Value, key: &str) -> String {
    args.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub async fn handle(app: &Arc<App>, name: &str, mut args: Value) -> Result<Value, String> {
    if name == super::online::ACTION {
        return super::online::change(app, &args);
    }
    if name == REFRESH_LISTS_ACTION {
        return Ok(refresh_lists(app));
    }
    if name == AGENT_MODELS_ACTION {
        let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(false);
        return agent_models(app, &text_of(&args, "agent"), refresh).await;
    }
    if let Some(map) = args.as_object_mut() {
        map.insert("action".into(), Value::String(name.to_string()));
    }
    action(app, &args, By::User).await
}

async fn list_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(handle(&app, "list", json!({})).await)
}

async fn view_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(handle(&app, "view", json!({})).await)
}

async fn action_post(State(app): State<Arc<App>>, Path(name): Path<String>, headers: HeaderMap, body: Option<axum::Json<Value>>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let args = match body.map(|b| b.0) {
        Some(Value::Object(map)) => Value::Object(map),
        None | Some(Value::Null) => json!({}),
        Some(_) => return reply(Err("The body must be a JSON object.".into())),
    };
    reply(handle(&app, &name, args).await)
}

pub fn is_ws(message: &Value) -> bool {
    message.get("type").and_then(Value::as_str) == Some(WS_KIND)
}

pub async fn ws_request(app: &Arc<App>, message: &Value) -> Option<Value> {
    if !is_ws(message) {
        return None;
    }
    let request = message.get("request").and_then(Value::as_str)?.to_string();
    let mut args = match message.get("args") {
        Some(Value::Object(map)) => map.clone(),
        _ => message.as_object().cloned().unwrap_or_default(),
    };
    args.remove("type");
    args.remove("request");
    let inner = args.remove("action").and_then(|v| v.as_str().map(str::to_string));
    let name = message.get("action").and_then(Value::as_str).map(str::to_string).or(inner).unwrap_or_else(|| "list".into());
    let kind = format!("{WS_KIND}_reply");
    Some(match handle(app, &name, Value::Object(args)).await {
        Ok(data) => json!({"type": kind, "request": request, "ok": true, "data": data}),
        Err(error) => json!({"type": kind, "request": request, "ok": false, "error": error}),
    })
}
