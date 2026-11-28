use super::proc::{self, Spec};
use super::{Cmd, Ctx, McpServer, PROMPT_MS, REQUEST_MS, SERVER};
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

const WORKSPACE_SCOPE: u8 = 1;
const DISABLED_TOOLS: &[&str] = &[
    "agent",
    "agent_dispatch",
    "agent_send",
    "agent_status",
    "agent_stop",
    "bash",
    "crush_info",
    "crush_logs",
    "job_output",
    "job_kill",
    "download",
    "edit",
    "multiedit",
    "lsp_diagnostics",
    "lsp_references",
    "lsp_restart",
    "lsp_symbols",
    "lsp_definition",
    "lsp_call_hierarchy",
    "lsp_rename",
    "lsp_replace_symbol",
    "fetch",
    "agentic_fetch",
    "glob",
    "grep",
    "ls",
    "question",
    "sourcegraph",
    "todos",
    "view",
    "write",
    "list_mcp_resources",
    "read_mcp_resource",
];

struct Api {
    http: Client,
    root: String,
    client: String,
}

impl Api {
    async fn request(&self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("http://localhost/v1{}{path}", self.root);
        let request = match body {
            Some(body) => self.http.post(url).json(&body),
            None => self.http.get(url),
        };
        let res = request
            .timeout(Duration::from_millis(REQUEST_MS))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(|e| e.to_string())?;
        let data = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string())?
        };
        if !status.is_success() {
            let message = data["message"]
                .as_str()
                .or_else(|| data["error"].as_str())
                .unwrap_or("the request was refused");
            return Err(format!("Crush returned {status}: {message}"));
        }
        Ok(data)
    }

    async fn events(&self) -> Result<Response, String> {
        tokio::time::timeout(
            Duration::from_millis(REQUEST_MS),
            self.http
                .get(format!(
                    "http://localhost/v1{}/events?client_id={}",
                    self.root, self.client
                ))
                .send(),
        )
        .await
        .map_err(|_| "Crush did not open its event stream.".to_string())?
        .and_then(Response::error_for_status)
        .map_err(|e| e.to_string())
    }

    async fn session(&self) -> Result<String, String> {
        let row = self
            .request("/sessions", Some(json!({"title": "Nixa Chat"})))
            .await?;
        let id = path_id(&row["id"]).ok_or("Crush did not return a session ID.")?;
        self.request(
            &format!("/current-session?client_id={}", self.client),
            Some(json!({"session_id": id})),
        )
        .await?;
        Ok(id.to_string())
    }

    async fn check_tools(&self) -> Result<(), String> {
        let config = self.request("/config", None).await?;
        let disabled = config
            .pointer("/options/disabled_tools")
            .and_then(Value::as_array);
        let all_off = disabled.is_some_and(|list| {
            DISABLED_TOOLS
                .iter()
                .all(|name| list.iter().any(|value| value.as_str() == Some(name)))
        });
        let external = config["mcp"]
            .as_object()
            .is_some_and(|mcp| mcp.keys().any(|k| k != SERVER));
        if !all_off || external {
            return Err("Crush did not apply the Nixa MCP-only tool settings.".into());
        }
        Ok(())
    }

    async fn models(&self) -> Result<Vec<Model>, String> {
        let providers = self.request("/providers", None).await?;
        let config = self.request("/config", None).await?;
        let configured = config["providers"]
            .as_object()
            .ok_or("Crush has no configured providers.")?;
        let providers = providers
            .as_array()
            .or_else(|| providers["providers"].as_array())
            .ok_or("Crush did not return its providers.")?;
        Ok(providers
            .iter()
            .filter_map(|provider| {
                let id = provider["id"].as_str()?;
                configured.contains_key(id).then_some((id, provider))
            })
            .flat_map(|(provider, row)| {
                row["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(move |model| {
                        let id = model["id"].as_str()?;
                        Some(Model {
                            id: format!("{provider}/{id}"),
                            name: model["name"].as_str().unwrap_or(id).into(),
                            description: provider.into(),
                            efforts: Vec::new(),
                            max_tokens: model["context_window"]
                                .as_u64()
                                .and_then(|n| u32::try_from(n).ok()),
                        })
                    })
            })
            .collect())
    }

    async fn select(&self, wanted: &str) -> Result<(), String> {
        let (provider, model) = wanted
            .split_once('/')
            .ok_or("Choose a Crush model from the model list.")?;
        self.request(
            "/config/model",
            Some(json!({"scope": WORKSPACE_SCOPE, "model_type": "large",
            "model": {"provider": provider, "model": model}})),
        )
        .await?;
        self.request("/agent/update", Some(json!({}))).await?;
        Ok(())
    }

    async fn cancel(&self, session: &str) -> Result<(), String> {
        self.request(
            &format!("/agent/sessions/{session}/cancel"),
            Some(json!({})),
        )
        .await?;
        Ok(())
    }
}

