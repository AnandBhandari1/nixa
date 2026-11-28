pub mod content;
pub mod store;

use crate::app::App;
use crate::util::log;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use content::Doc;
use serde_json::{Value, json};
use std::sync::Arc;
use store::{Act, Hit};

type Shared = State<Arc<App>>;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/content/{document}", get(content_get).post(content_post))
        .route("/answer-rules", get(rules_get).post(rules_post))
        .route("/answer-rules/check", post(rules_check))
        .route("/answer-rules/{id}/remove", post(rules_remove))
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

fn refused(app: &App, headers: &HeaderMap) -> Option<Response> {
    (!crate::window_key::allowed(app, headers)).then(crate::window_key::refused)
}

const NO_DOC: &str = "Choose memory or rules.";

async fn content_get(
    State(app): Shared,
    headers: HeaderMap,
    Path(document): Path<String>,
) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    let Some(doc) = Doc::parse(&document) else {
        return bad(StatusCode::NOT_FOUND, NO_DOC);
    };
    match content::read(&app.dir, &app.db_file, doc) {
        Ok(data) => ok(data),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

async fn content_post(
    State(app): Shared,
    headers: HeaderMap,
    Path(document): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    let Some(doc) = Doc::parse(&document) else {
        return bad(StatusCode::NOT_FOUND, NO_DOC);
    };
    let Some(text) = body.get("text").and_then(Value::as_str) else {
        return bad(StatusCode::BAD_REQUEST, "Send the complete text.");
    };
    let expected = body
        .get("expectedHash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match content::save(&app.dir, &app.db_file, doc, text, expected) {
        Ok(saved) => {
            let hash = saved.data.get("hash").cloned().unwrap_or(Value::Null);
            app.send_out(json!({"type": "content_changed", "document": doc.word(), "hash": hash}));
            log("content", &format!("{} saved from the window", doc.word()));
            let mut data = saved.data;
            if let Some(map) = data.as_object_mut() {
                map.insert("notesSaved".into(), json!(saved.notes));
            }
            ok(data)
        }
        Err(error) if error == doc.stale() || error == crate::soul::CONFLICT => {
            bad(StatusCode::CONFLICT, doc.stale())
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn rules_get(State(app): Shared, headers: HeaderMap) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    match store::load(&app.dir) {
        Ok(rules) => {
            let stamps: Vec<String> = rules.iter().map(store::stamp).collect();
            ok(
                json!({"rules": rules, "stamps": stamps, "max": store::MAX_RULES, "maxText": store::MAX_TEXT}),
            )
        }
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

fn save_rule(app: &App, input: &store::RuleInput) -> Result<store::AnswerRule, String> {
    let rule = store::save(&app.dir, input)?;
    if store::hard_stop_rule(input) {
        let what = format!("saved answer rule {} that names a push, deploy, publish, production or destructive step", rule.id);
        crate::modes::note(app, "auto-yes", &what, None, Some("Nixa changes answer rules itself"));
    }
    Ok(rule)
}

async fn rules_post(
    State(app): Shared,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    let saved = store::input_of(&body).and_then(|input| save_rule(&app, &input));
    match saved {
        Ok(rule) => {
            app.send_out(json!({"type": "answer_rules_changed", "id": rule.id}));
            ok(json!({"stamp": store::stamp(&rule), "rule": rule}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn rules_remove(
    State(app): Shared,
    headers: HeaderMap,
    Path(id): Path<u64>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    let expected = body
        .as_ref()
        .and_then(|b| b.get("expected"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match store::remove(&app.dir, id, expected.as_deref()) {
        Ok(rule) => {
            app.send_out(json!({"type": "answer_rules_changed", "id": rule.id}));
            ok(json!({"removed": rule.id}))
        }
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

async fn rules_check(
    State(app): Shared,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Some(no) = refused(&app, &headers) {
        return no;
    }
    let folder = body
        .get("folder")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let question = body
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match store::check(&app.dir, folder, question) {
        Ok(hit) => ok(hit_json(hit.as_ref())),
        Err(error) => bad(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

pub fn hit_json(hit: Option<&Hit>) -> Value {
    match hit {
        Some(hit) => {
            json!({"rule": hit.rule, "act": if hit.act == Act::Answer { "answer" } else { "watch" }})
        }
        None => json!({"rule": null, "act": null}),
    }
}

pub fn for_question(app: &App, folder: &str, question: &str) -> Option<Hit> {
    match store::decide(&app.dir, folder, question) {
        Ok(Some(hit)) => {
            let what = if hit.act == Act::Answer {
                "answers"
            } else {
                "would answer (trial)"
            };
            log("answer-rules", &format!("rule {} {what}", hit.rule.id));
            Some(hit)
        }
        Ok(None) => None,
        Err(error) => {
            log("answer-rules", &error);
            None
        }
    }
}

#[cfg(test)]
mod tests;
