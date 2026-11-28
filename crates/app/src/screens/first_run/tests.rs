use super::key::{self, Brain, Saving};
use super::perms::{self, Grant, Perm};
use super::voice::{self, Fetch, Models, ModelsState};
use super::*;
use gpui::{Context, Entity, Modifiers, Render, TestAppContext, VisualTestContext, Window, div};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

static ASKED: Mutex<Vec<Perm>> = Mutex::new(Vec::new());
static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FETCHED: AtomicBool = AtomicBool::new(false);

fn fake_check(p: Perm) -> Grant {
    match p {
        Perm::Mic => Grant::NotAsked,
        Perm::InputMonitoring => Grant::Denied,
        Perm::Accessibility => Grant::Allowed,
        Perm::ScreenRecording => Grant::Denied,
    }
}

fn fake_ask(p: Perm) {
    ASKED.lock().expect("asked").push(p);
}

fn fake_open(url: &str) {
    OPENED.lock().expect("opened").push(url.to_string());
}

fn fake_models() -> Result<Models, String> {
    let ready = FETCHED.load(Ordering::SeqCst);
    Ok(Models {
        ready,
        dir: "/Users/someone/voice-models".into(),
        missing: if ready { Vec::new() } else { vec!["speech to text".into()] },
        bytes: if ready { 0 } else { 487_211_443 },
    })
}

fn fake_fetch(tx: Sender<Fetch>) {
    tx.send(Fetch::Line("stt: downloading 487 MB".into())).ok();
    tx.send(Fetch::Line("stt: 50%".into())).ok();
    FETCHED.store(true, Ordering::SeqCst);
    tx.send(Fetch::Done(Ok(()))).ok();
}

fn hooks() -> Hooks {
    Hooks { check: fake_check, ask: fake_ask, open: fake_open, models: fake_models, fetch: fake_fetch }
}

type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

#[derive(Default)]
struct Fake {
    brains: Vec<Value>,
    choice: Option<String>,
    slow: Option<std::time::Duration>,
}

fn answer(fake: &mut Fake, method: &str, path: &str, body: &Value) -> (u16, Value) {
    match (method, path) {
        ("GET", "/brains") => (200, json!({"ok": true, "brains": fake.brains, "cli": [], "choice": fake.choice})),
        ("GET", "/health") => {
            let brain = fake.choice.as_ref().map(|c| json!({"name": c.trim_start_matches("api:"), "kind": "openai"}));
            (200, json!({"ok": true, "name": "Nixa", "brain": brain}))
        }
        ("POST", "/brains") => {
            if body["apiKey"] != json!("good-key") {
                return (400, json!({"ok": false, "error": "The key did not work. Nothing was saved."}));
            }
            let id = format!("b{}", fake.brains.len() + 1);
            fake.brains.push(json!({"id": id, "name": body["name"], "kind": body["kind"], "choice": format!("api:{id}")}));
            (200, json!({"ok": true, "id": id, "choice": format!("api:{id}")}))
        }
        ("POST", "/brain/choice") => {
            fake.choice = body["choice"].as_str().map(str::to_string);
            (200, json!({"ok": true}))
        }
        _ => (404, json!({"ok": false, "error": "no such route"})),
    }
}

fn serve(fake: Fake) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let fake = Arc::new(Mutex::new(fake));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut first = String::new();
            reader.read_line(&mut first).ok();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut raw = vec![0u8; length];
            reader.read_exact(&mut raw).ok();
            let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let slow = fake.lock().expect("fake").slow.filter(|_| method == "GET" && path == "/brains");
            if let Some(wait) = slow {
                std::thread::sleep(wait);
            }
            let (code, reply) = answer(&mut fake.lock().expect("fake"), &method, &path, &body);
            log.lock().expect("seen").push((method, path, body));
            let text = reply.to_string();
            let head = format!(
                "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                text.len()
            );
            stream.write_all(head.as_bytes()).ok();
            stream.write_all(text.as_bytes()).ok();
        }
    });
    (addr, seen)
}

fn posts(seen: &Seen) -> Vec<(String, Value)> {
    seen.lock()
        .expect("seen")
        .iter()
        .filter(|(m, _, _)| m == "POST")
        .map(|(_, p, b)| (p.clone(), b.clone()))
        .collect()
}

