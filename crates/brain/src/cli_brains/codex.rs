use super::proc::{self, Event, Proc, Spec};
use super::{Cmd, Ctx, McpServer, REQUEST_MS, SERVER, Setup, VERSION, deadline};
use crate::provider::Model;
use crate::reply::{AgentReply, parse_reply};
use crate::util::log;
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

pub const DISABLED_FEATURES: [&str; 15] = [
    "apps",
    "plugins",
    "remote_plugin",
    "plugin_discovery_prefetch",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "image_generation",
    "multi_agent",
    "goals",
    "in_app_browser",
    "tool_suggest",
    "skill_mcp_dependency_install",
    "hooks",
    "skill_search",
];

pub const FOLLOW_UP: &str = "You said you would act. Do it now with your tools, or say plainly that you cannot.";
pub const STUCK: &str = "Sorry, my Codex brain stopped answering. I am starting it again.";

pub fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "speak": {"type": "boolean", "description": "true for every Heard and Typed message. false only for a Note or an Event that needs no words."},
            "text": {"type": "string", "description": "What Nixa says aloud. Short plain sentences."},
            "heard": {"type": "string", "enum": ["me", "unfinished"], "description": "me: the usual value. unfinished: the last heard sentence clearly stops in the middle and Nixa waits for the rest."}
        },
        "required": ["speak", "text", "heard"],
        "additionalProperties": false
    })
}
pub fn approval_policy() -> Value {
    json!({"granular": {"sandbox_approval": false, "rules": false, "skill_approval": false, "request_permissions": false, "mcp_elicitations": true}})
}

pub fn toml_text(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

pub fn user_servers(config: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("[mcp_servers.") else { continue };
        let Some(inner) = rest.strip_suffix(']') else { continue };
        let name = if let Some(quoted) = inner.strip_prefix('"') {
            quoted.split('"').next().unwrap_or("").to_string()
        } else {
            inner.split('.').next().unwrap_or("").to_string()
        };
        if !name.is_empty() && name != SERVER && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

pub fn codex_config_file() -> PathBuf {
    match std::env::var("CODEX_HOME").ok().filter(|v| !v.is_empty()) {
        Some(home) => PathBuf::from(home).join("config.toml"),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".codex/config.toml"),
    }
}

fn key_part(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') { name.to_string() } else { toml_text(name) }
}

pub fn args(model: Option<&str>, mcp: &McpServer, others: &[String]) -> Vec<String> {
    let mut out = vec!["app-server".to_string()];
    for feature in DISABLED_FEATURES {
        out.push("--disable".into());
        out.push(feature.into());
    }
    for name in others {
        out.push("-c".into());
        out.push(format!("mcp_servers.{}.enabled=false", key_part(name)));
    }
    out.push("-c".into());
    out.push("web_search=\"disabled\"".into());
    if let Some(model) = model {
        out.push("-c".into());
        out.push(format!("model={}", toml_text(model)));
    }
    let list: Vec<String> = mcp.args.iter().map(|a| toml_text(a)).collect();
    let env: Vec<String> = mcp.env.iter().map(|(k, v)| format!("{k}={}", toml_text(v))).collect();
    for item in [
        format!("mcp_servers.{SERVER}.command={}", toml_text(&mcp.command)),
        format!("mcp_servers.{SERVER}.args=[{}]", list.join(", ")),
        format!("mcp_servers.{SERVER}.env={{{}}}", env.join(", ")),
        format!("mcp_servers.{SERVER}.default_tools_approval_mode=\"approve\""),
        format!("mcp_servers.{SERVER}.tool_timeout_sec=640"),
    ] {
        out.push("-c".into());
        out.push(item);
    }
    out
}

pub fn promises_action(text: &str) -> bool {
    let low = text.replace('\u{2019}', "'").to_lowercase();
    let low = low.replace("let me know", "").replace("i will not", "").replace("i'll not", "");
    let words: Vec<&str> = low.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|w| !w.is_empty()).collect();
    let pairs = words.windows(2).any(|w| matches!((w[0], w[1]), ("i", "will") | ("let", "me") | ("i'm", "going") | ("am", "going")));
    pairs || words.iter().any(|w| matches!(*w, "i'll" | "checking"))
}

