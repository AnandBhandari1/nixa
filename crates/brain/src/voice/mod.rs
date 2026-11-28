mod commands;
mod events;
mod settings;
pub mod speech;
mod talk;

pub use commands::{Change, Command, apply, parse};
pub use events::on_event;
pub use settings::{Listen, VoiceSettings};
pub use speech::{Spoken, say, speak_replies, spoken_line};
pub use talk::asks_user;

use crate::app::{App, lock};
use crate::util::log;
use axum::Router;
use axum::extract::State as AxState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nixa_voice::VoiceEvent;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    pub listening: bool,
    pub device: Option<String>,
    pub missing: bool,
    pub stt_ready: bool,
    pub error: Option<String>,
    pub user_talking: bool,
    pub nixa_talking: bool,
    pub holding: bool,
    pub level: f32,
    pub heard: u64,
}

pub struct State {
    pub settings_file: PathBuf,
    pub models: Mutex<PathBuf>,
    pub settings: Mutex<VoiceSettings>,
    pub live: Mutex<Live>,
    events: Mutex<Option<UnboundedSender<VoiceEvent>>>,
    restart: Mutex<()>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let settings_file = dir.join(settings::FILE);
        State {
            settings: Mutex::new(settings::load_at_start(&settings_file)),
            settings_file,
            models: Mutex::new(nixa_voice::ListenConfig::default().models_dir),
            live: Mutex::new(Live::default()),
            events: Mutex::new(None),
            restart: Mutex::new(()),
        }
    }
}

pub fn settings(app: &App) -> VoiceSettings {
    lock(&app.voice.settings).clone()
}

pub fn view(app: &App) -> Value {
    let s = settings(app);
    json!({
        "settings": s,
        "live": lock(&app.voice.live).clone(),
        "voice": settings::voice_name(&s),
        "speaking": crate::universe::nixa_speaking(),
        "keyAccess": nixa_voice::key_access(),
        "echoCancel": nixa_voice::echo_cancelled(),
    })
}

pub fn send_state(app: &App) {
    app.send_out(json!({"type": "voice", "voice": view(app)}));
}

pub fn set_nixa_talking(app: &App, on: bool) {
    let changed = {
        let mut live = lock(&app.voice.live);
        let changed = live.nixa_talking != on;
        live.nixa_talking = on;
        changed
    };
    if changed {
        app.send_out(events::talking_event("nixa", on));
    }
}

pub async fn run(app: Arc<App>) {
    let (tx, mut rx) = unbounded_channel();
    *lock(&app.voice.events) = Some(tx);
    nixa_voice::set_output_device(settings(&app).speaker_device);
    tokio::spawn(speech::speak_replies(app.clone()));
    restart_listener(&app).await;
    while let Some(event) = rx.recv().await {
        on_event(&app, event);
    }
}

pub async fn restart_listener(app: &Arc<App>) {
    let worker = app.clone();
    let done = tokio::task::spawn_blocking(move || {
        let app = worker;
        let _one = lock(&app.voice.restart);
        let s = settings(&app);
        if !settings::wants_listener(&s) {
            if nixa_voice::is_listening() {
                nixa_voice::stop();
            }
            lock(&app.voice.live).error = None;
            return None;
        }
        let models = lock(&app.voice.models).clone();
        if !settings::key_ready(&s, !nixa_voice::missing_models(&models, &nixa_voice::models(), false).is_empty()) {
            if nixa_voice::is_listening() {
                nixa_voice::stop();
            }
            lock(&app.voice.live).error = None;
            return None;
        }
        let Some(tx) = lock(&app.voice.events).clone() else { return Some("voice is not started yet".to_string()) };
        let headphones = crate::settings_api::headphones(&app.dir);
        let echo = nixa_voice::use_echo_cancel(true, headphones);
        let config = settings::listen_config(&s, &app.name(), &models, echo, headphones);
        let started = nixa_voice::start_listening(config, move |event| {
            let _ = tx.send(event);
        });
        match started {
            Ok(()) => None,
            Err(error) => Some(format!("{error:#}")),
        }
    })
    .await;
    let error = match done {
        Ok(error) => error,
        Err(join) => Some(join.to_string()),
    };
    if let Some(error) = error {
        log("voice", &format!("mic did not start: {error}"));
        lock(&app.voice.live).error = Some(error.clone());
        app.info(&format!("The mic did not start: {error}"));
    }
    send_state(app);
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    let Some(command) = parse(kind, message) else { return false };
    let app = app.clone();
    tokio::spawn(async move { handle(&app, command).await });
    true
}

pub async fn handle(app: &Arc<App>, command: Command) {
    let change = {
        let mut s = lock(&app.voice.settings);
        apply(&mut s, &command, &app.name())
    };
    if change.save {
        let s = settings(app);
        if let Err(error) = settings::save(&app.voice.settings_file, &s) {
            log("voice", &format!("could not save the voice settings: {error}"));
        }
    }
    if change.stop_speaking {
        nixa_voice::stop_speaking();
        set_nixa_talking(app, false);
    }
    if change.speaker {
        nixa_voice::set_output_device(settings(app).speaker_device);
    }
    if let Some(info) = &change.info {
        app.info(info);
    }
    match command {
        Command::HoldStart => {
            if !nixa_voice::hold_start() {
                app.info("Turn on the mic or pick the hotkey listen mode first.");
            }
        }
        Command::HoldEnd => {
            nixa_voice::hold_end();
        }
        Command::RefreshAudio => send_audio(app).await,
        Command::Stop => {
            if crate::agent::interrupt(app) {
                log("voice", "stopped the brain");
            }
            crate::talk::hold::answered(app);
        }
        _ => {}
    }
    if change.restart {
        restart_listener(app).await;
    } else {
        send_state(app);
    }
}

pub async fn audio_lists() -> Value {
    let lists = tokio::task::spawn_blocking(|| {
        let names = |list: anyhow::Result<Vec<nixa_voice::AudioDevice>>| list.map(|l| l.into_iter().map(|d| d.name).collect::<Vec<_>>()).unwrap_or_default();
        (names(nixa_voice::list_input_devices()), names(nixa_voice::list_output_devices()))
    })
    .await
    .unwrap_or_default();
    json!({"mics": lists.0, "speakers": lists.1})
}

async fn send_audio(app: &Arc<App>) {
    let mut lists = audio_lists().await;
    let s = settings(app);
    if let Some(out) = lists.as_object_mut() {
        out.insert("type".into(), json!("audio"));
        out.insert("mic".into(), json!(s.mic_device));
        out.insert("speaker".into(), json!(s.speaker_device));
    }
    app.send_out(lists);
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/voice", get(voice_get).post(voice_post)).route("/voice/devices", get(devices_get))
}

async fn voice_get(AxState(app): AxState<Arc<App>>) -> Response {
    reply(StatusCode::OK, json!({"ok": true, "voice": view(&app)}))
}

async fn voice_post(AxState(app): AxState<Arc<App>>, axum::Json(body): axum::Json<Value>) -> Response {
    let kind = body.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    let Some(command) = parse(&kind, &body) else {
        return reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": "type is set_mic, set_listen, set_voice, set_audio, refresh_audio, hold_start, hold_end, stop or voice_state, with its fields"}));
    };
    handle(&app, command).await;
    reply(StatusCode::OK, json!({"ok": true, "voice": view(&app)}))
}

async fn devices_get() -> Response {
    let mut lists = audio_lists().await;
    if let Some(out) = lists.as_object_mut() {
        out.insert("ok".into(), json!(true));
    }
    reply(StatusCode::OK, lists)
}

#[cfg(test)]
mod tests;
