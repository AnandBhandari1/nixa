use super::proc::{self, Event, Spec};
use super::{Kind, REQUEST_MS, VERSION};
use crate::app::App;
use crate::tools::{Args, ToolResult, need, schema, spec, text};
use crate::util::short_hash;
use nixa_protocol::{Status, TermInfo};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const NAME: &str = "nixa_session_send";
const MAX_MESSAGE: usize = 8_000;
const WAIT_MS: u64 = 10 * 60 * 1_000;
const QUEUE_MAX: usize = 32;
const OUTCOME_MS: u64 = 24 * 60 * 60 * 1_000;
const OUTCOME_MAX: usize = 2_000;

#[derive(Clone, Debug)]
struct Target {
    terminal: String,
    provider: String,
    session: String,
    cwd: PathBuf,
    space: Option<String>,
}

#[derive(Clone, Debug)]
struct Pending {
    key: String,
    target: Target,
    message: String,
}

#[derive(Default)]
struct SessionQueue {
    running: bool,
    pending: VecDeque<Pending>,
}

#[derive(Clone, Copy, Debug)]
struct Outcome {
    state: &'static str,
    at: u64,
    by_request: bool,
}

#[derive(Default)]
struct DirectState {
    queues: HashMap<String, SessionQueue>,
    outcomes: HashMap<String, Outcome>,
}

fn state() -> &'static Mutex<DirectState> {
    static STATE: OnceLock<Mutex<DirectState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(DirectState::default()))
}

pub fn specs() -> Vec<crate::turns::ToolSpec> {
    vec![spec(
        NAME,
        "Send a message to an exact live agent session through its structured provider channel when supported. It returns accepted, queued, duplicate, blocked or a clear terminal fallback. It never types into the terminal prompt.",
        schema(
            json!({
                "terminal": {"type": "string", "description": "Exact live terminal id from nixa_term_list."},
                "message": {"type": "string", "maxLength": MAX_MESSAGE},
                "space": {"type": "string", "description": "Optional space id or name; it must match the target terminal."},
                "request_id": {"type": "string", "maxLength": 160, "description": "Stable retry id. Reusing it never delivers the message twice."}
            }),
            &["terminal", "message"],
        ),
    )]
}

fn supported(provider: &str) -> Option<Kind> {
    match provider {
        "codex" => Some(Kind::Codex),
        "agy" => Some(Kind::Agy),
        "kimi" => Some(Kind::Kimi),
        "jcode" => Some(Kind::Jcode),
        "dim" => Some(Kind::Dim),
        _ => None,
    }
}

fn fallback(target: &Target) -> String {
    format!(
        "{} has no supported structured message channel for session {}. Use the safe terminal path: nixa_term_send to {} with submit=true (or nixa term send {} \"text\" --submit).",
        target.provider, target.session, target.terminal, target.terminal
    )
}

fn busy(term: &TermInfo) -> bool {
    matches!(term.status, Some(Status::Working))
}

fn blocked(target: &Target) -> String {
    format!(
        "Terminal {} is blocked on a prompt, so structured delivery was not attempted yet. Do not ask the user: read the prompt with nixa_term_read, answer it with nixa_term_send, then send this message again.",
        target.terminal
    )
}

fn unconfirmed(target: &Target, error: &str) -> String {
    format!(
        "Structured delivery to {} was not confirmed: {error}. Nothing was typed into the terminal. Do not ask the user: read the session with nixa_term_read now; if the message is not there, send it with nixa_term_send and submit=true.",
        target.terminal
    )
}

fn space_matches(
    app: &App,
    target: &TermInfo,
    wanted: Option<&str>,
) -> Result<Option<String>, String> {
    let spaces = app.spaces();
    let actual = target
        .labels
        .get(crate::deep::SPACE_LABEL)
        .and_then(|id| crate::spaces::find(&spaces, id))
        .or_else(|| crate::spaces::for_folder(&spaces, &target.cwd))
        .map(|space| space.id.clone());
    if let Some(wanted) = wanted.filter(|value| !value.trim().is_empty()) {
        let found = app
            .spaces()
            .into_iter()
            .find(|space| space.id == wanted || space.name.eq_ignore_ascii_case(wanted))
            .map(|space| space.id)
            .ok_or_else(|| format!("There is no space {wanted}. Nothing was sent."))?;
        if actual.as_deref() != Some(found.as_str()) {
            return Err(format!(
                "Terminal {} is not in space {found}. Nothing was sent.",
                target.id
            ));
        }
    }
    Ok(actual)
}