struct Host {
    view: Entity<FirstRunView>,
    events: Vec<FirstRunEvent>,
    _sub: gpui::Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

fn open(cx: &mut TestAppContext, addr: String, done: bool) -> (Entity<Host>, Entity<FirstRunView>, &mut VisualTestContext) {
    cx.update(nixa_ui::init);
    let (host, vcx) = cx.add_window_view(move |_, cx| {
        let view = super::new(hooks(), addr, done, cx);
        let sub = cx.subscribe(&view, |host: &mut Host, _, event: &FirstRunEvent, _| host.events.push(event.clone()));
        Host { view, events: Vec::new(), _sub: sub }
    });
    vcx.simulate_resize(gpui::size(gpui::px(900.), gpui::px(2400.)));
    let view = vcx.update(|_, cx| host.read(cx).view.clone());
    view.update(vcx, |v, cx| {
        super::set_spaces(v, Vec::new(), true);
        super::refresh(v, cx);
    });
    vcx.run_until_parked();
    (host, view, vcx)
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    vcx.update(|window, app| window.draw(app).clear());
    let bounds = vcx.debug_bounds(selector).unwrap_or_else(|| panic!("{selector} is not drawn"));
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
}

fn drawn(vcx: &mut VisualTestContext, selector: &'static str) -> bool {
    vcx.update(|window, app| window.draw(app).clear());
    vcx.debug_bounds(selector).is_some()
}

fn events(host: &Entity<Host>, vcx: &mut VisualTestContext) -> Vec<FirstRunEvent> {
    vcx.update(|_, cx| host.read(cx).events.clone())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mo-fr-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[gpui::test]
fn a_new_tab_with_no_space_shows_add_a_space(cx: &mut TestAppContext) {
    let (host, view, vcx) = open(cx, "127.0.0.1:1".into(), true);
    view.update(vcx, |v, cx| {
        super::set_spaces(v, Vec::new(), true);
        assert!(super::ask_for_space(v));
        assert!(!super::ask_for_space(v));
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(drawn(vcx, "first-run-need-space"));
    click(vcx, "first-run-need-space-add");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::PickSpace]);
    view.update(vcx, |v, _| {
        super::set_spaces(v, vec!["shop".into()], true);
        assert!(!v.need_space);
        assert!(!super::ask_for_space(v));
    });
    assert!(super::NEED_SPACE.starts_with("Add a space first"));
    assert!(super::NEED_SPACE_ERROR.starts_with("Add a space first"));
}

#[gpui::test]
fn cmd_t_in_the_window_with_no_space_shows_the_note(cx: &mut TestAppContext) {
    let dir = PathBuf::from(format!("/tmp/mo-fr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    unsafe {
        std::env::set_var(nixa_protocol::paths::ENV_DATA_DIR, &dir);
        std::env::set_var(nixa_protocol::paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
        std::env::set_var(nixa_protocol::control::ENV_APP_SOCKET, dir.join("a.sock"));
    }
    cx.update(nixa_ui::init);
    let (ws, vcx) = cx.add_window_view(crate::workspace::Workspace::new);
    vcx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    ws.update(vcx, |ws, cx| {
        ws.engine = crate::workspace::EngineState::Up;
        ws.spaces = crate::spaces::Spaces::default();
        cx.notify();
    });
    vcx.run_until_parked();
    ws.update(vcx, |ws, cx| ws.new_tab(Vec::new(), None, cx));
    vcx.run_until_parked();
    assert!(ws.read_with(vcx, |ws, _| ws.tabs.pending.is_empty()));
    let noted = vcx.update(|_, app| super::host::view(app).map(|v| v.read(app).need_space));
    assert_eq!(noted, Some(true));
    assert!(drawn(vcx, "first-run-need-space"));
}

#[test]
fn permission_texts_and_settings_links() {
    assert_eq!(
        perms::settings_url(Perm::Mic),
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
    );
    assert!(perms::settings_url(Perm::InputMonitoring).ends_with("?Privacy_ListenEvent"));
    assert!(perms::settings_url(Perm::Accessibility).ends_with("?Privacy_Accessibility"));
    assert!(perms::settings_url(Perm::ScreenRecording).ends_with("?Privacy_ScreenCapture"));
    let titles: Vec<&str> = perms::ALL.iter().map(|p| perms::title(*p)).collect();
    assert_eq!(titles, ["Microphone", "Input Monitoring", "Accessibility", "Screen Recording"]);
    assert!(perms::ALL.iter().all(|p| !perms::why(*p).is_empty()));
    assert!(perms::can_ask(Perm::Mic, Grant::NotAsked));
    assert!(!perms::can_ask(Perm::Mic, Grant::Denied));
    assert!(perms::can_ask(Perm::InputMonitoring, Grant::Denied));
    assert!(perms::can_ask(Perm::Accessibility, Grant::Denied));
    assert!(perms::can_ask(Perm::ScreenRecording, Grant::Denied));
    assert!(!perms::can_ask(Perm::ScreenRecording, Grant::Allowed));
    assert_eq!(perms::grant_words(Grant::NotAsked), "Not asked yet");
    let grants = perms::check_all(fake_check);
    assert_eq!(grants.len(), 4);
    assert_eq!(perms::allowed_count(&grants), 1);
}

#[test]
fn reads_the_voice_model_check() {
    let answer = json!({"dir": "/Users/someone/voice-models", "ready": false, "checked_hash": false, "models": [
        {"name": "vad", "what": "voice activity (Silero VAD)", "download_size": 643_854, "ready": true, "files": []},
        {"name": "stt", "what": "speech to text (Parakeet TDT 0.6b v3, int8)", "download_size": 487_211_443, "ready": false, "files": []}
    ]});
    let models = voice::parse_status(&answer).expect("models");
    assert!(!models.ready);
    assert_eq!(models.missing, vec!["speech to text (Parakeet TDT 0.6b v3, int8)".to_string()]);
    assert_eq!(models.bytes, 487_211_443);
    assert_eq!(voice::size_words(models.bytes), "about 487 MB");
    assert_eq!(voice::size_words(10), "less than 1 MB");
    let all = json!({"ready": true, "models": [{"name": "vad", "ready": true, "download_size": 1}]});
    let ready = voice::parse_status(&all).expect("ready");
    assert!(ready.ready && ready.missing.is_empty() && ready.bytes == 0);
    assert_eq!(voice::parse_status(&json!({"ready": true})), None);
    assert_eq!(voice::progress("  stt: 40%  "), Some("stt: 40%".into()));
    assert_eq!(voice::progress("   "), None);
    assert_eq!(voice::last_words("a\nb\n\nc\nd\ne", "none"), "b c d e");
    assert_eq!(voice::last_words("", "none"), "none");
    assert!(voice::CREDIT.contains("Parakeet TDT 0.6B v3 by NVIDIA"));
    assert!(voice::CREDIT.contains("CC BY 4.0 (creativecommons.org/licenses/by/4.0)"));
    assert!(voice::CREDIT.contains("Silero VAD"));
}

#[test]
fn reads_the_brain_and_builds_the_key_body() {
    assert_eq!(key::parse(&json!({"brains": [], "choice": null}), &json!({})), Brain::Missing);
    let brains = json!({"brains": [{"id": "b1", "name": "OpenAI", "kind": "openai", "choice": "api:b1"}], "choice": "api:b1"});
    assert_eq!(key::parse(&brains, &json!({})), Brain::Ready { name: "OpenAI".into() });
    assert_eq!(
        key::parse(&brains, &json!({"brain": {"name": "Codex"}})),
        Brain::Ready { name: "Codex".into() }
    );
    assert_eq!(key::unique_name("anthropic", &brains), "Anthropic");
    assert_eq!(key::unique_name("openai", &brains), "OpenAI 2");
    assert_eq!(
        key::body("openai", "OpenAI 2", "  some-key "),
        json!({"kind": "openai", "name": "OpenAI 2", "baseUrl": "", "apiKey": "some-key"})
    );
    assert!(key::key_problem("").is_some());
    assert!(key::key_problem("two words").is_some());
    assert!(key::key_problem("a\"b").is_some());
    assert_eq!(key::key_problem("good-key"), None);
}

#[test]
fn zero_spaces_always_shows_the_first_run() {
    assert!(!wanted(2, false));
    assert!(wanted(0, false));
    assert!(wanted(2, true));
}

#[test]
fn a_fresh_store_has_no_space_and_the_first_pick_makes_one() {
    let dir = temp("fresh");
    let conn = nixa_store::open(&dir.join("nixa.db")).expect("store");
    let list = crate::spaces::load(&conn).expect("spaces");
    assert!(list.spaces.is_empty());
    assert_eq!(list.active, None);
    assert!(!is_done(Some(&conn)));
    assert!(wanted(list.spaces.len(), false));
    let mut list = list;
    let id = crate::spaces::add(&mut list, "/Users/someone/shop");
    assert_eq!(list.spaces.len(), 1);
    assert_eq!(list.spaces[0].name, "shop");
    assert_eq!(list.active.as_deref(), Some(id.as_str()));
    mark_done(&conn).expect("done");
    assert!(is_done(Some(&conn)));
    assert!(!wanted(list.spaces.len(), false));
    assert!(!is_done(None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui::test]
fn with_no_space_it_asks_for_the_first_space(cx: &mut TestAppContext) {
    let (addr, _) = serve(Fake::default());
    let (host, view, vcx) = open(cx, addr, false);
    assert!(drawn(vcx, "first-run-add-space"));
    click(vcx, "first-run-add-space");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::PickSpace]);
    click(vcx, "first-run-finish");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::PickSpace]);
    assert!(!view.read_with(vcx, |v, _| v.done));
    view.update(vcx, |v, cx| {
        super::set_spaces(v, vec!["shop".into()], true);
        cx.notify();
    });
    vcx.run_until_parked();
    click(vcx, "first-run-finish");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::PickSpace, FirstRunEvent::Finish]);
    assert!(view.read_with(vcx, |v, _| v.done && !steps_shown(v)));
    view.update(vcx, |v, cx| {
        super::set_spaces(v, Vec::new(), true);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(drawn(vcx, "first-run-add-space"));
    assert!(view.read_with(vcx, |v, _| !steps_shown(v) && wanted(v.spaces.len(), v.asked)));
}

#[gpui::test]
fn permission_buttons_ask_and_open_system_settings(cx: &mut TestAppContext) {
    let (addr, _) = serve(Fake::default());
    let (_, _, vcx) = open(cx, addr, false);
    assert!(drawn(vcx, "first-run-ask-mic"));
    assert!(drawn(vcx, "first-run-ask-input"));
    assert!(!drawn(vcx, "first-run-open-accessibility"));
    assert!(drawn(vcx, "first-run-ask-screen"));
    click(vcx, "first-run-ask-mic");
    assert!(ASKED.lock().expect("asked").contains(&Perm::Mic));
    click(vcx, "first-run-open-screen");
    assert!(
        OPENED
            .lock()
            .expect("opened")
            .contains(&"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture".to_string())
    );
    click(vcx, "first-run-ask-input");
    assert!(ASKED.lock().expect("asked").contains(&Perm::InputMonitoring));
    click(vcx, "first-run-open-input");
    assert!(OPENED.lock().expect("opened").iter().any(|u| u.ends_with("Privacy_ListenEvent")));
}

#[gpui::test]
fn the_voice_models_download_shows_progress_and_ends_ready(cx: &mut TestAppContext) {
    FETCHED.store(false, Ordering::SeqCst);
    let (addr, _) = serve(Fake::default());
    let (_, view, vcx) = open(cx, addr, false);
    assert!(view.read_with(vcx, |v, _| matches!(&v.models, ModelsState::Known(m) if !m.ready && m.bytes == 487_211_443)));
    assert!(drawn(vcx, "first-run-model-credit"));
    click(vcx, "first-run-fetch");
    assert!(view.read_with(vcx, |v, _| matches!(&v.models, ModelsState::Known(m) if m.ready)));
    assert!(drawn(vcx, "first-run-model-credit"));
    view.update(vcx, |v, cx| {
        assert!(super::fetch_step(v, Some(Fetch::Line("stt: 70%".into())), cx));
        assert_eq!(v.models, ModelsState::Fetching { line: "stt: 70%".into() });
        assert!(!super::fetch_step(v, None, cx));
        assert_eq!(v.models, ModelsState::Failed("The download stopped.".into()));
        assert!(!super::fetch_step(v, Some(Fetch::Done(Err("No network".into()))), cx));
        assert_eq!(v.models, ModelsState::Failed("No network".into()));
    });
}

#[gpui::test]
fn the_brain_key_step_checks_saves_and_picks_the_brain(cx: &mut TestAppContext) {
    let (addr, seen) = serve(Fake::default());
    let (_, view, vcx) = open(cx, addr, false);
    assert!(view.read_with(vcx, |v, _| v.brain == Brain::Missing));
    click(vcx, "first-run-kind-openai");
    assert!(view.read_with(vcx, |v, _| v.kind == "openai"));
    view.update(vcx, |v, cx| v.key.update(cx, |k, cx| k.set_value("two words", cx)));
    click(vcx, "first-run-save-key");
    assert!(view.read_with(vcx, |v, _| matches!(&v.saving, Saving::Failed(w) if w.contains("space"))));
    assert!(posts(&seen).is_empty());
    view.update(vcx, |v, cx| v.key.update(cx, |k, cx| k.set_value("wrong-key", cx)));
    click(vcx, "first-run-save-key");
    assert!(view.read_with(vcx, |v, _| matches!(&v.saving, Saving::Failed(w) if w.contains("did not work"))));
    view.update(vcx, |v, cx| v.key.update(cx, |k, cx| k.set_value("good-key", cx)));
    click(vcx, "first-run-save-key");
    let sent = posts(&seen);
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[1].0, "/brains");
    assert_eq!(sent[1].1, json!({"kind": "openai", "name": "OpenAI", "baseUrl": "", "apiKey": "good-key"}));
    assert_eq!(sent[2], ("/brain/choice".to_string(), json!({"choice": "api:b1"})));
    assert!(view.read_with(vcx, |v, cx| v.brain == Brain::Ready { name: "b1".into() } && v.saving == Saving::Idle && v.key.read(cx).value().is_empty()));
    assert!(drawn(vcx, "first-run-brain-settings"));
}

#[gpui::test]
fn the_brain_settings_button_opens_chat_brain(cx: &mut TestAppContext) {
    let (addr, _) = serve(Fake::default());
    let (host, _, vcx) = open(cx, addr, false);
    click(vcx, "first-run-brain-settings");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::Settings(BRAIN_PART)]);
}

