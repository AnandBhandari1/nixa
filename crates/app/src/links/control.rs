use crate::commands::{self, Cmd};
use crate::workspace::{EngineState, Workspace};
use crate::{layout, panel, tabs, terms};
use gpui::{Context, PromptLevel, Window};
use nixa_protocol::control::{
    self, AppCommand, AppState, ControlReply, ControlRequest, Focus, PanelManifest, Permission,
    SpaceState, TabState, TermState,
};
use nixa_protocol::{TermId, TermInfo, paths};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{self, BufReader};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(25);
const WORK_ARGS: [&str; 3] = ["argv", "task", "cmd"];
const YES: &str = "Run";
const NO: &str = "Cancel";
const ANSWER_WAIT: Duration = Duration::from_secs(10);
const RUN_WAIT: Duration = Duration::from_secs(150);
const IDLE_WAIT: Duration = Duration::from_secs(300);
const NO_FILE: &str = "selected file not exposed";
const SHOW: [&str; 14] = [
    "picker",
    "favourites",
    "commands",
    "menu",
    "files",
    "git",
    "git:diff",
    "custom_agents",
    "models",
    "workflows",
    "tasks",
    "nixa-screen",
    "events",
    "actions",
];
const PANELS: [(&str, &str); 4] = [
    ("files", "Files"),
    ("git", "Git"),
    ("nixa", "Nixa chat"),
    ("menu", "Quick actions menu"),
];
const READ_WORDS: [&str; 22] = [
    "show", "open", "refresh", "search", "find", "next", "previous", "go", "jump", "zoom",
    "view", "list", "type", "sort", "focus", "hide", "history", "branches", "stashes", "blame",
    "graph", "log",
];
const DESTRUCTIVE_WORDS: [&str; 13] = [
    "close", "stop", "discard", "remove", "delete", "kill", "quit", "reset", "push", "forget",
    "drop", "clean", "clear",
];

pub struct Job {
    pub req: ControlRequest,
    pub back: Sender<ControlReply>,
}

#[derive(Debug, Clone, PartialEq)]
enum Action {
    Palette(Cmd),
    NewTab,
    SpaceSelect,
    TabSelect,
    TabClose,
    PaneFocus,
    Show,
    PanelShow,
    PanelHide,
    TitleDoubleClick,
    More(super::control_more::More),
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    info: AppCommand,
    action: Action,
}

pub fn start(window: &mut Window, cx: &mut Context<Workspace>) -> Option<String> {
    let path = control::app_socket_path();
    let listener = match listen(&path) {
        Ok(listener) => listener,
        Err(e) => return Some(format!("The window control socket is off: {e}")),
    };
    let (tx, rx) = channel();
    serve(listener, tx);
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let jobs: Vec<Job> = rx.try_iter().collect();
            for job in jobs {
                let Job { req, back } = job;
                if this
                    .update_in(cx, |ws, window, cx| dispatch(ws, req, back, window, cx))
                    .is_err()
                {
                    return;
                }
            }
        }
    })
    .detach();
    None
}

pub fn listen(path: &Path) -> io::Result<UnixListener> {
    let parent = path.parent().unwrap_or(Path::new("."));
    if parent == paths::data_dir() || !parent.exists() {
        paths::ensure_private_dir(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_socket() => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is not a socket, so Nixa leaves it alone", path.display()),
            ));
        }
        Ok(_) if UnixStream::connect(path).is_ok() => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another Nixa window already answers on {}", path.display()),
            ));
        }
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let hold = parent.join(format!(".app-sock-{}", std::process::id()));
    fs::remove_dir_all(&hold).ok();
    paths::ensure_private_dir(&hold)?;
    let inner = hold.join("s");
    let bound = UnixListener::bind(&inner).and_then(|listener| {
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o600))?;
        fs::rename(&inner, path)?;
        Ok(listener)
    });
    fs::remove_dir_all(&hold).ok();
    bound
}

pub fn serve(listener: UnixListener, tx: Sender<Job>) {
    thread::Builder::new()
        .name("nixa-app-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if !same_user(&stream) {
                    continue;
                }
                let tx = tx.clone();
                thread::spawn(move || talk(stream, &tx));
            }
        })
        .ok();
}

fn error(message: impl Into<String>) -> ControlReply {
    ControlReply::Error {
        message: message.into(),
    }
}

pub fn same_user(stream: &UnixStream) -> bool {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    let ok = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0;
    ok && uid == unsafe { libc::geteuid() }
}

fn talk(stream: UnixStream, tx: &Sender<Job>) {
    stream.set_read_timeout(Some(IDLE_WAIT)).ok();
    stream.set_write_timeout(Some(ANSWER_WAIT)).ok();
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    loop {
        let reply = match control::read_line::<ControlRequest>(&mut reader) {
            Ok(Some(req)) => handle(req, tx),
            Ok(None) => return,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                control::write_line(&mut writer, &error(format!("bad request: {e}"))).ok();
                return;
            }
            Err(_) => return,
        };
        if control::write_line(&mut writer, &reply).is_err() {
            return;
        }
    }
}

