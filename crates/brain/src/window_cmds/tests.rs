use super::*;
use crate::app::lock;
use crate::prompts::book::NewPrompt;
use nixa_protocol::control::{ControlReply, ControlRequest, Focus, read_line, write_line};
use nixa_protocol::id::TermId;
use std::io::BufReader;
use std::os::unix::net::UnixListener;
use tokio::sync::broadcast::Receiver;

fn open() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn drain(rx: &mut Receiver<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(value) = rx.try_recv() {
        out.push(value);
    }
    out
}

fn kinds(events: &[Value]) -> Vec<String> {
    events.iter().filter_map(|e| e["type"].as_str().map(str::to_string)).collect()
}

fn infos(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["type"] == "chat" && e["item"]["who"] == "info")
        .filter_map(|e| e["item"]["text"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn only_the_five_window_commands_are_taken() {
    for kind in KINDS {
        assert!(is_ws(&json!({"type": kind})), "{kind}");
    }
    for kind in ["say", "set_voice", "prompts", "", "show_all"] {
        assert!(!is_ws(&json!({"type": kind})), "{kind}");
    }
    assert!(!is_ws(&json!({"kind": "show"})));
    assert_eq!(request_of(&json!({"request": "w-1"})), Some(json!("w-1")));
    assert_eq!(request_of(&json!({"request": 7})), Some(json!(7)));
    assert_eq!(request_of(&json!({"request": " "})), None);
    assert_eq!(request_of(&json!({})), None);
}

#[tokio::test]
async fn set_watch_saves_the_timer_and_answers() {
    let (_dir, app) = open();
    let mut rx = app.out.subscribe();
    let reply = ws_request(&app, &json!({"type": "set_watch", "minutes": 5, "request": "w-1"})).await.unwrap();
    assert_eq!(reply, json!({"type": "set_watch_reply", "request": "w-1", "ok": true, "data": {"minutes": 5, "changed": true}}));
    assert_eq!(app.watch_minutes(), 5);
    assert_eq!(app.settings().watch_minutes, Some(5));
    let events = drain(&mut rx);
    assert_eq!(infos(&events), vec!["Watch the agents: every 5 minutes.".to_string()]);
    let snap = events.iter().find(|e| e["type"] == "snapshot").expect("snapshot event");
    assert_eq!(snap["snapshot"]["watchMinutes"], 5);
    let same = ws_request(&app, &json!({"type": "set_watch", "minutes": 5, "request": "w-2"})).await.unwrap();
    assert_eq!(same["data"]["changed"], false);
    assert!(infos(&drain(&mut rx)).is_empty());
    let off = ws_request(&app, &json!({"type": "set_watch", "minutes": 0, "request": 3})).await.unwrap();
    assert_eq!((off["request"].clone(), off["ok"].clone()), (json!(3), json!(true)));
    assert_eq!(app.watch_minutes(), 0);
    assert_eq!(infos(&drain(&mut rx)), vec!["Watch the agents: timer off.".to_string()]);
    for bad in [json!(241), json!(-1), json!(2.5), json!("5"), Value::Null] {
        let reply = ws_request(&app, &json!({"type": "set_watch", "minutes": bad, "request": "w-4"})).await.unwrap();
        assert_eq!(reply["ok"], false, "{bad}");
        assert!(reply["error"].as_str().unwrap().contains("0 to 240"), "{reply}");
    }
    assert_eq!(app.watch_minutes(), 0);
    assert!(ws_request(&app, &json!({"type": "set_watch", "minutes": 9})).await.is_none());
    assert_eq!(app.watch_minutes(), 9);
}

#[tokio::test]
async fn watch_dry_run_walks_the_saved_or_the_tested_tree() {
    let (_dir, app) = open();
    let mut rx = app.out.subscribe();
    let reply = ws_request(&app, &json!({"type": "watch_dry_run", "request": "d-1"})).await.unwrap();
    assert_eq!(reply["type"], "watch_dry_run_reply");
    assert_eq!(reply["ok"], true, "{reply}");
    let text = reply["data"]["text"].as_str().unwrap().to_string();
    assert!(!text.is_empty());
    assert_eq!(reply["data"]["tested"], false);
    assert_eq!(infos(&drain(&mut rx)), vec![text.clone()]);
    let tree = serde_json::to_value(lock(&app.trees).read(crate::trees::WATCH_TREE).unwrap()).unwrap();
    let tested = ws_request(&app, &json!({"type": "watch_dry_run", "tree": tree, "request": "d-2"})).await.unwrap();
    assert_eq!(tested["data"]["tested"], true, "{tested}");
    let lines = infos(&drain(&mut rx));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].ends_with("(Tested a changed tree that is not saved.)"), "{}", lines[0]);
    let bad = ws_request(&app, &json!({"type": "watch_dry_run", "tree": {"nope": 1}, "request": "d-3"})).await.unwrap();
    assert_eq!(bad["ok"], false);
    assert!(bad["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(infos(&drain(&mut rx)).is_empty());
}

#[tokio::test]
async fn automation_runs_a_schedule_command() {
    let (_dir, app) = open();
    let mut rx = app.out.subscribe();
    let on = ws_request(&app, &json!({"type": "automation", "request": "a-1", "action": "scheduler.set", "payload": {"enabled": true}})).await.unwrap();
    assert_eq!(on["type"], "automation_reply");
    assert_eq!(on["ok"], true, "{on}");
    assert_eq!(on["data"]["enabled"], true);
    assert!(on["data"]["schedules"].is_array());
    assert!(kinds(&drain(&mut rx)).contains(&"schedules".to_string()));
    assert!(crate::schedules::jobs::all_on(&app));
    let off = ws_request(&app, &json!({"type": "automation", "request": "a-2", "action": "scheduler.set", "payload": {"enabled": false}})).await.unwrap();
    assert_eq!(off["data"]["enabled"], false);
    let unknown = ws_request(&app, &json!({"type": "automation", "request": "a-3", "action": "nope", "payload": {}})).await.unwrap();
    assert_eq!((unknown["ok"].clone(), unknown["error"].clone()), (json!(false), json!("Unknown action nope.")));
    let gone = ws_request(&app, &json!({"type": "automation", "request": "a-4", "action": "schedule.run", "payload": {"id": "missing"}})).await.unwrap();
    assert_eq!(gone["ok"], false);
    let no_action = ws_request(&app, &json!({"type": "automation", "request": "a-5", "payload": {}})).await.unwrap();
    assert!(no_action["error"].as_str().unwrap().contains("needs an action"));
    let bad_payload = ws_request(&app, &json!({"type": "automation", "request": "a-6", "action": "scheduler.set", "payload": [1]})).await.unwrap();
    assert!(bad_payload["error"].as_str().unwrap().contains("payload"));
}

fn fake_window(socket: std::path::PathBuf) -> std::thread::JoinHandle<Option<ControlRequest>> {
    let listener = UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().ok()?;
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let asked: Option<ControlRequest> = read_line(&mut reader).ok()?;
        let mut writer = stream;
        let focus = Focus { terminal: Some(TermId::from_number(3)), ..Focus::default() };
        write_line(&mut writer, &ControlReply::Ran { command: "focus".into(), focus, asked: false }).ok()?;
        asked
    })
}

