use super::answer::{answer_by_click, answer_by_tool};
use super::link::Link;
use super::book::NewPrompt;
use super::scan::{add_and_tell, space_of};
use super::tools::{answer_of, list_text};
use super::snapshot;
use crate::app::{App, lock};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::sync::Arc;

type Shared = State<Arc<App>>;

fn ok(value: Value) -> Response {
    let mut body = json!({"ok": true});
    if let (Some(out), Value::Object(more)) = (body.as_object_mut(), value) {
        out.extend(more);
    }
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn bad(code: StatusCode, message: impl Into<String>) -> Response {
    (code, axum::Json(json!({"ok": false, "error": message.into()}))).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/prompts", get(list_get))
        .route("/prompts/event", post(event_post))
        .route("/prompts/{id}/answer", post(answer_post))
}

async fn list_get(State(app): Shared) -> Response {
    let prompts = lock(&app.prompts.book).items.clone();
    let mut view = snapshot(&app);
    if let Some(out) = view.as_object_mut() {
        out.remove("type");
        out.insert("prompts".into(), json!(prompts));
        out.insert("text".into(), json!(list_text(&app)));
    }
    ok(view)
}

async fn event_post(State(app): Shared, axum::Json(event): axum::Json<NewPrompt>) -> Response {
    let link = app.prompts.link();
    let term = match link.get(&event.terminal).await {
        Ok(term) if term.running => term,
        Ok(_) => return bad(StatusCode::FORBIDDEN, format!("Terminal {} does not run.", event.terminal)),
        Err(_) => return bad(StatusCode::FORBIDDEN, format!("{} is not a Nixa terminal.", event.terminal)),
    };
    if super::book::PromptKind::parse(&event.kind).is_none() {
        return bad(StatusCode::BAD_REQUEST, "kind is question, permission or plan");
    }
    let space = space_of(&term, &app.spaces());
    let event = NewPrompt { terminal: term.id.to_string(), place: event.place.clone().or_else(|| space.as_ref().map(|s| s.name.clone())), cwd: event.cwd.clone().or(Some(term.cwd.clone())), ..event };
    match add_and_tell(&app, event, space.as_ref()) {
        Some(prompt) => ok(json!({"id": prompt.id})),
        None => ok(json!({"duplicate": true})),
    }
}

fn indexes_of(body: &Value) -> Option<Vec<usize>> {
    body.get("indexes")?.as_array()?.iter().map(|v| v.as_u64().map(|n| n as usize)).collect()
}

pub async fn answer_body(app: &Arc<App>, link: &Link, id: &str, body: &Value) -> Result<String, String> {
    if let Some(indexes) = indexes_of(body) {
        return Ok(answer_by_click(app, link, id, &indexes).await);
    }
    let args = body.as_object().cloned().ok_or("send indexes, or choice, options or text")?;
    let given = answer_of(&args)?;
    let decide = args.get("decide").and_then(Value::as_bool) == Some(true);
    let reason = args.get("reason").and_then(Value::as_str).map(str::to_string);
    answer_by_tool(app, link, id, given, decide, reason).await
}

async fn answer_post(State(app): Shared, Path(id): Path<String>, headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if indexes_of(&body).is_some() && !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let link = app.prompts.link();
    match answer_body(&app, &link, &id, &body).await {
        Ok(text) => ok(json!({"result": text})),
        Err(error) => bad(StatusCode::BAD_REQUEST, error),
    }
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    match kind {
        "answer" => {
            let id = message.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let indexes = indexes_of(message).unwrap_or_default();
            let app = app.clone();
            tokio::spawn(async move {
                let link = app.prompts.link();
                answer_by_click(&app, &link, &id, &indexes).await;
            });
            true
        }
        "prompts" => {
            app.send_out(snapshot(app));
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn typed_answers_need_decide_and_never_wait_for_the_user() {
        let data = tempfile::tempdir().unwrap();
        let app = App::open(data.path()).unwrap();
        let link = Link::new(data.path().join("none.sock"));
        let event = NewPrompt { kind: "permission".into(), terminal: "m1".into(), text: Some("git push --force".into()), ..NewPrompt::default() };
        let id = lock(&app.prompts.book).add(event, crate::util::now_ms()).unwrap().id;
        let plain = answer_body(&app, &link, &id, &json!({"choice": "yes"})).await.unwrap_err();
        assert!(plain.contains("decide true"), "{plain}");
        let decided = answer_body(&app, &link, &id, &json!({"choice": "yes", "decide": true, "reason": "x"})).await.unwrap_err();
        assert!(decided.contains("hard stop") && decided.contains("Only the user answers it"), "{decided}");
        assert!(answer_body(&app, &link, &id, &json!("yes")).await.is_err());
    }
}
