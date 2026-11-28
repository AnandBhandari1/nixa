use super::check::{Flags, Step, app_scoped, prepare};
use super::gate::{self, Ask};
use super::vision::{Scope, brain_fits, pairs, resolve, scope_of, server_model};
use super::{BROWSER, EYES, HANDS, Seen};
use crate::app::{App, lock};
use crate::brains::SavedBrain;
use crate::provider::{Endpoint, Kind};
use crate::tools::Args;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap()
}

fn open() -> Flags {
    Flags { may_read: true }
}

fn app_in(dir: &Path) -> Arc<App> {
    App::open(&dir.join("data")).unwrap()
}

fn said(app: &App, line: &str, at: u64) {
    lock(&app.brain).last_user = Some((line.to_string(), at));
}

fn tiny_png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    out
}

fn tiny_jpeg() -> Vec<u8> {
    vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 50, 0, 100, 3, 0, 0, 0]
}

fn observed(app: &str, title: &str) -> Seen {
    Seen {
        at: crate::util::now_ms(),
        value: json!({
            "target": {"id": 5, "pid": 100, "app": app, "title": title, "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}, "signature": "abc"},
            "text": "Shop list",
            "elements": [
                {"id": "0.1", "role": "AXButton", "label": "Save", "enabled": true},
                {"id": "0.2", "role": "AXTextField", "label": "Name", "enabled": true, "value": ""},
                {"id": "0.3", "role": "AXSecureTextField", "label": "Password", "enabled": true},
                {"id": "0.4", "role": "AXButton", "label": "Off", "enabled": false},
            ],
        }),
    }
}

fn seen_with(key: &str, seen: Seen) -> HashMap<String, Seen> {
    HashMap::from([(key.to_string(), seen)])
}

#[test]
fn prepare_keeps_to_the_bounded_steps() {
    let none = HashMap::new();
    let now = crate::util::now_ms();
    assert!(prepare(EYES, &args(json!({"action": "windows"})), open(), &none, now).is_ok());
    assert!(prepare(EYES, &args(json!({"action": "windows", "cmd": "ls"})), open(), &none, now).is_err());
    assert!(prepare(EYES, &args(json!({"action": "click"})), open(), &none, now).is_err());
    assert!(prepare(HANDS, &args(json!({"action": "read", "window": 3})), open(), &none, now).is_err());
    assert!(prepare(EYES, &args(json!({"action": "read", "window": 0})), open(), &none, now).is_err());
    assert!(prepare(EYES, &args(json!({"action": "read", "window": "everything"})), open(), &none, now).is_err());
    assert!(prepare(EYES, &args(json!({"action": "read", "window": 7, "offset": 601})), open(), &none, now).is_err());
    let read = prepare(EYES, &args(json!({"action": "read", "window": "screen"})), open(), &none, now).unwrap();
    assert_eq!(read.offset, Some(0));
    let private = Flags { may_read: false };
    assert!(prepare(EYES, &args(json!({"action": "windows"})), private, &none, now).unwrap_err().contains("Privacy"));
    let vision = |v: Value| prepare(EYES, &args(v), open(), &none, now);
    assert!(vision(json!({"action": "vision", "name": "Tasks"})).unwrap_err().contains("question"));
    assert!(vision(json!({"action": "vision", "question": "What?"})).is_err());
    assert!(vision(json!({"action": "vision", "question": "What?", "name": "Tasks", "capture": "00000000-0000-0000-0000-000000000000"})).is_err());
    assert!(vision(json!({"action": "vision", "question": "What?", "capture": "/tmp/x.png"})).unwrap_err().contains("File paths"));
    assert!(vision(json!({"action": "vision", "question": "What?", "capture": "0123abcd-0000-4000-8000-0123456789ab"})).is_ok());
}

#[test]
fn hands_check_the_observation_and_the_control() {
    let now = crate::util::now_ms();
    let fine = seen_with("o1", observed("Notes", "Shop list"));
    let click = |el: &str, seen: &HashMap<String, Seen>, flags: Flags| prepare(HANDS, &args(json!({"action": "click", "observation": "o1", "element": el})), flags, seen, now);
    let step = click("0.1", &fine, open()).unwrap();
    assert_eq!(step.element.unwrap()["label"], "Save");
    assert_eq!(step.target.unwrap()["signature"], "abc");
    assert!(click("0.3", &fine, open()).is_ok());
    assert!(click("0.4", &fine, open()).is_err());
    assert!(click("9.9", &fine, open()).is_err());
    assert!(click("0.1", &seen_with("o1", observed("Terminal", "zsh")), open()).is_ok());
    let mut old = observed("Notes", "Shop list");
    old.at = now - super::SEEN_MS - 1;
    assert!(click("0.1", &seen_with("o1", old), open()).unwrap_err().contains("too old"));
    let typed = |el: &str, text: &str| prepare(HANDS, &args(json!({"action": "type", "observation": "o1", "element": el, "text": text})), open(), &fine, now);
    assert_eq!(typed("0.2", "milk and bread").unwrap().text.as_deref(), Some("milk and bread"));
    assert!(typed("0.1", "milk").unwrap_err().contains("text fields"));
    assert!(typed("0.2", "line one\nline two").is_ok());
    assert!(typed("0.2", &"a".repeat(2001)).is_err());
}

#[test]
fn browser_tool_validates_urls_and_browser_windows() {
    let none = HashMap::new();
    let now = crate::util::now_ms();
    let ok = prepare(BROWSER, &args(json!({"action": "open", "url": "https://example.com/docs"})), open(), &none, now).unwrap();
    assert_eq!(ok.action, "open");
    assert_eq!(ok.url.as_deref(), Some("https://example.com/docs"));
    assert!(prepare(BROWSER, &args(json!({"action": "open", "url": "file:///etc/passwd"})), open(), &none, now).is_err());
    assert!(prepare(BROWSER, &args(json!({"action": "open", "url": "https://user:pass@example.com"})), open(), &none, now).is_err());
    assert!(prepare(BROWSER, &args(json!({"action": "read", "window": "screen"})), open(), &none, now).is_err());
    assert!(prepare(BROWSER, &args(json!({"action": "read", "window": 9})), open(), &none, now).is_ok());
    let safari = seen_with("o1", observed("Safari", "Example Domain"));
    assert!(prepare(BROWSER, &args(json!({"action": "click", "observation": "o1", "element": "0.1"})), open(), &safari, now).is_ok());
    let scope = scope_of(&ok, &json!({"target": {"id": 42, "title": "Example Domain"}}));
    assert_eq!(scope.kind, "window");
    assert_eq!(scope.window, Some(json!(42)));
    assert_eq!(scope.name.as_deref(), Some("Example Domain"));
}

#[test]
fn only_nixa_window_reads_skip_the_card() {
    let step = |v: Value| -> Step { serde_json::from_value(v).unwrap() };
    assert!(app_scoped(&step(json!({"tool": EYES, "action": "shot", "name": "Tasks"}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "shot", "name": "  "}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "shot"}))));
    assert!(app_scoped(&step(json!({"tool": EYES, "action": "read", "window": "nixa"}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "read", "window": "screen"}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "read", "window": 4}))));
    assert!(app_scoped(&step(json!({"tool": EYES, "action": "vision", "name": "main", "question": "q"}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "vision", "capture": "x", "question": "q"}))));
    assert!(!app_scoped(&step(json!({"tool": EYES, "action": "windows"}))));
    assert!(gate::hard(&step(json!({"tool": HANDS, "action": "click", "element": {"label": "Deploy to production"}}))));
    assert!(gate::hard(&step(json!({"tool": HANDS, "action": "click", "element": {"label": "Buy now"}}))));
    assert!(!gate::hard(&step(json!({"tool": HANDS, "action": "click", "element": {"label": "Save"}}))));
}

