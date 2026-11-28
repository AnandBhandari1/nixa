use super::proc::{self, Event, Spec};
use super::{Cmd, Ctx, McpServer, PROMPT_MS, SERVER, deadline};
use crate::brains::supported_effort;
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use crate::sse::SseBuf;
use crate::util::{clip_flat, log};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub const FALLBACK_MODEL: &str = "opencode-go/gpt-6-luna";
pub const NO_LOCAL_MODEL: &str = "Privacy mode is on and no OpenCode model is set, so the cloud default OpenCode Go is not used. Pick a local model for OpenCode (for example an Ollama model) in Settings, Brain, or turn Privacy mode off.";

pub fn model_for(saved: Option<&str>, privacy_on: bool) -> Result<String, String> {
    match saved.map(str::trim).filter(|m| !m.is_empty()) {
        Some(model) => Ok(model.to_string()),
        None if privacy_on => Err(NO_LOCAL_MODEL.into()),
        None => Ok(FALLBACK_MODEL.to_string()),
    }
}
pub const START_MS: u64 = 30_000;
pub const API_MS: u64 = 30_000;
pub const WARM_MS: u64 = 20_000;
pub const CONTEXT_LIMIT: u64 = 60_000;
pub const STOP_MS: u64 = 5_000;
pub const HIDDEN_PROVIDERS: [&str; 2] = ["ollama", "opencode"];

pub fn provider_name(id: &str) -> &str {
    match id {
        "opencode" => "OpenCode Zen",
        "opencode-go" => "OpenCode Go",
        "kimi-code-plan-global" => "Kimi Code plan",
        "zai-coding-plan" => "Z.ai coding plan",
        "openai" => "OpenAI",
        "xai" => "xAI",
        other => other,
    }
}

pub fn split_model(id: &str) -> (String, String) {
    match id.split_once('/') {
        Some((provider, rest)) => (provider.to_string(), rest.to_string()),
        None => ("opencode".to_string(), id.to_string()),
    }
}

pub fn model_ref(id: &str, variant: Option<String>) -> Value {
    let (provider, model) = split_model(id);
    let mut out = json!({"providerID": provider, "id": model});
    if let Some(v) = variant {
        out["variant"] = json!(v);
    }
    out
}

fn cost_text(info: &Value) -> String {
    let list = info.get("cost").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(base) = list.iter().find(|c| c.get("tier").is_none()).or_else(|| list.first()) else { return String::new() };
    let input = base.get("input").and_then(Value::as_f64).unwrap_or(0.0);
    let output = base.get("output").and_then(Value::as_f64).unwrap_or(0.0);
    if input == 0.0 && output == 0.0 { "free".into() } else { format!("price {input} in, {output} out") }
}

pub fn usable(info: &Value) -> bool {
    let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("");
    !text("id").is_empty()
        && !text("providerID").is_empty()
        && !HIDDEN_PROVIDERS.contains(&text("providerID"))
        && info.get("enabled").and_then(Value::as_bool) != Some(false)
        && text("status") != "deprecated"
        && info.get("type").and_then(Value::as_str).unwrap_or("chat") == "chat"
        && info.pointer("/capabilities/tools").and_then(Value::as_bool) != Some(false)
}

pub fn to_model(info: &Value) -> Model {
    let provider = info.get("providerID").and_then(Value::as_str).unwrap_or("");
    let id = info.get("id").and_then(Value::as_str).unwrap_or("");
    let cost = cost_text(info);
    let description = [provider_name(provider).to_string(), cost].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
    let efforts = info
        .get("variants")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|v| v.get("id").and_then(Value::as_str)).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    Model { id: format!("{provider}/{id}"), name: info.get("name").and_then(Value::as_str).unwrap_or(id).to_string(), description, efforts, max_tokens: None }
}

pub fn read_models(body: &Value) -> Vec<Model> {
    let mut list: Vec<Model> = body.get("data").and_then(Value::as_array).map(|l| l.iter().filter(|i| usable(i)).map(to_model).collect()).unwrap_or_default();
    list.sort_by(|a, b| b.description.ends_with("free").cmp(&a.description.ends_with("free")).then(a.name.cmp(&b.name)));
    list
}

pub fn tool_allowed(action: &str) -> bool {
    action.starts_with(&format!("{SERVER}_"))
}

