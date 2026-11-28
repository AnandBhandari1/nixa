use crate::state::State;
use crate::tools::{Tool, parse, schema};
use anyhow::{Result, anyhow, bail};
use nixa_protocol::control::{self, AppCommand, AppState, ControlReply, ControlRequest};
use nixa_protocol::control_args::{check_args, resolve_args, resolve_terminal};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CALL_WAIT: Duration = Duration::from_secs(10);
const RUN_WAIT: Duration = Duration::from_secs(160);
const TERMINAL_HELP: &str = "Terminal number (for example 3) or id (for example m3) from nixa_app_summary";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    command: String,
    #[serde(default)]
    arguments: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FocusArgs {
    terminal: Value,
}

fn tool(name: &'static str, description: &str, input_schema: Value, worker: bool) -> Tool {
    Tool {
        name,
        description: description.to_string(),
        input_schema,
        worker,
    }
}

pub fn tools() -> Vec<Tool> {
    vec![
        tool(
            "nixa_app_summary",
            "Short summary of the Nixa window: the focused space, tab and terminal, every space with its tabs and terminals (number, id, name, status), the open panel and what the window does not expose. Use this first for most questions about the window.",
            schema(json!({}), &[]),
            true,
        ),
        tool(
            "nixa_app_state",
            "The Nixa window state as JSON: spaces, tabs, terminals with numbers and status, focus, open panel and overlay, the Files folder and the Git repo. Use this before you pick a target.",
            schema(json!({}), &[]),
            true,
        ),
        tool(
            "nixa_app_commands",
            "List every Nixa window command with its exact name, title, permission (read, write or destructive) and arguments. Use this when unsure how to do something in the window.",
            schema(json!({}), &[]),
            true,
        ),
        tool(
            "nixa_app_panel_actions",
            "Commands of the window panels (Files, Git, Nixa chat, menu), grouped by panel, with permission and arguments. Run one with nixa_app_run.",
            schema(json!({}), &[]),
            true,
        ),
        tool(
            "nixa_app_run",
            "Run a Nixa window command by its exact name from nixa_app_commands, with a JSON object of arguments. Read commands (open, show, select, switch) run at once. Every command runs at once. Never invent command names.",
            schema(
                json!({
                    "command": { "type": "string", "description": "Exact command name from nixa_app_commands" },
                    "arguments": { "type": "object", "description": "Command arguments", "additionalProperties": true },
                }),
                &["command"],
            ),
            false,
        ),
        tool(
            "nixa_app_focus",
            "Show and focus an existing terminal in the Nixa window: it selects its space and tab and moves the keyboard focus there.",
            schema(
                json!({ "terminal": { "type": ["string", "integer"], "description": TERMINAL_HELP } }),
                &["terminal"],
            ),
            false,
        ),
    ]
}

pub fn run(state: &State, name: &str, args: &Value) -> Option<Result<Value>> {
    let result = match name {
        "nixa_app_summary" => parse::<NoArgs>(args).and_then(|_| summary(state)),
        "nixa_app_state" => parse::<NoArgs>(args).and_then(|_| app_state(state).map(|s| json!(s))),
        "nixa_app_commands" => parse::<NoArgs>(args).and_then(|_| commands(state).map(|c| json!({ "commands": c }))),
        "nixa_app_panel_actions" => parse::<NoArgs>(args).and_then(|_| panel_actions(state)),
        "nixa_app_run" => parse::<RunArgs>(args).and_then(|a| run_command(state, a)),
        "nixa_app_focus" => parse::<FocusArgs>(args).and_then(|a| focus(state, &a.terminal)),
        _ => return None,
    };
    Some(result)
}

pub fn socket(state: &State) -> PathBuf {
    control::app_socket_in(&state.data_dir)
}

fn ask(state: &State, req: &ControlRequest) -> Result<ControlReply> {
    control::call(&socket(state), req, CALL_WAIT)
}

fn unexpected(reply: ControlReply) -> anyhow::Error {
    anyhow!("the Nixa window gave an unexpected reply: {reply:?}")
}

fn summary(state: &State) -> Result<Value> {
    match ask(state, &ControlRequest::Summary)? {
        ControlReply::Summary { text, .. } => Ok(Value::String(text)),
        other => Err(unexpected(other)),
    }
}

pub fn app_state(state: &State) -> Result<AppState> {
    app_state_at(&socket(state))
}

fn app_state_at(path: &Path) -> Result<AppState> {
    match control::call(path, &ControlRequest::State, CALL_WAIT)? {
        ControlReply::State { state } => Ok(state),
        other => Err(unexpected(other)),
    }
}

