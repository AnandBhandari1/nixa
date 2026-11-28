use crate::app::{App, lock};
use crate::talk::Said;
use crate::talk::speech::{PhoneSay, SayOptions};
use crate::util::{log, now_ms, read_json, write_json};
use axum::Router;
use axum::extract::State as Shared;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nixa_phone::{Deps, Door, Pcm, PhoneCommand, PhoneEvent, PhoneInfo, PhoneSink, SnapshotFn, Synth};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_util::sync::CancellationToken;

pub const FILE: &str = "phone-door.json";
pub const ADDRESS_ENV: &str = "NIXA_PHONE_ADDRESS";
pub const NOT_STARTED: &str = "The phone door did not start.";
pub const SNAPSHOT_ITEMS: usize = 50;
pub const SAY_RATE: u32 = 22_050;
const SYNTH_MS: u64 = 30_000;
const POLL: Duration = Duration::from_millis(20);
const SNAPSHOT_KINDS: [&str; 7] = ["chat", "reply", "voice", "busy", "talk", "chat_cleaned", "phone"];
const PASSED_KINDS: [&str; 3] = ["level", "heard", "say"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub on: bool,
    pub mobile: bool,
}

#[derive(Default)]
pub struct State {
    pub door: Mutex<Option<Door>>,
    pub sink: Mutex<Option<PhoneSink>>,
    pub start_error: Mutex<Option<String>>,
    pub camera: crate::camera::State,
}

pub fn saved(app: &App) -> Saved {
    read_json(&app.dir.join(FILE)).unwrap_or_default()
}

fn save(app: &App, change: impl FnOnce(&mut Saved)) -> Result<Saved, String> {
    let mut now = saved(app);
    change(&mut now);
    write_json(&app.dir.join(FILE), &now).map_err(|e| format!("Could not save {FILE}: {e}"))?;
    Ok(now)
}

pub fn door(app: &App) -> Option<Door> {
    lock(&app.phone.door).clone()
}

fn sink(app: &App) -> Option<PhoneSink> {
    lock(&app.phone.sink).clone()
}

pub fn info(app: &App) -> PhoneInfo {
    match door(app) {
        Some(door) => door.info(),
        None => PhoneInfo { error: Some(lock(&app.phone.start_error).clone().unwrap_or_else(|| NOT_STARTED.to_string())), ..PhoneInfo::default() },
    }
}

pub fn view(app: &App) -> Value {
    let caps = crate::camera::caps(app);
    json!({"mobile": saved(app).mobile, "phone": crate::camera::phone_view(&info(app), &caps), "camera": crate::camera::view(app)})
}

pub fn state_event(app: &App) -> Value {
    let now = view(app);
    json!({"type": "phone", "phone": now["phone"], "mobile": now["mobile"], "camera": now["camera"]})
}

pub fn send_state(app: &App) {
    app.send_out(state_event(app));
}

pub fn snapshot(app: &App) -> Value {
    let items = {
        let chat = lock(&app.chat);
        let list: Vec<_> = chat.list().iter().filter(|item| !item.mac_only).cloned().collect();
        list[list.len().saturating_sub(SNAPSHOT_ITEMS)..].to_vec()
    };
    json!({
        "name": app.name(),
        "busy": crate::agent::busy(app),
        "items": items,
        "voice": crate::voice::view(app),
        "talk": crate::talk::view(app),
        "mobile": saved(app).mobile,
    })
}

pub fn voice_on_phone(app: &App) -> bool {
    let now = saved(app);
    now.on && now.mobile && door(app).is_some()
}

fn sync_voice(app: &App) {
    crate::talk::speech::set_phone_on(app, voice_on_phone(app));
}

pub async fn run(app: Arc<App>) {
    let weak = Arc::downgrade(&app);
    let snap: SnapshotFn = Arc::new(move || weak.upgrade().map(|a| snapshot(&a)).unwrap_or(Value::Null));
    let (door, mut events) = match Door::new(deps(&app.dir, snap)) {
        Ok(made) => made,
        Err(e) => {
            log("phone", &format!("the phone door was not made: {e:#}"));
            *lock(&app.phone.start_error) = Some(format!("{NOT_STARTED} {e:#}"));
            return;
        }
    };
    let sink = PhoneSink::new(door.clone(), synth());
    *lock(&app.phone.door) = Some(door.clone());
    *lock(&app.phone.sink) = Some(sink.clone());
    tokio::spawn(crate::camera::sweep_loop(app.clone()));
    let (tx, rx) = unbounded_channel::<PhoneSay>();
    crate::talk::speech::set_phone(&app, Some(tx));
    sync_voice(&app);
    tokio::spawn(play_loop(door.clone(), sink.clone(), rx));
    tokio::spawn(relay_loop(app.clone(), door.clone()));
    if saved(&app).on {
        door.start().await;
    }
    while let Some(event) = events.recv().await {
        on_event(&app, &sink, event);
    }
}

