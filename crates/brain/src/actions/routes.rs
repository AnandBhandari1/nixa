use crate::app::App;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/actions", get(list_get)).route("/actions/run", post(run_post))
}

async fn list_get(State(app): State<Arc<App>>, headers: HeaderMap, Query(query): Query<HashMap<String, String>>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let list: Vec<Value> = super::matching(query.get("word").map(String::as_str)).iter().map(super::Action::view).collect();
    (StatusCode::OK, axum::Json(json!({"ok": true, "actions": list}))).into_response()
}

async fn run_post(State(app): State<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let name = body.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": "name is needed."}))).into_response();
    }
    let input = body.get("input").cloned().unwrap_or(Value::Null);
    let (code, out) = super::run::run(&app, &name, &input).await;
    (code, axum::Json(out)).into_response()
}
