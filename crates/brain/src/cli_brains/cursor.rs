use super::process_groups::Groups;
use super::proc::{self, Event, Spec};
use super::{Cmd, Ctx, McpServer, PROMPT_MS, SERVER, deadline};
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use crate::util::write_json;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

pub const COMMAND_MS: u64 = 30_000;
pub const NOT_SIGNED_IN: &str = "Cursor is not signed in. Run cursor-agent login outside Nixa, then pick this brain again.";

pub fn reply_rule() -> String {
    format!("Your own shell, file, web, search and subagent tools are refused. Use only the Nixa tools of the {SERVER} MCP server. Send coding and running work to a coding agent with nixa_term_send. Reply with only the JSON object {{\"speak\":boolean,\"text\":string,\"heard\":\"me\"|\"unfinished\"}}.")
}

pub fn strip_colors(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn read_models(text: &str) -> (Vec<Model>, Option<String>) {
    let mut models: Vec<Model> = Vec::new();
    let mut default = None;
    for line in strip_colors(text).lines() {
        let line = line.trim();
        let Some((id, label)) = line.split_once(" - ") else { continue };
        let id = id.trim();
        if id.is_empty() || !id.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') || !id.chars().all(|c| c.is_alphanumeric() || "_./:+-".contains(c)) {
            continue;
        }
        if models.iter().any(|m| m.id == id) {
            continue;
        }
        let mut name = label.trim().to_string();
        for tail in [" (current, default)", " (default, current)", " (default)", " (current)"] {
            if let Some(rest) = name.strip_suffix(tail) {
                if tail.contains("default") {
                    default = Some(id.to_string());
                }
                name = rest.to_string();
                break;
            }
        }
        models.push(Model { id: id.into(), name, description: "Cursor CLI".into(), efforts: Vec::new(), max_tokens: None });
    }
    (models, default)
}

pub fn guard_commands(mcp_command: &str) -> (String, String) {
    let bin = quote(mcp_command);
    (format!("{bin} guard tool {SERVER}"), format!("{bin} guard mcp {SERVER}"))
}

pub fn mcp_config(mcp: &McpServer, home: &str) -> Value {
    let mut env: serde_json::Map<String, Value> = mcp.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    env.insert("HOME".into(), json!(home));
    json!({"mcpServers": {SERVER: {"command": mcp.command, "args": mcp.args, "env": env}}})
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub const KEYCHAIN_ACCOUNT: &str = "cursor";

pub async fn load_key() -> Option<String> {
    if let Some(key) = std::env::var("CURSOR_API_KEY").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return Some(key);
    }
    crate::keys::keychain_key(KEYCHAIN_ACCOUNT).await
}