fn provider_session(term: &TermInfo) -> Option<(String, String)> {
    term.labels
        .get("resume")
        .and_then(|label| nixa_store::agents::session_from_label(label))
}

async fn target(app: &App, terminal: &str, wanted_space: Option<&str>) -> Result<Target, String> {
    let id = crate::engine::term_id(terminal).map_err(|error| error.to_string())?;
    let term = crate::engine::get(&id)
        .await
        .map_err(|error| error.to_string())?;
    if !term.running {
        return Err(format!(
            "Terminal {} is not live. Use a new session or nixa_term_send for its terminal state.",
            id
        ));
    }
    let space = space_matches(app, &term, wanted_space)?;
    let saved = nixa_store::open(&app.db_file).ok().and_then(|conn| {
        nixa_store::agents::agent_by_terminal(&conn, id.as_str())
            .ok()
            .flatten()
    });
    let labeled = provider_session(&term);
    let (provider, session) = match (saved.as_ref(), labeled) {
        (Some(agent), Some((provider, session))) => {
            if !agent.provider.is_empty() && agent.provider != provider {
                return Err(format!(
                    "Terminal {} changed provider while it was being resolved. Nothing was sent.",
                    id
                ));
            }
            if !agent.session.is_empty() && agent.session != session {
                return Err(format!(
                    "Terminal {} changed session while it was being resolved. Nothing was sent.",
                    id
                ));
            }
            (provider, session)
        }
        (Some(agent), None) if !agent.provider.is_empty() && !agent.session.is_empty() => {
            (agent.provider.clone(), agent.session.clone())
        }
        (_, Some(pair)) => pair,
        _ => {
            return Err(format!(
                "Terminal {} has no saved structured session id. Use nixa_term_send with submit=true for the safe terminal path.",
                id
            ));
        }
    };
    Ok(Target {
        terminal: id.to_string(),
        provider,
        session,
        cwd: PathBuf::from(term.cwd),
        space,
    })
}

fn target_key(app: &App, target: &Target) -> String {
    format!(
        "{}\n{}\n{}",
        app.dir.display(),
        target.terminal,
        target.session
    )
}

fn delivery_key(app: &App, target: &Target, request_id: Option<&str>, message: &str) -> String {
    let source = request_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(message);
    format!("{}\n{}", target_key(app, target), short_hash(source))
}

fn duplicate(outcome: &'static str) -> String {
    match outcome {
        "queued" => "Duplicate retry: this message is already queued for the same structured session; it was not sent again.".into(),
        "accepted" => "Duplicate retry: this message was already accepted by the structured session; it was not sent again.".into(),
        _ => "Duplicate retry: the earlier structured delivery outcome is uncertain; inspect the agent before retrying.".into(),
    }
}

fn blocks_retry(outcome: &Outcome) -> bool {
    outcome.by_request || outcome.state == "queued"
}

fn prune(outcomes: &mut HashMap<String, Outcome>, now: u64) {
    outcomes.retain(|_, outcome| outcome.state == "queued" || now.saturating_sub(outcome.at) < OUTCOME_MS);
    if outcomes.len() > OUTCOME_MAX {
        let mut ages: Vec<(u64, String)> = outcomes
            .iter()
            .filter(|(_, outcome)| outcome.state != "queued")
            .map(|(key, outcome)| (outcome.at, key.clone()))
            .collect();
        ages.sort();
        let extra = outcomes.len() - OUTCOME_MAX;
        for (_, key) in ages.into_iter().take(extra) {
            outcomes.remove(&key);
        }
    }
}

