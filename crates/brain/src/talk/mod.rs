pub mod confirm;
pub mod flow;
pub mod gather;
pub mod heard;
pub mod hold;
pub mod level;
pub mod presence;
pub mod progress;
pub mod room;
pub mod speech;
pub mod words;

#[cfg(test)]
mod hold_tests;
#[cfg(test)]
mod tests;

pub use confirm::{End, Said};
pub use flow::{ask_confirm, finish_confirm, hear, hear_later};

use crate::app::{App, lock};
use crate::tools::{Args, ToolResult};
use crate::turns::ToolSpec;
use crate::util::now_ms;
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use confirm::Confirms;
use gather::Gather;
use heard::Heard;
use hold::Hold;
use level::Meter;
use presence::Presence;
use progress::Progress;
use serde_json::{Value, json};
use speech::Speaker;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use words::Talk;

pub const TICK_MS: u64 = 100;
pub const EVENT_KIND: &str = "talk";
pub const NAMES: [&str; 1] = [progress::NAME];
pub const HEAR_CHARS: usize = 4000;

pub struct State {
    pub presence: Mutex<Presence>,
    pub gather: Mutex<Gather>,
    pub progress: Mutex<Progress>,
    pub confirms: Mutex<Confirms>,
    pub heard: Mutex<Heard>,
    pub hold: Mutex<Hold>,
    pub speaker: Mutex<Speaker>,
    pub meter: Mutex<Meter>,
    pub chain: tokio::sync::Mutex<()>,
    words: Mutex<Option<(String, Arc<Talk>)>>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        State {
            presence: Mutex::new(Presence::open(dir)),
            gather: Mutex::new(Gather::default()),
            progress: Mutex::new(Progress::default()),
            confirms: Mutex::new(Confirms::default()),
            heard: Mutex::new(Heard::default()),
            hold: Mutex::new(Hold::default()),
            speaker: Mutex::new(Speaker::default()),
            meter: Mutex::new(Meter::default()),
            chain: tokio::sync::Mutex::new(()),
            words: Mutex::new(None),
        }
    }
}

pub fn talk(app: &App) -> Arc<Talk> {
    let name = app.name();
    let mut words = lock(&app.talk.words);
    if let Some((cached, talk)) = words.as_ref()
        && *cached == name
    {
        return talk.clone();
    }
    let talk = Arc::new(Talk::new(&name));
    *words = Some((name, talk.clone()));
    talk
}

pub fn view(app: &App) -> Value {
    let now = now_ms();
    let presence = lock(&app.talk.presence).snapshot(now);
    let confirm = lock(&app.talk.confirms).open.clone();
    let progress = lock(&app.talk.progress).last().map(str::to_string);
    json!({
        "type": EVENT_KIND,
        "presence": presence,
        "confirm": confirm,
        "speaking": speech::speaking(app),
        "line": speech::current(app),
        "progress": progress,
    })
}

pub fn send_state(app: &App) {
    app.send_out(view(app));
}

pub fn specs() -> Vec<ToolSpec> {
    vec![progress::spec_of()]
}

pub fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    (name == progress::NAME).then(|| flow::progress_call(app, args))
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    match kind {
        "set_quiet" => {
            flow::set_quiet_panel(app, message.get("minutes").and_then(Value::as_f64).unwrap_or(0.0));
            true
        }
        "confirm" => {
            let approve = message.get("approve").and_then(Value::as_bool).unwrap_or(false);
            let id = message.get("id").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
            flow::finish_confirm(app, approve, id, if approve { End::Yes } else { End::No });
            true
        }
        "talk_state" => {
            send_state(app);
            true
        }
        _ => false,
    }
}

pub async fn run(app: Arc<App>) {
    lock(&app.talk.heard).quiet_was = lock(&app.talk.presence).quiet(now_ms());
    loop {
        tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
        flow::tick(&app, now_ms());
    }
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn bad(text: &str) -> Response {
    reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": text}))
}

fn ok(app: &App) -> Response {
    reply(StatusCode::OK, json!({"ok": true, "talk": view(app)}))
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/talk", get(talk_get))
        .route("/talk/quiet", post(quiet_post))
        .route("/talk/confirm", post(confirm_post))
        .route("/talk/ask", post(ask_post))
        .route("/talk/hear", post(hear_post))
}

async fn talk_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    ok(&app)
}

async fn quiet_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(minutes) = body.get("minutes").and_then(Value::as_f64) else { return bad("minutes is needed (0 ends quiet time)") };
    flow::set_quiet_panel(&app, minutes);
    ok(&app)
}

async fn confirm_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(approve) = body.get("approve").and_then(Value::as_bool) else { return bad("approve is needed (true or false)") };
    let id = body.get("id").and_then(Value::as_u64);
    let done = flow::finish_confirm(&app, approve, id, if approve { End::Yes } else { End::No });
    if !done {
        return bad("no open confirmation with that id");
    }
    ok(&app)
}

async fn ask_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let message = text("message");
    if message.is_empty() {
        return bad("message is needed");
    }
    auto_yes(&app, &message, &text("why"));
    reply(StatusCode::OK, json!({"ok": true, "approve": true, "talk": view(&app)}))
}

fn auto_yes(app: &App, message: &str, why: &str) {
    crate::modes::record(
        app,
        crate::modes::journal::DecisionInput {
            kind: "confirm".into(),
            choice: message.to_string(),
            reason: Some(if why.is_empty() { "Nixa says yes without a card".to_string() } else { why.to_string() }),
            outcome: Some("yes".into()),
            ..Default::default()
        },
    );
    app.send_out(json!({"type": "confirm_done", "id": null, "approve": true}));
}

async fn hear_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(text) = body.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
        return bad("text is needed");
    };
    if text.chars().count() > HEAR_CHARS {
        return bad("text is too long");
    }
    let typed = body.get("typed").and_then(Value::as_bool).unwrap_or(false);
    let now = now_ms();
    let said = if typed { Said::typed(text, now) } else { Said::heard(text, now, now) };
    hear(&app, said).await;
    ok(&app)
}
