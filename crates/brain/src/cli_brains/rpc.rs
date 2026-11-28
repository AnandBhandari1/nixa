use super::proc::{self, Event, Proc, Spec};
use super::{Cmd, Ctx, Kind, PROMPT_MS, REQUEST_MS};
use crate::provider::Model;
use crate::reply::{AgentReply, extract_json, parse_reply};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

struct Turn {
    id: u64,
    text: String,
    cancelled: bool,
    until: Instant,
}

pub(super) async fn run(ctx: &Ctx, rx: &mut UnboundedReceiver<Cmd>) -> Result<(), String> {
    let setup = super::setup(&ctx.app, ctx.kind)?;
    if ctx.kind == Kind::Omo {
        let spec = Spec {
            bin: setup.bin,
            args: ["--mode", "rpc"].map(str::to_string).to_vec(),
            cwd: setup.cwd,
            env: Vec::new(),
            stdin: true,
        };
        let preface = format!("{}\n\n{}", setup.instructions, super::text_tools::rule(&ctx.app));
        let mut p = proc::spawn(&ctx.app.cli.groups, ctx.kind.title(), &spec, false)?;
        let result = drive(ctx, &mut p, rx, setup.model, Some(preface)).await;
        p.stop();
        return result;
    }
    let config = setup.cwd.join("nixa-rpc.json");
    crate::util::write_json(
        &config,
        &json!({"mcp": {"enabled": false, "enableProjectConfig": false}}),
    )
    .map_err(|e| e.to_string())?;
    let mut args = [
        "--mode",
        "rpc",
        "--no-tools",
        "--no-extensions",
        "--no-skills",
        "--no-rules",
        "--no-context-files",
        "--no-session",
        "--no-ui",
        "--no-title",
        "--no-lsp",
        "--offline",
    ]
    .map(str::to_string)
    .to_vec();
    args.extend([
        "--config".into(),
        config.display().to_string(),
        "--system-prompt".into(),
        format!("{}\n\n{}", setup.instructions, super::text_tools::rule(&ctx.app)),
    ]);
    let spec = Spec {
        bin: setup.bin,
        args,
        cwd: setup.cwd,
        env: Vec::new(),
        stdin: true,
    };
    let mut p = proc::spawn(&ctx.app.cli.groups, ctx.kind.title(), &spec, false)?;
    let result = drive(ctx, &mut p, rx, setup.model, None).await;
    p.stop();
    result
}

fn stopped(p: &Proc, ctx: &Ctx) -> String {
    let tail = proc::tail_text(&p.tail);
    let title = ctx.kind.title();
    if ctx.kind == Kind::Omo && signed_out(&tail) {
        return "OmO is signed out. Run omo login in a terminal, then choose OmO again.".into();
    }
    if tail.is_empty() {
        format!("{title} stopped.")
    } else {
        format!("{title} stopped: {tail}")
    }
}

async fn request(ctx: &Ctx, p: &mut Proc, kind: &str, mut body: Value) -> Result<Value, String> {
    body["id"] = json!(kind);
    body["type"] = json!(kind);
    p.rpc.write(&body);
    tokio::time::timeout(Duration::from_millis(REQUEST_MS), async {
        loop {
            match p.events.recv().await {
                Some(Event::Message(row)) if row["type"] == "response" && row["id"] == kind => {
                    if row["success"] == true {
                        return Ok(row["data"].clone());
                    }
                    return Err(row["error"]
                        .as_str()
                        .unwrap_or("The request was refused.")
                        .to_string());
                }
                Some(Event::Message(_)) => {}
                _ => return Err(stopped(p, ctx)),
            }
        }
    })
    .await
    .map_err(|_| format!("{} did not answer {kind} in time.", ctx.kind.title()))?
}

pub(super) fn signed_out(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    ["not logged in", "not signed in", "signed out", "omo login", "please log in", "unauthorized"]
        .iter()
        .any(|word| text.contains(word))
}

fn model_id(row: &Value) -> Option<String> {
    let provider = row["provider"].as_str()?;
    let model = row["id"].as_str()?;
    (!provider.is_empty() && !model.is_empty()).then(|| format!("{provider}/{model}"))
}

async fn catalog(ctx: &Ctx, p: &mut Proc) -> Result<Vec<Model>, String> {
    let data = request(ctx, p, "get_available_models", json!({})).await?;
    let rows = data["models"]
        .as_array()
        .ok_or("The model response did not contain a list.")?;
    let models = rows
        .iter()
        .filter_map(|row| {
            let id = model_id(row)?;
            Some(Model {
                name: row["name"].as_str().unwrap_or(&id).to_string(),
                description: row["provider"].as_str().unwrap_or("").to_string(),
                id,
                efforts: Vec::new(),
                max_tokens: row["contextWindow"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok()),
            })
        })
        .collect::<Vec<_>>();
    ctx.models(&models);
    Ok(models)
}

async fn set_model(ctx: &Ctx, p: &mut Proc, wanted: &str) -> Result<(), String> {
    let (provider, model) = wanted
        .split_once('/')
        .ok_or("Choose a model from the provider's list.")?;
    let selected = request(
        ctx,
        p,
        "set_model",
        json!({"provider": provider, "modelId": model}),
    )
    .await?;
    ctx.model_now(model_id(&selected).as_deref().or(Some(wanted)));
    Ok(())
}

fn message_text(row: &Value) -> Option<String> {
    let blocks = row["content"].as_array()?;
    Some(
        blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect(),
    )
}

