use super::run::{self, UNTRUSTED};
use crate::app::{App, lock};
use crate::tools::Args;
use nixa_protocol::control::{self, AppCommand, AppState, ControlReply, ControlRequest, Focus, Permission, SpaceState, TermState};
use serde_json::{Value, json};
use std::io::BufReader;
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

type Seen = Arc<Mutex<Vec<ControlRequest>>>;

fn cmd(name: &str, permission: Permission, args: Value) -> AppCommand {
    AppCommand { name: name.into(), title: name.into(), permission, args }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn commands() -> Vec<AppCommand> {
    vec![
        cmd("panel.show", Permission::Read, object(json!({"panel": {"enum": ["files", "git", "nixa"]}}), &["panel"])),
        cmd("space.select", Permission::Read, object(json!({"id": {"type": "string"}}), &["id"])),
        cmd("tab.new", Permission::Read, object(json!({"argv": {"type": "array"}, "name": {"type": "string"}}), &[])),
        cmd("app.layout-save", Permission::Write, object(json!({}), &[])),
        cmd("tab.close", Permission::Destructive, object(json!({"id": {"type": "string"}}), &["id"])),
        cmd("git.commit", Permission::Write, object(json!({}), &[])),
    ]
}

fn window() -> AppState {
    let space = |id: &str, name: &str| SpaceState { id: id.into(), name: name.into(), folder: format!("/work/{name}"), active: id == "s1", tabs: Vec::new() };
    AppState {
        engine: "up".into(),
        focus: Focus { space: Some("s1".into()), tab: None, terminal: None },
        spaces: vec![space("s1", "shop"), space("s7", "notes")],
        loose: vec![serde_json::from_value::<TermState>(json!({
            "terminal": "shop-agent-a1b2c3", "number": 7, "name": "worker",
            "program": "sh", "running": true, "children": 0, "focused": false
        })).unwrap()],
        ..AppState::default()
    }
}

fn answer(req: ControlRequest) -> ControlReply {
    match req {
        ControlRequest::Commands => ControlReply::Commands { commands: commands() },
        ControlRequest::State => ControlReply::State { state: window() },
        ControlRequest::Focus { terminal } => ControlReply::Ran {
            command: "pane.focus".into(), focus: Focus { terminal: Some(terminal), ..window().focus }, asked: false
        },
        ControlRequest::Run { command, .. } if command == "tab.close" => {
            ControlReply::Error { message: "The user said no in the Nixa window. Nothing was run.".into() }
        }
        ControlRequest::Run { command, cleared, .. } => {
            let asked = !cleared && (commands().iter().any(|c| c.name == command && c.permission != Permission::Read) || command == "tab.new");
            ControlReply::Ran { command, focus: window().focus, asked }
        }
        _ => ControlReply::Error { message: "not in this fake".into() },
    }
}

fn fake_window(dir: &std::path::Path) -> Seen {
    let path = control::app_socket_in(dir);
    std::fs::remove_file(&path).ok();
    let listener = UnixListener::bind(&path).unwrap();
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut writer = conn;
            while let Ok(Some(req)) = control::read_line::<ControlRequest>(&mut reader) {
                log.lock().unwrap().push(req.clone());
                control::write_line(&mut writer, &answer(req)).unwrap();
            }
        }
    });
    seen
}

fn setup() -> (tempfile::TempDir, Arc<App>, Seen) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    let seen = fake_window(&app.dir);
    (dir, app, seen)
}

fn runs(seen: &Seen) -> Vec<ControlRequest> {
    seen.lock().unwrap().iter().filter(|r| matches!(r, ControlRequest::Run { .. })).cloned().collect()
}

fn args(value: Value) -> Args {
    value.as_object().unwrap().clone()
}

async fn run(app: &Arc<App>, value: Value) -> Result<String, String> {
    run::call(app, &args(value)).await
}

#[test]
fn fields_must_be_command_and_an_arguments_object() {
    assert_eq!(run::wanted(&args(json!({"command": " panel.show "}))).unwrap().command, "panel.show");
    assert!(run::wanted(&args(json!({"command": "panel.show", "argv": ["sh"]}))).unwrap_err().contains("Unknown field"));
    assert!(run::wanted(&args(json!({"command": ""}))).is_err());
    assert!(run::wanted(&args(json!({"command": "panel.show", "arguments": "files"}))).unwrap_err().contains("JSON object"));
    let read = cmd("tab.new", Permission::Read, json!({}));
    assert!(run::read_only(&read, &args(json!({"name": "x"}))));
    assert!(!run::read_only(&read, &args(json!({"argv": ["claude"]}))));
    assert!(!run::read_only(&cmd("tab.close", Permission::Destructive, json!({})), &args(json!({}))));
}

#[tokio::test]
async fn names_and_arguments_are_checked_before_anything_runs() {
    let (_dir, app, seen) = setup();
    let wrong_case = run(&app, json!({"command": "Panel.Show", "arguments": {"panel": "files"}})).await.unwrap_err();
    assert!(wrong_case.contains("no Nixa window command"));
    assert!(run(&app, json!({"command": "panel.show", "arguments": {"panel": "blueprint"}})).await.unwrap_err().contains("not an allowed value"));
    assert!(run(&app, json!({"command": "panel.show", "arguments": {"panel": "git", "argv": ["sh"]}})).await.unwrap_err().contains("Unknown argument"));
    assert!(run(&app, json!({"command": "tab.close"})).await.unwrap_err().contains("Missing required argument"));
    assert!(runs(&seen).is_empty());
}