fn commands(state: &State) -> Result<Vec<AppCommand>> {
    match ask(state, &ControlRequest::Commands)? {
        ControlReply::Commands { commands } => Ok(commands),
        other => Err(unexpected(other)),
    }
}

fn panel_actions(state: &State) -> Result<Value> {
    match ask(state, &ControlRequest::PanelActions)? {
        ControlReply::PanelActions { panels } => Ok(json!({ "panels": panels })),
        other => Err(unexpected(other)),
    }
}

fn execute(socket: &Path, command: &str, args: &Map<String, Value>) -> Result<Value> {
    let req = ControlRequest::Run {
        command: command.to_string(),
        args: Value::Object(args.clone()),
        cleared: false,
        token: String::new(),
    };
    match control::call(socket, &req, RUN_WAIT)? {
        ControlReply::Ran { command, focus, asked } => Ok(json!({
            "outcome": "ran",
            "command": command,
            "arguments": args,
            "askedUser": asked,
            "focus": focus,
            "note": "The window ran the command. Read nixa_app_summary to see the result; do not assume more than that.",
        })),
        other => Err(unexpected(other)),
    }
}

fn run_command(state: &State, given: RunArgs) -> Result<Value> {
    let name = given.command.trim().to_string();
    let list = commands(state)?;
    let Some(command) = list.iter().find(|c| c.name == name) else {
        bail!("There is no Nixa window command \"{name}\". Use nixa_app_commands or nixa_app_panel_actions for the exact names. Nothing was run.");
    };
    let app = app_state(state)?;
    let mut args = given.arguments;
    resolve_args(&app, &name, &mut args).map_err(anyhow::Error::msg)?;
    check_args(&command.args, &args).map_err(anyhow::Error::msg)?;
    execute(&socket(state), &name, &args)
}