pub struct Workspace {
    pub root: PathBuf,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn private_dir(path: &Path) -> Result<(), String> {
    nixa_protocol::paths::ensure_private_dir(path).map_err(|e| e.to_string())
}

pub fn prepare(base: &Path, mcp: &McpServer, api_key: Option<&str>, key_env: bool) -> Result<Workspace, String> {
    let api_key = api_key.map(str::trim).filter(|k| !k.is_empty());
    if api_key.is_none() && !key_env {
        return Err(NOT_SIGNED_IN.into());
    }
    private_dir(base)?;
    let name = super::try_token().ok_or("No random folder name could be made for the Cursor brain.")?;
    let root = base.join(format!("cursor-{}", &name[..12]));
    private_dir(&root)?;
    let workspace = Workspace { root: root.clone(), cwd: root.join("workspace"), env: Vec::new() };
    let home = root.join("home");
    let config = home.join(".cursor");
    private_dir(&config)?;
    private_dir(&workspace.cwd.join(".cursor"))?;
    let (tool_guard, mcp_guard) = guard_commands(&mcp.command);
    write_json(
        &config.join("cli-config.json"),
        &json!({
            "version": 1,
            "editor": {"vimMode": false},
            "approvalMode": "allowlist",
            "permissions": {"allow": [format!("Mcp({SERVER}:*)")], "deny": ["Shell(*)", "Read(**)", "Read(/**)", "Write(**)", "Write(/**)", "WebFetch(*)"]}
        }),
    )
    .map_err(|e| e.to_string())?;
    let real_home = std::env::var("HOME").unwrap_or_default();
    write_json(&workspace.cwd.join(".cursor/mcp.json"), &mcp_config(mcp, &real_home)).map_err(|e| e.to_string())?;
    write_json(
        &workspace.cwd.join(".cursor/hooks.json"),
        &json!({"version": 1, "hooks": {
            "preToolUse": [{"command": tool_guard, "failClosed": true, "timeout": 5}],
            "beforeMCPExecution": [{"command": mcp_guard, "failClosed": true, "timeout": 5}]
        }}),
    )
    .map_err(|e| e.to_string())?;
    let mut workspace = workspace;
    let h = home.display().to_string();
    workspace.env = vec![
        ("HOME".into(), h.clone()),
        ("XDG_CONFIG_HOME".into(), format!("{h}/.config")),
        ("CURSOR_CONFIG_DIR".into(), config.display().to_string()),
        ("CURSOR_DATA_DIR".into(), root.join("data").display().to_string()),
        ("AGENT_CLI_CREDENTIAL_STORE".into(), "file".into()),
        ("NO_OPEN_BROWSER".into(), "1".into()),
    ];
    if let Some(key) = api_key {
        workspace.env.push(("CURSOR_API_KEY".into(), key.to_string()));
    }
    Ok(workspace)
}

pub fn read_usage(value: &Value) -> Option<(u64, u64, u64)> {
    let row = value.as_object()?;
    if row.is_empty() {
        return None;
    }
    let n = |k: &str| row.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some((n("inputTokens"), n("outputTokens"), n("cacheReadTokens") + n("cacheWriteTokens")))
}

pub fn turn_args(model: Option<&str>, session: &str) -> Vec<String> {
    let mut out: Vec<String> = ["--print", "--output-format", "stream-json", "--stream-partial-output", "--approve-mcps", "--trust"].iter().map(|s| s.to_string()).collect();
    if let Some(m) = model {
        out.push("--model".into());
        out.push(m.into());
    }
    if !session.is_empty() {
        out.push("--resume".into());
        out.push(session.into());
    }
    out
}

#[derive(Default)]
pub struct Stream {
    pub text: String,
    pub prior: String,
    pub deltas: bool,
    pub session: String,
}

pub enum Step {
    Nothing,
    Commentary(String),
    Failed,
    Done { answer: String, session: String, usage: Option<(u64, u64, u64)> },
}

pub fn read_event(s: &mut Stream, event: &Value) -> Step {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    let subtype = event.get("subtype").and_then(Value::as_str).unwrap_or("");
    if kind == "system" && subtype == "init" {
        s.session = event.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
    }
    if kind == "assistant" {
        let chunk: String = event
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter(|i| i.get("type").and_then(Value::as_str) == Some("text")).filter_map(|i| i.get("text").and_then(Value::as_str)).collect())
            .unwrap_or_default();
        if event.get("timestamp_ms").is_some() && event.get("model_call_id").is_none() {
            s.text.push_str(&chunk);
            s.deltas = true;
        } else if event.get("timestamp_ms").is_none() && !s.deltas {
            s.text = chunk;
        }
    }
    if kind == "tool_call" && subtype == "started" {
        let said = if s.text.trim().is_empty() { None } else { Some(parse_reply(&extract_json(&s.text))) };
        s.prior.push_str(&s.text);
        s.text.clear();
        s.deltas = false;
        if let Some(said) = said.filter(|r| r.speak && !r.text.is_empty()) {
            return Step::Commentary(said.text);
        }
        return Step::Nothing;
    }
    if kind == "error" || (kind == "result" && (event.get("is_error").and_then(Value::as_bool) == Some(true) || subtype != "success")) {
        return Step::Failed;
    }
    if kind != "result" {
        return Step::Nothing;
    }
    let full = event.get("result").and_then(Value::as_str).unwrap_or("");
    let answer = if !s.text.trim().is_empty() {
        s.text.trim().to_string()
    } else if !s.prior.is_empty() && full.starts_with(&s.prior) {
        full[s.prior.len()..].trim().to_string()
    } else {
        full.trim().to_string()
    };
    let session = event.get("session_id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| s.session.clone());
    Step::Done { answer, session, usage: event.get("usage").and_then(read_usage) }
}

pub fn tools_found(text: &str) -> bool {
    text.lines().any(|line| {
        let Some(rest) = line.trim().strip_prefix("Tools for ") else { return false };
        let Some(open) = rest.rfind('(') else { return false };
        let count: String = rest[open + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        count.parse::<u64>().is_ok_and(|n| n > 0) && rest[open + 1 + count.len()..].starts_with("):")
    })
}

struct Live {
    ws: Workspace,
    bin: PathBuf,
    model: Option<String>,
    models: Vec<Model>,
    session: String,
    queue: Vec<String>,
    new_model: bool,
}

fn keep_session(live: &mut Live, session: String) {
    if std::mem::take(&mut live.new_model) {
        live.session.clear();
    } else {
        live.session = session;
    }
}

pub async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let key_env = std::env::var("CURSOR_AUTH_TOKEN").is_ok_and(|v| !v.trim().is_empty());
    let key = load_key().await;
    let ws = prepare(&ctx.app.cli.work_dir, &setup.mcp, key.as_deref(), key_env)?;
    let mut live = Live { ws, bin: setup.bin.clone(), model: setup.model.clone(), models: Vec::new(), session: String::new(), queue: Vec::new(), new_model: false };
    open(&ctx.app.cli.groups, &mut live).await?;
    ctx.models(&live.models);
    ctx.ready(live.model.as_deref());
    drive(ctx, &mut live, rx).await
}

async fn command(groups: &Groups, live: &Live, args: &[&str]) -> Result<(Option<i32>, String), String> {
    let spec = Spec { bin: live.bin.clone(), args: args.iter().map(|s| s.to_string()).collect(), cwd: live.ws.cwd.clone(), env: live.ws.env.clone(), stdin: false };
    proc::run_once(groups, "Cursor", &spec, COMMAND_MS).await
}

async fn open(groups: &Groups, live: &mut Live) -> Result<(), String> {
    let (code, out) = command(groups, live, &["status", "--format", "json"]).await?;
    let signed_in = serde_json::from_str::<Value>(&out).ok().and_then(|v| v.get("isAuthenticated").and_then(Value::as_bool)) == Some(true);
    if code != Some(0) || !signed_in {
        return Err(NOT_SIGNED_IN.into());
    }
    let (code, out) = command(groups, live, &["--list-models"]).await?;
    let (models, default) = read_models(&out);
    if code != Some(0) || models.is_empty() {
        return Err("Cursor did not list its models. Check cursor-agent --list-models outside Nixa.".into());
    }
    live.models = models;
    if live.model.is_none() {
        live.model = default;
    }
    if let Some(m) = &live.model
        && !live.models.iter().any(|x| &x.id == m) {
            return Err(format!("The saved Cursor model {m} is not offered to this account. Pick a listed model."));
        }
    let (code, _) = command(groups, live, &["mcp", "enable", SERVER]).await?;
    let (code, out) = if code == Some(0) { command(groups, live, &["mcp", "list-tools", SERVER]).await? } else { (code, String::new()) };
    if code != Some(0) || !tools_found(&out) {
        return Err("Cursor could not find the Nixa tools. Check that nixa-mcp sits next to nixa-brain.".into());
    }
    Ok(())
}

async fn one_turn(ctx: &Ctx, live: &mut Live, rx: &mut UnboundedReceiver<Cmd>, said: String) -> Result<bool, String> {
    ctx.turn_started();
    let prompt = format!("{}\n\n{}\n\n{said}", ctx.instructions(), reply_rule());
    let spec = Spec { bin: live.bin.clone(), args: turn_args(live.model.as_deref(), &live.session), cwd: live.ws.cwd.clone(), env: live.ws.env.clone(), stdin: true };
    let mut p = proc::spawn_with_input(&ctx.app.cli.groups, "Cursor", &spec, prompt)?;
    let mut stream = Stream::default();
    let started = Instant::now();
    let mut last = Instant::now();
    let limit = started + std::time::Duration::from_millis(PROMPT_MS);
    let outcome = loop {
        let until = deadline(true, last).min(limit);
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None | Some(Cmd::Stop) => break Ok(false),
                Some(Cmd::Tell(text)) => live.queue.push(text),
                Some(Cmd::Interrupt) => {
                    live.queue.clear();
                    live.session.clear();
                    break Ok(true);
                }
                Some(Cmd::Reset) => {
                    live.queue.clear();
                    live.session.clear();
                    break Ok(true);
                }
                Some(Cmd::Model(m)) => {
                    live.model = m;
                    live.new_model = true;
                    ctx.model_now(live.model.as_deref());
                }
                Some(Cmd::Models(reply)) => {
                    let _ = reply.send(Ok(live.models.clone()));
                }
            },
            event = p.events.recv() => {
                last = Instant::now();
                match event {
                    Some(Event::Message(message)) => match read_event(&mut stream, &message) {
                        Step::Nothing => {}
                        Step::Commentary(text) => ctx.commentary(&text),
                        Step::Failed => {
                            live.session.clear();
                            ctx.reply(AgentReply::failed("My Cursor brain failed. The CLI reported an error. Check the Cursor sign in and model access."));
                            break Ok(true);
                        }
                        Step::Done { answer, session, usage } => {
                            if let Some((i, o, c)) = usage {
                                ctx.usage(i, o, c);
                            }
                            keep_session(live, session);
                            ctx.reply(parse_reply(&extract_json(&answer)));
                            break Ok(true);
                        }
                    },
                    Some(Event::Ended(_)) | None => {
                        live.session.clear();
                        ctx.reply(AgentReply::failed("My Cursor brain failed. The CLI stopped without a result. Check the cursor-agent command and its sign in."));
                        break Ok(true);
                    }
                }
            }
            _ = tokio::time::sleep_until(until) => {
                live.session.clear();
                let why = if Instant::now() >= limit { "The turn took too long." } else { "It stopped answering." };
                ctx.reply(AgentReply::failed(format!("My Cursor brain failed. {why}")));
                break Ok(true);
            }
        }
    };
    p.stop();
    outcome
}