fn remember(state: &mut DirectState, key: &str, outcome: &'static str, by_request: bool) {
    let now = crate::util::now_ms();
    prune(&mut state.outcomes, now);
    state.outcomes.insert(
        key.to_string(),
        Outcome {
            state: outcome,
            at: now,
            by_request,
        },
    );
}

fn mark(key: &str, outcome: &'static str) {
    let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(saved) = state.outcomes.get_mut(key) {
        saved.state = outcome;
        saved.at = crate::util::now_ms();
    }
}

fn forget(key: &str) {
    let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
    state.outcomes.remove(key);
}

async fn live_target(app: &App, expected: &Target) -> Result<Target, String> {
    let current = target(app, &expected.terminal, expected.space.as_deref()).await?;
    if current.provider != expected.provider || current.session != expected.session {
        return Err(format!(
            "Terminal {} changed its structured session while the message was queued. Nothing was sent.",
            expected.terminal
        ));
    }
    Ok(current)
}

async fn wait_until_free(app: &App, expected: &Target) -> Result<Target, String> {
    let start = tokio::time::Instant::now();
    loop {
        let target = live_target(app, expected).await?;
        let id = crate::engine::term_id(&target.terminal).map_err(|error| error.to_string())?;
        let term = crate::engine::get(&id)
            .await
            .map_err(|error| error.to_string())?;
        if !term.running {
            return Err(format!(
                "{} ended before the queued structured message could be delivered.",
                expected.terminal
            ));
        }
        if matches!(term.status, Some(Status::Blocked)) {
            return Err(blocked(expected));
        }
        if !busy(&term) {
            return Ok(target);
        }
        if start.elapsed() >= Duration::from_millis(WAIT_MS) {
            return Err(format!(
                "{} stayed busy too long; the structured message remains undelivered. Use the terminal path after inspecting it.",
                expected.terminal
            ));
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

async fn drain(app: std::sync::Arc<App>, key: String) {
    loop {
        let next = {
            let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
            let Some(queue) = state.queues.get_mut(&key) else {
                return;
            };
            match queue.pending.pop_front() {
                Some(pending) => pending,
                None => {
                    queue.running = false;
                    return;
                }
            }
        };
        let result = match wait_until_free(&app, &next.target).await {
            Ok(target) => send_now(&app, &target, &next.message).await,
            Err(error) => {
                forget(&next.key);
                app.info(&format!(
                    "Queued structured message to {} was not sent: {error}",
                    next.target.terminal
                ));
                continue;
            }
        };
        mark(
            &next.key,
            if result.is_ok() {
                "accepted"
            } else {
                "uncertain"
            },
        );
        if let Err(error) = result {
            app.info(&format!(
                "Structured message to {} was not confirmed: {error}",
                next.target.terminal
            ));
        }
    }
}

async fn send_or_queue(
    app: &std::sync::Arc<App>,
    target: Target,
    message: String,
    request_id: Option<&str>,
    was_busy: bool,
) -> ToolResult {
    let key = delivery_key(app, &target, request_id, &message);
    let by_request = request_id.is_some_and(|id| !id.trim().is_empty());
    let queue_key = target_key(app, &target);
    let (queued, start_worker) = {
        let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(outcome) = state.outcomes.get(&key).copied()
            && blocks_retry(&outcome)
        {
            return Ok(duplicate(outcome.state));
        }
        let (should_queue, was_running) = {
            let queue = state.queues.entry(queue_key.clone()).or_default();
            (was_busy || queue.running, queue.running)
        };
        if should_queue {
            let needs_worker = !was_running;
            if state
                .queues
                .get(&queue_key)
                .is_some_and(|queue| queue.pending.len() >= QUEUE_MAX)
            {
                return Err(format!(
                    "Structured session {} is busy and its safe queue is full. Use the terminal fallback after inspecting it.",
                    target.session
                ));
            }
            state
                .queues
                .get_mut(&queue_key)
                .expect("queue inserted above")
                .pending
                .push_back(Pending {
                    key: key.clone(),
                    target: target.clone(),
                    message: message.clone(),
                });
            remember(&mut state, &key, "queued", by_request);
            if needs_worker {
                state
                    .queues
                    .get_mut(&queue_key)
                    .expect("queue inserted above")
                    .running = true;
            }
            (true, needs_worker)
        } else {
            state
                .queues
                .get_mut(&queue_key)
                .expect("queue inserted above")
                .running = true;
            remember(&mut state, &key, "accepted", by_request);
            (false, false)
        }
    };
    if queued {
        if start_worker {
            let app = app.clone();
            tokio::spawn(drain(app, queue_key));
        }
        return Ok(format!(
            "Queued for structured session {} in terminal {}. It will be delivered when the session is free; retries with the same request_id are ignored.",
            target.session, target.terminal
        ));
    }
    let mut reached = false;
    let result = match live_target(app, &target).await {
        Ok(target) => {
            reached = true;
            send_now(app, &target, &message).await
        }
        Err(error) => Err(error),
    };
    let start_queued = {
        let mut state = state().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(queue) = state.queues.get_mut(&queue_key) {
            if queue.pending.is_empty() {
                queue.running = false;
                false
            } else {
                queue.running = true;
                true
            }
        } else {
            false
        }
    };
    if start_queued {
        tokio::spawn(drain(app.clone(), queue_key.clone()));
    }
    match result {
        Ok(_) => Ok(format!(
            "Accepted by the structured {} session {} in terminal {}. A retry with the same request_id will not deliver it again.",
            target.provider, target.session, target.terminal
        )),
        Err(error) if !reached => {
            forget(&key);
            Err(format!(
                "Structured delivery to {} did not start: {error}. Nothing was sent, so a retry is safe.",
                target.terminal
            ))
        }
        Err(error) => {
            mark(&key, "uncertain");
            crate::modes::note(app, "auto-yes", &format!("check the unconfirmed structured message to {}", target.terminal), Some(target.terminal.as_str()), Some("Nixa checks and resends instead of asking"));
            Err(unconfirmed(&target, &error))
        }
    }
}

pub async fn call(app: &std::sync::Arc<App>, args: &Args) -> ToolResult {
    let terminal = need(args, "terminal")?;
    let message = need(args, "message")?;
    if message.chars().count() > MAX_MESSAGE {
        return Err(format!(
            "message is longer than {MAX_MESSAGE} characters. Nothing was sent."
        ));
    }
    let target = target(app, &terminal, text(args, "space").as_deref()).await?;
    if supported(&target.provider).is_none() {
        return Ok(fallback(&target));
    }
    if let Some(held) = crate::term_tools::work_hold(app, "send-work", &message, Some(&target.terminal)) {
        return Ok(held);
    }
    let id = crate::engine::term_id(&target.terminal).map_err(|error| error.to_string())?;
    let term = crate::engine::get(&id)
        .await
        .map_err(|error| error.to_string())?;
    if matches!(term.status, Some(Status::Blocked)) {
        crate::modes::note(app, "auto-yes", &format!("answer the open prompt in {} before the structured message", target.terminal), Some(target.terminal.as_str()), Some("Nixa answers the prompt instead of asking"));
        return Ok(blocked(&target));
    }
    send_or_queue(
        app,
        target,
        message,
        text(args, "request_id").as_deref(),
        busy(&term),
    )
    .await
}

async fn send_now(app: &std::sync::Arc<App>, target: &Target, message: &str) -> Result<(), String> {
    let kind = supported(&target.provider)
        .ok_or_else(|| "provider has no structured transport".to_string())?;
    let setup = super::setup(app, kind)?;
    let spec = match kind {
        Kind::Codex => Spec {
            bin: setup.bin.clone(),
            args: vec!["app-server".into()],
            cwd: target.cwd.clone(),
            env: Vec::new(),
            stdin: true,
        },
        _ => {
            let mut args = vec!["acp".into()];
            if kind == Kind::Jcode {
                args.extend(
                    [
                        "--no-update",
                        "--tool-profile",
                        "none",
                        "--disable-base-tools",
                    ]
                    .map(str::to_string),
                );
            }
            Spec {
                bin: setup.bin.clone(),
                args,
                cwd: target.cwd.clone(),
                env: if kind == Kind::Dim {
                    vec![("DIMCODE_UPDATE_CHECK".into(), "0".into())]
                } else {
                    Vec::new()
                },
                stdin: true,
            }
        }
    };
    let mut proc = proc::spawn(&app.cli.groups, kind.title(), &spec, true)?;
    let names = crate::tools::specs(app)
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    let result = match kind {
        Kind::Codex => codex(&mut proc, target, message).await,
        _ => acp(&mut proc, target, message, &names).await,
    };
    proc.stop();
    result
}

async fn codex(proc: &mut proc::Proc, target: &Target, message: &str) -> Result<(), String> {
    proc.rpc.request("initialize", json!({"clientInfo": {"name": "nixa", "title": "Nixa", "version": VERSION}, "capabilities": {"experimentalApi": true, "requestAttestation": false}}), REQUEST_MS).await?;
    proc.rpc.notify("initialized", json!({}));
    proc.rpc
        .request(
            "thread/resume",
            json!({"threadId": target.session, "cwd": target.cwd.display().to_string()}),
            REQUEST_MS,
        )
        .await?;
    let started = proc
        .rpc
        .request(
            "turn/start",
            json!({"threadId": target.session, "input": super::codex::input(message)}),
            REQUEST_MS,
        )
        .await?;
    let turn = started
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .ok_or("Codex did not return a turn id.")?
        .to_string();
    tokio::time::timeout(Duration::from_millis(WAIT_MS), async {
        loop {
            match proc.events.recv().await {
                Some(Event::Message(message))
                    if message.get("method").and_then(Value::as_str) == Some("turn/completed") =>
                {
                    let params = message.get("params").cloned().unwrap_or(Value::Null);
                    if params.pointer("/turn/id").and_then(Value::as_str) != Some(turn.as_str()) {
                        continue;
                    }
                    let status = params
                        .pointer("/turn/status")
                        .and_then(Value::as_str)
                        .unwrap_or("completed");
                    return if status == "completed" {
                        Ok(())
                    } else {
                        Err(format!("Codex ended the turn with status {status}."))
                    };
                }
                Some(Event::Message(message))
                    if message.get("method").and_then(Value::as_str) == Some("turn/failed") =>
                {
                    return Err("Codex failed the turn.".into());
                }
                Some(Event::Ended(code)) => {
                    return Err(format!(
                        "Codex app-server ended{}.",
                        code.map(|n| format!(" with code {n}")).unwrap_or_default()
                    ));
                }
                Some(Event::Message(message)) => {
                    if message.get("method").is_some() && message.get("id").is_some() {
                        super::codex::answer_request(proc, &message);
                    }
                }
                None => return Err("Codex app-server closed before the turn completed.".into()),
            }
        }
    })
    .await
    .map_err(|_| "Codex did not finish the structured turn in time.".to_string())??;
    Ok(())
}

async fn acp(
    proc: &mut proc::Proc,
    target: &Target,
    message: &str,
    names: &[String],
) -> Result<(), String> {
    let initialized = proc.rpc.request("initialize", json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false}, "clientInfo": {"name": "nixa", "title": "Nixa", "version": VERSION}}), REQUEST_MS).await?;
    if initialized
        .pointer("/agentCapabilities/loadSession")
        .and_then(Value::as_bool)
        == Some(false)
    {
        return Err("ACP provider does not support loading this session; no supported structured channel is available. Use nixa_term_send with submit=true after inspecting the terminal.".into());
    }
    let loaded = proc.rpc.request("session/load", json!({"sessionId": target.session, "cwd": target.cwd.display().to_string(), "mcpServers": []}), REQUEST_MS).await?;
    let loaded_id = loaded
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or(&target.session);
    let prompt = proc.rpc.request(
        "session/prompt",
        json!({"sessionId": loaded_id, "prompt": [{"type": "text", "text": message}]}),
        super::PROMPT_MS,
    );
    tokio::pin!(prompt);
    loop {
        tokio::select! {
            result = &mut prompt => {
                result?;
                break;
            }
            event = proc.events.recv() => match event {
                Some(Event::Message(message)) => {
                    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
                    if method == "session/request_permission" {
                        if let Some(id) = message.get("id") {
                            let (answer, _) = super::acp::agent_permission_answer(message.get("params").unwrap_or(&Value::Null), names);
                            proc.rpc.reply(id, answer);
                        }
                    } else if let Some(id) = message.get("id") {
                        proc.rpc.reply_error(id, &format!("{method} is not supported by Nixa."));
                    }
                }
                Some(Event::Ended(code)) => return Err(format!("ACP app-server ended{}.", code.map(|n| format!(" with code {n}")).unwrap_or_default())),
                None => return Err("ACP app-server closed before the prompt completed.".into()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supported_structured_providers_are_selected() {
        assert_eq!(supported("codex"), Some(Kind::Codex));
        assert_eq!(supported("kimi"), Some(Kind::Kimi));
        assert_eq!(supported("claude"), None);
    }

    #[test]
    fn retry_key_uses_request_id_without_storing_message_text() {
        let target = Target {
            terminal: "s1-codex-a1".into(),
            provider: "codex".into(),
            session: "session-1".into(),
            cwd: PathBuf::from("/tmp"),
            space: Some("s1".into()),
        };
        let one = delivery_key(
            &App::open(std::env::temp_dir().join("nixa-direct-key-test").as_path()).unwrap(),
            &target,
            Some("r1"),
            "first",
        );
        let two = delivery_key(
            &App::open(std::env::temp_dir().join("nixa-direct-key-test").as_path()).unwrap(),
            &target,
            Some("r1"),
            "second",
        );
        assert_eq!(one, two);
        assert!(!one.contains("first"));
    }

    #[test]
    fn retry_without_request_id_only_blocks_while_queued() {
        let queued = Outcome { state: "queued", at: 1, by_request: false };
        let accepted = Outcome { state: "accepted", at: 1, by_request: false };
        let with_id = Outcome { state: "accepted", at: 1, by_request: true };
        assert!(blocks_retry(&queued));
        assert!(!blocks_retry(&accepted));
        assert!(blocks_retry(&with_id));
    }

    #[test]
    fn old_outcomes_are_pruned_but_queued_ones_stay() {
        let mut outcomes = HashMap::new();
        outcomes.insert("old".to_string(), Outcome { state: "accepted", at: 0, by_request: true });
        outcomes.insert("wait".to_string(), Outcome { state: "queued", at: 0, by_request: true });
        outcomes.insert("new".to_string(), Outcome { state: "accepted", at: OUTCOME_MS, by_request: true });
        prune(&mut outcomes, OUTCOME_MS + 1);
        assert!(!outcomes.contains_key("old"));
        assert!(outcomes.contains_key("wait"));
        assert!(outcomes.contains_key("new"));
    }

    #[test]
    fn fallback_names_the_terminal_delivery_path() {
        let target = Target {
            terminal: "s1-claude-a1".into(),
            provider: "claude".into(),
            session: "c1".into(),
            cwd: PathBuf::from("/tmp"),
            space: None,
        };
        assert!(fallback(&target).contains("submit=true"));
        assert!(fallback(&target).contains("s1-claude-a1"));
    }

    #[test]
    fn automatic_blocked_and_unconfirmed_texts_give_a_next_step() {
        let target = Target {
            terminal: "s1-claude-a1".into(),
            provider: "claude".into(),
            session: "c1".into(),
            cwd: PathBuf::from("/tmp"),
            space: None,
        };
        assert!(blocked(&target).contains("Do not ask the user"));
        assert!(blocked(&target).contains("nixa_term_read"));
        assert!(!blocked(&target).contains("Decision"));
        assert!(unconfirmed(&target, "timed out").contains("Do not ask the user"));
    }
}
