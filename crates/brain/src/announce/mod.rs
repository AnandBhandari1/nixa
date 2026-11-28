pub(crate) mod identity;
mod resume;
mod speak;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use identity::{Identity, announcement, identity, is_shell, job_of, program_of, spoken};
pub use resume::{ENDED, LABEL as RESUME_LABEL, Offer, PID_LABEL, Plan, Timing, command_of, plan, safe_command, steady, tidy};
pub use speak::{Change, change, hush};

use crate::app::{App, lock};
use crate::prompts::link::Link;
use crate::spaces::Space;
use crate::util::log;
use axum::Router;
use axum::extract::State as AxState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nixa_protocol::{Event, TermInfo, client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub const VOICE_KEY: &str = "voice.agents";
pub const MAX_EVENTS: usize = 50;
pub const MERGE_MS: u64 = 5000;
pub const REFRESH_MS: u64 = 30_000;
pub const RETRY_MS: u64 = 3000;
pub const SPACE_LABELS: [&str; 2] = ["space", crate::deep::SPACE_LABEL];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEvent {
    pub time: u64,
    pub kind: String,
    pub text: String,
    pub terminal: String,
    #[serde(default)]
    pub space: String,
    #[serde(default)]
    pub job: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_number: Option<u32>,
    #[serde(default)]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offer: Option<Offer>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub info: TermInfo,
    pub agent: String,
    pub exit: Option<Option<i32>>,
}

pub struct State {
    pub numbers: Mutex<HashMap<String, u32>>,
    pub events: Mutex<VecDeque<AgentEvent>>,
    pub terms: Mutex<HashMap<String, Track>>,
    pub sent: Mutex<HashSet<String>>,
    pub socket: Mutex<Option<PathBuf>>,
    pub timing: Mutex<Timing>,
}

impl State {
    pub fn open(_dir: &Path) -> State {
        State {
            numbers: Mutex::new(HashMap::new()),
            events: Mutex::new(VecDeque::new()),
            terms: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashSet::new()),
            socket: Mutex::new(None),
            timing: Mutex::new(Timing::default()),
        }
    }

    pub fn link(&self) -> Link {
        match lock(&self.socket).clone() {
            Some(socket) => Link::new(socket),
            None => Link::default(),
        }
    }

    pub fn set_socket(&self, socket: Option<PathBuf>) {
        *lock(&self.socket) = socket;
    }

    pub fn list(&self) -> Vec<AgentEvent> {
        lock(&self.events).iter().cloned().collect()
    }
}

pub fn agent_key(program: &str) -> String {
    match program.trim().to_lowercase().as_str() {
        "cursor-agent" => "cursor".into(),
        "agyd" => "agy".into(),
        other => other.to_string(),
    }
}

pub fn agent_name(before: &str, info: &TermInfo) -> String {
    let program = program_of(info);
    if !is_shell(&program) {
        return agent_key(&program);
    }
    if let Some((provider, _)) = info.labels.get(RESUME_LABEL).and_then(|l| nixa_store::agents::session_from_label(l)) {
        return provider;
    }
    if before.is_empty() { "unknown".into() } else { before.to_string() }
}

pub fn merge(list: &mut VecDeque<AgentEvent>, event: AgentEvent) {
    let same = list.iter().position(|old| {
        !event.terminal.is_empty() && old.terminal == event.terminal && old.kind == event.kind && old.time.abs_diff(event.time) < MERGE_MS
    });
    if let Some(at) = same {
        list.remove(at);
    }
    list.push_front(event);
    list.truncate(MAX_EVENTS);
}

pub fn add_event(app: &App, event: AgentEvent) {
    merge(&mut lock(&app.announce.events), event.clone());
    app.send_out(json!({"type": "agent_event", "event": event}));
}

pub fn space_for(app: &App, info: &TermInfo) -> Option<Space> {
    let spaces = app.spaces();
    for key in SPACE_LABELS {
        if let Some(found) = info.labels.get(key).and_then(|id| crate::spaces::find(&spaces, id)) {
            return Some(found.clone());
        }
    }
    crate::spaces::for_folder(&spaces, &info.cwd).cloned()
}

