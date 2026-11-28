use super::proc::{self, Event, Proc, Spec};
use super::{Cmd, Ctx, Kind, McpServer, PROMPT_MS, REQUEST_MS, SERVER, Setup, VERSION, deadline};
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use crate::util::log;
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::time::Instant;

pub const CONTEXT_LIMIT: u64 = 90_000;

pub fn tool_rule(kind: Kind) -> String {
    let gateway = if kind == Kind::Agy {
        format!(" Antigravity may load only some Nixa tools as your own tools. Every other one still works: call it through the {SERVER} nixa_call tool with {{\"tool\": \"<name>\", \"args\": {{...}}}}. Call it with {{\"tool\": \"help\"}} to list every tool. Never say a Nixa tool is missing before you try it this way.")
    } else {
        String::new()
    };
    format!("Here you have no shell, no file tools and no web tools of your own: they are refused. Use only the Nixa tools of the {SERVER} server. To run tests, read files or change code, send the work to a coding agent with nixa_term_send, or read an agent's screen with nixa_term_read.{gateway}\n\nReply with only the JSON object {{\"speak\": boolean, \"text\": string, \"heard\": \"me\" | \"unfinished\"}}. No other words, no code fences.")
}

pub const REFUSED_NOTE: &str = "Note: your own shell, file and web tools are refused here, so that step did not run. Use the Nixa tools instead (for example nixa_term_read, or send the work to a coding agent with nixa_term_send), then reply with the JSON object.";

pub fn servers(mcp: &McpServer) -> Value {
    let env: Vec<Value> = mcp.env.iter().map(|(k, v)| json!({"name": k, "value": v})).collect();
    json!([{"name": SERVER, "command": mcp.command, "args": mcp.args, "env": env}])
}

pub fn strip_server(raw: &str) -> Option<String> {
    let name = raw.trim();
    let name = name.strip_prefix("Run ").unwrap_or(name).trim();
    [format!("mcp_{SERVER}_"), format!("mcp__{SERVER}__"), format!("{SERVER}: "), format!("{SERVER}:"), format!("{SERVER}/"), format!("{SERVER}.")]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix.as_str()).map(str::to_string))
}

fn known(name: &str, names: &[String]) -> bool {
    names.iter().any(|n| n == name) || name == "nixa_call"
}

pub fn tool_allowed(call: &Value, names: &[String]) -> bool {
    let named: Vec<&str> = ["name", "toolName"].iter().filter_map(|k| call.get(*k).and_then(Value::as_str)).collect();
    if !named.is_empty() {
        return named.iter().all(|raw| {
            let bare = strip_server(raw).unwrap_or_else(|| raw.trim().to_string());
            known(&bare, names)
        });
    }
    call.get("title").and_then(Value::as_str).is_some_and(|raw| {
        let bare = strip_server(raw).unwrap_or_else(|| raw.trim().to_string());
        known(&bare, names)
    })
}

pub fn permission_answer(params: &Value, names: &[String]) -> (Value, bool) {
    let allow = tool_allowed(params.get("toolCall").unwrap_or(&Value::Null), names);
    let options = params.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
    let kind = |o: &Value| o.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let pick = if allow {
        options.iter().find(|o| kind(o) == "allow_once")
    } else {
        options.iter().find(|o| kind(o) == "reject_once").or_else(|| options.iter().find(|o| kind(o).starts_with("reject")))
    };
    let answer = match pick.and_then(|o| o.get("optionId")) {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    };
    (answer, allow)
}

pub fn agent_permission_answer(params: &Value, names: &[String]) -> (Value, bool) {
    let options = params.get("options").and_then(Value::as_array).cloned().unwrap_or_default();
    let kind = |o: &Value| o.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let pick = options.iter().find(|o| kind(o) == "allow_once").or_else(|| options.iter().find(|o| kind(o).starts_with("allow")));
    match pick.and_then(|o| o.get("optionId")) {
        Some(id) => (json!({"outcome": {"outcome": "selected", "optionId": id}}), true),
        None => permission_answer(params, names),
    }
}

pub fn read_usage(value: &Value) -> Option<(u64, u64, u64)> {
    let row = value.as_object()?;
    if row.is_empty() {
        return None;
    }
    let count = |k: &str| row.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some((count("inputTokens"), count("outputTokens") + count("thoughtTokens"), count("cachedReadTokens") + count("cachedWriteTokens")))
}