pub fn handle(req: ControlRequest, tx: &Sender<Job>) -> ControlReply {
    if req == ControlRequest::Ping {
        return ControlReply::Pong {
            version: control::CONTROL_VERSION,
            pid: std::process::id(),
        };
    }
    if let ControlRequest::Shot { name, capture } = &req {
        return super::shot::reply(name, capture);
    }
    let limit = if matches!(req, ControlRequest::Run { .. }) { RUN_WAIT } else { ANSWER_WAIT };
    let (back, wait) = channel();
    if tx.send(Job { req, back }).is_err() {
        return error("The Nixa window is closed.");
    }
    wait_for(&wait, limit)
}

fn wait_for(wait: &Receiver<ControlReply>, limit: Duration) -> ControlReply {
    wait.recv_timeout(limit)
        .unwrap_or_else(|_| error("The Nixa window did not answer in time. Nothing is known about the result."))
}

fn space_for_new_tab(ws: &mut Workspace, folder: Option<&str>, cx: &mut Context<Workspace>) -> bool {
    if ws.active_space().is_some() {
        return true;
    }
    let folder = folder.map(str::to_string).unwrap_or_else(crate::spaces::home);
    eprintln!("nixa-app: added a space for {folder} to open a new tab");
    ws.add_space(folder, cx);
    ws.active_space().is_some()
}

pub fn dispatch(
    ws: &mut Workspace,
    req: ControlRequest,
    back: Sender<ControlReply>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if let ControlRequest::Run { command, args, cleared, token } = &req
        && let Some(title) = needs_yes(ws, command, args, *cleared, token)
    {
        ask_first(command.clone(), args.clone(), &title, back, window, cx);
        return;
    }
    back.send(answer(ws, req, window, cx)).ok();
}

pub fn starts_work(args: &Value) -> bool {
    args.as_object().is_some_and(|m| WORK_ARGS.iter().any(|k| m.contains_key(*k)))
}

pub fn cleared_by(cleared: bool, token: &str, window_token: Option<&str>) -> bool {
    cleared && !token.is_empty() && window_token.is_some_and(|t| !t.is_empty() && t == token)
}

fn needs_yes(ws: &Workspace, name: &str, args: &Value, cleared: bool, token: &str) -> Option<String> {
    let found = catalog(ws).into_iter().find(|e| e.info.name == name)?;
    let launches = matches!(found.action, Action::Palette(Cmd::Agent(_)));
    if found.info.permission != Permission::Destructive && !starts_work(args) && !launches {
        return None;
    }
    if cleared_by(cleared, token, paths::window_token().as_deref()) {
        return None;
    }
    Some(found.info.title)
}

fn ask_first(name: String, args: Value, title: &str, back: Sender<ControlReply>, window: &mut Window, cx: &mut Context<Workspace>) {
    let shown = if args.is_null() || args.as_object().is_some_and(Map::is_empty) { "no arguments".to_string() } else { args.to_string() };
    let detail = format!("An agent asks the window to run {name} ({title}) with {shown}.");
    let picked = window.prompt(PromptLevel::Warning, &format!("Run {name}?"), Some(&detail), &[YES, NO], cx);
    let asked_at = std::time::Instant::now();
    cx.spawn_in(window, async move |this, cx| {
        if picked.await.ok() != Some(0) {
            back.send(error(format!("The user said no to {name} in the Nixa window. Nothing was run."))).ok();
            return;
        }
        if asked_at.elapsed() >= RUN_WAIT {
            back.send(error(format!("The yes for {name} came after the caller stopped waiting. Nothing was run."))).ok();
            return;
        }
        let reply = this
            .update_in(cx, |ws, window, cx| match run(ws, &name, &args, window, cx) {
                Ok(()) => ControlReply::Ran { command: name.clone(), focus: focus(ws), asked: true },
                Err(message) => error(message),
            })
            .unwrap_or_else(|_| error("The Nixa window is closed."));
        back.send(reply).ok();
    })
    .detach();
}

