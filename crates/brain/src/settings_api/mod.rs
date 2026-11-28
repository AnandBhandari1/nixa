pub mod keywords;
pub mod work;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::util::log;
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use work::Work;

pub const OFF_FILE: &str = "off";
pub const HEADPHONES_FILE: &str = "headphones";
pub const PRIVACY_KEY: &str = "privacy.on";
pub const GIT_VOICE_KEY: &str = "git.speak";
pub const WATCH_MS: u64 = 2000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    pub nixa: Option<bool>,
    pub headphones: Option<bool>,
    pub keywords: Option<SystemTime>,
}

pub struct State {
    pub keywords_file: PathBuf,
    pub work: Mutex<Work>,
    pub keywords: Mutex<keywords::Loaded>,
    pub seen: Mutex<Seen>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        State {
            keywords_file: dir.join(keywords::FILE),
            work: Mutex::new(Work::default()),
            keywords: Mutex::new(keywords::Loaded { rows: keywords::defaults(), error: None, wrote: false }),
            seen: Mutex::new(Seen::default()),
        }
    }
}

fn store(app: &App) -> Result<nixa_store::Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("The store did not open: {e:#}"))
}

fn store_bool(app: &App, key: &str, default: bool) -> bool {
    store(app).ok().and_then(|conn| nixa_store::settings::setting_bool(&conn, key, default).ok()).unwrap_or(default)
}

fn save_bool(app: &App, key: &str, on: bool) -> Result<(), String> {
    nixa_store::settings::set_setting_bool(&store(app)?, key, on).map_err(|e| format!("Could not save {key}: {e:#}"))
}

fn flag(path: &Path, on: bool) -> Result<(), String> {
    let done = if on {
        path.parent().map(nixa_protocol::paths::ensure_private_dir).transpose().map(|_| ()).and_then(|_| std::fs::write(path, ""))
    } else if path.exists() {
        std::fs::remove_file(path)
    } else {
        Ok(())
    };
    done.map_err(|e| format!("Could not change {}: {e}", path.display()))
}

pub fn nixa_on(dir: &Path) -> bool {
    !dir.join(OFF_FILE).exists()
}

pub fn headphones(dir: &Path) -> bool {
    dir.join(HEADPHONES_FILE).exists()
}

pub fn privacy_on(app: &App) -> bool {
    store_bool(app, PRIVACY_KEY, true)
}


pub fn git_voice(app: &App) -> bool {
    store_bool(app, GIT_VOICE_KEY, true)
}

pub fn state_word(app: &App) -> &'static str {
    if !nixa_on(&app.dir) {
        return "off";
    }
    let live = lock(&app.voice.live).clone();
    if live.nixa_talking || crate::universe::nixa_speaking() {
        "speaking"
    } else if crate::agent::busy(app) {
        "thinking"
    } else if live.listening {
        "listening"
    } else {
        "idle"
    }
}

pub fn state_event(app: &App) -> Value {
    json!({
        "type": "state",
        "state": state_word(app),
        "nixa": nixa_on(&app.dir),
        "headphones": headphones(&app.dir),
        "privacy": privacy_on(app),
        "gitVoice": git_voice(app),
    })
}

pub fn work_view(app: &App) -> Value {
    let keywords = {
        let loaded = lock(&app.switches.keywords);
        keywords::view(&loaded.rows, loaded.error.as_deref())
    };
    let work = lock(&app.switches.work).clone();
    work::view(&work, keywords)
}

pub fn work_event(app: &App) -> Value {
    json!({"type": "work", "work": work_view(app)})
}

pub fn view(app: &App) -> Value {
    let mut out = state_event(app);
    if let Some(map) = out.as_object_mut() {
        map.remove("type");
        map.insert("agentVoice".into(), json!(crate::announce::is_on(app)));
        map.insert("work".into(), work_view(app));
    }
    out
}

pub fn send_state(app: &App) {
    app.send_out(state_event(app));
}

pub fn send_work(app: &App) {
    app.send_out(work_event(app));
}

fn carry(app: &App, note: String) {
    let mut brain = lock(&app.brain);
    brain.carry = if brain.carry.is_empty() { note } else { format!("{}\n\n{note}", brain.carry) };
}

