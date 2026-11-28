use crate::id::TermId;
use crate::paths;
use crate::term::Status;
use anyhow::{Context, Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const APP_SOCKET_FILE: &str = "app.sock";
pub const ENV_APP_SOCKET: &str = "NIXA_APP_SOCKET";
pub const MAX_CONTROL_LINE: usize = 4 * 1024 * 1024;
pub const CONTROL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    Ping,
    State,
    Summary,
    Commands,
    PanelActions,
    Run {
        command: String,
        #[serde(default)]
        args: Value,
        #[serde(default)]
        cleared: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        token: String,
    },
    Focus {
        terminal: TermId,
    },
    Shot {
        #[serde(default)]
        name: String,
        capture: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum ControlReply {
    Pong { version: u32, pid: u32 },
    State { state: AppState },
    Summary { text: String, state: AppState },
    Commands { commands: Vec<AppCommand> },
    PanelActions { panels: Vec<PanelManifest> },
    Ran {
        command: String,
        focus: Focus,
        #[serde(default)]
        asked: bool,
    },
    Shot {
        image: String,
        window: String,
        id: i64,
        bytes: u64,
    },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Read,
    Write,
    Destructive,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
            Permission::Destructive => "destructive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Focus {
    #[serde(default)]
    pub space: Option<String>,
    #[serde(default)]
    pub tab: Option<String>,
    #[serde(default)]
    pub terminal: Option<TermId>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppState {
    pub engine: String,
    pub focus: Focus,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub overlay: Option<String>,
    #[serde(default)]
    pub spaces: Vec<SpaceState>,
    #[serde(default)]
    pub loose: Vec<TermState>,
    #[serde(default)]
    pub files_root: Option<String>,
    #[serde(default)]
    pub git_repo: Option<String>,
    #[serde(default)]
    pub unavailable: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SpaceState {
    pub id: String,
    pub name: String,
    pub folder: String,
    pub active: bool,
    #[serde(default)]
    pub tabs: Vec<TabState>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TabState {
    pub id: String,
    pub name: String,
    pub active: bool,
    #[serde(default)]
    pub terminals: Vec<TermState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermState {
    pub terminal: TermId,
    #[serde(default)]
    pub number: Option<u32>,
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub note: Option<String>,
    pub running: bool,
    #[serde(default)]
    pub children: usize,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppCommand {
    pub name: String,
    pub title: String,
    pub permission: Permission,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelManifest {
    pub panel: String,
    pub title: String,
    pub actions: Vec<AppCommand>,
}

pub fn app_socket_in(data_dir: &Path) -> PathBuf {
    std::env::var_os(ENV_APP_SOCKET)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join(APP_SOCKET_FILE))
}

pub fn app_socket_path() -> PathBuf {
    app_socket_in(&paths::data_dir())
}

pub fn write_line<T: Serialize>(w: &mut dyn Write, msg: &T) -> io::Result<()> {
    let mut line = serde_json::to_string(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    w.write_all(line.as_bytes())?;
    w.flush()
}

pub fn read_line<T: DeserializeOwned>(r: &mut dyn BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    loop {
        line.clear();
        let limit = u64::try_from(MAX_CONTROL_LINE).unwrap_or(u64::MAX).saturating_add(1);
        let n = r.take(limit).read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if line.len() > MAX_CONTROL_LINE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        if !line.trim().is_empty() {
            return serde_json::from_str(line.trim_end())
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
        }
    }
}

pub fn call(path: &Path, req: &ControlRequest, timeout: Duration) -> Result<ControlReply> {
    let stream = UnixStream::connect(path).with_context(|| {
        format!("cannot reach the Nixa window at {}. Is nixa-app running?", path.display())
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut writer = stream.try_clone()?;
    write_line(&mut writer, req)?;
    let mut reader = BufReader::new(stream);
    match read_line::<ControlReply>(&mut reader)? {
        Some(ControlReply::Error { message }) => Err(anyhow!(message)),
        Some(reply) => Ok(reply),
        None => bail!("the Nixa window closed the connection"),
    }
}

fn term_line(t: &TermState) -> String {
    let number = t.number.map(|n| format!("terminal {n} = ")).unwrap_or_default();
    let status = t.status.map(|s| format!(" {s}")).unwrap_or_default();
    let helpers = if t.children > 0 { format!(", {} helpers", t.children) } else { String::new() };
    let focused = if t.focused { " (focused)" } else { "" };
    let name = if t.name.is_empty() { "terminal" } else { t.name.as_str() };
    format!("{number}{} {name}{status}{helpers}{focused}", t.terminal)
}

pub fn summary_text(state: &AppState) -> String {
    let mut lines = Vec::new();
    let space = state.spaces.iter().find(|s| Some(&s.id) == state.focus.space.as_ref());
    if let Some(space) = space {
        let tab = space.tabs.iter().find(|t| Some(&t.id) == state.focus.tab.as_ref());
        let mut line = format!("Focused: space {}", space.name);
        if let Some(tab) = tab {
            line.push_str(&format!(", tab {}", tab.name));
        }
        if let Some(id) = &state.focus.terminal {
            let number = space
                .tabs
                .iter()
                .flat_map(|t| t.terminals.iter())
                .find(|t| &t.terminal == id)
                .and_then(|t| t.number);
            match number {
                Some(n) => line.push_str(&format!(", terminal {n} ({id})")),
                None => line.push_str(&format!(", terminal {id}")),
            }
        }
        line.push('.');
        lines.push(line);
    }
    if state.engine != "up" {
        lines.push(format!("Terminal engine: {}.", state.engine));
    }
    for (at, space) in state.spaces.iter().enumerate() {
        let shown = if space.active { " [shown]" } else { "" };
        lines.push(format!(
            "Space {} {} (id {}, folder {}){shown}:",
            at + 1,
            space.name,
            space.id,
            space.folder
        ));
        if space.tabs.is_empty() {
            lines.push("  no tab".to_string());
        }
        for tab in &space.tabs {
            let active = if tab.active { " [active]" } else { "" };
            let terms: Vec<String> = tab.terminals.iter().map(term_line).collect();
            let terms = if terms.is_empty() { "no terminal".to_string() } else { terms.join("; ") };
            lines.push(format!("  tab {} (id {}){active}: {terms}", tab.name, tab.id));
        }
    }
    if state.spaces.is_empty() {
        lines.push("No spaces are open.".to_string());
    }
    if !state.loose.is_empty() {
        let terms: Vec<String> = state.loose.iter().map(term_line).collect();
        lines.push(format!("Not in a tab: {}", terms.join("; ")));
    }
    if let Some(panel) = &state.panel {
        lines.push(format!("Right panel: {panel}."));
    }
    if let Some(overlay) = &state.overlay {
        lines.push(format!("Open on top: {overlay}."));
    }
    if let Some(root) = &state.files_root {
        lines.push(format!("Files panel folder: {root}."));
    }
    if let Some(repo) = &state.git_repo {
        lines.push(format!("Git repo: {repo}."));
    }
    if !state.unavailable.is_empty() {
        lines.push(format!("Not exposed: {}.", state.unavailable.join("; ")));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn term(n: u64, focused: bool) -> TermState {
        TermState {
            terminal: TermId::from_number(n),
            number: Some(n as u32),
            name: "claude".into(),
            program: "claude".into(),
            status: Some(Status::Working),
            note: None,
            running: true,
            children: 2,
            focused,
        }
    }

    fn state() -> AppState {
        AppState {
            engine: "up".into(),
            focus: Focus {
                space: Some("s1".into()),
                tab: Some("t1".into()),
                terminal: Some(TermId::from_number(3)),
            },
            spaces: vec![SpaceState {
                id: "s1".into(),
                name: "shop".into(),
                folder: "~/shop".into(),
                active: true,
                tabs: vec![TabState {
                    id: "t1".into(),
                    name: "claude".into(),
                    active: true,
                    terminals: vec![term(3, true)],
                }],
            }],
            unavailable: vec!["selected file not exposed".into()],
            ..AppState::default()
        }
    }

    #[test]
    fn requests_and_replies_keep_their_shape() {
        let req: ControlRequest = serde_json::from_value(json!({"op": "run", "command": "tab.new"})).unwrap();
        assert_eq!(req, ControlRequest::Run { command: "tab.new".into(), args: Value::Null, cleared: false, token: String::new() });
        let cleared = ControlRequest::Run { command: "app.layout-save".into(), args: Value::Null, cleared: true, token: "abc".into() };
        let value = serde_json::to_value(&cleared).unwrap();
        assert_eq!(value, json!({"op": "run", "command": "app.layout-save", "args": null, "cleared": true, "token": "abc"}));
        let plain = serde_json::to_value(ControlRequest::Run { command: "x".into(), args: Value::Null, cleared: false, token: String::new() }).unwrap();
        assert!(plain.get("token").is_none());
        let req = ControlRequest::Focus { terminal: TermId::from_number(12) };
        assert_eq!(serde_json::to_value(&req).unwrap(), json!({"op": "focus", "terminal": "mc"}));
        let reply = ControlReply::State { state: state() };
        let back: ControlReply = serde_json::from_value(serde_json::to_value(&reply).unwrap()).unwrap();
        assert_eq!(back, reply);
        assert_eq!(serde_json::to_value(Permission::Destructive).unwrap(), json!("destructive"));
    }

    #[test]
    fn shot_requests_and_replies_keep_their_shape() {
        let req: ControlRequest = serde_json::from_value(json!({"op": "shot", "capture": "0123abcd-0000-0000-0000-000000000000"})).unwrap();
        assert_eq!(req, ControlRequest::Shot { name: String::new(), capture: "0123abcd-0000-0000-0000-000000000000".into() });
        let reply = ControlReply::Shot { image: "/tmp/a.png".into(), window: "Nixa".into(), id: 42, bytes: 10 };
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["reply"], json!("shot"));
        let back: ControlReply = serde_json::from_value(value).unwrap();
        assert_eq!(back, reply);
    }

    #[test]
    fn lines_round_trip_and_stop_at_the_end() {
        let mut out = Vec::new();
        write_line(&mut out, &ControlRequest::Ping).unwrap();
        write_line(&mut out, &ControlRequest::State).unwrap();
        let mut r = Cursor::new(out);
        assert_eq!(read_line::<ControlRequest>(&mut r).unwrap(), Some(ControlRequest::Ping));
        assert_eq!(read_line::<ControlRequest>(&mut r).unwrap(), Some(ControlRequest::State));
        assert_eq!(read_line::<ControlRequest>(&mut r).unwrap(), None);
        let mut bad = Cursor::new(b"{\"op\":\"nope\"}\n".to_vec());
        assert!(read_line::<ControlRequest>(&mut bad).is_err());
    }

    #[test]
    fn summary_names_focus_spaces_tabs_and_gaps() {
        let text = summary_text(&state());
        assert!(text.starts_with("Focused: space shop, tab claude, terminal 3 (m3)."));
        assert!(text.contains("Space 1 shop (id s1, folder ~/shop) [shown]:"));
        assert!(text.contains("tab claude (id t1) [active]: terminal 3 = m3 claude working, 2 helpers (focused)"));
        assert!(text.contains("Not exposed: selected file not exposed."));
        let empty = summary_text(&AppState { engine: "down".into(), ..AppState::default() });
        assert!(empty.contains("No spaces are open."));
        assert!(empty.contains("Terminal engine: down."));
    }
}
