mod book;
mod map;
mod resolve;

#[cfg(test)]
mod tests;

pub use book::{AGENTS, Action, Humor, KINDS, LIST_MS, OFF_LINE, OFF_PLAY, Row, SAMPLE_LINE, Source, VoiceList, builtin_list, check_voice, list_from, models_of, off_list, parse, rows_of};
pub use map::{Level, VoiceMap, clean, is_empty, normalize_agent, set, set_default, voice_key};
pub use resolve::{Ctx, Deps, Resolved, Rule, Tier, builtin, resolve, terminal_number};

use crate::app::{App, lock};
use crate::util::{log, now_ms, read_json, write_json};
use axum::Router;
use axum::extract::State as AxState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const FILE: &str = "voice-map.json";
pub const HUMOR_KEY: &str = "voice.humor";
pub const HUMOR_RULE: &str = " Only in your own spoken text replying to a Heard or Typed line from the user, you may add at most one short, dry, polite touch of humor, in plain words with no idioms or wordplay. Never add humor to agent messages, task text, fix messages, deep brain requests, commits or logs. Never use humor when the reply carries a failure, block, warning, push, deploy or money ask, or a question the user must answer. Facts and the length rule come first.";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Play {
    pub id: u64,
    pub active: bool,
}

pub struct State {
    pub file: PathBuf,
    pub map: Mutex<VoiceMap>,
    pub list: Mutex<Option<(VoiceList, u64)>>,
    pub play: Mutex<Play>,
    pub tool_runs: AtomicU64,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let file = dir.join(FILE);
        let map = read_json::<Value>(&file).map(|raw| clean(&raw)).unwrap_or_default();
        State { file, map: Mutex::new(map), list: Mutex::new(None), play: Mutex::new(Play::default()), tool_runs: AtomicU64::new(0) }
    }
}

pub fn map(app: &App) -> VoiceMap {
    lock(&app.voice_map.map).clone()
}

fn save(app: &App, next: VoiceMap) -> Result<(), String> {
    let saved = if is_empty(&next) {
        match std::fs::remove_file(&app.voice_map.file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    } else {
        write_json(&app.voice_map.file, &next)
    };
    *lock(&app.voice_map.map) = next;
    saved.map_err(|e| format!("The voice rules were not saved: {e}"))
}

pub fn cached(app: &App) -> Option<VoiceList> {
    lock(&app.voice_map.list).as_ref().map(|(list, _)| list.clone())
}

pub fn voice_on(app: &App) -> bool {
    crate::voice::settings(app).voice_on
}

pub fn tool_runs(app: &App) -> u64 {
    app.voice_map.tool_runs.load(Ordering::SeqCst)
}

pub async fn voice_list(app: &App, force: bool) -> VoiceList {
    if !voice_on(app) {
        return off_list(cached(app));
    }
    if !force
        && let Some((list, at)) = lock(&app.voice_map.list).clone()
        && now_ms().saturating_sub(at) < LIST_MS
    {
        return list;
    }
    app.voice_map.tool_runs.fetch_add(1, Ordering::SeqCst);
    let names = if cfg!(test) { Vec::new() } else { tokio::task::spawn_blocking(nixa_voice::list_voices).await.unwrap_or_default() };
    let list = list_from(names);
    *lock(&app.voice_map.list) = Some((list.clone(), now_ms()));
    list
}

pub fn own_voice(app: &App) -> String {
    crate::voice::settings(app).voice.filter(|v| !v.trim().is_empty()).unwrap_or_else(|| nixa_voice::voice_for("nixa").to_string())
}

pub fn deps(app: &App) -> Deps {
    let known = cached(app).filter(|l| l.source == Source::Speak).map(|l| l.voices);
    Deps { known, fallback: builtin_list(), nixa_voice: own_voice(app), numbers: lock(&app.announce.numbers).clone(), personas: crate::persona::voices(app) }
}

pub fn resolve_for(app: &App, ctx: &Ctx) -> Resolved {
    resolve(&map(app), ctx, &deps(app))
}

pub fn with_kind(ctx: &Ctx) -> Ctx {
    let mut ctx = ctx.clone();
    if ctx.kind.is_none() {
        let nixa = ctx.agent.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("nixa"));
        ctx.kind = Some(if nixa { "chat" } else { "other" }.into());
    }
    ctx
}

pub fn pick(app: &App, ctx: &Ctx) -> String {
    resolve_for(app, &with_kind(ctx)).voice
}

pub fn event_kind(kind: &str) -> &'static str {
    match kind.trim() {
        "blocked" | "resume" | "attention" => "attention",
        "done" | "finished" => "finished",
        "check" => "check",
        "alarm" => "alarm",
        "update" => "update",
        "deep" => "deep",
        "chat" => "chat",
        _ => "other",
    }
}

pub fn agent_voice(app: &App, event: &crate::announce::AgentEvent) -> String {
    let given = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    let ctx = Ctx { terminal: given(&event.terminal), model: None, agent: given(&event.agent), space: given(&event.space), kind: Some(event_kind(&event.kind).into()) };
    pick(app, &ctx)
}

pub fn humor(app: &App) -> Humor {
    nixa_store::open(&app.db_file)
        .and_then(|conn| nixa_store::settings::setting_text(&conn, HUMOR_KEY, Humor::Light.word()))
        .ok()
        .and_then(|word| Humor::parse(&word))
        .unwrap_or(Humor::Light)
}