#[test]
fn a_card_needs_a_fresh_yes_and_the_yes_is_used_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let step: Step = serde_json::from_value(json!({"tool": EYES, "action": "windows"})).unwrap();
    let message = |id: &str| gate::message(id, &step, None, false);
    said(&app, "yes list the windows", crate::util::now_ms() - 10);
    let first = gate::ask(&app, "k1", &step, false, message);
    let Ask::Held(reply) = first else { panic!("not held: {first:?}") };
    assert_eq!(reply["outcome"], "held");
    assert!(reply["note"].as_str().unwrap().contains("yes windows"));
    let id = reply["id"].as_str().unwrap().to_string();
    assert!(matches!(gate::ask(&app, "k1", &step, false, message), Ask::Held(_)));
    said(&app, "can you list the windows?", crate::util::now_ms() + 1);
    assert!(matches!(gate::ask(&app, "k1", &step, false, message), Ask::Held(_)));
    said(&app, "yes windows", crate::util::now_ms() + 2);
    assert_eq!(gate::ask(&app, "k1", &step, false, message), Ask::Go(id.clone()));
    let again = gate::ask(&app, "k1", &step, false, message);
    let Ask::Held(reply) = again else { panic!("the yes was used twice") };
    assert_ne!(reply["id"].as_str().unwrap(), id);
    let other = gate::ask(&app, "k2", &step, false, message);
    assert!(matches!(other, Ask::Held(_)));
    let log = std::fs::read_to_string(app.desktop.dir.join(gate::STEPS_FILE)).unwrap();
    assert!(log.contains("A newer desktop step replaced this card"));
}

