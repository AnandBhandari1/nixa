use crate::args::usage;
use crate::ctx::{self, CALL_TIMEOUT, Ctx};
use anyhow::{Result, anyhow};
use nixa_protocol::{Reply, Request, TermId, TermInfo, paths};
use nixa_store::agents::resume_command;
use serde_json::Value;
use std::io::{IsTerminal, Read, Write};

pub const LABEL: &str = "resume";
pub const PID_LABEL: &str = "resume.pid";
pub const PROVIDERS: [&str; 6] = ["claude", "codex", "omp", "cursor", "agy", "opencode"];
const SESSION_KEYS: [&str; 5] = ["session_id", "sessionId", "session", "conversation_id", "conversationId"];
const CURSOR_KEYS: [&str; 4] = ["conversation_id", "session_id", "sessionId", "session"];
const INPUT_MAX: u64 = 1 << 20;

pub fn provider_of(word: &str) -> Option<&'static str> {
    let word = match word {
        "claude-code" => "claude",
        "cursor-agent" => "cursor",
        "antigravity" | "agyd" => "agy",
        other => other,
    };
    PROVIDERS.iter().copied().find(|p| *p == word)
}

fn hook_json(input: &str) -> Option<Value> {
    serde_json::from_str::<Value>(input.trim()).ok().filter(Value::is_object)
}

fn keys_for(provider: &str) -> &'static [&'static str] {
    if provider == "cursor" { &CURSOR_KEYS } else { &SESSION_KEYS }
}

pub fn session_for(provider: &str, input: &str) -> Option<String> {
    let v = hook_json(input)?;
    keys_for(provider)
        .iter()
        .find_map(|k| v.get(k).and_then(Value::as_str))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn changes(command: &str, info: &TermInfo) -> Vec<(&'static str, String)> {
    let mut sets = Vec::new();
    if info.running
        && let Some(pid) = info.pid
    {
        let pid = pid.to_string();
        if info.labels.get(PID_LABEL) != Some(&pid) {
            sets.push((PID_LABEL, pid));
        }
    }
    if info.labels.get(LABEL).map(String::as_str) != Some(command) {
        sets.push((LABEL, command.to_string()));
    }
    sets
}

fn read_input() -> Result<String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(String::new());
    }
    let mut text = String::new();
    stdin.lock().take(INPUT_MAX).read_to_string(&mut text)?;
    Ok(text)
}

fn term_of(value: Option<String>) -> Result<TermId> {
    match value {
        Some(v) => v.parse::<TermId>().map_err(|e| usage(e.to_string())),
        None => paths::current_term_id()
            .ok_or_else(|| usage("nixa hooks session runs inside a nixa terminal (NIXA_TERM_ID), or give --term ID")),
    }
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--id", "--term"], false)?;
    let given = p.value(&["--id"]);
    let term = p.value(&["--term"]);
    p.flag(&["--checks-off"]);
    p.finish(1, false)?;
    let Some(word) = p.pos.first() else {
        return Err(usage(format!("nixa hooks session needs an agent: {}", PROVIDERS.join(", "))));
    };
    let provider = provider_of(word)
        .ok_or_else(|| usage(format!("unknown agent {word:?}, use {}", PROVIDERS.join(", "))))?;
    let id = term_of(term)?;
    let input = if given.is_some() { String::new() } else { read_input()? };
    let session = given
        .or_else(|| session_for(provider, &input))
        .ok_or_else(|| usage("no session id: give --id ID, or the hook JSON with session_id on stdin"))?;
    let mut client = ctx::connect(&ctx, Some(CALL_TIMEOUT))?;
    let info = match client.call(&Request::Get { id: id.clone() })? {
        Reply::Term { term } => term,
        other => return Err(ctx::unexpected(&other)),
    };
    let command = resume_command(provider, &session)
        .ok_or_else(|| anyhow!("the session id {session:?} is not safe to use in a command"))?;
    let sets = changes(&command, &info);
    for (key, value) in &sets {
        client.call(&Request::LabelSet { id: id.clone(), key: key.to_string(), value: Some(value.clone()) })?;
    }
    if ctx.json {
        let set: Vec<&str> = sets.iter().map(|(k, _)| *k).collect();
        writeln!(out, "{}", serde_json::json!({"id": id.to_string(), "command": command, "set": set}))?;
    } else if sets.is_empty() {
        writeln!(out, "{id} resume: no change ({command})")?;
    } else {
        writeln!(out, "{id} resume: {command}")?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(cmd: &[&str], pid: Option<u32>, running: bool) -> TermInfo {
        TermInfo {
            id: TermId::from_number(1),
            name: String::new(),
            labels: Default::default(),
            parent: None,
            children: vec![],
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            cwd: "/tmp".into(),
            pid,
            running,
            exit_code: None,
            status: None,
            status_note: None,
            keep: false,
            draft: false,
            title: None,
            rows: 24,
            cols: 80,
            created_ms: 0,
            seq: 0,
        }
    }

    #[test]
    fn session_id_comes_from_the_hook_json() {
        assert_eq!(session_for("claude", r#"{"session_id":"abc-123","hook_event_name":"SessionStart"}"#).as_deref(), Some("abc-123"));
        assert_eq!(session_for("claude", r#"{"sessionId":" s-9 "}"#).as_deref(), Some("s-9"));
        assert_eq!(session_for("claude", r#"{"conversation_id":"c-42","hook_event_name":"beforeSubmitPrompt"}"#).as_deref(), Some("c-42"));
        assert_eq!(session_for("claude", r#"{"session_id":"s-1","conversation_id":"c-42"}"#).as_deref(), Some("s-1"));
        assert_eq!(session_for("cursor", r#"{"session_id":"s-1","conversation_id":"c-42"}"#).as_deref(), Some("c-42"));
        assert_eq!(session_for("cursor", r#"{"session_id":"s-1"}"#).as_deref(), Some("s-1"));
        assert_eq!(session_for("claude", r#"{"session_id":""}"#), None);
        assert_eq!(session_for("claude", r#"{"session_id":7}"#), None);
        assert_eq!(session_for("claude", "not json"), None);
        assert_eq!(session_for("claude", r#"["session_id"]"#), None);
    }

    #[test]
    fn hook_aliases_select_the_expected_agent() {
        assert_eq!(provider_of("claude-code"), Some("claude"));
        assert_eq!(provider_of("agyd"), Some("agy"));
        assert_eq!(session_for("agy", r#"{"conversationId":"a-7","hookEventName":"PreInvocation"}"#).as_deref(), Some("a-7"));
        assert_eq!(provider_of("hermes"), None);
    }

    #[test]
    fn labels_are_written_only_when_they_change() {
        let mut t = info(&["claude"], Some(41), true);
        assert_eq!(changes("claude --resume a", &t), vec![(PID_LABEL, "41".to_string()), (LABEL, "claude --resume a".to_string())]);
        t.labels.insert(PID_LABEL.into(), "41".into());
        t.labels.insert(LABEL.into(), "claude --resume a".into());
        assert!(changes("claude --resume a", &t).is_empty());
        assert_eq!(changes("claude --resume b", &t), vec![(LABEL, "claude --resume b".to_string())]);
        t.labels.insert(PID_LABEL.into(), "ended".into());
        assert_eq!(changes("claude --resume a", &t), vec![(PID_LABEL, "41".to_string())]);
        let gone = info(&["claude"], Some(41), false);
        assert_eq!(changes("claude --resume a", &gone), vec![(LABEL, "claude --resume a".to_string())]);
    }
}
