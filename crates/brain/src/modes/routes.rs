use super::journal::{self, EXPLAIN_LIMIT};
use super::{complaints, scores};
use crate::app::App;
use axum::Router;
use axum::extract::{Query, State as AxState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

const LIST_MAX: usize = 200;

type Params = Query<HashMap<String, String>>;

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn answer(done: Result<Value, String>) -> Response {
    match done {
        Ok(body) => reply(StatusCode::OK, body),
        Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, json!({"ok": false, "error": error})),
    }
}

fn param<'a>(query: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    query.get(key).map(|v| v.trim()).filter(|v| !v.is_empty())
}

fn limit(query: &HashMap<String, String>, default: usize) -> usize {
    param(query, "limit").and_then(|v| v.parse::<usize>().ok()).unwrap_or(default).clamp(1, LIST_MAX)
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/modes", get(modes_get))
        .route("/modes/journal", get(journal_get))
        .route("/modes/changes", get(changes_get))
        .route("/modes/scores", get(scores_get))
        .route("/modes/records", get(records_get))
        .route("/modes/complaints", get(complaints_get))
}

async fn modes_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let count = super::store(&app).and_then(|conn| journal::count(&conn));
    answer(count.map(|count| json!({"ok": true, "modes": super::view(&app), "decisions": count})))
}

async fn journal_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Params) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let size = limit(&query, EXPLAIN_LIMIT);
    let rows = super::store(&app).and_then(|conn| match param(&query, "query") {
        Some(words) => journal::search(&conn, words, size),
        None => journal::last_for(&conn, param(&query, "terminal"), size),
    });
    answer(rows.map(|rows| json!({"ok": true, "decisions": rows, "text": journal::describe_all(&rows)})))
}

async fn changes_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Params) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let since = param(&query, "since").and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(|| super::clock::start_of_day(crate::util::now_ms()));
    let rows = super::store(&app).and_then(|conn| journal::changes_since(&conn, since, param(&query, "project")));
    answer(rows.map(|rows| json!({"ok": true, "changes": rows, "text": journal::describe_changes(&rows)})))
}

async fn scores_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Params) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let rows = super::store(&app).and_then(|conn| scores::top(&conn, param(&query, "project"), limit(&query, scores::SCORES_SHOWN)));
    answer(rows.map(|rows| {
        let lines: Vec<String> = rows.iter().map(scores::score_line).collect();
        let levels: Vec<&str> = rows.iter().map(|r| scores::level_of(r.good, r.bad).0.word()).collect();
        json!({"ok": true, "scores": rows, "levels": levels, "lines": lines})
    }))
}

async fn records_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let rows = super::store(&app).and_then(|conn| journal::records(&conn));
    answer(rows.map(|rows| json!({"ok": true, "records": rows, "lines": journal::describe_records(&rows)})))
}

async fn complaints_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap, Query(query): Params) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let rows = complaints::recent(&app.modes.complaints_file, limit(&query, complaints::RECENT));
    reply(StatusCode::OK, json!({"ok": true, "complaints": rows, "text": complaints::text_of(&rows)}))
}

pub fn ws(app: &Arc<App>, kind: &str, _message: &Value) -> bool {
    if kind != super::EVENT_KIND {
        return false;
    }
    super::send_state(app);
    true
}