pub fn mcp_connected(body: &Value, name: &str) -> bool {
    let data = body.get("data").unwrap_or(body);
    let entry = match data {
        Value::Array(list) => list.iter().find(|i| i.get("name").and_then(Value::as_str) == Some(name) || i.get("id").and_then(Value::as_str) == Some(name)).cloned(),
        Value::Object(map) => map.get(name).cloned(),
        _ => None,
    };
    let Some(entry) = entry else { return false };
    let status = entry.get("status").or_else(|| entry.get("state")).unwrap_or(&entry).to_string().to_lowercase();
    status.contains("connected") && !status.contains("disconnected") && !status.contains("failed") && !status.contains("error")
}

pub fn config(mcp: &McpServer, instructions: &str, model: &str) -> Value {
    let env: serde_json::Map<String, Value> = mcp.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    let mut command = vec![mcp.command.clone()];
    command.extend(mcp.args.iter().cloned());
    json!({
        "share": "disabled",
        "update": "disable",
        "snapshots": false,
        "lsp": false,
        "formatter": false,
        "websearch": false,
        "default_agent": "nixa",
        "model": model,
        "agents": {
            "nixa": {
                "mode": "primary",
                "description": "Nixa voice brain",
                "system": instructions,
                "permissions": [
                    {"action": "*", "resource": "*", "effect": "deny"},
                    {"action": format!("{SERVER}_*"), "resource": "*", "effect": "ask"}
                ]
            }
        },
        "mcp": {"servers": {SERVER: {"type": "local", "command": command, "environment": env, "codemode": false}}}
    })
}

pub fn token_total(data: &Value) -> (u64, u64, u64) {
    let n = |p: &str| data.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    (n("/tokens/input"), n("/tokens/output") + n("/tokens/reasoning"), n("/tokens/cache/read") + n("/tokens/cache/write"))
}

#[derive(Clone)]
pub(super) struct Api {
    pub(super) http: reqwest::Client,
    pub(super) base: String,
    pub(super) password: String,
}

impl Api {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>, ms: u64) -> Result<Value, String> {
        let mut request = self.http.request(method.clone(), format!("{}{path}", self.base)).basic_auth("opencode", Some(&self.password)).timeout(Duration::from_millis(ms));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|e| format!("OpenCode {method} {path} failed: {e}"))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!("OpenCode {method} {path} failed: {} {}", status.as_u16(), clip_flat(&text, 200)));
        }
        Ok(serde_json::from_str(&text).unwrap_or(json!({})))
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        self.call(reqwest::Method::GET, path, None, API_MS).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.call(reqwest::Method::POST, path, Some(body), API_MS).await
    }

    fn delete_later(&self, path: String) {
        let api = self.clone();
        tokio::spawn(async move {
            let _ = api.call(reqwest::Method::DELETE, &path, None, 5_000).await;
        });
    }
}

async fn listen(api: &Api, tx: &UnboundedSender<Value>, stop: &CancellationToken) -> Result<(), String> {
    let response = api.http.get(format!("{}/api/event", api.base)).basic_auth("opencode", Some(&api.password)).header("accept", "text/event-stream").send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("OpenCode events failed: {}", response.status().as_u16()));
    }
    let mut bytes = response.bytes_stream();
    let mut buf = SseBuf::default();
    loop {
        let chunk = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            next = bytes.next() => next,
        };
        let Some(chunk) = chunk else { return Ok(()) };
        let chunk = chunk.map_err(|e| e.to_string())?;
        for event in buf.push(&chunk) {
            if let Ok(value) = serde_json::from_str::<Value>(&event.data)
                && tx.send(value).is_err() {
                    return Ok(());
                }
        }
    }
}

async fn follow(api: Api, tx: UnboundedSender<Value>, stop: CancellationToken) {
    let mut wait = 500u64;
    while !stop.is_cancelled() && !tx.is_closed() {
        let began = Instant::now();
        if let Err(e) = listen(&api, &tx, &stop).await
            && !stop.is_cancelled() {
                log("cli-brain", &format!("opencode events stopped, connecting again: {e}"));
            }
        let _ = tx.send(json!({"type": "nixa.event_stream_gap"}));
        wait = if began.elapsed() > Duration::from_secs(30) { 500 } else { (wait * 2).min(10_000) };
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
        }
    }
}