#[test]
fn an_off_topic_yes_does_not_run_a_held_click() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let step: Step = serde_json::from_value(json!({"tool": HANDS, "action": "click", "observation": "o1", "element": {"id": "0.1", "label": "Save"}})).unwrap();
    let message = |id: &str| gate::message(id, &step, None, false);
    let Ask::Held(reply) = gate::ask(&app, "c", &step, false, message) else { panic!() };
    let id = reply["id"].as_str().unwrap().to_string();
    for (n, line) in ["yes", "go ahead", "ok do it", "yes, change the code", "yes please", "sure"].iter().enumerate() {
        said(&app, line, crate::util::now_ms() + 1 + n as u64);
        assert!(matches!(gate::ask(&app, "c", &step, false, message), Ask::Held(_)), "{line} ran the click");
    }
    said(&app, "no, don't click", crate::util::now_ms() + 20);
    assert!(matches!(gate::ask(&app, "c", &step, false, message), Ask::Held(_)));
    said(&app, "yes, click it", crate::util::now_ms() + 30);
    assert_eq!(gate::ask(&app, "c", &step, false, message), Ask::Go(id));
    assert!(gate::names_step("yes desktop", "type"));
    assert!(gate::names_step("ok fill it", "type"));
    assert!(!gate::names_step("type it later", "type"));
}

#[tokio::test]
async fn a_hard_stop_needs_the_card_not_chat() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let step: Step = serde_json::from_value(json!({"tool": HANDS, "action": "click", "element": {"label": "Deploy to production"}})).unwrap();
    let message = |id: &str| gate::message(id, &step, None, true);
    let Ask::Held(reply) = gate::ask(&app, "h", &step, true, message) else { panic!() };
    assert!(reply["note"].as_str().unwrap().contains("hard stop"));
    let id = reply["id"].as_str().unwrap().to_string();
    said(&app, "yes click deploy", crate::util::now_ms() + 1);
    assert!(matches!(gate::ask(&app, "h", &step, true, message), Ask::Held(_)));
    gate::answer(&app, &id, true).unwrap();
    assert_eq!(gate::ask(&app, "h", &step, true, message), Ask::Go(id));
}

#[tokio::test]
async fn the_panel_answers_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let step: Step = serde_json::from_value(json!({"tool": EYES, "action": "read", "window": "screen"})).unwrap();
    let message = |id: &str| gate::message(id, &step, None, false);
    let Ask::Held(reply) = gate::ask(&app, "k", &step, false, message) else { panic!() };
    let id = reply["id"].as_str().unwrap().to_string();
    assert!(gate::answer(&app, "someone-else", true).is_err());
    gate::answer(&app, &id, true).unwrap();
    assert_eq!(gate::ask(&app, "k", &step, false, message), Ask::Go(id));
    let Ask::Held(reply) = gate::ask(&app, "k", &step, false, message) else { panic!() };
    let id = reply["id"].as_str().unwrap().to_string();
    gate::answer(&app, &id, false).unwrap();
    assert!(lock(&app.desktop.held).is_none());
    assert!(gate::answer(&app, &id, true).is_err());
    let view = super::routes::view(&app);
    assert!(view["card"].is_null());
    assert!(view["recent"].as_array().unwrap().iter().any(|r| r["phase"] == "cancelled"));
}

