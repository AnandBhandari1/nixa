mod teams;

use crate::args;
use crate::ctx::{self, Ctx};
use crate::settings::{emit, flag_at, text_at};
use crate::{help, http};
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::Write;

pub fn row_line(r: &Value) -> String {
    let mut marks = Vec::new();
    if flag_at(r, "pinned") {
        marks.push("pinned".to_string());
    }
    if flag_at(r, "disabled") {
        marks.push("taken out".to_string());
    }
    if !flag_at(r, "listed") {
        marks.push("not listed".to_string());
    }
    let pins: Vec<&str> = r.get("pins").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if !pins.is_empty() {
        marks.push(format!("pins {}", pins.join(",")));
    }
    let mut line = format!("{:>3}. {}", r.get("rank").and_then(Value::as_u64).unwrap_or(0), text_at(r, "id"));
    let plan = [text_at(r, "company"), text_at(r, "plan")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
    if !plan.is_empty() {
        line.push_str(&format!("  ({plan})"));
    }
    if !marks.is_empty() {
        line.push_str(&format!("  [{}]", marks.join("] [")));
    }
    line
}

pub fn board_lines(board: &Value) -> Vec<String> {
    let rows = board.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut lines = vec![format!(
        "Model leaderboard: {} rows, {} listed, {} pinned.",
        rows.len(),
        board.get("listed").and_then(Value::as_u64).unwrap_or(0),
        board.get("pinned").and_then(Value::as_u64).unwrap_or(0)
    )];
    lines.extend(rows.iter().map(row_line));
    let choices = board.get("choices").and_then(Value::as_array).cloned().unwrap_or_default();
    if !choices.is_empty() {
        lines.push(String::new());
        lines.push("Picks by capability:".into());
        for c in &choices {
            let chosen = c.get("chosen").map(|m| text_at(m, "id")).filter(|s| !s.is_empty()).unwrap_or("none");
            lines.push(format!("  {}: {chosen} ({})", text_at(c, "capability"), text_at(c, "rule")));
        }
    }
    lines
}

pub fn rows_lines(data: &Value) -> Vec<String> {
    data.get("rows").and_then(Value::as_array).map(|rows| rows.iter().map(row_line).collect()).unwrap_or_default()
}

pub fn pin_body(id: &str, on: bool) -> Value {
    json!({"id": id, "pinned": on})
}

pub fn move_body(id: &str, rank: u64) -> Result<Value> {
    if rank == 0 {
        return Err(args::usage("rank starts at 1"));
    }
    Ok(json!({"id": id, "rank": rank}))
}

pub fn rank_body(task: Option<String>, caps: Option<String>) -> Value {
    let mut body = Map::new();
    if let Some(task) = task {
        body.insert("task".into(), json!(task));
    }
    if let Some(caps) = caps {
        let list: Vec<String> = caps.split(',').map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect();
        body.insert("capabilities".into(), json!(list));
    }
    Value::Object(body)
}

pub const AGENT_WAIT: std::time::Duration = std::time::Duration::from_secs(75);

pub fn agent_lines(agent: &str, data: &Value) -> Vec<String> {
    let models: Vec<&str> = data.get("models").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let mut lines = vec![format!("{agent}: {} models{}", models.len(), if data.get("stale").and_then(Value::as_bool).unwrap_or(false) { " (old list, reading again)" } else { "" })];
    lines.extend(models.iter().map(|m| format!("  {m}")));
    if let Some(error) = data.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("Error: {error}"));
    }
    lines
}