struct Turn {
    session: String,
    turn: u64,
    busy: bool,
    texts: Vec<String>,
    request: String,
    tool_seen: bool,
    spoke: bool,
    context: u64,
    total: u64,
    previous: (u64, u64, u64),
    queue: Vec<String>,
    model: String,
    models: Vec<Model>,
    stopping: bool,
}

type Done = (u64, Result<Value, String>);

pub async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let model = model_for(setup.model.as_deref(), crate::privacy::privacy_on(&ctx.app))?;
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).map_err(|e| e.to_string())?;
    let password = super::new_token();
    let env = vec![
        ("OPENCODE_SERVER_PASSWORD".to_string(), password.clone()),
        ("OPENCODE_CONFIG_CONTENT".to_string(), config(&setup.mcp, &ctx.instructions(), &model).to_string()),
        ("OPENCODE_DISABLE_CLAUDE_CODE".to_string(), "1".to_string()),
        ("OPENCODE_DISABLE_PROJECT_CONFIG".to_string(), "1".to_string()),
        ("OPENCODE_DISABLE_AUTOUPDATE".to_string(), "1".to_string()),
    ];
    let spec = Spec { bin: setup.bin.clone(), args: vec!["serve".into(), "--port".into(), port.to_string(), "--hostname".into(), "127.0.0.1".into()], cwd: setup.cwd.clone(), env, stdin: false };
    let mut p = proc::spawn(&ctx.app.cli.groups, "OpenCode", &spec, false)?;
    let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().unwrap_or_default();
    let api = Api { http, base: format!("http://127.0.0.1:{port}"), password };
    let stop = CancellationToken::new();
    let out = drive(ctx, &mut p, rx, &api, &stop, model, &setup.cwd).await;
    {
        let mut live = crate::app::lock(&ctx.app.cli.live);
        if live.opencode.as_ref().is_some_and(|(base, _)| *base == api.base) {
            live.opencode = None;
        }
    }
    stop.cancel();
    p.stop();
    out
}

fn ended(p: &proc::Proc, code: Option<i32>) -> String {
    let tail = proc::tail_text(&p.tail);
    let code = code.map(|c| format!(" with code {c}")).unwrap_or_default();
    if tail.is_empty() { format!("The OpenCode server stopped{code}.") } else { format!("The OpenCode server stopped{code}: {tail}") }
}