#[test]
fn the_log_is_private_and_the_mirror_is_short() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    for n in 0..40 {
        gate::audit(&app.desktop, json!({"id": format!("row{n}"), "phase": "requested"})).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&app.desktop.dir), 0o700);
    assert_eq!(mode(&app.desktop.dir.join(gate::STEPS_FILE)), 0o600);
    assert_eq!(mode(&app.desktop.dir.join(gate::RECENT_FILE)), 0o600);
    assert_eq!(std::fs::read_to_string(app.desktop.dir.join(gate::STEPS_FILE)).unwrap().lines().count(), 40);
    assert_eq!(lock(&app.desktop.recent).len(), gate::RECENT_MAX);
    assert_eq!(gate::load_recent(&app.desktop.dir).len(), gate::RECENT_MAX);
    assert!(!gate::permission(&app.desktop).allow_reads);
    gate::save_permission(&app.desktop, true).unwrap();
    assert!(gate::permission(&app.desktop).allow_reads);
    assert_eq!(gate::iso_time(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(gate::iso_time(86_400_000 + 3_723_004), "1970-01-02T01:02:03.004Z");
    let id = gate::new_id();
    assert!(super::check::is_capture_id(&id), "{id}");
}

#[tokio::test]
async fn captures_resolve_only_from_our_own_log() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let state = &app.desktop;
    nixa_protocol::paths::ensure_private_dir(&state.dir).unwrap();
    assert!(resolve(state, "/etc/passwd").unwrap_err().contains("not a capture id"));
    let id = "0123abcd-0000-4000-8000-0123456789ab";
    assert!(resolve(state, id).unwrap_err().contains("No capture"));
    let image = state.dir.join(format!("{id}.png"));
    std::fs::write(&image, tiny_png(30, 20)).unwrap();
    let step = json!({"tool": EYES, "action": "shot", "name": "Tasks"});
    gate::audit(state, json!({"id": id, "phase": "result", "step": step, "result": {"image": image, "window": "Tasks", "id": 9, "capturedAt": "2026-01-02T03:04:05.000Z"}})).unwrap();
    let found = resolve(state, id).unwrap();
    assert_eq!((found.width, found.height), (30, 20));
    assert_eq!(found.scope.kind, "nixa-window");
    assert_eq!(found.captured_at, "2026-01-02T03:04:05.000Z");
    let screen_id = "0123abcd-0000-4000-8000-0123456789ac";
    std::fs::write(state.dir.join(format!("{screen_id}.png")), tiny_png(5, 5)).unwrap();
    gate::audit(state, json!({"id": screen_id, "phase": "result", "step": {"tool": EYES, "action": "read", "window": "screen"}, "result": {"image": state.dir.join(format!("{screen_id}.png"))}})).unwrap();
    assert!(resolve(state, screen_id).is_ok());
    let fake = "0123abcd-0000-4000-8000-0123456789ad";
    std::fs::write(state.dir.join(format!("{fake}.png")), tiny_png(5, 5)).unwrap();
    gate::audit(state, json!({"id": fake, "phase": "result", "step": {"tool": HANDS, "action": "click"}, "result": {"image": state.dir.join(format!("{fake}.png"))}})).unwrap();
    assert!(resolve(state, fake).is_err());
    let read: Step = serde_json::from_value(json!({"tool": EYES, "action": "read", "window": 12})).unwrap();
    assert_eq!(scope_of(&read, &json!({"target": {"title": "Shop"}})), Scope { kind: "window".into(), window: Some(json!(12)), name: Some("Shop".into()) });
}

#[test]
fn results_show_a_page_of_live_controls() {
    let elements: Vec<Value> = (0..30).map(|n| json!({"id": format!("0.{n}"), "role": if n % 2 == 0 { "AXButton" } else { "AXStaticText" }, "label": "x".repeat(200), "enabled": true})).collect();
    let shown = super::format::result(&json!({"text": "t".repeat(5000), "elements": elements, "offset": 0}));
    assert_eq!(shown["totalControls"], 15);
    assert_eq!(shown["elements"].as_array().unwrap().len(), 15);
    assert!(shown.get("nextOffset").is_none());
    assert_eq!(shown["text"].as_str().unwrap().len(), super::format::TEXT_MAX);
    assert_eq!(shown["elements"][0]["label"].as_str().unwrap().len(), super::format::LABEL_MAX);
    let many: Vec<Value> = (0..45).map(|n| json!({"id": format!("0.{n}"), "role": "AXLink", "label": "l", "enabled": true})).collect();
    let page = super::format::result(&json!({"elements": many, "offset": 20}));
    assert_eq!(page["elements"][0]["id"], "0.20");
    assert_eq!(page["nextOffset"], 40);
    assert_eq!(super::format::result(&json!("plain")), json!("plain"));
}

