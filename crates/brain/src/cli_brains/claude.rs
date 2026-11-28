use super::proc::{self, Event, Proc, Spec};
use super::{Cmd, Ctx, PROMPT_MS, SERVER, Setup};
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

fn models() -> Vec<Model> {
    [
        ("sonnet", "Claude Sonnet"),
        ("opus", "Claude Opus"),
        ("haiku", "Claude Haiku"),
    ]
    .into_iter()
    .map(|(id, name)| Model {
        id: id.into(),
        name: name.into(),
        description: "Claude Code model alias".into(),
        efforts: Vec::new(),
        max_tokens: None,
    })
    .collect()
}

fn spec(setup: &Setup, model: Option<&str>) -> Spec {
    let mut args = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--no-session-persistence",
        "--restricted",
        "--tools",
        "",
        "--permission-mode",
        "dontAsk",
        "--disable-slash-commands",
        "--strict-mcp-config",
    ]
    .map(str::to_string)
    .to_vec();
    let mcp = &setup.mcp;
    let env: serde_json::Map<String, Value> = mcp
        .env
        .iter()
        .map(|(key, value)| (key.clone(), json!(value)))
        .collect();
    let config = json!({"mcpServers": {(SERVER): {"type":"stdio", "command": mcp.command, "args": mcp.args, "env": env}}});
    args.extend([
        "--mcp-config".into(),
        config.to_string(),
        "--allowedTools".into(),
        format!("mcp__{SERVER}__*"),
        "--settings".into(),
        r#"{"disableAllHooks":true}"#.into(),
        "--system-prompt".into(),
        format!("{}\n\n{}", setup.instructions, super::cursor::reply_rule()),
    ]);
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    Spec {
        bin: setup.bin.clone(),
        args,
        cwd: setup.cwd.clone(),
        env: Vec::new(),
        stdin: true,
    }
}

fn finish(ctx: &Ctx, result: &Value, partial: &str) {
    let usage = &result["usage"];
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    ctx.usage(
        count("input_tokens"),
        count("output_tokens"),
        count("cache_read_input_tokens").saturating_add(count("cache_creation_input_tokens")),
    );
    if result["is_error"] == true
        || result["subtype"]
            .as_str()
            .is_some_and(|tag| tag != "success")
    {
        let error = result["result"]
            .as_str()
            .filter(|text| !text.is_empty())
            .or_else(|| result["errors"].as_array()?.first()?.as_str())
            .unwrap_or("Claude Code could not finish the reply.");
        ctx.reply(AgentReply::failed(error.to_string()));
        return;
    }
    let text = result["result"]
        .as_str()
        .filter(|text| !text.is_empty())
        .unwrap_or(partial);
    ctx.reply(parse_reply(&extract_json(text)));
}

pub(super) async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    let list = models();
    ctx.models(&list);
    let mut model = setup.model.clone();
    ctx.ready(model.as_deref());
    let mut running: Option<Proc> = None;
    let mut queue = VecDeque::<String>::new();
    let mut until = None;
    let mut partial = String::new();
    let mut restart = false;
    loop {
        if until.is_none() {
            if std::mem::take(&mut restart) {
                running = None;
            }
            if let Some(body) = queue.pop_front() {
                let p = match &mut running {
                    Some(p) => p,
                    None => running.insert(proc::spawn(
                        &ctx.app.cli.groups,
                        ctx.kind.title(),
                        &spec(&setup, model.as_deref()),
                        false,
                    )?),
                };
                ctx.turn_started();
                p.rpc
                    .write(&json!({"type": "user", "message": {"role": "user",
                    "content": [{"type": "text", "text": format!("Chat message:\n{body}")}]}}));
                partial.clear();
                until = Some(Instant::now() + Duration::from_millis(PROMPT_MS));
            }
        }
        let deadline = until.unwrap_or_else(|| super::deadline(false, Instant::now()));
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None | Some(Cmd::Stop) => return Ok(()),
                Some(Cmd::Tell(body)) => queue.push_back(body),
                Some(Cmd::Interrupt | Cmd::Reset) => {
                    running = None;
                    until = None;
                    queue.clear();
                    ctx.busy(false);
                }
                Some(Cmd::Model(wanted)) => {
                    if model != wanted {
                        model = wanted;
                        restart = true;
                        ctx.model_now(model.as_deref());
                    }
                }
                Some(Cmd::Models(answer)) => {
                    let _ = answer.send(Ok(list.clone()));
                }
            },
            event = async {
                match &mut running {
                    Some(p) => p.events.recv().await,
                    None => std::future::pending().await,
                }
            } => match event {
                Some(Event::Message(row)) => match row["type"].as_str() {
                    Some("system") if row["subtype"] == "init" => {
                        ctx.model_now(model.as_deref().or_else(|| row["model"].as_str()));
                    }
                    Some("stream_event") if until.is_some() => {
                        let stream = &row["event"];
                        if stream["type"] == "message_start" {
                            partial.clear();
                        } else if stream.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta")
                            && let Some(delta) = stream.pointer("/delta/text").and_then(Value::as_str)
                            && partial.len() + delta.len() <= proc::LINE_MAX as usize {
                            partial.push_str(delta);
                        }
                    }
                    Some("result") if until.take().is_some() => {
                        finish(ctx, &row, &partial);
                        if queue.is_empty() {
                            ctx.busy(false);
                        }
                    }
                    _ => {}
                },
                _ if until.is_none() => running = None,
                _ => {
                    let tail = running.as_ref().map(|p| proc::tail_text(&p.tail)).unwrap_or_default();
                    return Err(if tail.is_empty() { "Claude Code stopped before replying.".into() }
                        else { format!("Claude Code stopped before replying: {tail}") });
                }
            },
            _ = tokio::time::sleep_until(deadline), if until.is_some() => {
                return Err("Claude Code did not finish the reply in time.".into());
            }
        }
    }
}