fn id() -> String {
    let token = super::new_token();
    format!(
        "{}-{}-4{}-8{}-{}",
        &token[..8],
        &token[8..12],
        &token[12..15],
        &token[15..18],
        &token[18..30]
    )
}

fn path_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

fn mcp_config(mcp: &McpServer) -> Value {
    let env: serde_json::Map<String, Value> = mcp
        .env
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    json!({
        (SERVER): {
            "type": "stdio",
            "command": mcp.command,
            "args": mcp.args,
            "env": env,
        }
    })
}

fn tool_rule() -> String {
    format!("Here you have no shell, no file tools and no web tools of your own: they are disabled. Use only the Nixa tools of the {SERVER} MCP server. To run tests, read files or change code, send the work to a coding agent with nixa_term_send, or read an agent's screen with nixa_term_read.\n\nReply with only the JSON object {{\"speak\": boolean, \"text\": string, \"heard\": \"me\" | \"unfinished\"}}. No other words, no code fences.")
}

pub(super) async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let cwd = setup.cwd.join("crush");
    nixa_protocol::paths::ensure_private_dir(&cwd).map_err(|e| e.to_string())?;
    let profile = cwd.join("config");
    nixa_protocol::paths::ensure_private_dir(&profile).map_err(|e| e.to_string())?;
    let allowed: Vec<String> = ctx
        .tool_names()
        .into_iter()
        .map(|name| format!("mcp_{SERVER}_{name}"))
        .collect();
    crate::util::write_json(
        &profile.join("crush.json"),
        &json!({
            "options": {"disabled_tools": DISABLED_TOOLS, "disable_provider_auto_update": true},
            "mcp": mcp_config(&setup.mcp),
            "permissions": {"allowed_tools": allowed},
            "lsp": {},
        }),
    )
    .map_err(|e| e.to_string())?;
    let socket = cwd.join(format!("c-{}-{}.sock", std::process::id(), ctx.id));
    let spec = Spec {
        bin: setup.bin,
        args: vec![
            "server".into(),
            "--host".into(),
            format!("unix://{}", socket.display()),
        ],
        cwd: cwd.clone(),
        env: vec![("CRUSH_GLOBAL_CONFIG".into(), profile.display().to_string())],
        stdin: false,
    };
    let mut process = proc::spawn(&ctx.app.cli.groups, ctx.kind.title(), &spec, false)?;
    let out = match connect(&socket, &cwd).await {
        Ok(api) => tokio::select! {
            result = drive(ctx, rx, &api, setup.model) => result,
            _ = async {
                while let Some(proc::Event::Message(_)) = process.events.recv().await {}
            } => Err("The Crush server stopped.".into()),
        },
        Err(error) => Err(error),
    };
    process.stop();
    let _ = std::fs::remove_file(&socket);
    out
}