pub fn final_answer(phase: Option<&str>, text: &str) -> bool {
    match phase {
        Some("final_answer") => true,
        Some(_) => false,
        None => serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("speak").is_some()),
    }
}

fn number(row: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| row.get(*k).and_then(Value::as_u64))
}

pub fn read_usage(params: &Value) -> Option<(u64, u64, u64)> {
    let nested = params.get("tokenUsage");
    let row = [nested.and_then(|n| n.get("last")), nested.and_then(|n| n.get("total")), params.get("last"), params.get("total")]
        .into_iter()
        .flatten()
        .find(|v| v.is_object())?;
    let input = number(row, &["inputTokens", "input_tokens"]);
    let cached = number(row, &["cachedInputTokens", "cached_input_tokens"]);
    let output = number(row, &["outputTokens", "output_tokens"]);
    let reasoning = number(row, &["reasoningOutputTokens", "reasoning_output_tokens"]);
    if input.is_none() && cached.is_none() && output.is_none() && reasoning.is_none() {
        return None;
    }
    Some((input.unwrap_or(0), output.unwrap_or(0) + reasoning.unwrap_or(0), cached.unwrap_or(0)))
}

pub fn read_context(params: &Value) -> Option<(u64, u64)> {
    let usage = params.get("tokenUsage")?;
    let last = usage.get("last")?;
    let total = last.get("totalTokens").and_then(Value::as_u64);
    let input = last.get("inputTokens").and_then(Value::as_u64);
    if total.is_none() && input.is_none() {
        return None;
    }
    let used = total.unwrap_or(input.unwrap_or(0) + last.get("outputTokens").and_then(Value::as_u64).unwrap_or(0));
    let window = usage.get("modelContextWindow").and_then(Value::as_u64).unwrap_or(0);
    Some((used, window))
}

pub fn read_models(items: &[Value]) -> Vec<Model> {
    items
        .iter()
        .filter(|item| item.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter_map(|item| {
            let id = item.get("id").or_else(|| item.get("model")).and_then(Value::as_str).filter(|s| !s.is_empty())?;
            let efforts = item
                .get("supportedReasoningEfforts")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|e| e.as_str().or_else(|| e.get("reasoningEffort").and_then(Value::as_str)).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let name = item.get("displayName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(id);
            let description = item.get("description").and_then(Value::as_str).unwrap_or("");
            Some(Model { id: id.into(), name: name.into(), description: description.into(), efforts, max_tokens: None })
        })
        .collect()
}

pub fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text, "text_elements": []}])
}

#[derive(Default)]
struct Turn {
    thread: String,
    id: Option<String>,
    starting: bool,
    replied: bool,
    partial: String,
    spoke: bool,
    user: bool,
    held: Option<AgentReply>,
    queue: Vec<String>,
    model: Option<String>,
    models: Vec<Model>,
    ended: Vec<String>,
}

impl Turn {
    fn busy(&self) -> bool {
        self.id.is_some() || self.starting
    }

    fn drop_turn(&mut self) {
        if let Some(id) = self.id.take() {
            self.ended.push(id);
            if self.ended.len() > 16 {
                self.ended.remove(0);
            }
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.id = None;
        self.starting = false;
        self.replied = false;
        self.partial.clear();
        self.spoke = false;
        self.held = None;
    }
}

pub async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let config = std::fs::read_to_string(codex_config_file()).unwrap_or_default();
    let spec = Spec { bin: setup.bin.clone(), args: args(setup.model.as_deref(), &setup.mcp, &user_servers(&config)), cwd: setup.cwd.clone(), env: Vec::new(), stdin: true };
    let mut p = proc::spawn(&ctx.app.cli.groups, "Codex", &spec, true)?;
    let out = drive(ctx, &mut p, rx, &setup).await;
    p.stop();
    out
}

fn ended(p: &Proc, code: Option<i32>) -> String {
    let tail = proc::tail_text(&p.tail);
    let code = code.map(|c| format!(" with code {c}")).unwrap_or_default();
    if tail.is_empty() { format!("The Codex program stopped{code}.") } else { format!("The Codex program stopped{code}: {tail}") }
}