pub fn is_on(app: &App) -> bool {
    nixa_store::open(&app.db_file)
        .and_then(|conn| nixa_store::settings::setting_bool(&conn, VOICE_KEY, true))
        .unwrap_or(true)
}

pub fn view(app: &App) -> Value {
    let numbers: std::collections::BTreeMap<String, u32> = lock(&app.announce.numbers).iter().map(|(k, v)| (k.clone(), *v)).collect();
    json!({"on": is_on(app), "events": app.announce.list(), "numbers": numbers})
}

pub fn send_state(app: &App) {
    app.send_out(json!({"type": "agent_voice", "on": is_on(app)}));
}

pub fn set_on(app: &App, on: bool) -> anyhow::Result<()> {
    let saved = nixa_store::open(&app.db_file).and_then(|conn| nixa_store::settings::set_setting_bool(&conn, VOICE_KEY, on));
    if !on && !cfg!(test) {
        nixa_voice::stop_speaking();
    }
    send_state(app);
    saved
}

pub fn numbers_of(value: &Value) -> Option<HashMap<String, u32>> {
    let map = value.get("numbers")?.as_object()?;
    Some(map.iter().filter_map(|(k, v)| v.as_u64().filter(|n| *n > 0 && *n <= u32::MAX as u64).map(|n| (k.clone(), n as u32))).collect())
}

pub fn ws(app: &Arc<App>, kind: &str, message: &Value) -> bool {
    match kind {
        "set_agent_voice" => {
            let Some(on) = message.get("on").and_then(Value::as_bool) else { return false };
            if let Err(error) = set_on(app, on) {
                log("announce", &format!("could not save the agent voice switch: {error:#}"));
            }
            true
        }
        "agent_numbers" => {
            let Some(numbers) = numbers_of(message) else { return false };
            *lock(&app.announce.numbers) = numbers;
            true
        }
        "agent_voice" => {
            send_state(app);
            true
        }
        _ => false,
    }
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/announce", get(announce_get).post(announce_post)).route("/announce/numbers", post(numbers_post))
}

async fn announce_get(AxState(app): AxState<Arc<App>>) -> Response {
    let mut body = view(&app);
    body["ok"] = json!(true);
    reply(StatusCode::OK, body)
}

async fn announce_post(AxState(app): AxState<Arc<App>>, headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(on) = body.get("on").and_then(Value::as_bool) else {
        return reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": "give on: true or false"}));
    };
    match set_on(&app, on) {
        Ok(()) => reply(StatusCode::OK, json!({"ok": true, "on": on})),
        Err(error) => reply(StatusCode::INTERNAL_SERVER_ERROR, json!({"ok": false, "error": format!("the switch was not saved: {error:#}")})),
    }
}

async fn numbers_post(AxState(app): AxState<Arc<App>>, headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(numbers) = numbers_of(&body) else {
        return reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": "give numbers: {\"m1\": 1}"}));
    };
    let count = numbers.len();
    *lock(&app.announce.numbers) = numbers;
    reply(StatusCode::OK, json!({"ok": true, "count": count}))
}

pub enum Msg {
    Up,
    Event(Box<Event>),
}

pub fn watch(socket: PathBuf, tx: UnboundedSender<Msg>) {
    std::thread::spawn(move || {
        let mut said = false;
        loop {
            if tx.is_closed() {
                return;
            }
            let result = client::connect(&socket).and_then(|c| {
                said = false;
                let _ = tx.send(Msg::Up);
                c.subscribe(Vec::new(), false, |event| tx.send(Msg::Event(Box::new(event))).is_ok())
            });
            if let Err(error) = result
                && !said
            {
                log("announce", &format!("event stream down, trying again: {error}"));
                said = true;
            }
            std::thread::sleep(Duration::from_millis(RETRY_MS));
        }
    });
}