pub fn read_models(created: &Value, title: &str) -> (Vec<Model>, String) {
    let block = created.get("models").cloned().unwrap_or(Value::Null);
    let list = block.get("availableModels").and_then(Value::as_array).cloned().unwrap_or_default();
    let models = list
        .iter()
        .filter_map(|item| {
            let id = item.get("modelId").and_then(Value::as_str).filter(|s| !s.is_empty())?;
            let name = item.get("name").and_then(Value::as_str).unwrap_or(id);
            let description = item.get("description").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(title);
            Some(Model { id: id.into(), name: name.into(), description: description.into(), efforts: Vec::new(), max_tokens: None })
        })
        .collect();
    (models, block.get("currentModelId").and_then(Value::as_str).unwrap_or("").to_string())
}

struct Session {
    id: String,
    turn: u64,
    busy: bool,
    text: String,
    spoke: bool,
    refused: bool,
    retried: bool,
    context: u64,
    queue: Vec<String>,
    fresh: bool,
    model: Option<String>,
    models: Vec<Model>,
}

type Done = (u64, Result<Value, String>);

pub async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let mut args = vec!["acp".into()];
    if ctx.kind == Kind::Jcode {
        args.extend(["--no-update", "--tool-profile", "none", "--disable-base-tools", "--mcp-tools", "eager"].map(str::to_string));
    }
    let env = if ctx.kind == Kind::Dim {
        vec![("DIMCODE_UPDATE_CHECK".into(), "0".into())]
    } else {
        Vec::new()
    };
    let spec = Spec { bin: setup.bin.clone(), args, cwd: setup.cwd.clone(), env, stdin: true };
    let mut p = proc::spawn(&ctx.app.cli.groups, ctx.kind.title(), &spec, true)?;
    let out = drive(ctx, &mut p, rx, &setup).await;
    p.stop();
    out
}

fn ended(ctx: &Ctx, p: &Proc, code: Option<i32>) -> String {
    let tail = proc::tail_text(&p.tail);
    let code = code.map(|c| format!(" with code {c}")).unwrap_or_default();
    let title = ctx.kind.title();
    if tail.is_empty() { format!("The {title} program stopped{code}.") } else { format!("The {title} program stopped{code}: {tail}") }
}

async fn new_session(ctx: &Ctx, p: &Proc, setup: &Setup, s: &mut Session) -> Result<String, String> {
    let mcp = servers(&setup.mcp);
    let mut params = json!({"cwd": setup.cwd.display().to_string(), "mcpServers": mcp});
    if ctx.kind == Kind::Agy {
        params["_meta"] = json!({"antigravity": {"systemPrompt": {"append": format!("{}\n\n{}", ctx.instructions(), tool_rule(ctx.kind))}}});
    }
    let created = p.rpc.request("session/new", params, REQUEST_MS).await.map_err(|error| {
        if ctx.kind == Kind::Dim && error.to_ascii_lowercase().contains("authentication required") {
            "DimCode needs provider credentials. Set up authentication in DimCode, then retry.".into()
        } else {
            error
        }
    })?;
    let id = created.get("sessionId").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("{} did not open a session.", ctx.kind.title()))?.to_string();
    let (models, current) = read_models(&created, &format!("{} plan", ctx.kind.title()));
    if !models.is_empty() {
        ctx.models(&models);
        s.models = models;
    }
    match s.model.clone() {
        Some(wanted) if wanted != current => {
            if let Err(e) = p.rpc.request("session/set_model", json!({"sessionId": id, "modelId": wanted}), REQUEST_MS).await {
                log("cli-brain", &format!("{} model {wanted} not set, using its default: {e}", ctx.kind.word()));
                s.model = Some(current).filter(|c| !c.is_empty());
            }
        }
        None if !current.is_empty() => s.model = Some(current),
        _ => {}
    }
    s.fresh = ctx.kind != Kind::Agy;
    Ok(id)
}

fn close_session(p: &Proc, id: &str) {
    if id.is_empty() {
        return;
    }
    let rpc = p.rpc.clone();
    let id = id.to_string();
    tokio::spawn(async move {
        let _ = rpc.request("session/close", json!({"sessionId": id}), 5_000).await;
    });
}

