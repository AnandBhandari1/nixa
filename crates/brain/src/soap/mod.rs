pub mod banter;
pub mod fun;
pub mod memory;
pub mod scene;

#[cfg(test)]
mod fun_tests;
#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::privacy::report::{Reach, url_reach};
use crate::util::{log, now_ms};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use scene::Ask;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SOAP_KIND: &str = "office_soap";
pub const MODELS_KIND: &str = "office_soap_models";
pub const MODEL_KIND: &str = "office_soap_model";
pub const SCENE_EVENT: &str = "office_scene";
pub const DEFAULT_MODEL: &str = "gemma4:e4b-it-qat";
pub const DEFAULT_URL: &str = "http://127.0.0.1:11434";
pub const GAP_MS: u64 = 15_000;
pub const CHAT_SECS: u64 = 25;
pub const TAGS_SECS: u64 = 5;
pub const MODEL_CHARS: usize = 100;

#[derive(Debug, Default)]
pub struct Gate {
    pub running: bool,
    pub last_at: u64,
}

#[derive(Default)]
pub struct State {
    pub gate: Mutex<Gate>,
    pub fun: Mutex<fun::Fun>,
}

pub fn open_gate(gate: &mut Gate, now: u64) -> bool {
    if gate.running || (gate.last_at > 0 && now.saturating_sub(gate.last_at) < GAP_MS) {
        return false;
    }
    gate.running = true;
    gate.last_at = now;
    true
}

fn close_gate(app: &App) {
    lock(&app.soap.gate).running = false;
}

pub const USED_KEY: &str = "office.soap.used";

pub fn used(app: &App) -> bool {
    lock(&app.soap.gate).last_at > 0 || nixa_store::open(&app.db_file).ok().and_then(|c| nixa_store::settings::setting_bool(&c, USED_KEY, false).ok()).unwrap_or(false)
}

pub fn mark_used(app: &App) {
    if let Ok(conn) = nixa_store::open(&app.db_file)
        && let Err(e) = nixa_store::settings::set_setting_bool(&conn, USED_KEY, true)
    {
        crate::util::log("soap", &format!("the soap use mark was not saved: {e:#}"));
    }
}

pub fn model(app: &App) -> String {
    app.settings().office_soap_model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| DEFAULT_MODEL.into())
}

pub fn url(app: &App) -> String {
    app.settings()
        .office_soap_url
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.into())
}

pub fn good_model(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MODEL_CHARS {
        return Err(format!("The model name needs 1 to {MODEL_CHARS} characters."));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-' | '/')) {
        return Err("The model name can have letters, numbers and : . _ - / only.".into());
    }
    Ok(name.to_string())
}

pub fn set_model(app: &App, name: &str) -> Result<String, String> {
    let name = good_model(name)?;
    app.save_settings(|s| s.office_soap_model = Some(name.clone())).map_err(|e| format!("The model was not saved: {e}"))?;
    Ok(name)
}

fn allowed_url(app: &App, url: &str) -> Result<(), String> {
    if url_reach(url).0 != Reach::ThisMac && crate::privacy::privacy_on(app) {
        return Err("Privacy mode is on and the office soap url is not on this Mac.".into());
    }
    Ok(())
}

pub async fn installed(http: &reqwest::Client, url: &str) -> Result<Vec<String>, String> {
    let reply = http
        .get(format!("{url}/api/tags"))
        .timeout(Duration::from_secs(TAGS_SECS))
        .send()
        .await
        .map_err(|_| format!("Ollama is not reachable at {url}."))?;
    if !reply.status().is_success() {
        return Err(format!("Ollama answered {} at {url}.", reply.status().as_u16()));
    }
    let value: Value = reply.json().await.map_err(|_| "Ollama sent a broken model list.".to_string())?;
    Ok(names_of(&value))
}

