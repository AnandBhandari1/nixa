mod route;
mod sound;
mod tasks;
pub mod voice_log;

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

pub use route::{parse_note, routes};
pub use sound::{Direction, METEOR_MS, Quiet, Sound, allowed, pick, stop as stop_sound, wave};
pub use tasks::handoff;

use crate::app::{App, lock};
use crate::util::now_ms;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

pub const FLOW_KIND: &str = "universe_flow";
pub const SCREEN_FLOW_KIND: &str = "flow";
pub const DEEP: &str = "deep";
pub const METEOR_KIND: &str = "universe_meteor";
pub const OFFICE_SAY_KIND: &str = "office_say";
pub const LINE_MAX: usize = 160;
pub const OFFICE_WAIT_MS: u64 = 2500;
pub const NIXA: &str = "nixa";
pub const SAME_MS: u64 = 1500;
pub const KEEP_MS: u64 = 10_000;

static OFFICE_PLAYING: AtomicUsize = AtomicUsize::new(0);

pub fn nixa_speaking() -> bool {
    nixa_voice::is_speaking() && OFFICE_PLAYING.load(Ordering::SeqCst) == 0
}

fn office_part(live: &AtomicUsize, start: bool) {
    if start {
        live.fetch_add(1, Ordering::SeqCst);
        OFFICE_PLAYING.fetch_add(1, Ordering::SeqCst);
    } else if live.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
        OFFICE_PLAYING.fetch_sub(1, Ordering::SeqCst);
    }
}

fn office_done(live: &AtomicUsize) {
    let left = live.swap(0, Ordering::SeqCst);
    OFFICE_PLAYING.fetch_sub(left, Ordering::SeqCst);
}

static SENT: LazyLock<broadcast::Sender<(String, String)>> = LazyLock::new(|| broadcast::channel(256).0);

#[derive(Default)]
pub struct Recent {
    pairs: HashMap<(String, String), u64>,
    into: HashMap<String, u64>,
}

#[derive(Default)]
pub struct State {
    pub recent: Mutex<Recent>,
    pub sound: Mutex<Sound>,
    pub tasks: Mutex<HashMap<i64, String>>,
    pub voice: Mutex<voice_log::VoiceLog>,
    socket: Mutex<Option<PathBuf>>,
}

impl State {
    pub fn set_socket(&self, path: Option<PathBuf>) {
        *lock(&self.socket) = path;
    }

    pub fn socket(&self) -> PathBuf {
        lock(&self.socket).clone().unwrap_or_else(nixa_protocol::paths::socket_path)
    }
}

pub fn pass(recent: &mut Recent, from: &str, to: &str, now: u64, by_nixa: bool) -> bool {
    recent.pairs.retain(|_, at| now.saturating_sub(*at) < KEEP_MS);
    recent.into.retain(|_, at| now.saturating_sub(*at) < KEEP_MS);
    if from.is_empty() || to.is_empty() || from == to {
        return false;
    }
    if by_nixa && recent.into.get(to).is_some_and(|at| now.saturating_sub(*at) < SAME_MS) {
        return false;
    }
    let key = (from.to_string(), to.to_string());
    if recent.pairs.get(&key).is_some_and(|at| now.saturating_sub(*at) < SAME_MS) {
        return false;
    }
    recent.pairs.insert(key, now);
    recent.into.insert(to.to_string(), now);
    true
}

pub fn flow_event(from: &str, to: &str, at: u64) -> Value {
    json!({"type": FLOW_KIND, "from": from, "to": to, "at": at})
}

pub fn short_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= LINE_MAX {
        return Some(flat);
    }
    let mut out: String = flat.chars().take(LINE_MAX - 1).collect();
    out.push('…');
    Some(out)
}

pub fn screen_flow_event(from: &str, to: &str, kind: &str, text: Option<&str>) -> Value {
    let mut event = json!({"type": SCREEN_FLOW_KIND, "from": from, "to": to, "kind": kind});
    if let Some(line) = text.and_then(short_line) {
        event["text"] = json!(line);
    }
    event
}

pub fn screen_flow(app: &App, from: &str, to: &str, kind: &str, text: Option<&str>) {
    if !from.is_empty() && !to.is_empty() {
        app.send_out(screen_flow_event(from, to, kind, text));
    }
}

pub fn flow(app: &App, from: &str, to: &str) -> bool {
    note(app, from.trim(), to.trim(), false)
}