pub fn deps(dir: &Path, snap: SnapshotFn) -> Deps {
    let mut deps = Deps::new(dir, snap);
    if let Some(address) = std::env::var(ADDRESS_ENV).ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()) {
        let names = nixa_phone::DoorNames { ips: vec![address.clone()], dns: Vec::new() };
        deps.address = Arc::new(move || Some(address.clone()));
        deps.names = Arc::new(move |_| names.clone());
    }
    deps
}

pub async fn stop(app: &App) {
    if let Some(door) = door(app)
        && door.is_on()
    {
        door.stop().await;
    }
}

async fn play_loop(door: Door, sink: PhoneSink, mut rx: UnboundedReceiver<PhoneSay>) {
    while let Some(job) = rx.recv().await {
        if job.stop.is_cancelled() {
            let _ = job.done.send(nixa_voice::SayResult { interrupted: true, ms: 0 });
            continue;
        }
        let keep = job.options.is_nixa() && !door.text_only();
        let mut played = sink.play(&job.text, &job.voice, keep);
        let result = tokio::select! {
            result = played.wait() => result,
            _ = job.stop.cancelled() => {
                played.stop();
                played.wait().await
            }
        };
        let _ = job.done.send(nixa_voice::SayResult { interrupted: result.interrupted, ms: result.ms });
    }
}

async fn relay_loop(app: Arc<App>, door: Door) {
    let mut rx = app.out.subscribe();
    loop {
        match rx.recv().await {
            Ok(value) => relay(&app, &door, &value),
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        }
    }
}

pub fn relayed(app: &App, value: &Value) -> Option<Value> {
    let kind = value.get("type").and_then(Value::as_str)?;
    if PASSED_KINDS.contains(&kind) {
        return Some(value.clone());
    }
    SNAPSHOT_KINDS.contains(&kind).then(|| json!({"type": "snapshot", "snapshot": snapshot(app)}))
}

fn relay(app: &App, door: &Door, value: &Value) {
    if door.clients() == 0 {
        return;
    }
    if let Some(out) = relayed(app, value) {
        door.relay(&out);
    }
}

fn on_event(app: &Arc<App>, sink: &PhoneSink, event: PhoneEvent) {
    match event {
        PhoneEvent::Changed => send_state(app),
        PhoneEvent::Voice { phone, ready } => {
            log("phone", &format!("{phone} voice {}", if ready { "ready" } else { "off" }));
            if ready {
                play_missed(sink.clone());
            }
            send_state(app);
        }
        PhoneEvent::TalkStart { phone, auto } => {
            crate::talk::hold::start(app, &phone);
            crate::talk::flow::on_speech_start(app);
            if !auto {
                crate::talk::speech::barge_in(app);
            }
        }
        PhoneEvent::TalkPeek { phone, pcm } => {
            if !peek_wanted(app) {
                return;
            }
            let app = app.clone();
            tokio::spawn(async move { peek(&app, &phone, pcm).await });
        }
        PhoneEvent::Talk { phone, pcm, started_at, auto } => {
            crate::talk::hold::end(app, &phone);
            crate::talk::flow::on_speech_end(app);
            let app = app.clone();
            tokio::spawn(async move { heard(&app, &phone, pcm, started_at, auto).await });
        }
        PhoneEvent::TalkCancel { phone } => {
            crate::talk::flow::on_speech_end(app);
            crate::talk::hold::cancel(app, &phone);
            log("phone", &format!("{phone} talk cancelled"));
        }
        PhoneEvent::Command { phone, command } => on_command(app, &phone, command),
        PhoneEvent::Caps { phone, camera } => {
            log("phone", &format!("{phone} camera looks {}", if camera { "supported" } else { "not supported" }));
            if let Err(e) = crate::camera::set_caps(app, &phone, camera) {
                log("phone", &e);
            }
            send_state(app);
        }
        PhoneEvent::LookFrames { phone, id, frames } => crate::camera::frames_came(app, &phone, id, frames),
        PhoneEvent::LookFail { phone, id, why } => crate::camera::look_failed(app, &phone, id, &why),
    }
}

pub fn peek_wanted(app: &App) -> bool {
    if !crate::talk::speech::speaking(app) {
        return false;
    }
    let headphones = crate::settings_api::headphones(&app.dir);
    !nixa_voice::mic_open_during_voice(headphones, nixa_voice::echo_cancelled(), voice_on_phone(app))
}

fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

pub fn real_words(text: &str) -> bool {
    words_of(text).iter().any(|w| w.chars().count() >= 2)
}