fn act(action: &str, body: &Value) -> Result<Value> {
    let reply = http::window_call("POST", &format!("/models/{action}"), Some(body))?;
    Ok(reply.get("data").cloned().unwrap_or(Value::Null))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help")) {
        out.write_all(help::MODELS.as_bytes())?;
        return Ok(0);
    }
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("ls");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    if what == "teams" || what == "team" {
        return teams::run(ctx, rest, out);
    }
    let (mut p, ctx) = ctx::command(ctx, rest, &["--task", "--caps"], false)?;
    match what {
        "ls" | "list" | "view" => {
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/models/view", None)?;
            let board = reply.get("data").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &board, &board_lines(&board).join("\n"))
        }
        "pin" | "unpin" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage(format!("models {what} needs a model ID like agent::model")))?;
            let data = act("edit", &pin_body(id, what == "pin"))?;
            let done = if what == "pin" { "is pinned" } else { "is not pinned now" };
            emit(&ctx, out, &data, &format!("{id} {done}."))
        }
        "move" => {
            p.finish(2, false)?;
            let (Some(id), Some(rank)) = (p.pos.first(), p.pos.get(1)) else {
                return Err(args::usage("models move needs a model ID and a rank: nixa models move agent::model 2"));
            };
            let data = act("move", &move_body(id, args::number(rank, "rank")?)?)?;
            emit(&ctx, out, &data, &format!("{id} is at rank {rank}.\n{}", rows_lines(&data).join("\n")))
        }
        "rank" => {
            p.finish(0, false)?;
            let data = act("rank", &rank_body(p.value(&["--task"]), p.value(&["--caps"])))?;
            emit(&ctx, out, &data, &format!("Ranked again.\n{}", rows_lines(&data).join("\n")))
        }
        "refresh" => {
            p.finish(0, false)?;
            let data = act("refresh", &json!({}))?;
            let text = format!("Public scores refreshed: {}.", data.get("refreshed").map(Value::to_string).unwrap_or_else(|| "unknown".into()));
            emit(&ctx, out, &data, &text)
        }
        "agent" => {
            let refresh = p.flag(&["--refresh"]);
            p.finish(1, false)?;
            let agent = p.pos.first().ok_or_else(|| args::usage("models agent needs an agent name: nixa models agent codex [--refresh]"))?;
            let reply = http::window_call_within("POST", "/models/agent_models", Some(&json!({"agent": agent, "refresh": refresh})), AGENT_WAIT)?;
            let data = reply.get("data").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &data, &agent_lines(agent, &data).join("\n"))
        }
        "lists" => {
            p.finish(0, false)?;
            let data = act("refresh_lists", &json!({}))?;
            let started: Vec<&str> = data.get("started").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            emit(&ctx, out, &data, &format!("Reading the model lists again in the background: {}.", if started.is_empty() { "no installed agents".to_string() } else { started.join(", ") }))
        }
        other => Err(args::usage(format!("models takes ls, pin, unpin, move, rank, refresh, agent, lists or teams, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        assert_eq!(pin_body("claude::opus", true), json!({"id": "claude::opus", "pinned": true}));
        assert_eq!(move_body("a::b", 2).unwrap(), json!({"id": "a::b", "rank": 2}));
        assert!(move_body("a::b", 0).is_err());
        assert_eq!(rank_body(None, Some("code, ui".into())), json!({"capabilities": ["code", "ui"]}));
        assert_eq!(rank_body(Some("fix it".into()), None), json!({"task": "fix it"}));
    }

    #[test]
    fn agent_text() {
        let lines = agent_lines("codex", &json!({"models": ["gpt-a", "gpt-b"], "stale": true, "error": "slow"}));
        assert_eq!(lines, vec!["codex: 2 models (old list, reading again)", "  gpt-a", "  gpt-b", "Error: slow"]);
    }

    #[test]
    fn board_text() {
        let board = json!({"listed": 1, "pinned": 1, "rows": [{"rank": 1, "id": "claude::opus", "listed": true, "pinned": true, "company": "Anthropic", "plan": "Max", "pins": ["code"]}], "choices": [{"capability": "code", "chosen": {"id": "claude::opus"}, "rule": "capability-pin"}]});
        let lines = board_lines(&board);
        assert_eq!(lines[0], "Model leaderboard: 1 rows, 1 listed, 1 pinned.");
        assert_eq!(lines[1], "  1. claude::opus  (Anthropic, Max)  [pinned] [pins code]");
        assert_eq!(lines[4], "  code: claude::opus (capability-pin)");
    }
}