fn brain(kind: Kind, url: &str) -> SavedBrain {
    SavedBrain { id: "abcd1234".into(), name: "mine".into(), kind, base_url: url.into(), model: String::new(), effort: None, key_env: None, added_at: 0 }
}

#[test]
fn picture_models_fit_their_servers() {
    assert!(brain_fits("claude-sonnet-4-5", &brain(Kind::Anthropic, "https://api.anthropic.com")));
    assert!(!brain_fits("gpt-5", &brain(Kind::Anthropic, "https://api.anthropic.com")));
    assert!(brain_fits("gpt-5", &brain(Kind::Openai, "https://api.openai.com/v1")));
    assert!(brain_fits("o3", &brain(Kind::Openai, "https://api.openai.com/v1")));
    assert!(!brain_fits("claude-opus-4", &brain(Kind::Openai, "https://api.openai.com/v1")));
    assert!(brain_fits("anything", &brain(Kind::Openai, "http://localhost:11434/v1")));
    let listed = vec!["gpt-5-2026-01-01".to_string(), "gpt-5-mini".to_string(), "gpt-4o".to_string()];
    assert_eq!(server_model(&listed, "gpt-4o").as_deref(), Some("gpt-4o"));
    assert_eq!(server_model(&listed, "gpt-5").as_deref(), Some("gpt-5-2026-01-01"));
    assert_eq!(server_model(&listed, "gemini"), None);
    let row = nixa_store::models::ModelRow {
        id: "claude::claude-sonnet-4-5".into(),
        agent: "claude".into(),
        model: "claude-sonnet-4-5".into(),
        efforts: Vec::new(),
        rank: 1,
        listed: true,
        details: nixa_store::models::ModelDetails::default(),
        decision: String::new(),
        note: String::new(),
    };
    let brains = vec![brain(Kind::Anthropic, "https://api.anthropic.com"), brain(Kind::Anthropic, "http://127.0.0.1:8080")];
    assert_eq!(pairs(std::slice::from_ref(&row), &brains, false).len(), 2);
    assert_eq!(pairs(std::slice::from_ref(&row), &brains, true).len(), 1);
}

#[test]
fn picture_requests_have_the_right_shape() {
    let picture = nixa_desktop::picture::check_image(&tiny_png(2, 2)).unwrap();
    let anthropic = Endpoint { kind: Kind::Anthropic, base_url: "https://api.anthropic.com".into(), api_key: String::new() };
    let (url, body) = super::look::request_of(&anthropic, "claude-x", "sys", "What?", std::slice::from_ref(&picture));
    assert_eq!(url, "https://api.anthropic.com/v1/messages");
    assert_eq!(body["messages"][0]["content"][0]["source"]["media_type"], "image/png");
    assert_eq!(body["system"], "sys");
    let openai = Endpoint { kind: Kind::Openai, base_url: "http://localhost:1234/v1/".into(), api_key: String::new() };
    let (url, body) = super::look::request_of(&openai, "gpt-x", "sys", "What?", std::slice::from_ref(&picture));
    assert_eq!(url, "http://localhost:1234/v1/chat/completions");
    assert!(body["messages"][1]["content"][1]["image_url"]["url"].as_str().unwrap().starts_with("data:image/png;base64,"));
    assert_eq!(super::look::answer_of(Kind::Anthropic, &json!({"content": [{"type": "text", "text": "A list"}]})), "A list");
    assert_eq!(super::look::answer_of(Kind::Openai, &json!({"choices": [{"message": {"content": "Two rows"}}]})), "Two rows");
}