async fn drive(ctx: &Ctx, live: &mut Live, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    loop {
        if !live.queue.is_empty() {
            let said = std::mem::take(&mut live.queue).join("\n\n");
            if !one_turn(ctx, live, rx, said).await? {
                return Ok(());
            }
            continue;
        }
        ctx.busy(false);
        let Some(cmd) = rx.recv().await else { return Ok(()) };
        match cmd {
            Cmd::Stop => return Ok(()),
            Cmd::Tell(text) => live.queue.push(text),
            Cmd::Interrupt => {}
            Cmd::Reset => live.session.clear(),
            Cmd::Model(m) => {
                live.model = m;
                live.session.clear();
                ctx.model_now(live.model.as_deref());
            }
            Cmd::Models(reply) => {
                let _ = reply.send(Ok(live.models.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_lines() {
        let text = "\u{1b}[1mAvailable models\u{1b}[0m\n\nauto - Auto (current, default)\ngpt-5.5 - GPT 5.5\nbad line\ngpt-5.5 - again\n";
        let (models, default) = read_models(text);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "Auto");
        assert_eq!(default, Some("auto".into()));
    }

    #[test]
    fn guard_and_config() {
        let (tool, mcp_guard) = guard_commands("/a b/nixa-mcp");
        assert_eq!(tool, format!("'/a b/nixa-mcp' guard tool {SERVER}"));
        assert_eq!(mcp_guard, format!("'/a b/nixa-mcp' guard mcp {SERVER}"));
        let mcp = McpServer { command: "/m".into(), args: vec!["bridge".into()], env: vec![("K".into(), "v".into())] };
        let c = mcp_config(&mcp, "/Users/x");
        assert_eq!(c["mcpServers"][SERVER]["env"]["HOME"], "/Users/x");
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn model_change_in_a_turn_starts_a_new_session() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = McpServer { command: "/m".into(), args: vec![], env: vec![] };
        let ws = prepare(dir.path(), &mcp, Some("k"), false).unwrap();
        let hooks = std::fs::read_to_string(ws.cwd.join(".cursor/hooks.json")).unwrap();
        assert!(hooks.contains("guard mcp"));
        let mut live = Live { ws, bin: "/x".into(), model: Some("a".into()), models: Vec::new(), session: "old".into(), queue: Vec::new(), new_model: false };
        keep_session(&mut live, "s2".into());
        assert_eq!(live.session, "s2");
        live.new_model = true;
        keep_session(&mut live, "s3".into());
        assert_eq!(live.session, "");
        assert!(!live.new_model);
    }

    #[test]
    fn prepares_private_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = McpServer { command: "/m".into(), args: vec![], env: vec![] };
        assert_eq!(prepare(dir.path(), &mcp, None, false).err().as_deref(), Some(NOT_SIGNED_IN));
        assert_eq!(prepare(dir.path(), &mcp, Some("  "), false).err().as_deref(), Some(NOT_SIGNED_IN));
        let ws = prepare(dir.path(), &mcp, Some("key_x"), false).unwrap();
        let root = ws.root.clone();
        assert!(ws.cwd.join(".cursor/hooks.json").exists());
        assert!(!root.join("home/.cursor/auth.json").exists());
        let files: Vec<String> = walk_files(&root);
        assert!(files.iter().all(|f| !f.ends_with("auth.json")), "{files:?}");
        for f in &files {
            assert!(!std::fs::read_to_string(f).unwrap_or_default().contains("key_x"), "{f}");
        }
        assert!(ws.env.iter().any(|(k, v)| k == "CURSOR_API_KEY" && v == "key_x"));
        assert!(ws.env.iter().any(|(k, v)| k == "AGENT_CLI_CREDENTIAL_STORE" && v == "file"));
        let by_env = prepare(dir.path(), &mcp, None, true).unwrap();
        assert!(!by_env.env.iter().any(|(k, _)| k == "CURSOR_API_KEY"));
        drop(by_env);
        drop(ws);
        assert!(!root.exists());
    }

    fn walk_files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path.display().to_string());
            }
        }
        out
    }

    #[test]
    fn a_left_cursor_folder_is_swept_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join(super::super::WORK_DIR);
        std::fs::create_dir_all(work.join("cursor-abc/home/.cursor")).unwrap();
        std::fs::write(work.join("cursor-abc/home/.cursor/cli-config.json"), "{}").unwrap();
        std::fs::create_dir_all(work.join("other")).unwrap();
        let state = super::super::State::open(dir.path());
        assert!(!work.join("cursor-abc").exists());
        assert!(work.join("other").exists());
        assert_eq!(state.token.len(), 48);
    }

    #[test]
    fn reads_stream_events() {
        let mut s = Stream::default();
        assert!(matches!(read_event(&mut s, &json!({"type": "system", "subtype": "init", "session_id": "s1"})), Step::Nothing));
        read_event(&mut s, &json!({"type": "assistant", "timestamp_ms": 1, "message": {"content": [{"type": "text", "text": "{\"speak\":true,\"text\":\"Looking\"}"}]}}));
        assert!(matches!(read_event(&mut s, &json!({"type": "tool_call", "subtype": "started"})), Step::Commentary(t) if t == "Looking"));
        read_event(&mut s, &json!({"type": "assistant", "timestamp_ms": 2, "message": {"content": [{"type": "text", "text": "{\"speak\":true,\"text\":\"Done\"}"}]}}));
        match read_event(&mut s, &json!({"type": "result", "subtype": "success", "result": "x", "usage": {"inputTokens": 4, "outputTokens": 1}})) {
            Step::Done { answer, session, usage } => {
                assert!(answer.contains("Done"));
                assert_eq!(session, "s1");
                assert_eq!(usage, Some((4, 1, 0)));
            }
            _ => panic!("no result"),
        }
        assert!(matches!(read_event(&mut Stream::default(), &json!({"type": "result", "subtype": "error"})), Step::Failed));
        assert!(tools_found("Tools for nixa_brain (12):\n- a"));
        assert!(!tools_found("Tools for nixa_brain (0):"));
        assert!(!turn_args(Some("m"), "s").iter().any(|a| a == "hi"));
        assert_eq!(turn_args(None, "").last().map(String::as_str), Some("--trust"));
    }
}