#[tokio::test]
async fn read_commands_run_at_once_even_when_work_is_held() {
    let (_dir, app, seen) = setup();
    lock(&app.switches.work).hold = Some(crate::settings_api::work::Hold { at: 1, alias: "hold".into() });
    let done = run(&app, json!({"command": "panel.show", "arguments": {"panel": "git"}})).await.unwrap();
    assert!(done.starts_with(UNTRUSTED));
    let body: Value = serde_json::from_str(done.lines().nth(1).unwrap()).unwrap();
    assert_eq!(body["outcome"], "ran");
    assert_eq!(body["askedUser"], false);
    run(&app, json!({"command": "space.select", "arguments": {"id": "2"}})).await.unwrap();
    assert_eq!(runs(&seen)[1], ControlRequest::Run { command: "space.select".into(), args: json!({"id": "s7"}), cleared: false, token: String::new() });
}

#[tokio::test]
async fn write_runs_with_no_ask_after_the_brain_checks() {
    let (_dir, app, seen) = setup();
    let done = run(&app, json!({"command": "app.layout-save"})).await.unwrap();
    assert!(done.contains("\"askedUser\":false"), "{done}");
    assert!(done.contains("\"outcome\":\"ran\""));
    let sent = runs(&seen);
    assert_eq!(sent.len(), 1);
    let ControlRequest::Run { cleared, token, .. } = &sent[0] else { panic!("not a run") };
    assert!(*cleared);
    assert!(!token.is_empty());
    assert_eq!(token, &app.window_token);
    let no = run(&app, json!({"command": "tab.close", "arguments": {"id": "t1"}})).await.unwrap_err();
    assert!(no.contains("said no"));
    assert_eq!(runs(&seen).len(), 2);
}

#[tokio::test]
async fn read_commands_are_not_marked_cleared() {
    let (_dir, app, seen) = setup();
    run(&app, json!({"command": "panel.show", "arguments": {"panel": "files"}})).await.unwrap();
    let sent = runs(&seen);
    let ControlRequest::Run { cleared, token, .. } = &sent[0] else { panic!("not a run") };
    assert!(!*cleared);
    assert!(token.is_empty());
}

#[tokio::test]
async fn the_hold_keyword_stops_only_new_work() {
    let (_dir, app, seen) = setup();
    let ran = run(&app, json!({"command": "tab.new", "arguments": {"argv": ["claude"]}})).await.unwrap();
    assert!(ran.contains("\"outcome\":\"ran\""), "{ran}");
    assert!(run(&app, json!({"command": "app.layout-save"})).await.unwrap().contains("\"outcome\":\"ran\""));
    lock(&app.switches.work).hold = Some(crate::settings_api::work::Hold { at: crate::util::now_ms(), alias: "hold".into() });
    assert!(run(&app, json!({"command": "app.layout-save"})).await.unwrap().contains("\"outcome\":\"ran\""));
    let held = run(&app, json!({"command": "tab.new", "arguments": {"argv": ["claude"]}})).await.unwrap();
    assert!(held.starts_with("Hold is on"), "{held}");
    assert!(!held.contains("Held for you"));
    assert_eq!(runs(&seen).len(), 3);
}

#[tokio::test]
async fn both_chat_paths_reach_the_same_runner() {
    let (_dir, app, seen) = setup();
    assert!(crate::tools::specs(&app).iter().any(|s| s.name == "nixa_app_run"));
    let listed = crate::cli_brains::door::tool_list(&app);
    assert!(listed.as_array().unwrap().iter().any(|t| t["name"] == "nixa_app_run"));
    let call = crate::turns::ToolCall { id: "t1".into(), name: "nixa_app_run".into(), args: args(json!({"command": "panel.show", "arguments": {"panel": "files"}})) };
    let (text, error) = crate::tools::call(&app, &call).await;
    assert!(!error, "{text}");
    let (text, error) = crate::cli_brains::door::run_tool(&app, "nixa_app_run", &json!({"command": "panel.show", "arguments": {"panel": "nixa"}})).await;
    assert!(!error, "{text}");
    assert_eq!(runs(&seen).len(), 2);
}

#[tokio::test]
async fn focus_uses_the_display_number_or_id_without_running_a_command() {
    let (_dir, app, seen) = setup();
    let id = "shop-agent-a1b2c3";
    for (target, expected) in [
        (json!(7), true), (json!(id), true), (json!(8), false),
    ] {
        let (text, error) = crate::cli_brains::door::run_tool(
            &app, "nixa_app_focus", &json!({"terminal": target}),
        ).await;
        assert_eq!(!error, expected, "{text}");
        if expected {
            assert!(text.contains(id), "{text}");
        } else {
            assert!(text.contains("There is no terminal 8"), "{text}");
        }
    }
    let focus = ControlRequest::Focus { terminal: id.parse().unwrap() };
    assert_eq!(*seen.lock().unwrap(), [ControlRequest::State, focus.clone(), focus, ControlRequest::State]);
}