fn prompt(ctx: &Ctx, p: &Proc, s: &mut Session, body: String, done: &mpsc::UnboundedSender<Done>) {
    s.turn += 1;
    s.text.clear();
    s.spoke = false;
    let body = if std::mem::take(&mut s.fresh) { format!("{}\n\n{}\n\n{body}", ctx.instructions(), tool_rule(ctx.kind)) } else { body };
    let rpc = p.rpc.clone();
    let params = json!({"sessionId": s.id, "prompt": [{"type": "text", "text": body}]});
    let mine = s.turn;
    let done = done.clone();
    tokio::spawn(async move {
        let result = rpc.request("session/prompt", params, PROMPT_MS).await;
        let _ = done.send((mine, result));
    });
}

fn finish(ctx: &Ctx, s: &mut Session, ok: bool, problem: &str) {
    let last = std::mem::take(&mut s.text);
    let last = last.trim();
    s.spoke = false;
    s.busy = false;
    let title = ctx.kind.title();
    let reply = if ok {
        if last.is_empty() { AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: false } } else { parse_reply(&extract_json(last)) }
    } else if !last.is_empty() {
        AgentReply { failed: true, ..parse_reply(&extract_json(last)) }
    } else if !problem.is_empty() {
        AgentReply::failed(format!("My {title} brain failed. {problem}"))
    } else {
        AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: true }
    };
    ctx.reply(reply);
}

fn after_turn(ctx: &Ctx, p: &Proc, s: &mut Session, done: &mpsc::UnboundedSender<Done>) {
    if s.busy {
        return;
    }
    if s.queue.is_empty() {
        ctx.busy(false);
        return;
    }
    let body = std::mem::take(&mut s.queue).join("\n\n");
    s.busy = true;
    s.refused = false;
    s.retried = false;
    ctx.turn_started();
    prompt(ctx, p, s, body, done);
}

fn turn_ended(ctx: &Ctx, p: &Proc, s: &mut Session, result: Result<Value, String>, done: &mpsc::UnboundedSender<Done>) {
    let row = match result {
        Ok(row) => row,
        Err(e) => {
            log("cli-brain", &format!("{} prompt failed: {e}", ctx.kind.word()));
            finish(ctx, s, false, &e);
            after_turn(ctx, p, s, done);
            return;
        }
    };
    if let Some((input, output, cached)) = row.get("usage").and_then(read_usage) {
        s.context = input + cached;
        ctx.usage(input, output, cached);
        ctx.context(s.context, CONTEXT_LIMIT);
    }
    let reason = row.get("stopReason").and_then(Value::as_str).unwrap_or("");
    if reason == "end_turn" && s.text.trim().is_empty() && s.refused && !s.retried {
        s.retried = true;
        s.refused = false;
        log("cli-brain", &format!("{} asked again after a refused tool", ctx.kind.word()));
        prompt(ctx, p, s, REFUSED_NOTE.into(), done);
        return;
    }
    if reason == "cancelled" {
        s.text.clear();
        s.busy = false;
        after_turn(ctx, p, s, done);
        return;
    }
    if reason != "end_turn" {
        log("cli-brain", &format!("{} turn ended: {reason}", ctx.kind.word()));
    }
    let ok = reason == "end_turn" || !s.text.trim().is_empty();
    let problem = if reason == "refusal" { "It refused.".to_string() } else { format!("It stopped: {}.", if reason.is_empty() { "no reason" } else { reason }) };
    finish(ctx, s, ok, &problem);
    after_turn(ctx, p, s, done);
}

fn on_update(ctx: &Ctx, s: &mut Session, params: &Value) {
    if params.get("sessionId").and_then(Value::as_str) != Some(s.id.as_str()) {
        return;
    }
    let update = params.get("update").cloned().unwrap_or(Value::Null);
    match update.get("sessionUpdate").and_then(Value::as_str).unwrap_or("") {
        "agent_message_chunk" => {
            if update.pointer("/content/type").and_then(Value::as_str) == Some("text")
                && let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                    s.text.push_str(text);
                }
        }
        "tool_call" => {
            if !s.spoke && !s.text.trim().is_empty() {
                s.spoke = true;
                let said = parse_reply(&extract_json(&s.text));
                if said.speak && !said.text.is_empty() {
                    ctx.commentary(&said.text);
                }
            }
            s.text.clear();
        }
        _ => {}
    }
}