pub async fn refresh(app: &Arc<App>) {
    let Ok(list) = app.announce.link().list().await else { return };
    let ids: Vec<String> = {
        let mut terms = lock(&app.announce.terms);
        let mut fresh = HashMap::new();
        for info in list {
            let key = info.id.to_string();
            let old = terms.remove(&key);
            let before = old.as_ref().map(|t| t.agent.clone()).unwrap_or_default();
            let exit = old.and_then(|t| t.exit);
            let agent = agent_name(&before, &info);
            fresh.insert(key, Track { info, agent, exit });
        }
        *terms = fresh;
        terms.keys().cloned().collect()
    };
    lock(&app.announce.sent).retain(|key| ids.iter().any(|id| key.split(':').next() == Some(id.as_str())));
    for id in ids {
        tokio::spawn(resume::check(app.clone(), id));
    }
}

pub fn track(app: &App, id: &str) -> Option<Track> {
    lock(&app.announce.terms).get(id).cloned()
}

pub async fn on_event(app: &Arc<App>, event: Event) {
    let key = event.term_id().to_string();
    match event {
        Event::Created { term } => {
            let agent = agent_name("", &term);
            lock(&app.announce.terms).insert(key.clone(), Track { info: term, agent, exit: None });
            tokio::spawn(resume::check(app.clone(), key));
        }
        Event::ParentCleared { id, parent } => {
            let mut terms = lock(&app.announce.terms);
            if let Some(child) = terms.get_mut(id.as_str()) {
                child.info.parent = None;
            }
            if let Some(old) = terms.get_mut(parent.as_str()) {
                old.info.children.retain(|child| child != &id);
            }
        }
        Event::Status { status, note, .. } => {
            let found = {
                let mut terms = lock(&app.announce.terms);
                let Some(t) = terms.get_mut(&key) else { return };
                let before = t.info.status;
                t.info.status = status;
                t.info.status_note = note;
                (before, t.clone())
            };
            let (before, track) = found;
            if status == Some(nixa_protocol::Status::Error) && before != status {
                let (event, _) = speak::event_for(app, &track, "error", "reported an error");
                add_event(app, event);
            } else if let Some(change) = change(before, status) {
                speak::announce(app, &track, change);
            }
        }
        Event::Exit { code, .. } => {
            let keep = {
                let mut terms = lock(&app.announce.terms);
                let Some(t) = terms.get_mut(&key) else { return };
                t.info.running = false;
                t.info.exit_code = code;
                t.exit = Some(code);
                t.info.keep
            };
            if !keep {
                tokio::spawn(resume::check(app.clone(), key));
            }
        }
        Event::Respawned { pid, .. } => {
            {
                let mut terms = lock(&app.announce.terms);
                let Some(t) = terms.get_mut(&key) else { return };
                t.info.running = true;
                t.info.pid = pid;
                t.info.cmd.clear();
            }
            tokio::spawn(resume::check(app.clone(), key));
        }
        Event::Label { labels, .. } => {
            {
                let mut terms = lock(&app.announce.terms);
                let Some(t) = terms.get_mut(&key) else { return };
                t.info.labels = labels;
                t.agent = agent_name(&t.agent, &t.info);
            }
            tokio::spawn(resume::check(app.clone(), key));
        }
        Event::Renamed { name, .. } => {
            if let Some(t) = lock(&app.announce.terms).get_mut(&key) {
                t.info.name = name;
            }
        }
        Event::Draft { active, .. } => {
            if let Some(t) = lock(&app.announce.terms).get_mut(&key) {
                t.info.draft = active;
            }
        }
        Event::Removed { .. } => {
            lock(&app.announce.terms).remove(&key);
        }
        _ => {}
    }
}

pub async fn run(app: Arc<App>) {
    let (tx, mut rx) = unbounded_channel();
    watch(app.announce.link().socket, tx);
    let mut timer = tokio::time::interval(Duration::from_millis(REFRESH_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(Msg::Up) => refresh(&app).await,
                Some(Msg::Event(event)) => on_event(&app, *event).await,
                None => break,
            },
            _ = timer.tick() => refresh(&app).await,
        }
    }
}