#[tokio::test]
async fn tools_read_the_screen_without_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    crate::settings_api::set_privacy(&app, false).unwrap();
    let (out, failed) = crate::tools::call(&app, &crate::turns::ToolCall { id: "c1".into(), name: EYES.into(), args: args(json!({"action": "windows"})) }).await;
    assert!(!failed, "{out}");
    let seen: Value = serde_json::from_str(&out).unwrap();
    assert_ne!(seen["outcome"], "held", "{out}");
    let (out, failed) = crate::tools::call(&app, &crate::turns::ToolCall { id: "c2".into(), name: HANDS.into(), args: args(json!({"action": "click", "observation": "none", "element": "0.1"})) }).await;
    assert!(failed && out.contains("observation"), "{out}");
    let log = std::fs::read_to_string(app.desktop.dir.join(gate::STEPS_FILE)).unwrap_or_default();
    assert!(!log.contains("\"phase\":\"held\""), "{log}");
}
#[test]
fn a_declutter_vision_needs_no_question_and_keeps_its_rubric() {
    let none = HashMap::new();
    let now = crate::util::now_ms();
    let vision = |v: Value| prepare(EYES, &args(v), open(), &none, now);
    let step = vision(json!({"action": "vision", "name": "Tasks", "rubric": "declutter"})).unwrap();
    assert_eq!(step.rubric.as_deref(), Some("declutter"));
    assert_eq!(step.question.as_deref(), Some(super::declutter::QUESTION));
    let step = vision(json!({"action": "vision", "name": "main", "rubric": "declutter", "question": "Is the top bar too busy?"})).unwrap();
    assert_eq!(step.question.as_deref(), Some("Is the top bar too busy?"));
    assert!(vision(json!({"action": "vision", "name": "main", "rubric": "pretty"})).unwrap_err().contains("rubric must be one of: declutter"));
    assert!(vision(json!({"action": "vision", "name": "main", "question": "What?"})).unwrap().rubric.is_none());
    assert!(prepare(EYES, &args(json!({"action": "shot", "name": "main", "rubric": "declutter"})), open(), &none, now).is_err());
    let saved: Step = serde_json::from_value(json!({"tool": EYES, "action": "vision", "name": "main", "question": "q"})).unwrap();
    assert!(saved.rubric.is_none());
}

#[test]
fn own_window_pictures_are_checked_and_moved_in() {
    use nixa_protocol::control::ControlReply;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let capture = "0123abcd-0000-4000-8000-0123456789ab";
    let image = dir.path().join("desktop").join(format!("{capture}.png"));
    let made = dir.path().join("shots").join(format!("{capture}.png"));
    let reply = ControlReply::Shot { image: made.to_string_lossy().into_owned(), window: "Nixa".into(), id: 7, bytes: 40 };
    let (from, out) = super::own::shot_value(reply, capture, &image, "sidebar").unwrap();
    assert_eq!(from, made);
    assert_eq!(out["own"], true);
    assert_eq!(out["window"], "Nixa");
    assert!(out["note"].as_str().unwrap().contains("inside the main window"));
    let wrong = ControlReply::Shot { image: dir.path().join("other.png").to_string_lossy().into_owned(), window: "Nixa".into(), id: 7, bytes: 40 };
    assert!(super::own::shot_value(wrong, capture, &image, "main").unwrap_err().contains("wrong name"));
    let refused = ControlReply::Error { message: "No open Nixa window is named \"x\".".into() };
    assert!(super::own::shot_value(refused, capture, &image, "x").unwrap_err().contains("No open Nixa window"));
    std::fs::create_dir_all(made.parent().unwrap()).unwrap();
    std::fs::create_dir_all(image.parent().unwrap()).unwrap();
    std::fs::write(&made, tiny_png(4, 3)).unwrap();
    super::own::take_file(&made, &image).unwrap();
    assert!(!made.exists());
    let info = nixa_desktop::picture::png_info(&image).unwrap();
    assert_eq!((info.width, info.height), (4, 3));
    assert_eq!(std::fs::metadata(&image).unwrap().permissions().mode() & 0o777, 0o600);
}

async fn vision_fail_then_ok(body: axum::Json<Value>) -> Result<axum::Json<Value>, (axum::http::StatusCode, axum::Json<Value>)> {
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let images = body
        .pointer("/messages/1/content")
        .and_then(Value::as_array)
        .map(|parts| parts.iter().filter(|p| p.get("type").and_then(Value::as_str) == Some("image_url")).count())
        .unwrap_or(0);
    if images > 1 {
        return Err((axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({"error": {"message": "too many"}}))));
    }
    Ok(axum::Json(json!({"choices": [{"message": {"content": "A person sits."}}]})))
}