fn note(app: &App, from: &str, to: &str, by_nixa: bool) -> bool {
    let now = now_ms();
    if !pass(&mut lock(&app.universe.recent), from, to, now, by_nixa) {
        return false;
    }
    app.send_out(flow_event(from, to, now));
    true
}

pub fn sent(terminal: &str, text: &str) {
    let terminal = terminal.trim();
    if !terminal.is_empty() {
        let _ = SENT.send((terminal.to_string(), text.to_string()));
    }
}

pub fn office_ctx(who: &str) -> crate::voice_map::Ctx {
    if who == NIXA {
        return crate::voice_map::Ctx::nixa("chat");
    }
    crate::voice_map::Ctx { terminal: Some(who.to_string()), kind: Some("chat".into()), ..Default::default() }
}

pub fn office_line(value: &Value) -> Option<(String, String)> {
    let who = value.get("who").and_then(Value::as_str).map(str::trim).filter(|w| !w.is_empty())?;
    let text = value.get("text").and_then(Value::as_str).and_then(short_line)?;
    Some((who.to_string(), text))
}

pub fn voice_note(app: &App, who: &str, text: &str, state: &str, src: &str) {
    voice_log::note(&mut lock(&app.universe.voice), who, text, state, src, now_ms());
    app.send_out(voice_log::event(who, text, state, src));
}

pub fn voice_status(app: &App) -> Value {
    let on = crate::voice::settings(app).voice_on;
    voice_log::status(&lock(&app.universe.voice), on, now_ms())
}

fn office_say(app: &Arc<App>, value: &Value) {
    let Some((who, text)) = office_line(value) else { return };
    if !crate::voice::settings(app).voice_on || lock(&app.talk.presence).quiet(now_ms()) {
        voice_note(app, &who, &text, voice_log::SKIP, voice_log::OFFICE);
        return;
    }
    if cfg!(test) {
        return;
    }
    let voice = crate::voice_map::pick(app, &office_ctx(&who));
    let app = app.clone();
    std::thread::spawn(move || {
        let until = now_ms() + OFFICE_WAIT_MS;
        while nixa_voice::is_speaking() {
            if now_ms() > until {
                voice_note(&app, &who, &text, voice_log::SKIP, voice_log::OFFICE);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let began = Arc::new(AtomicBool::new(false));
        let live = Arc::new(AtomicUsize::new(0));
        let hook = {
            let (app, who, text, began, live) = (app.clone(), who.clone(), text.clone(), began.clone(), live.clone());
            move |_: &str, start: bool| {
                office_part(&live, start);
                if start && !began.swap(true, Ordering::SeqCst) {
                    voice_note(&app, &who, &text, voice_log::START, voice_log::OFFICE);
                }
            }
        };
        let (_, done) = nixa_voice::speak_with_parts(&text, &voice, hook);
        let _ = done.recv();
        office_done(&live);
        let state = if began.load(Ordering::SeqCst) { voice_log::END } else { voice_log::SKIP };
        voice_note(&app, &who, &text, state, voice_log::OFFICE);
    });
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    if kind == OFFICE_SAY_KIND {
        office_say(app, value);
        return true;
    }
    if kind != METEOR_KIND {
        return false;
    }
    if value.get("stop").and_then(Value::as_bool) == Some(true) {
        sound::stop(app);
        return true;
    }
    let direction = value.get("direction").and_then(Value::as_str).and_then(Direction::parse);
    let at = value.get("at").and_then(Value::as_u64);
    if let (Some(direction), Some(at)) = (direction, at) {
        sound::meteor(app, direction, at);
    }
    true
}

pub fn on_out(app: &Arc<App>, value: &Value) {
    let on = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
    match value.get("type").and_then(Value::as_str) {
        Some(crate::queue::CHANGED) => tasks::changed(app, value.get("id").and_then(Value::as_i64)),
        Some("talking") if on("on") => sound::stop(app),
        Some("level") if on("talking") => sound::stop(app),
        _ => {}
    }
}

pub async fn run(app: Arc<App>) {
    let mut sends = SENT.subscribe();
    let mut out = app.out.subscribe();
    tasks::seed(&app);
    loop {
        tokio::select! {
            got = sends.recv() => match got {
                Ok((terminal, text)) => {
                    if !lock(&app.deep).owns(&terminal) {
                        screen_flow(&app, NIXA, &terminal, "send", Some(&text));
                    }
                    note(&app, NIXA, &terminal, true);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
            got = out.recv() => match got {
                Ok(value) => on_out(&app, &value),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            },
        }
    }
}