async fn trim_if_large(ctx: &Ctx, p: &Proc, setup: &Setup, s: &mut Session) -> String {
    if s.busy || s.context < CONTEXT_LIMIT {
        return String::new();
    }
    let old = s.id.clone();
    match new_session(ctx, p, setup, s).await {
        Ok(id) => {
            log("cli-brain", &format!("{} chat trimmed at {} tokens", ctx.kind.word(), s.context));
            s.id = id;
            s.context = 0;
            close_session(p, &old);
            ctx.trim_note()
        }
        Err(e) => {
            log("cli-brain", &format!("{} trim failed: {e}", ctx.kind.word()));
            String::new()
        }
    }
}

async fn on_cmd(ctx: &Ctx, p: &Proc, setup: &Setup, s: &mut Session, cmd: Cmd, done: &mpsc::UnboundedSender<Done>) -> Result<bool, String> {
    match cmd {
        Cmd::Stop => return Ok(false),
        Cmd::Tell(text) => {
            if s.busy {
                s.queue.push(text);
                return Ok(true);
            }
            let note = trim_if_large(ctx, p, setup, s).await;
            s.busy = true;
            s.refused = false;
            s.retried = false;
            ctx.turn_started();
            prompt(ctx, p, s, format!("{note}{text}"), done);
        }
        Cmd::Interrupt => {
            s.queue.clear();
            if s.busy {
                p.rpc.notify("session/cancel", json!({"sessionId": s.id}));
                s.turn += 1;
                s.busy = false;
                s.text.clear();
            }
        }
        Cmd::Reset => {
            s.queue.clear();
            if s.busy {
                p.rpc.notify("session/cancel", json!({"sessionId": s.id}));
                s.turn += 1;
                s.busy = false;
            }
            let old = s.id.clone();
            s.id = new_session(ctx, p, setup, s).await?;
            s.context = 0;
            s.text.clear();
            close_session(p, &old);
            ctx.model_now(s.model.as_deref());
        }
        Cmd::Model(model) => {
            if let Some(wanted) = &model
                && let Err(e) = p.rpc.request("session/set_model", json!({"sessionId": s.id, "modelId": wanted}), REQUEST_MS).await {
                    ctx.app.info(&format!("{} did not take the model {wanted}: {e}", ctx.kind.title()));
                    return Ok(true);
                }
            s.model = model;
            ctx.model_now(s.model.as_deref());
        }
        Cmd::Models(reply) => {
            let _ = reply.send(Ok(s.models.clone()));
        }
    }
    Ok(true)
}

fn on_message(ctx: &Ctx, p: &Proc, s: &mut Session, message: &Value, names: &[String]) {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match (method, message.get("id")) {
        ("session/update", _) => on_update(ctx, s, &params),
        ("session/request_permission", Some(id)) => {
            let (answer, allow) = permission_answer(&params, names);
            if !allow {
                s.refused = true;
            }
            let title = params.pointer("/toolCall/title").and_then(Value::as_str).unwrap_or("");
            log("cli-brain", &format!("{} tool {title}: {}", ctx.kind.word(), if allow { "allowed" } else { "refused" }));
            p.rpc.reply(id, answer);
        }
        (other, Some(id)) if !other.is_empty() => p.rpc.reply_error(id, &format!("{other} is not supported by Nixa.")),
        _ => {}
    }
}

