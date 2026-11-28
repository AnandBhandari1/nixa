use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use crate::settings::{emit, full_path, text_at};
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::Write;

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

pub fn run_line(r: &Value) -> String {
    format!("{}  {}  {}  step {}  {}", text_at(r, "id"), text_at(r, "team"), text_at(r, "state"), text_at(r, "step"), text_at(r, "task"))
}

pub fn team_lines(t: &Value) -> Vec<String> {
    let mut lines = vec![format!("{}  {}  (flow {})", text_at(t, "id"), text_at(t, "name"), text_at(t, "flow"))];
    let purpose = text_at(t, "purpose");
    if !purpose.is_empty() {
        lines.push(format!("  {purpose}"));
    }
    let steps = strings(t, "steps");
    if !steps.is_empty() {
        lines.push(format!("  steps: {}", steps.join(" -> ")));
    }
    lines
}

pub fn list_lines(snap: &Value) -> Vec<String> {
    let teams = snap.get("teams").and_then(Value::as_array).cloned().unwrap_or_default();
    let runs = snap.get("runs").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut lines = vec![format!("{} teams.", teams.len())];
    for t in &teams {
        lines.extend(team_lines(t));
    }
    if !runs.is_empty() {
        lines.push(String::new());
        lines.push(format!("Last runs: {}", runs.len()));
        lines.extend(runs.iter().map(|r| format!("  {}", run_line(r))));
    }
    if let Some(error) = snap.get("error").and_then(Value::as_str) {
        lines.push(error.to_string());
    }
    lines
}

pub fn start_body(team: &str, task: &str, space: Option<String>, project: Option<String>) -> Result<Value> {
    if task.trim().is_empty() {
        return Err(args::usage("teams start needs the task text"));
    }
    if space.is_none() && project.is_none() {
        return Err(args::usage("teams start needs --space NAME or --project DIR"));
    }
    let mut payload = Map::new();
    payload.insert("team".into(), json!(team));
    payload.insert("task".into(), json!(task));
    if let Some(space) = space {
        payload.insert("space".into(), json!(space));
    }
    if let Some(project) = project {
        payload.insert("project".into(), json!(full_path(&project)));
    }
    Ok(json!({"action": "start", "payload": payload}))
}

pub fn simple_body(action: &str, key: &str, id: &str) -> Value {
    json!({"action": action, "payload": {key: id}})
}

fn post(body: &Value) -> Result<Value> {
    http::window_call("POST", "/teams", Some(body))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("ls");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (mut p, ctx) = ctx::command(ctx, rest, &["--space", "--project"], false)?;
    match what {
        "ls" | "list" => {
            p.finish(0, false)?;
            let snap = http::window_call("GET", "/teams", None)?;
            emit(&ctx, out, &snap, &list_lines(&snap).join("\n"))
        }
        "start" => {
            let team = p.pos.first().cloned().ok_or_else(|| args::usage("teams start needs a team ID"))?;
            let task = p.pos[1..].join(" ");
            let body = start_body(&team, &task, p.value(&["--space"]), p.value(&["--project"]))?;
            p.pos.clear();
            p.finish(0, false)?;
            let reply = post(&body)?;
            let run = reply.get("run").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &reply, &format!("Team run started: {}", run_line(&run)))
        }
        "stop" | "resume" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage(format!("teams {what} needs a run ID")))?;
            let reply = post(&simple_body(what, "run", id))?;
            let run = reply.get("run").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &reply, &run_line(&run))
        }
        "rm" | "remove" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage("teams rm needs a team ID"))?;
            let reply = post(&simple_body("remove", "id", id))?;
            emit(&ctx, out, &reply, &format!("Team {} is removed.", text_at(&reply, "removed")))
        }
        other => Err(args::usage(format!("models teams takes ls, start, stop, resume or rm, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies() {
        let body = start_body("t1", "fix login", Some("web".into()), None).unwrap();
        assert_eq!(body, json!({"action": "start", "payload": {"team": "t1", "task": "fix login", "space": "web"}}));
        assert!(start_body("t1", "", Some("web".into()), None).is_err());
        assert!(start_body("t1", "x", None, None).is_err());
        assert_eq!(simple_body("stop", "run", "r1"), json!({"action": "stop", "payload": {"run": "r1"}}));
    }

    #[test]
    fn list_text() {
        let snap = json!({"teams": [{"id": "t1", "name": "Feature", "flow": "task", "purpose": "", "steps": ["Builder", "Tester"]}], "runs": []});
        let lines = list_lines(&snap);
        assert_eq!(lines, vec!["1 teams.", "t1  Feature  (flow task)", "  steps: Builder -> Tester"]);
    }
}