#[gpui::test]
fn a_brain_that_does_not_answer_says_wait(cx: &mut TestAppContext) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    drop(listener);
    let (_, view, vcx) = open(cx, addr, false);
    assert!(view.read_with(vcx, |v, _| matches!(v.brain, Brain::Down(_))));
    assert!(drawn(vcx, "first-run-brain-again"));
}

#[gpui::test]
fn a_tick_rechecks_only_while_shown(cx: &mut TestAppContext) {
    let (addr, _) = serve(Fake::default());
    let (_, view, vcx) = open(cx, addr, false);
    view.update(vcx, |v, cx| {
        v.grants.clear();
        v.shown = false;
        assert!(!super::tick(v, cx));
        assert!(v.grants.is_empty());
        v.shown = true;
        assert!(super::tick(v, cx));
        assert_eq!(v.grants.len(), 4);
    });
}

#[gpui::test]
fn a_brain_check_longer_than_a_tick_is_not_dropped(cx: &mut TestAppContext) {
    let slow = super::TICK + std::time::Duration::from_millis(400);
    let (addr, seen) = serve(Fake { slow: Some(slow), ..Fake::default() });
    let (_, view, vcx) = open(cx, addr, false);
    let asked = |seen: &Seen| seen.lock().expect("seen").iter().filter(|(m, p, _)| m == "GET" && p == "/brains").count();
    let before = asked(&seen);
    let clock = std::time::Instant::now();
    let started = view.update(vcx, |v, cx| {
        v.brain = Brain::Loading;
        super::load_brain(v, cx);
        let started = v.loads;
        for _ in 0..3 {
            super::tick(v, cx);
        }
        assert_eq!(v.loads, started);
        assert!(v.loading);
        started
    });
    vcx.executor().advance_clock(super::TICK * 3);
    vcx.run_until_parked();
    assert!(clock.elapsed() >= slow);
    let after = asked(&seen);
    view.update(vcx, |v, cx| {
        assert_eq!(v.brain, Brain::Missing);
        assert!(!v.loading);
        assert_eq!((after - before) as u64, v.loads - started + 1);
        let now = v.loads;
        super::tick(v, cx);
        assert_eq!(v.loads, now + 1);
    });
    vcx.run_until_parked();
    view.update(vcx, |v, cx| {
        assert_eq!(v.brain, Brain::Missing);
        assert!(!v.loading);
        let now = v.loads;
        super::load_brain(v, cx);
        super::load_brain(v, cx);
        assert_eq!(v.loads, now + 2);
    });
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.brain == Brain::Missing && !v.loading));
}