async fn connect(socket: &Path, cwd: &Path) -> Result<Api, String> {
    let http = Client::builder()
        .unix_socket(socket.to_path_buf())
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let until = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        if Instant::now() >= until {
            return Err("Crush did not start its private server in time.".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut api = Api {
        http,
        root: String::new(),
        client: id(),
    };
    let row = api
        .request(
            "/workspaces",
            Some(json!({"path": cwd, "id": id(), "client_id": api.client,
        "data_dir": cwd.join("data"), "permission_mode": "yolo"})),
        )
        .await?;
    let workspace = path_id(&row["id"]).ok_or("Crush did not return a workspace ID.")?;
    api.root = format!("/workspaces/{workspace}");
    Ok(api)
}

struct Turn {
    id: String,
    until: Instant,
    cancelled: bool,
}

fn event_reply(row: &Value, id: &str, session: &str) -> Option<AgentReply> {
    if row["type"] != "run_complete" {
        return None;
    }
    let data = row.pointer("/payload/payload")?;
    if data["run_id"] != id || data["session_id"] != session {
        return None;
    }
    if data["cancelled"] == true {
        return Some(AgentReply::said(crate::agent::STOPPED));
    }
    if let Some(error) = data["error"].as_str().filter(|s| !s.is_empty()) {
        return Some(AgentReply::failed(error.to_string()));
    }
    Some(parse_reply(&extract_json(
        data["text"].as_str().unwrap_or(""),
    )))
}

async fn drive(
    ctx: &Ctx,
    rx: &mut UnboundedReceiver<Cmd>,
    api: &Api,
    saved: Option<String>,
) -> Result<(), String> {
    let mut events = api.events().await?;
    api.check_tools().await?;
    api.request("/agent/init", Some(json!({"interactive": false})))
        .await?;
    let mut session = api.session().await?;
    let mut models = api.models().await?;
    ctx.models(&models);
    let state = api.request("/agent", None).await?;
    let current = &state["model_cfg"];
    let default = current["provider"]
        .as_str()
        .zip(current["model"].as_str())
        .map(|(p, m)| format!("{p}/{m}"));
    ctx.ready(default.as_deref());
    let mut wanted = saved.filter(|m| default.as_ref() != Some(m));
    let mut queue = VecDeque::<String>::new();
    let mut bytes = Vec::<u8>::new();
    let mut turn: Option<Turn> = None;
    loop {
        if turn.is_none() {
            if let Some(model) = wanted.take() {
                match api.select(&model).await {
                    Ok(()) => ctx.model_now(Some(&model)),
                    Err(error) => ctx
                        .app
                        .info(&format!("Crush did not select {model}: {error}")),
                }
            }
            if let Some(body) = queue.pop_front() {
                let run = id();
                let prompt = format!(
                    "{}\n\n{}\n\nChat message:\n{body}",
                    ctx.instructions(),
                    tool_rule()
                );
                ctx.turn_started();
                api.request(
                    "/agent",
                    Some(json!({"session_id": session, "run_id": run, "prompt": prompt})),
                )
                .await?;
                turn = Some(Turn {
                    id: run,
                    until: Instant::now() + Duration::from_millis(PROMPT_MS),
                    cancelled: false,
                });
            }
        }
        let until = turn
            .as_ref()
            .map(|turn| turn.until)
            .unwrap_or_else(|| super::deadline(false, Instant::now()));
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None | Some(Cmd::Stop) => return Ok(()),
                Some(Cmd::Tell(body)) => queue.push_back(body),
                Some(Cmd::Interrupt) => {
                    queue.clear();
                    if let Some(turn) = &mut turn {
                        turn.cancelled = true;
                        api.cancel(&session).await?;
                    }
                }
                Some(Cmd::Reset) => {
                    if turn.take().is_some() {
                        api.cancel(&session).await?;
                    }
                    queue.clear();
                    session = api.session().await?;
                    ctx.busy(false);
                }
                Some(Cmd::Model(model)) => wanted = model,
                Some(Cmd::Models(answer)) => {
                    let result = if turn.is_none() {
                        api.models().await.inspect(|list| { models.clone_from(list); ctx.models(list); })
                    } else {
                        Ok(models.clone())
                    };
                    let _ = answer.send(result);
                }
            },
            chunk = events.chunk() => {
                let part = chunk.map_err(|e| e.to_string())?.ok_or("The Crush event stream closed.")?;
                bytes.extend_from_slice(&part);
                let mut offset = 0;
                while let Some(end) = bytes[offset..].iter().position(|byte| *byte == b'\n') {
                    let line = &bytes[offset..offset + end];
                    offset += end + 1;
                    if let Some(active) = &turn
                        && let Some(data) = line.strip_prefix(b"data: ")
                        && let Ok(row) = serde_json::from_slice::<Value>(data)
                        && let Some(reply) = event_reply(&row, &active.id, &session) {
                        if !active.cancelled {
                            ctx.reply(reply);
                        }
                        turn = None;
                        if queue.is_empty() {
                            ctx.busy(false);
                        }
                    }
                }
                bytes.drain(..offset);
                if bytes.len() > proc::LINE_MAX as usize {
                    return Err("Crush sent an oversized event.".into());
                }
            },
            _ = tokio::time::sleep_until(until), if turn.is_some() => {
                return Err("Crush did not finish the reply in time.".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serves_only_the_nixa_mcp_server() {
        let mcp = McpServer { command: "/x/nixa-mcp".into(), args: vec!["bridge".into()], env: vec![("NIXA_TOOL_TOKEN".into(), "t".into())] };
        let c = mcp_config(&mcp);
        assert_eq!(c.as_object().unwrap().len(), 1);
        assert_eq!(c[SERVER]["type"], "stdio");
        assert_eq!(c[SERVER]["command"], "/x/nixa-mcp");
        assert_eq!(c[SERVER]["args"], json!(["bridge"]));
        assert_eq!(c[SERVER]["env"]["NIXA_TOOL_TOKEN"], "t");
        assert!(tool_rule().contains(SERVER));
    }

    #[test]
    fn accepts_only_the_current_run_completion() {
        let event = json!({"type": "run_complete", "payload": {"type": "created", "payload": {
            "run_id": "one", "session_id": "two", "text": "{\"speak\":false,\"text\":\"Ready\"}"
        }}});
        assert!(event_reply(&event, "other", "two").is_none());
        assert!(event_reply(&event, "one", "other").is_none());
        let reply = event_reply(&event, "one", "two").unwrap();
        assert_eq!(reply.text, "Ready");
        assert!(!reply.speak);
    }
}