pub fn answer(
    ws: &mut Workspace,
    req: ControlRequest,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> ControlReply {
    match req {
        ControlRequest::Ping => ControlReply::Pong {
            version: control::CONTROL_VERSION,
            pid: std::process::id(),
        },
        ControlRequest::State => ControlReply::State {
            state: state(ws, cx),
        },
        ControlRequest::Summary => {
            let state = state(ws, cx);
            ControlReply::Summary {
                text: control::summary_text(&state),
                state,
            }
        }
        ControlRequest::Commands => ControlReply::Commands {
            commands: catalog(ws).into_iter().map(|e| e.info).collect(),
        },
        ControlRequest::PanelActions => ControlReply::PanelActions {
            panels: panels(ws),
        },
        ControlRequest::Run { command, args, .. } => match run(ws, &command, &args, window, cx) {
            Ok(()) => ControlReply::Ran {
                command,
                focus: focus(ws),
                asked: false,
            },
            Err(message) => error(message),
        },
        ControlRequest::Focus { terminal } => match focus_term(ws, &terminal, cx) {
            Ok(()) => ControlReply::Ran {
                command: "pane.focus".into(),
                focus: focus(ws),
                asked: false,
            },
            Err(message) => error(message),
        },
        ControlRequest::Shot { name, capture } => super::shot::reply(&name, &capture),
    }
}

pub fn focus(ws: &Workspace) -> Focus {
    let tab = ws.active_tab();
    Focus {
        space: ws.spaces.active.clone(),
        tab: tab.map(|t| t.id.clone()),
        terminal: tab.map(|t| t.focus.clone()),
    }
}

fn engine_word(engine: &EngineState) -> String {
    match engine {
        EngineState::Starting => "starting".into(),
        EngineState::Up => "up".into(),
        EngineState::Down(why) => format!("down: {why}"),
    }
}

fn term_state(ws: &Workspace, id: &TermId, focused: bool) -> TermState {
    let info: Option<&TermInfo> = ws.terms.get(id);
    TermState {
        terminal: id.clone(),
        number: ws.tabs.numbers.get(id).copied(),
        name: info.map(terms::term_name).unwrap_or_else(|| "starting".into()),
        program: info.map(|t| terms::program(&t.cmd)).unwrap_or_default(),
        status: info.and_then(|t| t.status),
        note: info.and_then(|t| t.status_note.clone()),
        running: info.is_some_and(|t| t.running),
        children: info.map(|t| t.children.len()).unwrap_or_default(),
        focused,
    }
}

fn overlay(ws: &Workspace) -> Option<String> {
    let open = [
        (ws.palette.is_some(), "command list"),
        (ws.picker.is_some(), "go to picker"),
        (ws.favourites.is_some(), "favourites"),
        (ws.ctx.is_some(), "right-click menu"),
        (ws.menu.is_some(), "quick actions menu"),
    ];
    open.iter().find(|(on, _)| *on).map(|(_, name)| name.to_string())
}

pub fn state(ws: &Workspace, cx: &Context<Workspace>) -> AppState {
    let now = focus(ws);
    let spaces = ws
        .spaces
        .spaces
        .iter()
        .map(|space| {
            let active_space = ws.spaces.active.as_deref() == Some(space.id.as_str());
            let tabs = ws
                .space_tabs(&space.id)
                .map(|tab| {
                    let active = ws.tabs.active.get(&space.id) == Some(&tab.id);
                    let terminals = layout::leaves(&tab.root)
                        .iter()
                        .map(|id| term_state(ws, id, active_space && active && tab.focus == *id))
                        .collect();
                    TabState {
                        id: tab.id.clone(),
                        name: ws.tab_name(tab),
                        active,
                        terminals,
                    }
                })
                .collect();
            SpaceState {
                id: space.id.clone(),
                name: space.name.clone(),
                folder: space.folder.clone(),
                active: active_space,
                tabs,
            }
        })
        .collect();
    AppState {
        engine: engine_word(&ws.engine),
        focus: now,
        panel: ws.panel.map(|k| panel::title(k).to_string()),
        overlay: overlay(ws),
        spaces,
        loose: ws.loose().iter().map(|t| term_state(ws, &t.id, false)).collect(),
        files_root: ws.files.read(cx).root.clone(),
        git_repo: ws.changes.read(cx).root.clone(),
        unavailable: vec![NO_FILE.into()],
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn group(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Files(_) => "files",
        Cmd::Changes(_) | Cmd::History(_) | Cmd::GitAi(_) | Cmd::Git(_) => "git",
        Cmd::Chat(_) | Cmd::Question(_) => "nixa",
        Cmd::Menu(_) => "menu",
        Cmd::Spaces(_) => "space",
        Cmd::Agents(_) => "agents",
        Cmd::Tasks(_) => "tasks",
        Cmd::Models(_) => "models",
        Cmd::Workflows(_) => "workflows",
        Cmd::CustomAgents(_) => "custom-agents",
        Cmd::Events(_) => "events",
        Cmd::Rail(_) => "rail",
        Cmd::Bar(_) => "tabbar",
        Cmd::Brain(_) => "brain",
        Cmd::Settings(_) => "settings",
        Cmd::Panel(_) => "panel",
        Cmd::Screen(_) | Cmd::NixaScreen => "nixa-screen",
        Cmd::Theme(_) => "theme",
        Cmd::Agent(_) => "agent",
        _ => "app",
    }
}

fn command_name(cmd: &Cmd, label: &str) -> String {
    let group = group(cmd);
    let rest = label.split_once(": ").map(|(_, rest)| rest).unwrap_or(label);
    let full = slug(rest);
    let word = full
        .strip_prefix(group)
        .and_then(|w| w.strip_prefix('-'))
        .unwrap_or(&full)
        .to_string();
    if word.is_empty() || word == group {
        format!("{group}.open")
    } else {
        format!("{group}.{word}")
    }
}

pub fn permission(cmd: &Cmd, label: &str) -> Permission {
    match cmd {
        Cmd::Settings(_)
        | Cmd::Panel(_)
        | Cmd::Picker
        | Cmd::Favourites
        | Cmd::Actions
        | Cmd::Theme(_)
        | Cmd::NixaScreen
        | Cmd::ZoomPane
        | Cmd::ToggleSidebar
        | Cmd::StepTab(_)
        | Cmd::StepSpace(_)
        | Cmd::CustomAgents(_)
        | Cmd::Tasks(_)
        | Cmd::Workflows(_)
        | Cmd::Files(_)
        | Cmd::Menu(crate::screens::menu::Command::Open(_)) => return Permission::Read,
        Cmd::NewTab | Cmd::SplitRight | Cmd::SplitDown
        | Cmd::Rail(crate::screens::rail::RailAction::Commands) => return Permission::Write,
        Cmd::ClosePane | Cmd::CloseTab => return Permission::Destructive,
        _ => {}
    }
    let rest = label.split_once(": ").map(|(_, rest)| rest).unwrap_or(label);
    let lower = rest.to_lowercase();
    let rest = lower
        .strip_prefix(group(cmd))
        .map(str::trim_start)
        .unwrap_or(&lower);
    let words: Vec<String> = rest
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let toggle = words.iter().any(|w| w == "open") && words.iter().any(|w| w == "close");
    if toggle {
        Permission::Read
    } else if words.iter().any(|w| DESTRUCTIVE_WORDS.contains(&w.as_str())) {
        Permission::Destructive
    } else if words.first().is_some_and(|w| READ_WORDS.contains(&w.as_str())) {
        Permission::Read
    } else {
        Permission::Write
    }
}

fn string_prop(about: &str) -> Value {
    json!({ "type": "string", "description": about })
}

fn args(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn entry(name: &str, title: &str, permission: Permission, args: Value, action: Action) -> Entry {
    Entry {
        info: AppCommand {
            name: name.into(),
            title: title.into(),
            permission,
            args,
        },
        action,
    }
}

fn structured() -> Vec<Entry> {
    let show: Vec<&str> = SHOW.to_vec();
    let panels: Vec<&str> = ["files", "git", "nixa"].to_vec();
    vec![
        entry(
            "tab.new",
            "New tab with a terminal. argv runs a program instead of the login shell.",
            Permission::Write,
            args(
                json!({
                    "argv": { "type": "array", "items": { "type": "string" }, "description": "Program and its arguments. Default: the login shell" },
                    "name": string_prop("Tab name"),
                    "folder": string_prop("Folder to start in. Default: the space folder"),
                    "space": string_prop("Space id from the state. Default: the shown space"),
                }),
                &[],
            ),
            Action::NewTab,
        ),
        entry(
            "space.select",
            "Show a space.",
            Permission::Read,
            args(json!({ "id": string_prop("Space id from the state") }), &["id"]),
            Action::SpaceSelect,
        ),
        entry(
            "tab.select",
            "Show a tab.",
            Permission::Read,
            args(json!({ "id": string_prop("Tab id from the state") }), &["id"]),
            Action::TabSelect,
        ),
        entry(
            "tab.close",
            "Close a tab and end its terminals.",
            Permission::Destructive,
            args(json!({ "id": string_prop("Tab id from the state") }), &["id"]),
            Action::TabClose,
        ),
        entry(
            "pane.focus",
            "Show and focus a terminal.",
            Permission::Read,
            args(json!({ "terminal": string_prop("Terminal id, for example m3") }), &["terminal"]),
            Action::PaneFocus,
        ),
        entry(
            "screen.show",
            "Open a window, list or popup of the app.",
            Permission::Read,
            args(json!({ "what": { "anyOf": [{ "type": "string", "enum": show }, { "type": "string", "pattern": "^editor:[^|]+\\|readonly$", "description": "Open an editor window without write access" }] } }), &["what"]),
            Action::Show,
        ),
        entry(
            "panel.show",
            "Show a right panel.",
            Permission::Read,
            args(json!({ "panel": { "type": "string", "enum": panels } }), &["panel"]),
            Action::PanelShow,
        ),
        entry(
            "panel.hide",
            "Hide the right panel.",
            Permission::Read,
            args(json!({}), &[]),
            Action::PanelHide,
        ),
        entry(
            "app.title-double-click",
            "Do what a double-click on the title bar does: zoom or restore the window.",
            Permission::Read,
            args(json!({}), &[]),
            Action::TitleDoubleClick,
        ),
    ]
}

fn catalog(ws: &Workspace) -> Vec<Entry> {
    let mut list = structured();
    for (info, more) in super::control_more::entries() {
        list.push(Entry { info, action: Action::More(more) });
    }
    for item in commands::items(ws) {
        if matches!(
            item.cmd,
            Cmd::Space(_) | Cmd::Tab(_)
        ) {
            continue;
        }
        let base = command_name(&item.cmd, &item.label);
        let mut name = base.clone();
        let mut n = 2;
        while list.iter().any(|e| e.info.name == name) {
            name = format!("{base}-{n}");
            n += 1;
        }
        let title = if item.hint.is_empty() {
            item.label.clone()
        } else {
            format!("{} ({})", item.label, item.hint)
        };
        let permission = permission(&item.cmd, &item.label);
        list.push(entry(&name, &title, permission, args(json!({}), &[]), Action::Palette(item.cmd)));
    }
    list
}

pub fn panels(ws: &Workspace) -> Vec<PanelManifest> {
    let list = catalog(ws);
    PANELS
        .iter()
        .map(|(panel, title)| PanelManifest {
            panel: panel.to_string(),
            title: title.to_string(),
            actions: list
                .iter()
                .filter(|e| matches!(&e.action, Action::Palette(cmd) if group(cmd) == *panel))
                .map(|e| e.info.clone())
                .collect(),
        })
        .collect()
}

fn text_arg(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("Argument \"{key}\" must be a string. Nothing was run.")),
    }
}

fn need(args: &Map<String, Value>, key: &str) -> Result<String, String> {
    text_arg(args, key)?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing required argument \"{key}\". Nothing was run."))
}