pub fn humor_rule(app: &App) -> &'static str {
    if humor(app) == Humor::Light { HUMOR_RULE } else { "" }
}

fn set_humor(app: &App, value: Humor) -> Result<(), String> {
    nixa_store::open(&app.db_file)
        .and_then(|conn| nixa_store::settings::set_setting_text(&conn, HUMOR_KEY, value.word()))
        .map_err(|e| format!("Humor was not saved: {e:#}"))
}

pub fn models(app: &App) -> Vec<(String, String)> {
    let rows = nixa_store::open(&app.db_file).and_then(|conn| nixa_store::agent_map::map_rows(&conn)).unwrap_or_default();
    let pairs: Vec<(String, String)> = rows.into_iter().filter(|r| !r.off).map(|r| (r.agent, r.model)).collect();
    models_of(&pairs)
}

pub fn live(app: &App) -> Value {
    let spaces: Vec<String> = app.spaces().into_iter().map(|s| s.name).filter(|n| !n.trim().is_empty() && n != "~").collect();
    let mut terms: Vec<crate::announce::Track> = lock(&app.announce.terms).values().filter(|t| t.info.running).cloned().collect();
    terms.sort_by_key(|t| (t.info.id.to_string().len(), t.info.id.to_string()));
    let terminals: Vec<Value> = terms
        .iter()
        .map(|t| {
            let address = t.info.id.to_string();
            let space = crate::announce::space_for(app, &t.info).map(|s| s.name).unwrap_or_default();
            let tab = if t.info.name.trim().is_empty() { address.clone() } else { t.info.name.trim().to_string() };
            let place = if space.is_empty() { "no space".to_string() } else { space.clone() };
            json!({"address": address, "agent": t.agent, "space": space, "label": format!("{} · {place} · {tab}", t.agent)})
        })
        .collect();
    json!({"spaces": spaces, "terminals": terminals})
}

pub async fn state(app: &App, rows: &[Row]) -> Value {
    let list = voice_list(app, false).await;
    let map = map(app);
    let deps = deps(app);
    let facts = book::Facts { map: &map, list: &list, deps: &deps, humor: humor(app), models: models(app), voice_on: voice_on(app) };
    let mut out = book::state(&facts, rows);
    out["live"] = live(app);
    out
}

fn finish(app: &App, id: u64) {
    {
        let mut play = lock(&app.voice_map.play);
        if play.id != id {
            return;
        }
        play.active = false;
    }
    app.send_out(json!({"type": "voices_idle", "id": id}));
}

pub fn play(app: &Arc<App>, voice: &str) -> Result<u64, String> {
    if !voice_on(app) {
        return Err(OFF_PLAY.into());
    }
    let id = {
        let mut play = lock(&app.voice_map.play);
        play.id += 1;
        play.active = true;
        play.id
    };
    app.voice_map.tool_runs.fetch_add(1, Ordering::SeqCst);
    if cfg!(test) {
        finish(app, id);
        return Ok(id);
    }
    nixa_voice::stop_speaking();
    crate::voice::set_nixa_talking(app, true);
    let done = nixa_voice::speak(SAMPLE_LINE, voice);
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let _ = done.recv();
        finish(&app, id);
        if !nixa_voice::is_speaking() {
            crate::voice::set_nixa_talking(&app, false);
        }
    });
    Ok(id)
}

pub fn stop(app: &App) -> bool {
    {
        let mut play = lock(&app.voice_map.play);
        if !play.active {
            return false;
        }
        play.id += 1;
        play.active = false;
    }
    if !cfg!(test) {
        nixa_voice::stop_speaking();
        crate::voice::set_nixa_talking(app, false);
    }
    true
}

pub async fn run(app: &Arc<App>, args: &Value) -> Result<Value, String> {
    let action = parse(args)?;
    let rows = rows_of(args.get("rows"));
    match &action {
        Action::State => {}
        Action::Humor(value) => set_humor(app, *value)?,
        Action::Set { .. } | Action::Clear { .. } | Action::Reset => {
            let list = voice_list(app, false).await;
            let mut next = map(app);
            if book::change(&mut next, &action, &list.voices)? {
                save(app, next)?;
            }
        }
        Action::Play { voice, ctx } => {
            if !voice_on(app) {
                return Err(OFF_PLAY.into());
            }
            let list = voice_list(app, false).await;
            let voice = match voice {
                Some(name) => check_voice(&list.voices, name)?,
                None => resolve_for(app, ctx).voice,
            };
            let id = play(app, &voice)?;
            return Ok(json!({"voice": voice, "id": id}));
        }
        Action::Stop => return Ok(json!({"stopped": stop(app)})),
    }
    Ok(state(app, &rows).await)
}

pub async fn warm(app: Arc<App>) {
    if voice_on(&app) {
        voice_list(&app, true).await;
    }
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/voices", get(voices_get).post(voices_post))
}

async fn voices_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(StatusCode::OK, json!({"ok": true, "data": state(&app, &[]).await}))
}

async fn voices_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    match run(&app, &body).await {
        Ok(data) => reply(StatusCode::OK, json!({"ok": true, "data": data})),
        Err(error) => {
            log("voices", &error);
            reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error}))
        }
    }
}
