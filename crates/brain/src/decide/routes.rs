use super::settings::{self, Settings};
use super::{MODELS, Usage, ask, choice, picture, predicate, server_text, text, usage};
use crate::app::App;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::sync::Arc;

pub const EVENT: &str = "decide.settings";

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/decide", get(status_get).post(settings_post)).route("/decide/test", post(test_post))
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

pub fn status_view(settings: &Settings, usage: &Usage, server: &str) -> Value {
    json!({"settings": settings, "route": settings.route(), "models": MODELS, "server": server, "today": usage})
}

pub fn status(app: &App) -> Value {
    status_view(&settings::load(app), &usage(app), server_text(app))
}

async fn status_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(Ok(status(&app)))
}

async fn settings_post(State(app): State<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let done = settings::save(&app, &body);
    if done.is_ok() {
        app.send_out(json!({"type": EVENT, "status": status(&app)}));
    }
    answer(done.map(|_| status(&app)))
}

pub fn test_questions(body: &Value) -> Result<Vec<Value>, String> {
    let list = |key: &str| body.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
    let choices = list("choices");
    let statement = body.get("yes_no").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    if let Some(statement) = statement {
        out.push(predicate("yes_no", statement));
    }
    if !choices.is_empty() {
        let pairs: Vec<(&str, &str)> = choices.iter().map(|c| (c.as_str(), "")).collect();
        let instructions = body.get("question").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Which one fits the input best?");
        out.push(choice("pick", instructions, &pairs));
    }
    if out.is_empty() {
        return Err("give yes_no (a statement) or choices (a list)".into());
    }
    Ok(out)
}

async fn test_post(State(app): State<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(input) = body.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
        return answer(Err("text is needed".into()));
    };
    let questions = match test_questions(&body) {
        Ok(q) => q,
        Err(e) => return answer(Err(e)),
    };
    let current = settings::load(&app);
    let model = body.get("model").and_then(Value::as_str).filter(|m| MODELS.contains(m)).unwrap_or(&current.model).to_string();
    let mut parts = vec![text(input)];
    if let Some(path) = body.get("image").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
        let path = std::path::Path::new(path);
        let found = match nixa_desktop::picture::read_picture(path) {
            Ok(found) => found,
            Err(e) => return answer(Err(format!("The image was not read: {e}"))),
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
        parts.push(picture(&found, &name));
    }
    let result = ask(&app, &model, parts, questions).await.map(|a| {
        let answers: Vec<String> = a.answers.iter().map(|x| x.short()).collect();
        json!({"model": model, "answers": answers, "sure": a.all_sure(current.level), "level": current.level, "cost": a.cost, "input_tokens": a.input_tokens})
    });
    answer(result)
}