async fn drive(ctx: &Ctx, p: &mut Proc, rx: &mut UnboundedReceiver<Cmd>, setup: &Setup) -> Result<(), String> {
    let hello = json!({
        "protocolVersion": 1,
        "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
        "clientInfo": {"name": "nixa", "title": "Nixa", "version": VERSION},
    });
    if let Err(e) = p.rpc.request("initialize", hello, REQUEST_MS).await {
        return Err(if e.contains("stopped") { ended(ctx, p, None) } else { e });
    }
    let mut s = Session { id: String::new(), turn: 0, busy: false, text: String::new(), spoke: false, refused: false, retried: false, context: 0, queue: Vec::new(), fresh: false, model: setup.model.clone(), models: Vec::new() };
    s.id = new_session(ctx, p, setup, &mut s).await?;
    ctx.ready(s.model.as_deref());
    let names = ctx.tool_names();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Done>();
    let mut last = Instant::now();
    loop {
        let until = deadline(s.busy, last);
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { return Ok(()) };
                if !on_cmd(ctx, p, setup, &mut s, cmd, &done_tx).await? {
                    return Ok(());
                }
                last = Instant::now();
            }
            Some((turn, result)) = done_rx.recv() => {
                last = Instant::now();
                while let Ok(event) = p.events.try_recv() {
                    match event {
                        Event::Message(message) => on_message(ctx, p, &mut s, &message, &names),
                        Event::Ended(code) => return Err(ended(ctx, p, code)),
                    }
                }
                if turn == s.turn && s.busy {
                    turn_ended(ctx, p, &mut s, result, &done_tx);
                }
            }
            event = p.events.recv() => {
                last = Instant::now();
                match event {
                    Some(Event::Message(message)) => on_message(ctx, p, &mut s, &message, &names),
                    Some(Event::Ended(code)) => return Err(ended(ctx, p, code)),
                    None => return Err(ended(ctx, p, None)),
                }
            }
            _ = tokio::time::sleep_until(until) => {
                p.rpc.notify("session/cancel", json!({"sessionId": s.id}));
                s.turn += 1;
                finish(ctx, &mut s, false, "It stopped answering.");
                after_turn(ctx, p, &mut s, &done_tx);
                last = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["nixa_term_list".into(), "remember".into()]
    }

    #[test]
    fn allows_only_nixa_tools() {
        assert!(tool_allowed(&json!({"title": "Run mcp_nixa_brain_nixa_term_list"}), &names()));
        assert!(tool_allowed(&json!({"title": "nixa_brain: remember"}), &names()));
        assert!(tool_allowed(&json!({"title": "mcp__nixa_brain__nixa_term_list"}), &names()));
        assert!(tool_allowed(&json!({"name": "nixa_term_list"}), &names()));
        assert!(!tool_allowed(&json!({"name": "bash", "title": "nixa_term_list check"}), &names()));
        assert!(!tool_allowed(&json!({"title": "nixa_term_list check"}), &names()));
        assert!(tool_allowed(&json!({"title": "nixa_term_list"}), &names()));
        assert!(!tool_allowed(&json!({"title": "Run nixa_term_list now"}), &names()));
        assert!(!tool_allowed(&json!({"title": "mcp_nixa_brain_nixa_term_list; rm -rf x"}), &names()));
        assert!(tool_allowed(&json!({"title": "mcp_nixa_brain_nixa_call"}), &names()));
        assert!(!tool_allowed(&json!({"title": "Run shell: ls"}), &names()));
        assert!(!tool_allowed(&json!({"title": "Read file remember.txt"}), &names()));
        assert!(!tool_allowed(&json!({}), &names()));
    }

    #[test]
    fn answers_permission() {
        let options = json!([{"kind": "allow_once", "optionId": "a"}, {"kind": "reject_once", "optionId": "r"}]);
        let (yes, allow) = permission_answer(&json!({"toolCall": {"title": "nixa_term_list"}, "options": options}), &names());
        assert!(allow);
        assert_eq!(yes["outcome"]["optionId"], "a");
        let (no, allow) = permission_answer(&json!({"toolCall": {"title": "bash"}, "options": options}), &names());
        assert!(!allow);
        assert_eq!(no["outcome"]["optionId"], "r");
        let (none, _) = permission_answer(&json!({"toolCall": {"title": "bash"}, "options": []}), &names());
        assert_eq!(none["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn agent_tools_are_allowed() {
        let options = json!([{"kind": "allow_once", "optionId": "a"}, {"kind": "reject_once", "optionId": "r"}]);
        let params = json!({"toolCall": {"title": "bash"}, "options": options});
        let (yes, allow) = agent_permission_answer(&params, &names());
        assert!(allow);
        assert_eq!(yes["outcome"]["optionId"], "a");
    }

    #[test]
    fn reads_usage_and_models() {
        assert_eq!(read_usage(&json!({"inputTokens": 5, "outputTokens": 2, "thoughtTokens": 1, "cachedReadTokens": 3})), Some((5, 3, 3)));
        assert_eq!(read_usage(&json!({})), None);
        let (models, current) = read_models(&json!({"models": {"availableModels": [{"modelId": "g1", "name": "G1"}, {"modelId": ""}], "currentModelId": "g1"}}), "Kimi plan");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].description, "Kimi plan");
        assert_eq!(current, "g1");
    }

    #[test]
    fn server_list_shape() {
        let mcp = McpServer { command: "/m".into(), args: vec!["bridge".into()], env: vec![("A".into(), "b".into())] };
        let list = servers(&mcp);
        assert_eq!(list[0]["name"], SERVER);
        assert_eq!(list[0]["env"][0]["name"], "A");
        assert!(tool_rule(Kind::Agy).contains("nixa_call"));
        assert!(!tool_rule(Kind::Kimi).contains("nixa_call"));
    }
}
