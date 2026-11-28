use super::gate;
use crate::app::{App, lock};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn view(app: &App) -> Value {
    let state = &app.desktop;
    let card = lock(&state.held).as_ref().filter(|h| crate::util::now_ms().saturating_sub(h.at) <= gate::CARD_MS).map(gate::card_view);
    json!({
        "card": card,
        "recent": *lock(&state.recent),
        "allowReads": gate::permission(state).allow_reads,
    })
}

fn answer(result: Result<Value, String>) -> Response {
    match result {
        Ok(Value::Object(mut map)) => {
            map.insert("ok".into(), json!(true));
            (StatusCode::OK, axum::Json(Value::Object(map))).into_response()
        }
        Ok(other) => (StatusCode::OK, axum::Json(json!({"ok": true, "data": other}))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, axum::Json(json!({"ok": false, "error": error}))).into_response(),
    }
}

async fn desktop_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(view(&app)))
}

async fn answer_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let id = body.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let yes = match body.get("answer").and_then(Value::as_str) {
        Some("yes") => true,
        Some("no") => false,
        _ => return answer(Err("answer must be yes or no.".into())),
    };
    answer(gate::answer(&app, &id, yes))
}

async fn reads_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(on) = body.get("allowReads").and_then(Value::as_bool) else { return answer(Err("allowReads must be true or false.".into())) };
    answer(gate::save_permission(&app.desktop, on).map(|p| json!({"allowReads": p.allow_reads})))
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/desktop", get(desktop_get)).route("/desktop/answer", post(answer_post)).route("/desktop/reads", post(reads_post))
}