fn focus(state: &State, terminal: &Value) -> Result<Value> {
    let app = app_state(state)?;
    let id = resolve_terminal(&app, terminal).map_err(anyhow::Error::msg)?;
    match ask(state, &ControlRequest::Focus { terminal: id.clone() })? {
        ControlReply::Ran { focus, .. } => {
            let outcome = if focus.terminal.as_ref() == Some(&id) { "focused" } else { "unknown" };
            Ok(json!({ "outcome": outcome, "terminal": id, "focus": focus }))
        }
        other => Err(unexpected(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Mode;
    use nixa_protocol::{Status, TermId};
    use nixa_protocol::control::{Focus, Permission, SpaceState, TabState, TermState};
    use std::io::BufReader;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn term(n: u64) -> TermState {
        TermState {
            terminal: TermId::from_number(n),
            number: Some(n as u32),
            name: format!("agent {n}"),
            program: "sh".into(),
            status: Some(Status::Idle),
            note: None,
            running: true,
            children: 0,
            focused: n == 1,
        }
    }

    fn app() -> AppState {
        AppState {
            engine: "up".into(),
            focus: Focus {
                space: Some("s1".into()),
                tab: Some("t1".into()),
                terminal: Some(TermId::from_number(1)),
            },
            spaces: vec![
                SpaceState {
                    id: "s1".into(),
                    name: "shop".into(),
                    folder: "/Users/someone/shop".into(),
                    active: true,
                    tabs: vec![TabState {
                        id: "t1".into(),
                        name: "agent 1".into(),
                        active: true,
                        terminals: vec![term(1), term(2)],
                    }],
                },
                SpaceState {
                    id: "s7".into(),
                    name: "notes".into(),
                    folder: "/Users/someone/notes".into(),
                    active: false,
                    tabs: Vec::new(),
                },
            ],
            ..AppState::default()
        }
    }

    fn cmd(name: &str, permission: Permission, args: Value) -> AppCommand {
        AppCommand {
            name: name.into(),
            title: name.into(),
            permission,
            args,
        }
    }

    struct Fake {
        dir: PathBuf,
        seen: Arc<Mutex<Vec<ControlRequest>>>,
    }

    fn fake(tag: &str) -> Fake {
        let dir = PathBuf::from(format!("/tmp/mw{tag}{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(control::APP_SOCKET_FILE);
        std::fs::remove_file(&path).ok();
        let listener = UnixListener::bind(&path).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let focus = Arc::new(Mutex::new(app().focus));
        let (s, f) = (seen.clone(), focus.clone());
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { return };
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut writer = conn;
                while let Ok(Some(req)) = control::read_line::<ControlRequest>(&mut reader) {
                    s.lock().unwrap().push(req.clone());
                    let reply = answer(req, &f);
                    control::write_line(&mut writer, &reply).unwrap();
                }
            }
        });
        Fake { dir, seen }
    }

    fn answer(req: ControlRequest, focus: &Mutex<Focus>) -> ControlReply {
        let mut state = app();
        state.focus = focus.lock().unwrap().clone();
        match req {
            ControlRequest::State => ControlReply::State { state },
            ControlRequest::Summary => ControlReply::Summary { text: control::summary_text(&state), state },
            ControlRequest::Commands => ControlReply::Commands {
                commands: vec![
                    cmd("app.next-theme", Permission::Read, schema(json!({}), &[])),
                    cmd("tab.new", Permission::Read, json!({"type": "object", "properties": {"argv": {"type": "array"}, "name": {"type": "string"}}, "additionalProperties": false})),
                    cmd("tab.close", Permission::Destructive, schema(json!({"id": {"type": "string"}}), &["id"])),
                    cmd("space.select", Permission::Read, schema(json!({"id": {"type": "string"}}), &["id"])),
                ],
            },
            ControlRequest::Run { command, .. } if command == "tab.close" => ControlReply::Error {
                message: "The user said no in the Nixa window. Nothing was run.".into(),
            },
            ControlRequest::Run { command, .. } => ControlReply::Ran {
                asked: command == "tab.new",
                command,
                focus: state.focus,
            },
            ControlRequest::Focus { terminal } => {
                let mut f = focus.lock().unwrap();
                f.terminal = Some(terminal);
                ControlReply::Ran { command: "pane.focus".into(), focus: f.clone(), asked: false }
            }
            ControlRequest::PanelActions => ControlReply::PanelActions { panels: Vec::new() },
            ControlRequest::Ping => ControlReply::Pong { version: 1, pid: 1 },
            ControlRequest::Shot { .. } => ControlReply::Error { message: "No picture in this test window.".into() },
        }
    }

    fn state_for(fake: &Fake) -> State {
        State::new(Mode::Full, PathBuf::from("/nonexistent/e.sock"), fake.dir.clone(), None)
    }

    fn runs(fake: &Fake) -> Vec<ControlRequest> {
        fake.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, ControlRequest::Run { .. } | ControlRequest::Focus { .. }))
            .cloned()
            .collect()
    }

    #[test]
    fn every_run_goes_to_the_window_which_owns_the_yes() {
        let fake = fake("a");
        let state = state_for(&fake);
        let done = run_command(&state, RunArgs { command: "app.next-theme".into(), arguments: Map::new() }).unwrap();
        assert_eq!(done["outcome"], "ran");
        assert_eq!(done["askedUser"], false);
        let mut args = Map::new();
        args.insert("argv".into(), json!(["sh", "-c", "echo hi"]));
        let done = run_command(&state, RunArgs { command: "tab.new".into(), arguments: args }).unwrap();
        assert_eq!(done["askedUser"], true);
        assert!(done.get("confirmationRequired").is_none());
        let mut args = Map::new();
        args.insert("id".into(), json!("t1"));
        let err = run_command(&state, RunArgs { command: "tab.close".into(), arguments: args }).unwrap_err();
        assert!(err.to_string().contains("said no"));
        assert_eq!(runs(&fake).len(), 3);
        let err = run_command(&state, RunArgs { command: "app.nope".into(), arguments: Map::new() }).unwrap_err();
        assert!(err.to_string().contains("no Nixa window command"));
        assert_eq!(runs(&fake).len(), 3);
        std::fs::remove_dir_all(&fake.dir).ok();
    }

    #[test]
    fn space_numbers_become_ids_and_focus_uses_numbers() {
        let fake = fake("c");
        let state = state_for(&fake);
        let mut args = Map::new();
        args.insert("id".into(), json!("2"));
        run_command(&state, RunArgs { command: "space.select".into(), arguments: args }).unwrap();
        let done = focus(&state, &json!(2)).unwrap();
        assert_eq!(done["outcome"], "focused");
        assert_eq!(done["terminal"], "m2");
        let sent = runs(&fake);
        assert_eq!(sent[0], ControlRequest::Run { command: "space.select".into(), args: json!({"id": "s7"}), cleared: false, token: String::new() });
        assert_eq!(sent[1], ControlRequest::Focus { terminal: TermId::from_number(2) });
        let text = summary(&state).unwrap();
        assert!(text.as_str().unwrap().contains("Space 2 notes"));
        std::fs::remove_dir_all(&fake.dir).ok();
    }

    #[test]
    fn no_window_gives_a_clear_error() {
        let state = State::new(Mode::Full, PathBuf::from("/nonexistent/e.sock"), PathBuf::from("/nonexistent/mw"), None);
        let err = app_state(&state).unwrap_err();
        assert!(format!("{err:#}").contains("Is nixa-app running?"));
    }
}