pub fn owner(app: &App) -> String {
    crate::privacy::names_of(app).owner
}

pub fn privacy_start_line(owner: &str, on: bool) -> String {
    if on {
        return String::new();
    }
    format!("Private and confidential switch: off. {owner} turned it off in Settings. While it is off, the Private and confidential part of the soul does not apply: answer questions about {owner} and this computer as normal.")
}

pub fn privacy_switch_note(owner: &str, on: bool) -> String {
    if on {
        format!("Note: {owner} turned the Private and confidential switch on in Settings. Follow the Private and confidential part of the soul again from now on.")
    } else {
        format!("Note: {owner} turned the Private and confidential switch off in Settings. Until it is on again, the Private and confidential part of the soul does not apply: answer questions about {owner} and this computer as normal.")
    }
}

pub fn start_lines(app: &App) -> String {
    privacy_start_line(&owner(app), privacy_on(app))
}

fn quiet(app: &App) {
    crate::agent::interrupt(app);
    crate::talk::hold::silence(app);
    nixa_voice::stop_speaking();
    crate::voice::set_nixa_talking(app, false);
    if nixa_voice::is_listening() {
        nixa_voice::stop();
    }
    lock(&app.voice.live).listening = false;
}

fn nixa_changed(app: &Arc<App>, on: bool) {
    let name = app.name();
    if on {
        app.info(&format!("{name} is on."));
        let app = app.clone();
        tokio::spawn(async move { crate::voice::restart_listener(&app).await });
    } else {
        quiet(app);
        app.info(&format!("{name} is off. No mic and no {name} voice. Turn it on again in Settings, General."));
    }
    log("switches", if on { "nixa on" } else { "nixa off" });
}

pub fn set_nixa(app: &Arc<App>, on: bool) -> Result<(), String> {
    let was = nixa_on(&app.dir);
    flag(&app.dir.join(OFF_FILE), !on)?;
    lock(&app.switches.seen).nixa = Some(on);
    if was != on {
        nixa_changed(app, on);
    }
    send_state(app);
    Ok(())
}

pub fn resume(app: &Arc<App>) -> Result<(), String> {
    if lock(&app.talk.presence).quiet(crate::util::now_ms()) {
        crate::talk::flow::set_quiet_panel(app, 0.0);
    }
    set_nixa(app, true)
}

pub fn set_headphones(app: &App, on: bool) -> Result<(), String> {
    flag(&app.dir.join(HEADPHONES_FILE), on)?;
    lock(&app.switches.seen).headphones = Some(on);
    log("switches", if on { "headphones on" } else { "headphones off" });
    send_state(app);
    Ok(())
}

pub fn restart_voice(app: &Arc<App>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move { crate::voice::restart_listener(&app).await });
}

pub fn set_git_voice(app: &App, on: bool) -> Result<(), String> {
    save_bool(app, GIT_VOICE_KEY, on)?;
    app.send_out(json!({"type": "git_voice", "on": on}));
    send_state(app);
    Ok(())
}

pub fn set_privacy(app: &App, on: bool) -> Result<(), String> {
    let was = privacy_on(app);
    save_bool(app, PRIVACY_KEY, on)?;
    if was != on {
        let (name, owner) = (app.name(), owner(app));
        app.info(&if on {
            format!("Private and confidential is on. {name} will not answer questions about {owner}'s private things or this computer.")
        } else {
            format!("Private and confidential is off. {name} answers questions about {owner} and this computer as normal.")
        });
        carry(app, privacy_switch_note(&owner, on));
        log("switches", if on { "privacy on" } else { "privacy off" });
        crate::phone_door::send_state(app);
    }
    send_state(app);
    Ok(())
}

pub fn clear_keyword(app: &App, what: &str) -> Result<(), String> {
    let line = work::clear(&mut lock(&app.switches.work), what)?;
    if let Some(line) = line {
        app.info(&line);
        let note = work::note(&lock(&app.switches.work));
        carry(app, note);
    }
    send_work(app);
    Ok(())
}

fn on_of(value: &Value) -> Result<bool, String> {
    value.get("on").and_then(Value::as_bool).ok_or_else(|| "on must be true or false.".to_string())
}

fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn apply(app: &Arc<App>, kind: &str, value: &Value) -> Option<Result<(), String>> {
    let done = match kind {
        "set_nixa" => on_of(value).and_then(|on| set_nixa(app, on)),
        "resume" => resume(app),
        "set_headphones" => on_of(value).and_then(|on| set_headphones(app, on)).inspect(|_| restart_voice(app)),
        "set_git_voice" => on_of(value).and_then(|on| set_git_voice(app, on)),
        "set_privacy" => on_of(value).and_then(|on| set_privacy(app, on)),
        "keyword_clear" => clear_keyword(app, text_of(value, "what")).inspect(|_| {
            if text_of(value, "what") == "hold" {
                crate::prompts::scan::tell_untold(app);
            }
        }),
        "switches" => {
            send_state(app);
            send_work(app);
            Ok(())
        }
        _ => return None,
    };
    Some(done)
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> Option<Result<(), String>> {
    let done = apply(app, kind, value)?;
    if let Err(error) = &done {
        log("switches", &format!("{kind}: {error}"));
        if value.get("request").is_none() {
            app.send_out(json!({"type": "error", "about": kind, "text": error}));
        }
    }
    Some(done)
}

pub fn ws_answer(kind: &str, request: &str, done: Result<(), String>) -> Value {
    match done {
        Ok(()) => crate::server::ws_reply(kind, request, true),
        Err(error) => json!({"type": format!("{kind}_reply"), "request": request, "ok": false, "error": error}),
    }
}

pub fn reload_keywords(app: &App) -> bool {
    let stamp = std::fs::metadata(&app.switches.keywords_file).and_then(|m| m.modified()).ok();
    {
        let mut seen = lock(&app.switches.seen);
        if seen.keywords.is_some() && seen.keywords == stamp {
            return false;
        }
        seen.keywords = stamp;
    }
    let before = lock(&app.switches.keywords).clone();
    let mut loaded = keywords::load(&app.switches.keywords_file, &before.rows);
    if loaded.wrote {
        lock(&app.switches.seen).keywords = std::fs::metadata(&app.switches.keywords_file).and_then(|m| m.modified()).ok();
    }
    if let Some(error) = &loaded.error
        && before.error.as_ref() != Some(error)
    {
        app.info(&format!("The keyword list was not loaded: {} The last good list stays.", crate::util::clip(error, 200)));
    }
    loaded.wrote = false;
    let changed = loaded != before;
    *lock(&app.switches.keywords) = loaded;
    changed
}

pub fn tick(app: &Arc<App>) {
    let nixa = nixa_on(&app.dir);
    let phones = headphones(&app.dir);
    let (first, nixa_moved, phones_moved) = {
        let mut seen = lock(&app.switches.seen);
        let first = seen.nixa.is_none();
        let nixa_moved = seen.nixa.is_some_and(|m| m != nixa);
        let phones_moved = seen.headphones.is_some_and(|h| h != phones);
        seen.nixa = Some(nixa);
        seen.headphones = Some(phones);
        (first, nixa_moved, phones_moved || first)
    };
    if nixa_moved {
        nixa_changed(app, nixa);
    } else if first && !nixa {
        quiet(app);
    }
    if nixa_moved || phones_moved {
        send_state(app);
    }
    if phones_moved && !first {
        restart_voice(app);
    }
    if reload_keywords(app) {
        send_work(app);
    }
}

pub async fn run(app: Arc<App>) {
    loop {
        tick(&app);
        tokio::time::sleep(Duration::from_millis(WATCH_MS)).await;
    }
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/switches", get(switches_get).post(switches_post))
}

async fn switches_get(AxState(app): AxState<Arc<App>>) -> Response {
    reply(StatusCode::OK, json!({"ok": true, "switches": view(&app)}))
}

async fn switches_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let kind = text_of(&body, "type").to_string();
    match apply(&app, &kind, &body) {
        None => reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": "type is set_nixa, set_headphones, set_git_voice, set_privacy, keyword_clear or switches, with its fields"})),
        Some(Err(error)) => reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error})),
        Some(Ok(())) => reply(StatusCode::OK, json!({"ok": true, "switches": view(&app)})),
    }
}
