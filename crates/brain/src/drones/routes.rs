//! HTTP routes for `/drones` and `/drones/{action}`.

use super::tools;
use crate::app::{App, lock};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Map, Value, json};
use std::sync::Arc;

type Shared = State<Arc<App>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/drones", get(drones_read)).route(
        "/drones/{action}",
        get(drones_get_action).post(drones_post_action),
    )
}

fn ok(value: Value) -> Response {
    let mut body = json!({"ok": true});
    if let (Some(out), Value::Object(more)) = (body.as_object_mut(), value) {
        out.extend(more);
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn bad(code: StatusCode, message: impl Into<String>) -> Response {
    (
        code,
        axum::Json(json!({"ok": false, "error": message.into()})),
    )
        .into_response()
}

async fn drones_read(State(app): Shared) -> Response {
    let view = lock(&app.drones.store).read_view();
    ok(json!({"drones": view}))
}

async fn drones_get_action(State(app): Shared, Path(action): Path<String>) -> Response {
    if action == "snapshot" {
        let snap = lock(&app.drones.store).snapshot();
        return ok(json!({"drones": snap}));
    }
    if !matches!(action.as_str(), "read" | "facts" | "history" | "dry_run") {
        return bad(
            StatusCode::METHOD_NOT_ALLOWED,
            format!("{action} needs POST."),
        );
    }
    let args = Map::new();
    match tools::run_action(&app, &action, &args, "cli").await {
        Ok(text) => ok(json!({"text": text})),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn drones_post_action(
    State(app): Shared,
    Path(action): Path<String>,
    body: Bytes,
) -> Response {
    let args: Map<String, Value> = if body.is_empty() {
        Map::new()
    } else {
        match serde_json::from_slice(&body) {
            Ok(args) => args,
            Err(error) => {
                return bad(
                    StatusCode::BAD_REQUEST,
                    format!("Expected a JSON object: {error}"),
                );
            }
        }
    };
    let who = args
        .get("who")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cli")
        .to_string();
    match tools::run_action(&app, &action, &args, &who).await {
        Ok(text) => ok(json!({"text": text})),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}