fn argv(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    match args.get("argv") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| "argv must be a list of strings. Nothing was run.".to_string()),
        Some(_) => Err("argv must be a list of strings. Nothing was run.".into()),
    }
}

fn check_keys(info: &AppCommand, args: &Map<String, Value>) -> Result<(), String> {
    let known = info.args.get("properties").and_then(Value::as_object);
    match args.keys().find(|k| known.is_none_or(|p| !p.contains_key(*k))) {
        Some(key) => Err(format!("Unknown argument \"{key}\" for {}. Nothing was run.", info.name)),
        None => Ok(()),
    }
}

pub fn run(
    ws: &mut Workspace,
    name: &str,
    given: &Value,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), String> {
    let Some(found) = catalog(ws).into_iter().find(|e| e.info.name == name) else {
        return Err(format!("There is no window command \"{name}\". Ask for the command list. Nothing was run."));
    };
    let empty = Map::new();
    let args = match given {
        Value::Null => &empty,
        Value::Object(map) => map,
        _ => return Err("Arguments must be a JSON object. Nothing was run.".into()),
    };
    check_keys(&found.info, args)?;
    match found.action {
        Action::Palette(cmd) => commands::run(ws, cmd, window, cx),
        Action::NewTab => {
            if ws.engine != EngineState::Up {
                return Err("The terminal engine is not running yet. Nothing was run.".into());
            }
            if let Some(space) = text_arg(args, "space")? {
                if crate::spaces::find(&ws.spaces, &space).is_none() {
                    return Err(format!("There is no space {space}. Nothing was run."));
                }
                ws.select_space(&space, cx);
            }
            let folder = text_arg(args, "folder")?;
            if let Some(folder) = &folder
                && !Path::new(folder).is_dir()
            {
                return Err(format!("{folder} is not a folder. Nothing was run."));
            }
            let run = argv(args)?;
            if !space_for_new_tab(ws, folder.as_deref(), cx) {
                crate::screens::first_run::host::need_space(cx);
                return Err(crate::screens::first_run::NEED_SPACE_ERROR.into());
            }
            ws.new_tab_in(run, text_arg(args, "name")?, folder, cx);
        }
        Action::SpaceSelect => {
            let id = need(args, "id")?;
            if crate::spaces::find(&ws.spaces, &id).is_none() {
                return Err(format!("There is no space {id}. Nothing was run."));
            }
            ws.select_space(&id, cx);
        }
        Action::TabSelect => {
            let id = need(args, "id")?;
            if tabs::find(&ws.tabs, &id).is_none() {
                return Err(format!("There is no tab {id}. Nothing was run."));
            }
            ws.select_tab(&id, cx);
        }
        Action::TabClose => {
            let id = need(args, "id")?;
            if tabs::find(&ws.tabs, &id).is_none() {
                return Err(format!("There is no tab {id}. Nothing was run."));
            }
            ws.close_tab(&id, cx);
        }
        Action::PaneFocus => {
            let text = need(args, "terminal")?;
            let id: TermId = text
                .trim()
                .parse()
                .map_err(|_| format!("{text} is not a terminal id. Nothing was run."))?;
            focus_term(ws, &id, cx)?;
        }
        Action::Show => {
            let what = need(args, "what")?;
            let editor = what.strip_prefix(crate::screens::editor::SHOW)
                .and_then(|value| value.strip_suffix("|readonly"))
                .is_some_and(|path| !path.trim().is_empty() && !path.contains('|'));
            if !SHOW.contains(&what.as_str()) && !editor {
                return Err(format!("{what} is not a screen the window can open. Nothing was run."));
            }
            ws.show(&what, window, cx);
        }
        Action::PanelShow => {
            let word = need(args, "panel")?;
            match panel::from_word(&word) {
                Some(kind) => ws.set_panel(Some(kind), cx),
                None => return Err(format!("{word} is not a panel. Use files, git or nixa. Nothing was run.")),
            }
        }
        Action::PanelHide => ws.set_panel(None, cx),
        Action::TitleDoubleClick => {
            if !crate::screens::tabbar::title_double_click(window) {
                return Err("The window is full screen, so the title bar does nothing. Nothing was run.".into());
            }
        }
        Action::More(more) => super::control_more::run(ws, more, args, cx)?,
    }
    cx.notify();
    Ok(())
}