#[gpui::test]
fn the_done_button_at_the_top_closes_the_steps(cx: &mut TestAppContext) {
    let (addr, _) = serve(Fake::default());
    let (host, view, vcx) = open(cx, addr, false);
    view.update(vcx, |v, cx| {
        super::set_spaces(v, vec!["shop".into()], true);
        v.asked = true;
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(drawn(vcx, "first-run-close"));
    click(vcx, "first-run-close");
    assert_eq!(events(&host, vcx), vec![FirstRunEvent::Finish]);
    assert!(view.read_with(vcx, |v, _| v.done && !v.asked && !wanted(v.spaces.len(), v.asked)));
}

#[gpui::test]
fn spaces_without_the_done_flag_show_the_terminals_and_a_pick_closes_the_steps(cx: &mut TestAppContext) {
    let dir = PathBuf::from(format!("/tmp/mo-fr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    unsafe {
        std::env::set_var(nixa_protocol::paths::ENV_DATA_DIR, &dir);
        std::env::set_var(nixa_protocol::paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
        std::env::set_var(nixa_protocol::control::ENV_APP_SOCKET, dir.join("a.sock"));
    }
    cx.update(nixa_ui::init);
    let (ws, vcx) = cx.add_window_view(crate::workspace::Workspace::new);
    vcx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(800.)));
    let id = ws.update(vcx, |ws, cx| {
        ws.engine = crate::workspace::EngineState::Up;
        let mut list = crate::spaces::Spaces::default();
        let id = crate::spaces::add(&mut list, "/tmp/shop");
        ws.spaces = list;
        if let Some(v) = super::host::view(cx) {
            v.update(cx, |v, _| v.done = false);
        }
        cx.notify();
        id
    });
    vcx.run_until_parked();
    let shown = |vcx: &mut VisualTestContext| ws.read_with(vcx, super::host::shown);
    assert!(!shown(vcx));
    assert!(ws.read_with(vcx, |ws, _| is_done(ws.store.as_ref())));
    vcx.update(|_, app| super::host::run(Action::Open, app));
    vcx.run_until_parked();
    assert!(shown(vcx));
    assert!(drawn(vcx, "first-run-close"));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!shown(vcx));
    assert!(ws.read_with(vcx, |ws, _| is_done(ws.store.as_ref())));
    vcx.update(|_, app| super::host::run(Action::Open, app));
    ws.update(vcx, |ws, cx| ws.select_space(&id, cx));
    vcx.run_until_parked();
    assert!(!shown(vcx));
    vcx.update(|_, app| super::host::run(Action::Open, app));
    ws.update(vcx, |ws, cx| {
        ws.engine = crate::workspace::EngineState::Up;
        ws.new_tab(Vec::new(), None, cx);
    });
    assert!(!shown(vcx));
}

#[gpui::test]
fn adding_the_first_space_saves_the_done_flag(cx: &mut TestAppContext) {
    let dir = PathBuf::from(format!("/tmp/mo-fd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    unsafe {
        std::env::set_var(nixa_protocol::paths::ENV_DATA_DIR, &dir);
        std::env::set_var(nixa_protocol::paths::ENV_ENGINE_SOCKET, dir.join("e.sock"));
        std::env::set_var(nixa_protocol::control::ENV_APP_SOCKET, dir.join("a.sock"));
    }
    cx.update(nixa_ui::init);
    let (ws, vcx) = cx.add_window_view(crate::workspace::Workspace::new);
    let conn = nixa_store::open(&dir.join("fd.db")).expect("store");
    ws.update(vcx, |ws, cx| {
        ws.store = Some(conn);
        ws.spaces = crate::spaces::Spaces::default();
        if let Some(v) = super::host::view(cx) {
            v.update(cx, |v, _| v.done = false);
        }
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(ws.read_with(vcx, super::host::shown));
    ws.update(vcx, |ws, cx| ws.add_space("/tmp/shop".into(), cx));
    vcx.run_until_parked();
    assert!(!ws.read_with(vcx, super::host::shown));
    let done = vcx.update(|_, app| super::host::view(app).map(|v| v.read(app).done));
    assert_eq!(done, Some(true));
    assert!(ws.read_with(vcx, |ws, _| is_done(ws.store.as_ref())));
    let _ = std::fs::remove_dir_all(&dir);
}