async fn start_thread(ctx: &Ctx, p: &Proc, setup: &Setup, turn: &mut Turn) -> Result<(), String> {
    let mut params = json!({
        "cwd": setup.cwd.display().to_string(),
        "sandbox": "read-only",
        "approvalPolicy": approval_policy(),
        "ephemeral": true,
        "developerInstructions": ctx.instructions(),
    });
    if let Some(model) = &turn.model {
        params["model"] = json!(model);
    }
    let started = p.rpc.request("thread/start", params, REQUEST_MS).await?;
    turn.thread = started.pointer("/thread/id").and_then(Value::as_str).ok_or("Codex did not open a thread.")?.to_string();
    if let Some(model) = started.get("model").and_then(Value::as_str) {
        turn.model = Some(model.to_string());
    }
    Ok(())
}

async fn list_models(p: &Proc) -> Result<Vec<Model>, String> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let mut params = json!({"includeHidden": false, "limit": 100});
        if let Some(c) = &cursor {
            params["cursor"] = json!(c);
        }
        let listed = p.rpc.request("model/list", params, REQUEST_MS).await?;
        items.extend(listed.get("data").and_then(Value::as_array).cloned().unwrap_or_default());
        cursor = listed.get("nextCursor").and_then(Value::as_str).map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(read_models(&items))
}

fn effort_for(ctx: &Ctx, turn: &Turn) -> Option<String> {
    let wanted = ctx.effort(turn.model.as_deref())?;
    let known = turn.models.iter().find(|m| Some(&m.id) == turn.model.as_ref())?;
    known.efforts.contains(&wanted).then_some(wanted)
}

async fn start_turn(ctx: &Ctx, p: &Proc, turn: &mut Turn, text: &str, user: bool) -> Result<(), String> {
    if turn.models.is_empty() && ctx.effort(turn.model.as_deref()).is_some() {
        match list_models(p).await {
            Ok(list) => {
                ctx.models(&list);
                turn.models = list;
            }
            Err(e) => log("cli-brain", &format!("codex models not listed, using the default effort: {e}")),
        }
    }
    turn.clear();
    turn.user = user;
    turn.starting = true;
    ctx.turn_started();
    let mut params = json!({"threadId": turn.thread, "input": input(text), "outputSchema": reply_schema()});
    if let Some(model) = &turn.model {
        params["model"] = json!(model);
    }
    if let Some(effort) = effort_for(ctx, turn) {
        params["effort"] = json!(effort);
    }
    let started = p.rpc.request("turn/start", params, REQUEST_MS).await;
    turn.starting = false;
    let started = started?;
    if turn.id.is_none()
        && let Some(id) = started.pointer("/turn/id").and_then(Value::as_str) {
            turn.id = Some(id.to_string());
        }
    Ok(())
}

async fn next_or_idle(ctx: &Ctx, p: &Proc, turn: &mut Turn) -> Result<(), String> {
    if turn.busy() {
        return Ok(());
    }
    if turn.queue.is_empty() {
        ctx.busy(false);
        return Ok(());
    }
    let text = std::mem::take(&mut turn.queue).join("\n\n");
    let user = text.starts_with("Typed") || text.starts_with("Heard");
    start_turn(ctx, p, turn, &text, user).await
}

pub fn answer_request(p: &Proc, message: &Value) {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "mcpServer/elicitation/request" => p.rpc.reply(&id, json!({"action": "decline", "content": null, "_meta": null})),
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => p.rpc.reply(&id, json!({"decision": "decline"})),
        "execCommandApproval" | "applyPatchApproval" => p.rpc.reply(&id, json!({"decision": "denied"})),
        "item/permissions/requestApproval" => p.rpc.reply(&id, json!({"permissions": {}, "scope": "turn"})),
        other => p.rpc.reply_error(&id, &format!("Nixa does not handle {other}.")),
    }
    if !method.is_empty() {
        log("cli-brain", &format!("codex asked {method}, answered no"));
    }
}