pub fn focus_term(ws: &mut Workspace, id: &TermId, cx: &mut Context<Workspace>) -> Result<(), String> {
    let placed = |ws: &Workspace| {
        ws.tabs
            .list
            .iter()
            .find(|t| layout::leaves(&t.root).contains(id))
            .map(|t| t.id.clone())
    };
    if placed(ws).is_none() {
        if !ws.terms.contains_key(id) || !ws.loose().iter().any(|t| &t.id == id) {
            return Err(format!("Terminal {id} is not in the Nixa window. Nothing was done."));
        }
        ws.adopt(id.clone(), cx);
    }
    let Some(tab) = placed(ws) else {
        return Err(format!("Terminal {id} could not be shown. Nothing was done."));
    };
    ws.select_tab(&tab, cx);
    ws.focus_pane(&tab, id, cx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::screens::files::Command as FilesCommand;
    use crate::screens::git::changes::Command as GitCommand;

    fn short_dir(tag: &str) -> PathBuf {
        PathBuf::from(format!("/tmp/mac{tag}{}", std::process::id()))
    }

    #[test]
    fn the_socket_takes_only_the_same_user() {
        let (a, b) = UnixStream::pair().unwrap();
        assert!(same_user(&a));
        assert!(same_user(&b));
    }

    #[test]
    fn names_and_permissions_come_from_the_palette() {
        assert_eq!(command_name(&Cmd::Changes(GitCommand::Push), "Git: push"), "git.push");
        assert_eq!(command_name(&Cmd::Files(FilesCommand::Refresh), "Files: refresh the list"), "files.refresh-the-list");
        assert_eq!(command_name(&Cmd::NewTab, "New tab"), "app.new-tab");
        assert_eq!(command_name(&Cmd::Settings("General"), "Settings"), "settings.open");
        assert_eq!(permission(&Cmd::Changes(GitCommand::Push), "Git: push"), Permission::Destructive);
        assert_eq!(permission(&Cmd::Changes(GitCommand::DiscardAll), "Git: discard all changes"), Permission::Destructive);
        assert_eq!(permission(&Cmd::Changes(GitCommand::ShowGraph), "Git: show the graph"), Permission::Read);
        assert_eq!(permission(&Cmd::Changes(GitCommand::Commit), "Git: commit the staged changes"), Permission::Write);
        assert_eq!(permission(&Cmd::ClosePane, "Close pane"), Permission::Destructive);
        assert_eq!(permission(&Cmd::NewTab, "New tab"), Permission::Write);
        assert_eq!(permission(&Cmd::SplitRight, "Split right"), Permission::Write);
        assert_eq!(permission(&Cmd::SplitDown, "Split down"), Permission::Write);
        assert_eq!(permission(&Cmd::Rail(crate::screens::rail::RailAction::Commands), "Open the project command menu"), Permission::Write);
        assert_eq!(permission(&Cmd::Tasks(crate::screens::tasks::TasksCommand::Open), "Tasks"), Permission::Read);
        assert_eq!(permission(&Cmd::Workflows(crate::screens::workflows::WorkflowsCommand::Open), "Workflows"), Permission::Read);
        assert_eq!(permission(&Cmd::CustomAgents(crate::screens::custom_agents::CustomAgentsCommand::Open), "Custom agents"), Permission::Read);
        assert_eq!(structured().into_iter().find(|entry| entry.info.name == "tab.new").unwrap().info.permission, Permission::Write);
        assert_eq!(permission(&Cmd::Theme(1), "Next theme"), Permission::Read);
        assert_eq!(slug("New space in another folder…"), "new-space-in-another-folder");
        assert_eq!(command_name(&Cmd::Files(FilesCommand::Collapse), "Files: close all folders"), "files.close-all-folders");
        assert_eq!(permission(&Cmd::Files(FilesCommand::Collapse), "Files: close all folders"), Permission::Read);
        assert_eq!(permission(&Cmd::Brain(crate::links::procs::Cmd::StopBrain), "Open or close the Files panel"), Permission::Read);
        assert_eq!(permission(&Cmd::Brain(crate::links::procs::Cmd::StopBrain), "Stop the Nixa brain"), Permission::Destructive);
    }

    #[test]
    fn the_socket_is_owner_only_and_a_live_one_is_kept() {
        let dir = short_dir("a");
        fs::remove_dir_all(&dir).ok();
        let path = dir.join("app.sock");
        let first = listen(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let again = listen(&path).unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AddrInUse);
        drop(first);
        let second = listen(&path);
        assert!(second.is_ok());
        let names: Vec<_> = fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_plain_file_is_never_replaced() {
        let dir = short_dir("b");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.sock");
        fs::write(&path, "keep").unwrap();
        assert!(listen(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ping_is_answered_off_the_window_and_jobs_reach_it() {
        let dir = short_dir("c");
        fs::remove_dir_all(&dir).ok();
        let path = dir.join("app.sock");
        let listener = listen(&path).unwrap();
        let (tx, rx) = channel::<Job>();
        serve(listener, tx);
        let window = thread::spawn(move || {
            let job = rx.recv().unwrap();
            assert_eq!(job.req, ControlRequest::Summary);
            job.back.send(ControlReply::Summary { text: "hello".into(), state: AppState::default() }).unwrap();
        });
        let pong = control::call(&path, &ControlRequest::Ping, Duration::from_secs(2)).unwrap();
        assert!(matches!(pong, ControlReply::Pong { pid, .. } if pid == std::process::id()));
        let reply = control::call(&path, &ControlRequest::Summary, Duration::from_secs(2)).unwrap();
        assert!(matches!(reply, ControlReply::Summary { text, .. } if text == "hello"));
        window.join().unwrap();
        let closed = control::call(&path, &ControlRequest::State, Duration::from_secs(2)).unwrap_err();
        assert!(closed.to_string().contains("window is closed"));
        fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::layout::{Node, row};
    use crate::tabs::Tab;
    use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
    use std::path::PathBuf;
    use std::sync::Once;

    fn t(n: u64) -> TermId {
        TermId::from_number(n)
    }

    fn data_dir() -> PathBuf {
        PathBuf::from(format!("/tmp/mo-w89-{}", std::process::id()))
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        static ENV: Once = Once::new();
        ENV.call_once(|| unsafe {
            std::env::set_var(paths::ENV_DATA_DIR, data_dir());
            std::env::set_var(paths::ENV_ENGINE_SOCKET, data_dir().join("e.sock"));
            std::env::set_var(control::ENV_APP_SOCKET, data_dir().join("a.sock"));
        });
        cx.update(nixa_ui::init);
        let (ws, cx) = cx.add_window_view(Workspace::new);
        cx.simulate_resize(size(px(1280.), px(800.)));
        ws.update(cx, |ws, cx| {
            ws.engine = EngineState::Up;
            ws.tabs_path = data_dir().join("w89-tabs.json");
            if ws.spaces.spaces.is_empty() {
                crate::spaces::add(&mut ws.spaces, &crate::spaces::home());
            }
            let space = ws.active_space().map(|s| s.id.clone()).expect("a space");
            for (tab, n) in [("t1", 1), ("t2", 3)] {
                ws.tabs.list.retain(|x| x.id != tab);
                let root = if n == 1 { row(vec![Node::Leaf(t(1)), Node::Leaf(t(2))]) } else { Node::Leaf(t(3)) };
                ws.tabs.list.push(Tab {
                    id: tab.into(),
                    space: space.clone(),
                    root,
                    focus: t(n),
                    zoom: None,
                });
                ws.tabs.numbers.insert(t(n), n as u32);
            }
            ws.tabs.numbers.insert(t(2), 2);
            ws.tabs.active.insert(space, "t1".into());
            cx.notify();
        });
        cx.run_until_parked();
        (ws, cx)
    }

    fn ask(ws: &Entity<Workspace>, cx: &mut VisualTestContext, req: ControlRequest) -> ControlReply {
        let reply = ws.update_in(cx, |ws, window, cx| answer(ws, req, window, cx));
        cx.run_until_parked();
        reply
    }

    #[gpui::test]
    fn state_and_summary_show_spaces_tabs_and_focus(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let ControlReply::State { state } = ask(&ws, cx, ControlRequest::State) else {
            panic!("no state");
        };
        assert_eq!(state.engine, "up");
        assert_eq!(state.focus.tab.as_deref(), Some("t1"));
        assert_eq!(state.focus.terminal, Some(t(1)));
        let space = state.spaces.iter().find(|s| s.active).expect("shown space");
        let tab = space.tabs.iter().find(|x| x.id == "t1").expect("tab t1");
        assert!(tab.active);
        assert_eq!(tab.terminals.len(), 2);
        assert!(tab.terminals[0].focused && !tab.terminals[1].focused);
        assert_eq!(tab.terminals[1].number, Some(2));
        assert_eq!(state.unavailable, vec![NO_FILE.to_string()]);
        let ControlReply::Summary { text, .. } = ask(&ws, cx, ControlRequest::Summary) else {
            panic!("no summary");
        };
        assert!(text.contains("tab shell (id t1) [active]: terminal 1 = m1 starting (focused)"));
    }

    #[gpui::test]
    fn commands_and_panels_are_listed_once(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let ControlReply::Commands { commands } = ask(&ws, cx, ControlRequest::Commands) else {
            panic!("no commands");
        };
        let mut names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        for want in ["tab.new", "pane.focus", "screen.show", "git.push", "files.refresh-the-list", "app.split-right"] {
            assert!(names.contains(&want), "{want} missing");
        }
        assert!(!names.iter().any(|n| n.contains("blueprint")));
        let push = commands.iter().find(|c| c.name == "git.push").unwrap();
        assert_eq!(push.permission, Permission::Destructive);
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count);
        let ControlReply::PanelActions { panels } = ask(&ws, cx, ControlRequest::PanelActions) else {
            panic!("no panels");
        };
        let words: Vec<&str> = panels.iter().map(|p| p.panel.as_str()).collect();
        assert_eq!(words, vec!["files", "git", "nixa", "menu"]);
        assert!(panels.iter().all(|p| !p.actions.is_empty()));
        assert!(panels[1].actions.iter().all(|a| a.name.starts_with("git.")));
    }

    fn send(ws: &Entity<Workspace>, cx: &mut VisualTestContext, req: ControlRequest) -> Receiver<ControlReply> {
        let (back, wait) = channel();
        ws.update_in(cx, |ws, window, cx| dispatch(ws, req, back, window, cx));
        cx.run_until_parked();
        wait
    }

    #[gpui::test]
    fn a_close_asks_the_user_first(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let tabs = |ws: &Entity<Workspace>, cx: &mut VisualTestContext| ws.update(cx, |ws, cx| state(ws, cx).spaces.iter().map(|s| s.tabs.len()).sum::<usize>());
        let before = tabs(&ws, cx);
        let close = ControlRequest::Run { command: "tab.close".into(), args: json!({"id": "t2"}), cleared: false, token: String::new() };
        let wait = send(&ws, cx, close.clone());
        assert!(cx.has_pending_prompt());
        assert!(wait.try_recv().is_err());
        cx.simulate_prompt_answer(NO);
        cx.run_until_parked();
        assert!(matches!(wait.try_recv(), Ok(ControlReply::Error { message }) if message.contains("said no")));
        assert_eq!(tabs(&ws, cx), before);
        let forged = ControlRequest::Run { command: "tab.close".into(), args: json!({"id": "t2"}), cleared: true, token: "not-the-token".into() };
        let wait = send(&ws, cx, forged);
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer(YES);
        cx.run_until_parked();
        assert!(matches!(wait.try_recv(), Ok(ControlReply::Ran { asked: true, .. })));
        let wait = send(&ws, cx, ControlRequest::Run { command: "tab.select".into(), args: json!({"id": "t1"}), cleared: false, token: String::new() });
        assert!(!cx.has_pending_prompt());
        assert!(matches!(wait.try_recv(), Ok(ControlReply::Ran { asked: false, .. })));
    }

    #[test]
    fn only_the_window_token_clears_a_step() {
        assert!(cleared_by(true, "abc", Some("abc")));
        assert!(!cleared_by(false, "abc", Some("abc")));
        assert!(!cleared_by(true, "abd", Some("abc")));
        assert!(!cleared_by(true, "", Some("")));
        assert!(!cleared_by(true, "abc", None));
        assert!(starts_work(&json!({"argv": ["sh"]})));
        assert!(!starts_work(&json!({"name": "x"})));
        assert!(!starts_work(&Value::Null));
    }

    #[gpui::test]
    fn tab_new_with_no_space_adds_one(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        ws.update(cx, |ws, _| ws.store = Some(nixa_store::open_memory().unwrap()));
        ws.update(cx, |ws, cx| {
            ws.spaces = crate::spaces::Spaces::default();
            cx.notify();
        });
        cx.run_until_parked();
        ws.update(cx, |ws, _| ws.engine = EngineState::Up);
        let folder = std::env::temp_dir().display().to_string();
        let wait = send(&ws, cx, ControlRequest::Run { command: "tab.new".into(), args: json!({"argv": ["sh"], "folder": folder}), cleared: false, token: String::new() });
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer(YES);
        cx.run_until_parked();
        let reply = wait.try_recv();
        assert!(!matches!(&reply, Ok(ControlReply::Error { message }) if message.starts_with("Add a space first")), "{reply:?}");
        assert!(ws.read_with(cx, |ws, _| ws.active_space().is_some()), "{reply:?}");
    }

    #[gpui::test]
    fn run_and_focus_move_the_window(cx: &mut TestAppContext) {
        let (ws, cx) = setup(cx);
        let reply = ask(&ws, cx, ControlRequest::Focus { terminal: t(3) });
        let ControlReply::Ran { focus, .. } = reply else {
            panic!("focus failed: {reply:?}");
        };
        assert_eq!(focus.tab.as_deref(), Some("t2"));
        assert_eq!(focus.terminal, Some(t(3)));
        let reply = ask(&ws, cx, ControlRequest::Focus { terminal: t(40) });
        assert!(matches!(reply, ControlReply::Error { message } if message.contains("not in the Nixa window")));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "tab.select".into(), args: json!({"id": "t1"}), cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Ran { focus, .. } if focus.tab.as_deref() == Some("t1")));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "panel.show".into(), args: json!({"panel": "git"}), cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Ran { .. }));
        assert_eq!(ws.read_with(cx, |ws, _| ws.panel), Some(crate::workspace::PanelKind::Git));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "panel.show".into(), args: json!({"panel": "blueprint"}), cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Error { .. }));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "panel.hide".into(), args: json!({"x": 1}), cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Error { message } if message.contains("Unknown argument")));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "app.nope".into(), args: Value::Null, cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Error { .. }));
        let reply = ask(&ws, cx, ControlRequest::Run { command: "app.show-or-hide-the-sidebar".into(), args: Value::Null, cleared: false, token: String::new() });
        assert!(matches!(reply, ControlReply::Ran { .. }), "{reply:?}");
    }
}