pub(super) async fn wait_up(api: &Api, p: &mut proc::Proc) -> Result<(), String> {
    let until = Instant::now() + Duration::from_millis(START_MS);
    while Instant::now() < until {
        if let Ok(Event::Ended(code)) = p.events.try_recv() {
            return Err(ended(p, code));
        }
        if api.get("/api/model/default").await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("The OpenCode server did not start in time.".into())
}

async fn warm_tools(api: &Api) {
    if let Err(e) = api.post(&format!("/api/experimental/mcp/{SERVER}/connect"), json!({})).await {
        log("cli-brain", &format!("opencode tool connect failed: {e}"));
    }
    let until = Instant::now() + Duration::from_millis(WARM_MS);
    while Instant::now() < until {
        if api.get("/api/mcp").await.is_ok_and(|body| mcp_connected(&body, SERVER)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    log("cli-brain", "opencode tools were not ready in time");
}

fn variant(ctx: &Ctx, t: &Turn) -> Option<String> {
    supported_effort(t.models.iter().find(|m| m.id == t.model), ctx.effort(Some(&t.model)).as_deref())
}

async fn new_session(ctx: &Ctx, api: &Api, t: &Turn, cwd: &std::path::Path) -> Result<String, String> {
    let body = json!({"agent": "nixa", "model": model_ref(&t.model, variant(ctx, t)), "title": "Nixa", "location": {"directory": cwd.display().to_string()}});
    let created = api.post("/api/session", body).await?;
    created.pointer("/data/id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| "OpenCode did not create a session.".to_string())
}

fn finish(ctx: &Ctx, t: &mut Turn, ok: bool, problem: &str) {
    let last = t.texts.pop().unwrap_or_default();
    t.texts.clear();
    t.spoke = false;
    t.busy = false;
    let quiet = !ok && !crate::agent::from_user(&t.request);
    let reply = if !last.trim().is_empty() {
        AgentReply { failed: !ok, ..parse_reply(&extract_json(&last)) }
    } else if !ok && !problem.is_empty() {
        AgentReply::failed(format!("My OpenCode brain failed. {problem}"))
    } else {
        AgentReply { calls: Vec::new(), speak: false, text: String::new(), heard: "me".into(), failed: !ok }
    };
    ctx.reply(if quiet { AgentReply { speak: false, ..reply } } else { reply });
}

async fn recover_failure(ctx: &Ctx, turn: &mut Turn, problem: &str, terminal_failure: bool) {
    use crate::agent::recovery;
    if !ctx.mine() { return; }
    crate::util::log("cli-brain", &format!("opencode turn failed: {}", crate::util::clip_flat(problem, 300)));
    if terminal_failure && !recovery::retryable(None, problem) && !turn.tool_seen {
        finish(ctx, turn, false, problem);
        return;
    }
    let cancel = CancellationToken::new();
    crate::app::lock(&ctx.app.brain).cancel = Some(cancel.clone());
    turn.busy = false;
    turn.texts.clear();
    let uncertain = recovery::outcome_uncertain(problem, terminal_failure, turn.tool_seen);
    if uncertain { turn.queue.clear(); }
    let reply = recovery::recover(&ctx.app, None, &turn.request, uncertain, &cancel).await;
    crate::app::lock(&ctx.app.brain).cancel = None;
    if !cancel.is_cancelled() { ctx.reply(reply); }
}

fn prompt(api: &Api, t: &mut Turn, body: String, done: &UnboundedSender<Done>) {
    t.turn += 1;
    t.texts.clear();
    t.request.clone_from(&body);
    t.tool_seen = false;
    t.spoke = false;
    t.busy = true;
    let api = api.clone();
    let path = format!("/api/session/{}/prompt", t.session);
    let mine = t.turn;
    let done = done.clone();
    tokio::spawn(async move {
        let result = api.call(reqwest::Method::POST, &path, Some(json!({"text": body})), PROMPT_MS).await;
        let _ = done.send((mine, result));
    });
}

fn make_way(t: &mut Turn, text: String) -> bool {
    if !t.busy || t.stopping || !super::make_way(Some(crate::agent::from_user(&t.request)), t.busy, &text) {
        return false;
    }
    t.stopping = true;
    t.queue.insert(0, text);
    true
}

fn settle(t: &mut Turn) {
    log("cli-brain", "opencode dropped a background turn for a user message");
    t.turn += 1;
    t.stopping = false;
    t.busy = false;
    t.spoke = false;
    t.texts.clear();
}

fn after_turn(ctx: &Ctx, api: &Api, t: &mut Turn, done: &UnboundedSender<Done>) {
    if t.busy {
        return;
    }
    if t.queue.is_empty() {
        ctx.busy(false);
        return;
    }
    let body = std::mem::take(&mut t.queue).join("\n\n");
    ctx.turn_started();
    prompt(api, t, body, done);
}

async fn on_event(ctx: &Ctx, api: &Api, t: &mut Turn, event: &Value, done: &UnboundedSender<Done>) {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    let data = event.get("data").or_else(|| event.get("properties")).cloned().unwrap_or(Value::Null);
    if kind == "nixa.event_stream_gap" {
        if t.busy { t.tool_seen = true; }
        return;
    }
    if kind == "permission.asked" {
        let id = data.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() || data.get("sessionID").and_then(Value::as_str) != Some(t.session.as_str()) {
            return;
        }
        t.tool_seen = true;
        let action = data.get("action").and_then(Value::as_str).unwrap_or("");
        let allow = tool_allowed(action);
        log("cli-brain", &format!("opencode tool {action}: {}", if allow { "allowed" } else { "refused" }));
        let path = format!("/api/session/{}/permission/{id}/reply", t.session);
        if let Err(e) = api.post(&path, json!({"decision": if allow { "once" } else { "reject" }})).await {
            log("cli-brain", &format!("opencode permission reply failed: {e}"));
        }
        return;
    }
    if data.get("sessionID").and_then(Value::as_str) != Some(t.session.as_str()) || !t.busy {
        return;
    }
    if t.stopping && kind.starts_with("session.execution.") {
        settle(t);
        after_turn(ctx, api, t, done);
        return;
    }
    match kind {
        "session.text.ended" => {
            if let Some(text) = data.get("text").and_then(Value::as_str) {
                t.texts.push(text.to_string());
            }
        }
        "session.usage.updated" => {
            let total = token_total(&data);
            let size = data.pointer("/tokens/input").and_then(Value::as_u64).unwrap_or(0) + total.2;
            let step = if size >= t.total { size - t.total } else { size };
            t.total = size;
            if step > 0 {
                t.context = step;
                ctx.context(step, CONTEXT_LIMIT);
            }
            let delta = (total.0.saturating_sub(t.previous.0), total.1.saturating_sub(t.previous.1), total.2.saturating_sub(t.previous.2));
            t.previous = total;
            ctx.usage(delta.0, delta.1, delta.2);
        }
        "session.tool.called" => {
            t.tool_seen = true;
            if !t.spoke
                && let Some(last) = t.texts.last() {
                    t.spoke = true;
                    let said = parse_reply(&extract_json(last));
                    if said.speak && !said.text.is_empty() {
                        ctx.commentary(&said.text);
                    }
                }
        }
        "session.execution.succeeded" => {
            finish(ctx, t, true, "");
            after_turn(ctx, api, t, done);
        }
        "session.execution.failed" => {
            let why = data.pointer("/error/message").and_then(Value::as_str).unwrap_or("").to_string();
            recover_failure(ctx, t, &why, true).await;
            after_turn(ctx, api, t, done);
        }
        "session.execution.interrupted" | "session.execution.cancelled" => {
            t.texts.clear();
            t.busy = false;
            after_turn(ctx, api, t, done);
        }
        _ => {}
    }
}

async fn trim_if_large(ctx: &Ctx, api: &Api, t: &mut Turn, cwd: &std::path::Path) -> String {
    if t.busy || t.context < CONTEXT_LIMIT {
        return String::new();
    }
    match new_session(ctx, api, t, cwd).await {
        Ok(fresh) => {
            api.delete_later(format!("/api/session/{}", t.session));
            t.session = fresh;
            t.context = 0;
            t.total = 0;
            t.previous = (0, 0, 0);
            ctx.trim_note()
        }
        Err(e) => {
            log("cli-brain", &format!("opencode trim failed: {e}"));
            String::new()
        }
    }
}

async fn list_models(api: &Api) -> Result<Vec<Model>, String> {
    Ok(read_models(&api.get("/api/model").await?))
}

async fn on_cmd(ctx: &Ctx, api: &Api, t: &mut Turn, cmd: Cmd, done: &UnboundedSender<Done>, cwd: &std::path::Path) -> Result<bool, String> {
    match cmd {
        Cmd::Stop => return Ok(false),
        Cmd::Tell(text) => {
            if make_way(t, text.clone()) {
                if let Err(e) = api.post(&format!("/api/session/{}/interrupt", t.session), json!({})).await {
                    log("cli-brain", &format!("opencode interrupt failed: {e}"));
                }
                return Ok(true);
            }
            if t.busy {
                t.queue.push(text);
                return Ok(true);
            }
            let note = trim_if_large(ctx, api, t, cwd).await;
            ctx.turn_started();
            prompt(api, t, format!("{note}{text}"), done);
        }
        Cmd::Interrupt => {
            t.queue.clear();
            t.stopping = false;
            if t.busy {
                let _ = api.post(&format!("/api/session/{}/interrupt", t.session), json!({})).await;
                t.turn += 1;
                t.busy = false;
                t.texts.clear();
            }
        }
        Cmd::Reset => {
            t.queue.clear();
            t.stopping = false;
            if t.busy {
                let _ = api.post(&format!("/api/session/{}/interrupt", t.session), json!({})).await;
                t.turn += 1;
                t.busy = false;
            }
            let fresh = new_session(ctx, api, t, cwd).await?;
            api.delete_later(format!("/api/session/{}", t.session));
            t.session = fresh;
            t.context = 0;
            t.total = 0;
            t.previous = (0, 0, 0);
            t.texts.clear();
        }
        Cmd::Model(model) => {
            let wanted = model.unwrap_or_else(|| FALLBACK_MODEL.to_string());
            if t.models.is_empty() {
                t.models = list_models(api).await.unwrap_or_default();
            }
            let old = std::mem::replace(&mut t.model, wanted.clone());
            if let Err(e) = api.post(&format!("/api/session/{}/model", t.session), json!({"model": model_ref(&wanted, variant(ctx, t))})).await {
                t.model = old;
                ctx.app.info(&format!("OpenCode did not take the model {wanted}: {e}"));
            }
            ctx.model_now(Some(&t.model));
        }
        Cmd::Models(reply) => {
            let listed = list_models(api).await;
            if let Ok(list) = &listed {
                ctx.models(list);
                t.models = list.clone();
            }
            let _ = reply.send(listed);
        }
    }
    Ok(true)
}

async fn drive(ctx: &Ctx, p: &mut proc::Proc, rx: &mut UnboundedReceiver<Cmd>, api: &Api, stop: &CancellationToken, model: String, cwd: &std::path::Path) -> Result<(), String> {
    wait_up(api, p).await?;
    crate::app::lock(&ctx.app.cli.live).opencode = Some((api.base.clone(), api.password.clone()));
    let (event_tx, mut events) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(follow(api.clone(), event_tx, stop.clone()));
    warm_tools(api).await;
    let mut t = Turn { session: String::new(), turn: 0, busy: false, texts: Vec::new(), request: String::new(), tool_seen: false, spoke: false, context: 0, total: 0, previous: (0, 0, 0), queue: Vec::new(), model, models: Vec::new(), stopping: false };
    if ctx.effort(Some(&t.model)).is_some() {
        t.models = list_models(api).await.unwrap_or_default();
    }
    t.session = new_session(ctx, api, &t, cwd).await?;
    ctx.ready(Some(&t.model));
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Done>();
    let mut last = Instant::now();
    let out = loop {
        let until = if t.stopping { last + Duration::from_millis(STOP_MS) } else { deadline(t.busy, last) };
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break Ok(()) };
                match on_cmd(ctx, api, &mut t, cmd, &done_tx, cwd).await {
                    Ok(true) => {}
                    Ok(false) => break Ok(()),
                    Err(e) => break Err(e),
                }
                last = Instant::now();
            }
            Some((turn, result)) = done_rx.recv() => {
                if turn == t.turn && t.stopping && result.is_err() {
                    settle(&mut t);
                    after_turn(ctx, api, &mut t, &done_tx);
                } else if let Err(e) = result
                    && turn == t.turn && t.busy {
                        recover_failure(ctx, &mut t, &e, false).await;
                        after_turn(ctx, api, &mut t, &done_tx);
                    }
            }
            Some(event) = events.recv() => {
                last = Instant::now();
                on_event(ctx, api, &mut t, &event, &done_tx).await;
            }
            event = p.events.recv() => match event {
                Some(Event::Ended(code)) => {
                    if t.busy {
                        recover_failure(ctx, &mut t, "The OpenCode process exited.", false).await;
                        ctx.busy(false);
                    }
                    break Err(ended(p, code));
                }
                None => {
                    if t.busy {
                        recover_failure(ctx, &mut t, "The OpenCode connection closed.", false).await;
                        ctx.busy(false);
                    }
                    break Err(ended(p, None));
                }
                Some(Event::Message(_)) => {}
            },
            _ = tokio::time::sleep_until(until) => {
                if t.stopping {
                    settle(&mut t);
                } else {
                    let _ = api.post(&format!("/api/session/{}/interrupt", t.session), json!({})).await;
                    t.turn += 1;
                    recover_failure(ctx, &mut t, "It stopped answering.", false).await;
                }
                after_turn(ctx, api, &mut t, &done_tx);
                last = Instant::now();
            }
        }
    };
    if !t.session.is_empty() {
        let _ = api.call(reqwest::Method::DELETE, &format!("/api/session/{}", t.session), None, 3_000).await;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_refs_models() {
        assert_eq!(split_model("opencode-go/gpt-x"), ("opencode-go".into(), "gpt-x".into()));
        assert_eq!(model_for(None, true).unwrap_err(), NO_LOCAL_MODEL);
        assert_eq!(model_for(Some("  "), true).unwrap_err(), NO_LOCAL_MODEL);
        assert_eq!(model_for(Some("ollama/qwen3"), true).unwrap(), "ollama/qwen3");
        assert_eq!(model_for(None, false).unwrap(), FALLBACK_MODEL);
        assert_eq!(split_model("solo"), ("opencode".into(), "solo".into()));
        assert_eq!(model_ref("a/b", Some("high".into()))["variant"], "high");
        assert!(model_ref("a/b", None).get("variant").is_none());
    }

    #[test]
    fn reads_model_list() {
        let body = json!({"data": [
            {"id": "m0", "providerID": "opencode", "name": "Zen"},
            {"id": "m1", "providerID": "opencode-go", "name": "Zed", "cost": [{"input": 0, "output": 0}], "variants": [{"id": "high"}]},
            {"id": "m2", "providerID": "openai", "name": "Alpha", "cost": [{"input": 1, "output": 2}], "variants": [{"id": "high"}]},
            {"id": "m3", "providerID": "ollama", "name": "Hidden"},
            {"id": "m6", "providerID": "anthropic", "name": "Cloud"},
            {"id": "m4", "providerID": "openai", "status": "deprecated"},
            {"id": "m5", "providerID": "openai", "capabilities": {"tools": false}}
        ]});
        let models = read_models(&body);
        let mut ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["anthropic/m6", "openai/m2", "opencode-go/m1"]);
        let go = models.iter().find(|m| m.id == "opencode-go/m1").unwrap();
        assert_eq!(go.description, "OpenCode Go, free");
        assert_eq!(go.efforts, vec!["high".to_string()]);
    }

    #[test]
    fn tools_and_mcp_state() {
        assert!(tool_allowed("nixa_brain_nixa_term_list"));
        assert!(!tool_allowed("bash"));
        assert!(mcp_connected(&json!({"data": {SERVER: {"status": "connected"}}}), SERVER));
        assert!(!mcp_connected(&json!({"data": [{"name": SERVER, "status": "failed"}]}), SERVER));
        assert!(!mcp_connected(&json!({"data": [{"name": SERVER, "status": "disconnected"}]}), SERVER));
        assert!(!mcp_connected(&json!({}), SERVER));
    }

    #[test]
    fn config_shape() {
        let mcp = McpServer { command: "/m".into(), args: vec!["bridge".into()], env: vec![("K".into(), "v".into())] };
        let c = config(&mcp, "Be Nixa.", "a/b");
        assert_eq!(c["mcp"]["servers"][SERVER]["command"], json!(["/m", "bridge"]));
        assert_eq!(c["mcp"]["servers"][SERVER]["environment"]["K"], "v");
        assert_eq!(c["agents"]["nixa"]["permissions"][1]["action"], format!("{SERVER}_*"));
        assert_eq!(token_total(&json!({"tokens": {"input": 3, "output": 2, "reasoning": 1, "cache": {"read": 4}}})), (3, 3, 4));
    }

    fn turn(request: &str, busy: bool) -> Turn {
        Turn { session: "s".into(), turn: 3, busy, texts: vec!["Check-in text".into()], request: request.into(), tool_seen: false, spoke: false, context: 0, total: 0, previous: (0, 0, 0), queue: vec!["Event: later".into()], model: "a/b".into(), models: Vec::new(), stopping: false }
    }

    #[test]
    fn a_user_message_goes_first_and_the_check_in_is_dropped() {
        let mut t = turn("Event: Check-in. Do this now", true);
        assert!(make_way(&mut t, "Typed: \"ok\"".into()));
        assert!(t.stopping);
        assert_eq!(t.queue[0], "Typed: \"ok\"");
        assert!(!make_way(&mut t, "Typed: \"again\"".into()));
        settle(&mut t);
        assert!(!t.busy && !t.stopping && t.texts.is_empty());
        assert_eq!(t.turn, 4);
        let mut user = turn("Typed: \"run the tests\"", true);
        assert!(!make_way(&mut user, "Typed: \"ok\"".into()));
        let mut idle = turn("Event: Check-in", false);
        assert!(!make_way(&mut idle, "Typed: \"ok\"".into()));
        let mut event = turn("Event: Check-in", true);
        assert!(!make_way(&mut event, "Event: Nixa was idle".into()));
    }
}