fn turn_of(params: &Value) -> Option<&str> {
    params.pointer("/turn/id").or_else(|| params.get("turnId")).and_then(Value::as_str)
}

fn stale(turn: &Turn, params: &Value) -> bool {
    let Some(id) = turn_of(params) else { return false };
    turn.ended.iter().any(|e| e == id) || turn.id.as_deref().is_some_and(|current| current != id)
}

async fn on_note(ctx: &Ctx, p: &Proc, turn: &mut Turn, message: &Value) -> Result<(), String> {
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    if let Some(thread) = params.get("threadId").and_then(Value::as_str)
        && thread != turn.thread {
            return Ok(());
        }
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if matches!(method, "turn/started" | "item/completed" | "turn/completed") && stale(turn, &params) {
        return Ok(());
    }
    match method {
        "turn/started" => {
            if let Some(id) = params.pointer("/turn/id").and_then(Value::as_str) {
                turn.id = Some(id.to_string());
            }
        }
        "item/completed" => {
            let item = params.get("item").cloned().unwrap_or(Value::Null);
            let Some(text) = item.get("text").and_then(Value::as_str) else { return Ok(()) };
            if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
                return Ok(());
            }
            let phase = item.get("phase").and_then(Value::as_str);
            if phase == Some("commentary") {
                if !turn.spoke && !text.trim().is_empty() {
                    turn.spoke = true;
                    let said = parse_reply(&crate::reply::extract_json(text));
                    ctx.commentary(if said.text.is_empty() { text } else { &said.text });
                }
                return Ok(());
            }
            if !final_answer(phase, text) {
                if !text.trim().is_empty() {
                    turn.partial = text.to_string();
                }
                return Ok(());
            }
            turn.partial.clear();
            let reply = parse_reply(text);
            turn.replied = true;
            if turn.user && promises_action(&reply.text) {
                turn.held = Some(reply);
                return Ok(());
            }
            ctx.reply(reply);
        }
        "turn/completed" => {
            let status = params.pointer("/turn/status").and_then(Value::as_str).unwrap_or("");
            let held = turn.held.take();
            let pending = held.clone().or_else(|| (status == "completed" && !turn.replied && !turn.partial.is_empty()).then(|| parse_reply(&turn.partial)));
            let again = pending.as_ref().is_some_and(|r| turn.user && promises_action(&r.text));
            let replied = turn.replied;
            turn.clear();
            if status == "failed" {
                let why = params.pointer("/turn/error/message").and_then(Value::as_str).unwrap_or("");
                ctx.reply(AgentReply::failed(format!("Sorry, that failed. {why}").trim().to_string()));
            } else if again {
                log("cli-brain", "codex promised an action with no tool call, asking again");
                if let Err(e) = start_turn(ctx, p, turn, FOLLOW_UP, false).await {
                    log("cli-brain", &format!("codex follow-up failed: {e}"));
                    if let Some(reply) = pending {
                        ctx.reply(reply);
                    }
                } else {
                    return Ok(());
                }
            } else if let Some(reply) = pending {
                ctx.reply(reply);
            } else if status == "completed" && !replied {
                ctx.reply(AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: false });
            }
            next_or_idle(ctx, p, turn).await?;
        }
        "thread/tokenUsage/updated" => {
            if let Some((input, output, cached)) = read_usage(&params) {
                ctx.usage(input, output, cached);
            }
            if let Some((used, window)) = read_context(&params) {
                ctx.context(used, window);
            }
        }
        "error" => log("cli-brain", &format!("codex error: {}", crate::util::clip_flat(&params.to_string(), 300))),
        _ => {}
    }
    Ok(())
}