fn update_turn(ctx: &Ctx, turn: &mut Turn, row: &Value) -> Option<AgentReply> {
    let kind = row["type"].as_str()?;
    let message = &row["message"];
    match kind {
        "message_start" if message["role"] == "assistant" => turn.text.clear(),
        "message_update" if message["role"] == "assistant" => {
            let event = &row["assistantMessageEvent"];
            if event["type"] == "text_delta"
                && let Some(delta) = event["delta"].as_str()
                && turn.text.len() + delta.len() <= proc::LINE_MAX as usize
            {
                turn.text.push_str(delta);
            }
        }
        "message_end" if message["role"] == "assistant" => {
            if let Some(text) = message_text(message) {
                turn.text = text;
            }
            let usage = &message["usage"];
            let count = |key: &str| usage[key].as_u64().unwrap_or(0);
            ctx.usage(
                count("input"),
                count("output"),
                count("cacheRead").saturating_add(count("cacheWrite")),
            );
        }
        "response" if row["id"] == turn.id && row["success"] == false => {
            return Some(AgentReply::failed(
                row["error"]
                    .as_str()
                    .unwrap_or("The prompt was refused.")
                    .to_string(),
            ));
        }
        "prompt_result" if row["id"] == turn.id => {
            let error = row.pointer("/error/message").and_then(Value::as_str);
            let reply = match row["status"].as_str() {
                Some("completed") if row["yielded"] == false => return None,
                Some("completed") => parse_reply(&extract_json(&turn.text)),
                Some("aborted") => AgentReply::failed(error.unwrap_or(crate::agent::STOPPED)),
                _ => AgentReply::failed(
                    error.unwrap_or("The chat brain could not finish the turn."),
                ),
            };
            return Some(reply);
        }
        "agent_end" if row.get("yielded").is_none() && row["isTerminal"] != false => {
            let last = row["messages"].as_array().and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "assistant")
            });
            if let Some(last) = last {
                if last["stopReason"] == "error" {
                    return Some(AgentReply::failed(
                        last["errorMessage"]
                            .as_str()
                            .unwrap_or("The chat brain failed.")
                            .to_string(),
                    ));
                }
                if let Some(text) = message_text(last) {
                    turn.text = text;
                }
            }
            return Some(parse_reply(&extract_json(&turn.text)));
        }
        _ => {}
    }
    None
}

async fn drive(
    ctx: &Ctx,
    p: &mut Proc,
    rx: &mut UnboundedReceiver<Cmd>,
    model: Option<String>,
    rules: Option<String>,
) -> Result<(), String> {
    let mut preface = rules.clone();
    let state = request(ctx, p, "get_state", json!({})).await?;
    let mut models = catalog(ctx, p).await?;
    let current = model_id(&state["model"]);
    ctx.ready(current.as_deref());
    let mut wanted = model.filter(|m| current.as_ref() != Some(m));
    let mut model_error = None;
    let mut seq = 0_u64;
    let mut turn: Option<Turn> = None;
    let mut queue = VecDeque::<String>::new();
    loop {
        if turn.is_none() {
            if let Some(model) = wanted.take() {
                model_error = set_model(ctx, p, &model).await.err().map(|error| {
                    format!("{} could not select {model}: {error}", ctx.kind.title())
                });
                if queue.is_empty() && let Some(error) = &model_error {
                    ctx.app.info(error);
                }
            }
            if let Some(body) = queue.pop_front() {
                if let Some(error) = &model_error {
                    ctx.reply(AgentReply::failed(error.clone()));
                    if queue.is_empty() {
                        ctx.busy(false);
                    }
                    continue;
                }
                seq += 1;
                ctx.turn_started();
                let message = match preface.take() {
                    Some(rules) => format!("{rules}\n\nChat message:\n{body}"),
                    None => format!("Chat message:\n{body}"),
                };
                p.rpc.write(&json!({"id": seq, "type": "prompt", "message": message}));
                turn = Some(Turn {
                    id: seq,
                    text: String::new(),
                    cancelled: false,
                    until: Instant::now() + Duration::from_millis(PROMPT_MS),
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
                        p.rpc.write(&json!({"type": "abort"}));
                    }
                }
                Some(Cmd::Reset) => {
                    turn = None;
                    queue.clear();
                    p.rpc.write(&json!({"type": "abort"}));
                    request(ctx, p, "new_session", json!({})).await?;
                    preface.clone_from(&rules);
                    ctx.busy(false);
                }
                Some(Cmd::Model(model)) => wanted = model,
                Some(Cmd::Models(answer)) => {
                    let result = if turn.is_none() {
                        catalog(ctx, p).await.inspect(|list| models.clone_from(list))
                    } else {
                        Ok(models.clone())
                    };
                    let _ = answer.send(result);
                }
            },
            event = p.events.recv() => match event {
                Some(Event::Message(row)) => {
                    if let Some(active) = &mut turn && let Some(reply) = update_turn(ctx, active, &row) {
                        if !active.cancelled {
                            ctx.reply(reply);
                        }
                        turn = None;
                        if queue.is_empty() {
                            ctx.busy(false);
                        }
                    }
                }
                _ => return Err(stopped(p, ctx)),
            },
            _ = tokio::time::sleep_until(until), if turn.is_some() => {
                return Err(format!("{} did not finish the reply in time.", ctx.kind.title()));
            }
        }
    }
}