pub fn own_voice(heard: &str, said: &str) -> bool {
    let heard = words_of(heard);
    if heard.is_empty() {
        return false;
    }
    let said: std::collections::HashSet<String> = words_of(said).into_iter().collect();
    let found = heard.iter().filter(|w| said.contains(*w)).count();
    found * 10 >= heard.len() * 7
}

pub enum Peek {
    Quiet,
    Own,
    Cut,
}

pub fn peek_step(heard: &str, said: Option<&str>) -> Peek {
    if !real_words(heard) {
        return Peek::Quiet;
    }
    if said.is_some_and(|s| own_voice(heard, s)) {
        return Peek::Own;
    }
    Peek::Cut
}

async fn peek(app: &Arc<App>, phone: &str, pcm: Vec<i16>) {
    let dir = parakeet_dir(app);
    let text = match tokio::task::spawn_blocking(move || nixa_voice::transcribe(&dir, &pcm)).await {
        Ok(Ok(t)) => t.text,
        Ok(Err(e)) => {
            log("phone", &format!("{phone} wake check failed: {e:#}"));
            return;
        }
        Err(e) => {
            log("phone", &format!("{phone} wake check stopped: {e}"));
            return;
        }
    };
    if !crate::talk::speech::speaking(app) {
        return;
    }
    let said = crate::talk::speech::last_said(app);
    match peek_step(text.trim(), said.as_deref()) {
        Peek::Quiet => log("phone", &format!("{phone} wake check had no words, Nixa keeps speaking")),
        Peek::Own => log("phone", &format!("{phone} wake check was Nixa's own voice")),
        Peek::Cut => {
            log("phone", &format!("{phone} wake check heard the user, Nixa stops speaking"));
            crate::talk::flow::on_speech_start(app);
            crate::talk::speech::barge_in(app);
        }
    }
}

fn play_missed(sink: PhoneSink) {
    let missed = sink.take_missed();
    if missed.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for line in missed {
            sink.play(&line.text, &line.voice, false).wait().await;
        }
    });
}

fn parakeet_dir(app: &App) -> PathBuf {
    let models = lock(&app.voice.models).clone();
    nixa_voice::ListenConfig { models_dir: models, ..nixa_voice::ListenConfig::default() }.parakeet_dir()
}

async fn heard(app: &Arc<App>, phone: &str, pcm: Vec<i16>, started_at: u64, auto: bool) {
    let dir = parakeet_dir(app);
    let done = tokio::task::spawn_blocking(move || nixa_voice::transcribe(&dir, &pcm)).await;
    let text = match done {
        Ok(Ok(t)) => t.text,
        Ok(Err(e)) => {
            log("phone", &format!("{phone} speech to text failed: {e:#}"));
            app.info(&format!("The phone speech was not turned into text: {e:#}"));
            crate::talk::hold::answered(app);
            return;
        }
        Err(e) => {
            log("phone", &format!("{phone} speech to text stopped: {e}"));
            crate::talk::hold::answered(app);
            return;
        }
    };
    let text = text.trim().to_string();
    app.send_out(json!({"type": "heard", "text": text}));
    if text.is_empty() {
        crate::talk::hold::answered(app);
        return;
    }
    let said = Said { pressed: !auto, mac: false, ..Said::heard(&text, started_at, now_ms()) };
    crate::talk::hear_later(app, said);
}

fn stop_all(app: &Arc<App>) {
    lock(&app.talk.gather).drop_all();
    crate::talk::speech::stop(app);
    if crate::agent::interrupt(app) {
        log("phone", "stopped the brain");
    }
    crate::talk::hold::answered(app);
    app.send_out(json!({"type": "stopped"}));
}

fn on_command(app: &Arc<App>, phone: &str, command: PhoneCommand) {
    match command {
        PhoneCommand::Text(text) => crate::talk::hear_later(app, Said { mac: false, ..Said::typed(&text, now_ms()) }),
        PhoneCommand::SetMobile(on) => {
            log("phone", &format!("{phone} asked to turn mobile mode {}; only the Mac changes it, so nothing changed", if on { "on" } else { "off" }));
        }
        PhoneCommand::Stop => stop_all(app),
    }
}

pub fn set_mobile(app: &App, on: bool) -> Result<(), String> {
    save(app, |s| s.mobile = on)?;
    sync_voice(app);
    if let Some(sink) = sink(app)
        && !on
    {
        sink.clear_missed();
    }
    if on && voice_on_phone(app) {
        nixa_voice::stop_speaking();
    }
    log("phone", if on { "voice on the phone" } else { "voice on the Mac" });
    send_state(app);
    Ok(())
}

pub async fn set_door(app: &App, on: bool) -> Result<(), String> {
    let door = door(app).ok_or_else(|| info(app).error.unwrap_or_else(|| NOT_STARTED.to_string()))?;
    save(app, |s| s.on = on)?;
    if on {
        door.start().await;
    } else {
        door.stop().await;
    }
    sync_voice(app);
    send_state(app);
    match door.info().error {
        Some(error) if on => Err(error),
        _ => Ok(()),
    }
}