async fn on_cmd(ctx: &Ctx, p: &Proc, setup: &Setup, turn: &mut Turn, cmd: Cmd) -> Result<bool, String> {
    match cmd {
        Cmd::Stop => return Ok(false),
        Cmd::Tell(text) => {
            if let Some(id) = turn.id.clone() {
                let steer = p.rpc.request("turn/steer", json!({"threadId": turn.thread, "expectedTurnId": id, "input": input(&text)}), REQUEST_MS).await;
                match steer {
                    Ok(_) => return Ok(true),
                    Err(e) => log("cli-brain", &format!("codex steer failed, the message waits for the next turn: {e}")),
                }
                turn.queue.push(text);
            } else if turn.busy() {
                turn.queue.push(text);
            } else {
                let user = text.starts_with("Typed") || text.starts_with("Heard") || text.contains("\n\nTyped") || text.contains("\n\nHeard");
                if let Err(e) = start_turn(ctx, p, turn, &text, user).await {
                    turn.clear();
                    ctx.reply(AgentReply::failed(format!("My Codex brain failed. {e}")));
                    ctx.busy(false);
                }
            }
        }
        Cmd::Interrupt => {
            turn.queue.clear();
            if let Some(id) = turn.id.clone() {
                let _ = p.rpc.request("turn/interrupt", json!({"threadId": turn.thread, "turnId": id}), REQUEST_MS).await;
            }
            turn.drop_turn();
        }
        Cmd::Reset => {
            turn.queue.clear();
            if let Some(id) = turn.id.clone() {
                let _ = p.rpc.request("turn/interrupt", json!({"threadId": turn.thread, "turnId": id}), REQUEST_MS).await;
            }
            turn.drop_turn();
            start_thread(ctx, p, setup, turn).await?;
            ctx.model_now(turn.model.as_deref());
        }
        Cmd::Model(model) => {
            turn.model = model;
            ctx.model_now(turn.model.as_deref());
        }
        Cmd::Models(reply) => {
            let listed = list_models(p).await;
            if let Ok(list) = &listed {
                ctx.models(list);
                turn.models = list.clone();
            }
            let _ = reply.send(listed);
        }
    }
    Ok(true)
}