#[tokio::test]
async fn show_focuses_the_terminal_of_an_open_question() {
    let (dir, app) = open();
    let mut rx = app.out.subscribe();
    let event = NewPrompt { kind: "permission".into(), terminal: "m3".into(), agent: "claude".into(), text: Some("Run tests?".into()), ..NewPrompt::default() };
    let id = lock(&app.prompts.book).add(event, crate::util::now_ms()).unwrap().id;
    let socket = dir.path().join("a.sock");
    let window = fake_window(socket.clone());
    let shown = show_on(&app, &json!({"type": "show", "id": id.to_uppercase()}), socket.clone()).await.unwrap();
    assert_eq!(shown, json!({"id": id, "terminal": "m3", "shown": true}));
    assert_eq!(window.join().unwrap(), Some(ControlRequest::Focus { terminal: TermId::from_number(3) }));
    assert!(infos(&drain(&mut rx)).is_empty());
    std::fs::remove_file(&socket).unwrap();
    let closed = show_on(&app, &json!({"type": "show", "id": id}), socket).await.unwrap_err();
    assert!(closed.starts_with("Could not show claude's terminal."), "{closed}");
    assert_eq!(infos(&drain(&mut rx)), vec!["Could not show claude's terminal.".to_string()]);
    let gone = ws_request(&app, &json!({"type": "show", "id": "zzzz", "request": "s-1"})).await.unwrap();
    assert_eq!(gone, json!({"type": "show_reply", "request": "s-1", "ok": false, "error": ANSWERED}));
    assert_eq!(infos(&drain(&mut rx)), vec![ANSWERED.to_string()]);
    let empty = ws_request(&app, &json!({"type": "show", "request": "s-2"})).await.unwrap();
    assert!(empty["error"].as_str().unwrap().contains("needs the id"));
    assert_eq!(agent_word(" "), "the agent");
}

#[tokio::test]
async fn snapshot_sends_the_whole_view_to_every_window() {
    let (_dir, app) = open();
    let mut rx = app.out.subscribe();
    let reply = ws_request(&app, &json!({"type": "snapshot", "request": 11})).await.unwrap();
    assert_eq!((reply["type"].clone(), reply["request"].clone(), reply["ok"].clone()), (json!("snapshot_reply"), json!(11), json!(true)));
    let data = &reply["data"];
    for key in ["name", "busy", "items", "voice", "talk", "mobile", "state", "nixa", "privacy", "work", "watchMinutes", "phone", "prompts"] {
        assert!(data.get(key).is_some(), "missing {key} in {data}");
    }
    assert_eq!(data["watchMinutes"], crate::app::WATCH_DEFAULT_MINUTES);
    assert!(data["prompts"].get("type").is_none());
    let events = drain(&mut rx);
    assert_eq!(kinds(&events), ["snapshot", "state", "work", "prompts"]);
    assert_eq!(events[0]["snapshot"], *data);
    assert!(ws_request(&app, &json!({"type": "snapshot"})).await.is_none());
    assert_eq!(kinds(&drain(&mut rx)).len(), 4);
}

#[test]
fn timer_lines() {
    assert_eq!(timer_line(0), "Watch the agents: timer off.");
    assert_eq!(timer_line(1), "Watch the agents: every minute.");
    assert_eq!(timer_line(30), "Watch the agents: every 30 minutes.");
    assert_eq!(minutes_of(&json!({"minutes": MAX_MINUTES})), Ok(MAX_MINUTES));
}