#[tokio::test]
async fn a_model_error_on_three_frames_retries_with_the_middle_frame() {
    use super::vision::{ONE_FRAME_NOTE, describe_frames};
    use crate::brains::Checked;
    use crate::provider::Kind;
    use axum::routing::{get, post};
    use nixa_store::models::{self, By, ModelEdit, model_rows};
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    let conn = crate::models::open(&app).unwrap();
    models::update(&conn, |cache| {
        models::pick::ensure_map(cache, crate::util::now_ms() as i64, &models::lists::fixed_models())?;
        Ok(())
    })
    .unwrap();
    let id = model_rows(&models::load(&conn).unwrap())
        .into_iter()
        .find(|r| r.listed)
        .map(|r| r.id)
        .expect("a listed model");
    models::update(&conn, |cache| {
        models::pick::edit_model(
            cache,
            &id,
            &ModelEdit { capabilities: Some(vec!["vision".into()]), pins: Some(vec!["vision".into()]), ..ModelEdit::default() },
            By::User,
        )?;
        Ok(())
    })
    .unwrap();
    let model = model_rows(&models::load(&conn).unwrap()).into_iter().find(|r| r.id == id).map(|r| r.model).expect("vision row");
    let listed = super::vision::vision_rows(&models::load(&conn).unwrap())
        .into_iter()
        .map(|r| r.model)
        .collect::<Vec<_>>();
    let tags = listed.clone();
    let router = axum::Router::new()
        .route(
            "/api/tags",
            get(move || {
                let tags = tags.clone();
                async move { axum::Json(json!({"models": tags.iter().map(|name| json!({"name": name, "capabilities": ["vision"]})).collect::<Vec<_>>()})) }
            }),
        )
        .route(
            "/v1/models",
            get(move || {
                let listed = listed.clone();
                async move {
                    axum::Json(json!({
                        "data": listed.iter().map(|id| json!({"id": id})).collect::<Vec<_>>()
                    }))
                }
            }),
        )
        .route("/v1/chat/completions", post(vision_fail_then_ok));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    app.brains
        .add(Checked { name: "Vision".into(), kind: Kind::Ollama, base_url: base, key_env: None, model })
        .unwrap();
    let jpeg = tiny_jpeg();
    nixa_desktop::picture::check_image(&jpeg).expect("test jpeg");
    let files: Vec<_> = (0..3)
        .map(|i| {
            let path = dir.path().join(format!("frame-{i}.jpg"));
            std::fs::write(&path, &jpeg).unwrap();
            path
        })
        .collect();
    let answer = describe_frames(&app, &files, "What moves?", false).await.unwrap();
    assert_eq!(answer.frames_sent, 1);
    assert!(answer.text.contains(ONE_FRAME_NOTE));
}

#[test]
fn safari_open_takes_only_public_urls() {
    let flags = Flags { may_read: true };
    let seen = HashMap::new();
    let open = |url: &str| prepare(BROWSER, json!({"action": "open", "url": url}).as_object().unwrap(), flags, &seen, 0);
    for bad in ["http://127.0.0.1:8080", "http://127.0.0.1:7421/settings", "http://192.168.1.20/", "http://localhost/", "https://example.com:8443/"] {
        assert!(open(bad).is_err(), "{bad}");
    }
    assert!(open("https://example.com/docs").is_ok());
}

#[test]
fn safari_follows_browser_on_and_privacy_for_every_brain() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    assert_eq!(super::browser_allowed(&app).unwrap_err(), crate::browser::OFF_TEXT);
    crate::browser::set(&app, &json!({"on": true})).unwrap();
    crate::settings_api::set_privacy(&app, true).unwrap();
    assert!(super::browser_allowed(&app).unwrap_err().contains("Private and confidential"));
    crate::settings_api::set_privacy(&app, false).unwrap();
    assert!(super::browser_allowed(&app).is_ok());
}

#[tokio::test]
async fn a_hands_click_waits_for_its_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_in(dir.path());
    crate::settings_api::set_privacy(&app, false).unwrap();
    lock(&app.desktop.seen).insert("o1".into(), observed("TextEdit", "Notes"));
    let click = args(json!({"action": "click", "observation": "o1", "element": "0.1"}));
    let (out, failed) = crate::tools::call(&app, &crate::turns::ToolCall { id: "c1".into(), name: HANDS.into(), args: click }).await;
    assert!(!failed, "{out}");
    let held: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(held["outcome"], "held", "{out}");
    let log = std::fs::read_to_string(app.desktop.dir.join(gate::STEPS_FILE)).unwrap_or_default();
    assert!(log.contains("\"phase\":\"held\"") && !log.contains("\"phase\":\"allowed\""), "{log}");
}