async fn drive(ctx: &Ctx, p: &mut Proc, rx: &mut UnboundedReceiver<Cmd>, setup: &Setup) -> Result<(), String> {
    let hello = json!({"clientInfo": {"name": "nixa", "title": "Nixa", "version": VERSION}, "capabilities": {"experimentalApi": true, "requestAttestation": false}});
    if let Err(e) = p.rpc.request("initialize", hello, REQUEST_MS).await {
        return Err(if e.contains("stopped") { ended(p, None) } else { e });
    }
    p.rpc.notify("initialized", json!({}));
    let mut turn = Turn { model: setup.model.clone(), ..Turn::default() };
    start_thread(ctx, p, setup, &mut turn).await?;
    ctx.ready(turn.model.as_deref());
    let mut last = Instant::now();
    loop {
        let until = deadline(turn.busy(), last);
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return Ok(()) };
                if !on_cmd(ctx, p, setup, &mut turn, cmd).await? {
                    return Ok(());
                }
                last = Instant::now();
            }
            event = p.events.recv() => {
                last = Instant::now();
                match event {
                    Some(Event::Message(message)) => {
                        if message.get("method").is_some() && message.get("id").is_some() {
                            answer_request(p, &message);
                        } else if message.get("method").is_some() {
                            on_note(ctx, p, &mut turn, &message).await?;
                        }
                    }
                    Some(Event::Ended(code)) => return Err(ended(p, code)),
                    None => return Err(ended(p, None)),
                }
            }
            _ = tokio::time::sleep_until(until) => {
                ctx.reply(AgentReply::failed(STUCK));
                return Err("It stopped answering.".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp() -> McpServer {
        McpServer { command: "/x/nixa-mcp".into(), args: vec!["bridge".into()], env: vec![("NIXA_TOOL_TOKEN".into(), "t\"k".into())] }
    }

    #[test]
    fn late_events_of_a_stopped_turn_are_dropped() {
        let mut turn = Turn { id: Some("tu1".into()), ..Turn::default() };
        turn.drop_turn();
        assert_eq!(turn.ended, vec!["tu1".to_string()]);
        let old = json!({"threadId": "th", "turn": {"id": "tu1", "status": "interrupted"}});
        assert!(stale(&turn, &old));
        turn.starting = true;
        assert!(stale(&turn, &old));
        turn.id = Some("tu2".into());
        assert!(stale(&turn, &old));
        assert!(!stale(&turn, &json!({"turn": {"id": "tu2"}})));
        assert!(stale(&turn, &json!({"turnId": "tu3"})));
        assert!(!stale(&turn, &json!({"threadId": "th"})));
        for n in 0..20 {
            turn.id = Some(format!("x{n}"));
            turn.drop_turn();
        }
        assert_eq!(turn.ended.len(), 16);
    }

    #[test]
    fn builds_args() {
        let list = args(Some("gpt-x"), &mcp(), &["node_repl".into(), "computer-use".into()]);
        assert_eq!(list[0], "app-server");
        assert!(list.contains(&"mcp_servers.node_repl.enabled=false".to_string()));
        assert!(list.contains(&"mcp_servers.computer-use.enabled=false".to_string()));
        assert!(list.contains(&"model=\"gpt-x\"".to_string()));
        assert!(list.contains(&format!("mcp_servers.{SERVER}.command=\"/x/nixa-mcp\"")));
        assert!(list.contains(&format!("mcp_servers.{SERVER}.args=[\"bridge\"]")));
        assert!(list.contains(&format!("mcp_servers.{SERVER}.env={{NIXA_TOOL_TOKEN=\"t\\\"k\"}}")));
        assert_eq!(list.iter().filter(|a| *a == "--disable").count(), DISABLED_FEATURES.len());
    }

    #[test]
    fn reads_only_server_names_never_values() {
        let config = "api_key = \"sk_secret_1\"\n[mcp_servers.notes]\ncommand = \"/bin/notes\"\nenv = { TOKEN = \"tok_secret_2\" }\n[mcp_servers.notes.env]\nKEY = \"tok_secret_3\"\n";
        let names = user_servers(config);
        assert_eq!(names, vec!["notes".to_string()]);
        let kept = names.join(" ");
        for secret in ["sk_secret_1", "/bin/notes", "tok_secret_2", "tok_secret_3"] {
            assert!(!kept.contains(secret));
        }
        let mcp = McpServer { command: "/m".into(), args: vec![], env: vec![] };
        let line = args(None, &mcp, &names).join(" ");
        assert!(line.contains("mcp_servers.notes.enabled=false"));
        assert!(!line.contains("secret"));
    }

    #[test]
    fn reads_user_servers() {
        let config = "[mcp_servers.node_repl]\ncommand='x'\n[mcp_servers.node_repl.env]\n[mcp_servers.\"computer-use\"]\n[mcp_servers.nixa_brain]\n[other]\n";
        assert_eq!(user_servers(config), vec!["node_repl".to_string(), "computer-use".to_string()]);
    }

    #[test]
    fn spots_promises() {
        assert!(promises_action("I'll check m3 now."));
        assert!(promises_action("Let me look."));
        assert!(promises_action("I\u{2019}m going to read it"));
        assert!(!promises_action("Let me know if you want more."));
        assert!(!promises_action("m3 is done."));
        assert!(!promises_action("I will not do that."));
    }

    #[test]
    fn final_answers() {
        assert!(final_answer(Some("final_answer"), "x"));
        assert!(!final_answer(Some("commentary"), "{\"speak\":true}"));
        assert!(final_answer(None, "{\"speak\":true,\"text\":\"a\"}"));
        assert!(!final_answer(None, "plain"));
    }

    #[test]
    fn reads_usage_context_and_models() {
        let params = json!({"tokenUsage": {"last": {"inputTokens": 10, "cachedInputTokens": 4, "outputTokens": 3, "reasoningOutputTokens": 2, "totalTokens": 19}, "modelContextWindow": 1000}});
        assert_eq!(read_usage(&params), Some((10, 5, 4)));
        assert_eq!(read_context(&params), Some((19, 1000)));
        assert_eq!(read_usage(&json!({})), None);
        let models = read_models(&[
            json!({"id": "a", "displayName": "A", "supportedReasoningEfforts": [{"reasoningEffort": "low"}, "high"]}),
            json!({"model": "b"}),
            json!({"id": "c", "hidden": true}),
        ]);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].efforts, vec!["low".to_string(), "high".to_string()]);
        assert_eq!(models[1].name, "b");
    }
}