pub async fn apply(app: &App, body: &Value) -> Result<(), String> {
    let kind = body.get("type").and_then(Value::as_str).unwrap_or_default();
    let on = || body.get("on").and_then(Value::as_bool).ok_or_else(|| "on must be true or false.".to_string());
    match kind {
        "set_phone" => set_door(app, on()?).await,
        "set_mobile" => set_mobile(app, on()?),
        "phone_new_key" => {
            let door = door(app).ok_or_else(|| NOT_STARTED.to_string())?;
            door.new_key().await.map_err(|e| format!("A new code was not made: {e:#}"))
        }
        "phone_remove" => {
            let id = body.get("id").and_then(Value::as_str).map(str::trim).unwrap_or_default();
            let door = door(app).ok_or_else(|| NOT_STARTED.to_string())?;
            if !door.remove(id) {
                return Err("No such phone.".to_string());
            }
            crate::camera::forget_caps(app, id)?;
            crate::camera::log::forget(&crate::camera::dir(app), id, crate::util::now_ms())?;
            send_state(app);
            Ok(())
        }
        kind if crate::camera::is_command(kind) => {
            crate::camera::apply(app, kind, body)?;
            send_state(app);
            Ok(())
        }
        _ => Err(format!("The phone door does not know the step {kind:?}.")),
    }
}

pub fn is_ws(kind: &str) -> bool {
    matches!(kind, "set_phone" | "set_mobile" | "phone_new_key" | "phone_remove" | "phone_say") || crate::camera::is_command(kind)
}

pub fn ws(app: &Arc<App>, kind: &str, value: &Value) -> bool {
    if !is_ws(kind) {
        return false;
    }
    if kind == "phone_say" {
        return say(app, value);
    }
    let (app, value) = (app.clone(), value.clone());
    tokio::spawn(async move {
        if let Err(e) = apply(&app, &value).await {
            log("phone", &e);
            app.info(&e);
        }
    });
    true
}

pub fn say_options(value: &Value) -> SayOptions {
    let field = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    SayOptions {
        voice: field("voice"),
        agent: Some(field("agent").unwrap_or_else(|| "nixa".into())),
        terminal: field("terminal"),
        model: field("model"),
        space: field("space"),
        kind: field("kind"),
        mac_only: false,
    }
}

fn say(app: &Arc<App>, value: &Value) -> bool {
    let Some(text) = value.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    crate::talk::speech::speak_later(app, text, say_options(value));
    true
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/phone", get(phone_get).post(phone_post))
}

fn answer(app: &App, error: Option<String>) -> Response {
    let mut body = view(app);
    body["ok"] = json!(error.is_none());
    let code = if error.is_some() { StatusCode::BAD_REQUEST } else { StatusCode::OK };
    if let Some(error) = error {
        body["error"] = json!(error);
    }
    (code, axum::Json(body)).into_response()
}

async fn phone_get(Shared(app): Shared<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    answer(&app, None)
}

async fn phone_post(Shared(app): Shared<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let error = apply(&app, &body).await.err();
    answer(&app, error)
}

fn installed(bin: &Path) -> Option<PathBuf> {
    if bin.is_absolute() {
        return bin.exists().then(|| bin.to_path_buf());
    }
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(bin)).find(|p| p.exists())
}

pub fn synth() -> Synth {
    let ptts = installed(&nixa_phone::sink::ptts_bin()).map(nixa_phone::sink::ptts_synth);
    Arc::new(move |text, voice, stop| ptts.as_ref().and_then(|p| p(text, voice, stop)))
}

fn temp_wav() -> PathBuf {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let n = COUNT.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("nixa-phone-say-{}-{n}.wav", std::process::id()))
}

pub fn mac_synth(text: &str, voice: Option<&str>, stop: &CancellationToken) -> Option<Pcm> {
    if stop.is_cancelled() || text.trim().is_empty() {
        return None;
    }
    let wav = temp_wav();
    let mut cmd = Command::new("/usr/bin/say");
    if let Some(voice) = voice {
        cmd.args(["-v", voice]);
    }
    cmd.arg("-o").arg(&wav).arg("--file-format=WAVE").arg(format!("--data-format=LEI16@{SAY_RATE}")).args(["-f", "-"]);
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let started = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if stop.is_cancelled() || started.elapsed() > Duration::from_millis(SYNTH_MS) => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(_) => break false,
        }
    };
    let raw = std::fs::read(&wav).ok();
    let _ = std::fs::remove_file(&wav);
    if !ok || stop.is_cancelled() {
        return None;
    }
    raw.and_then(|raw| nixa_phone::sink::wav_pcm(&raw))
}

#[cfg(test)]
mod tests;