pub fn names_of(tags: &Value) -> Vec<String> {
    let mut names: Vec<String> = tags
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("name").or_else(|| m.get("model")).and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn models_event(model: &str, url: &str, found: Result<Vec<String>, String>) -> Value {
    let (models, error) = match found {
        Ok(list) => (list, Value::Null),
        Err(e) => (Vec::new(), json!(e)),
    };
    json!({"type": MODELS_KIND, "model": model, "url": url, "models": models, "error": error})
}

pub async fn models_now(app: &App) -> Value {
    let (model, url) = (model(app), url(app));
    let found = match allowed_url(app, &url) {
        Ok(()) => installed(&app.http, &url).await,
        Err(e) => Err(e),
    };
    models_event(&model, &url, found)
}

async fn ask_model(http: &reqwest::Client, url: &str, model: &str, ask: &Ask, story: &memory::Story) -> Result<Value, String> {
    let reply = http
        .post(format!("{url}/api/chat"))
        .timeout(Duration::from_secs(CHAT_SECS))
        .json(&scene::body(model, ask, &story.scenes, &story.lines))
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Ollama took too long to write the scene.".to_string() } else { format!("Ollama is not reachable at {url}.") })?;
    let code = reply.status().as_u16();
    if !reply.status().is_success() {
        return Err(if code == 404 { format!("The model {model} is not installed in Ollama.") } else { format!("Ollama answered {code}.") });
    }
    let value: Value = reply.json().await.map_err(|_| "Ollama sent a broken reply.".to_string())?;
    let mut made = scene::parse_reply(&value, ask)?;
    scene::drop_repeats(&mut made, &story.lines);
    Ok(made)
}

pub fn today() -> String {
    crate::extras::usage::day_key(now_ms())
}

pub async fn scene_now(app: &App, ask: &Ask) -> Result<Value, String> {
    let url = url(app);
    allowed_url(app, &url)?;
    mark_used(app);
    let path = app.dir.join(memory::FILE);
    let mut story = memory::load(&path, &today());
    let made = ask_model(&app.http, &url, &model(app), ask, &story).await?;
    memory::add(&mut story, scene::summary(&made, ask));
    memory::add_lines(&mut story, made["lines"].as_array().into_iter().flatten().filter_map(|l| l["text"].as_str()).map(str::to_string).collect());
    if let Err(e) = memory::save(&path, &story) {
        log("soap", &format!("the story memory was not saved: {e}"));
    }
    Ok(made)
}

pub fn scene_event(result: Result<Value, String>) -> Value {
    match result {
        Ok(made) => json!({"type": SCENE_EVENT, "scene": made}),
        Err(error) => json!({"type": SCENE_EVENT, "error": error}),
    }
}

fn start(app: &Arc<App>, value: &Value) {
    let ask = match scene::parse_ask(value) {
        Ok(ask) => ask,
        Err(error) => {
            app.send_out(scene_event(Err(error)));
            return;
        }
    };
    if crate::agent::busy(app) || !open_gate(&mut lock(&app.soap.gate), now_ms()) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let result = scene_now(&app, &ask).await;
        close_gate(&app);
        if let Err(error) = &result {
            log("soap", &format!("no scene: {error}"));
        }
        app.send_out(scene_event(result));
    });
}

fn models_reply(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        let event = models_now(&app).await;
        app.send_out(event);
    });
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    if fun::ws(app, kind, value) || banter::ws(app, kind, value) {
        return true;
    }
    match kind {
        SOAP_KIND => start(app, value),
        MODELS_KIND => models_reply(app),
        MODEL_KIND => {
            let name = value.get("model").and_then(Value::as_str).unwrap_or("");
            match set_model(app, name) {
                Ok(_) => models_reply(app),
                Err(error) => {
                    log("soap", &error);
                    app.send_out(models_event(&model(app), &url(app), Err(error)));
                }
            }
        }
        _ => return false,
    }
    true
}

pub fn sample_ask() -> Ask {
    let names = crate::persona::WOMEN.iter().take(2).chain(crate::persona::MEN.iter().take(1));
    let spots = ["tea", "sofa", "window"];
    let cast = names
        .zip(spots)
        .enumerate()
        .map(|(i, (name, at))| scene::Person {
            id: format!("m{}", i + 1),
            name: name.to_string(),
            role: "staff".into(),
            space: "nixa".into(),
            status: "idle".into(),
            at: at.into(),
        })
        .collect::<Vec<_>>();
    let work = vec![scene::Work { who: "m1".into(), name: cast[0].name.clone(), task: "Fix login page".into() }];
    Ask { cast, work, hour: 15 }
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

async fn status_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let mut event = models_now(&app).await;
    event["fun"] = fun::status(&lock(&app.soap.fun), now_ms());
    reply(Ok(event))
}

async fn talk_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(Ok(crate::universe::voice_status(&app)))
}

async fn fun_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let line = body.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if line.is_empty() {
        return reply(Err("Give a line to rewrite.".into()));
    }
    let who = body.get("who").and_then(Value::as_str).map(str::trim).filter(|w| !w.is_empty()).unwrap_or("Nixa");
    let pair = fun::Pair { from: scene::cut(who, fun::NAME_CHARS), to: "the team".into(), first: scene::cut(&line, fun::TEXT_CHARS * 2), second: String::new(), known: Vec::new() };
    let out = fun::rewrite(&app, &pair, Duration::from_millis(fun::LIMIT_MS)).await;
    let state = fun::status(&lock(&app.soap.fun), now_ms());
    reply(Ok(json!({"model": model(&app), "who": pair.from, "original": pair.first, "line": fun::said_json(&out.first), "serious": out.serious, "fun": state})))
}

async fn model_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    if let Err(error) = set_model(&app, body.get("model").and_then(Value::as_str).unwrap_or("")) {
        return reply(Err(error));
    }
    let event = models_now(&app).await;
    app.send_out(event.clone());
    reply(Ok(event))
}

async fn test_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    if !open_gate(&mut lock(&app.soap.gate), now_ms()) {
        return reply(Err(format!("A scene was asked less than {} s ago or is still being written. Try again soon.", GAP_MS / 1000)));
    }
    let ask = sample_ask();
    let result = scene_now(&app, &ask).await;
    close_gate(&app);
    reply(result.map(|made| json!({"model": model(&app), "url": url(&app), "scene": made, "names": ask.cast.iter().map(|p| (p.id.clone(), json!(p.name))).collect::<serde_json::Map<_, _>>()})))
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/office/soap", get(status_get))
        .route("/office/soap/model", post(model_post))
        .route("/office/soap/test", post(test_post))
        .route("/office/fun/test", post(fun_post))
        .route("/office/talk", get(talk_get))
}
