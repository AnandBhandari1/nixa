use crate::args::{self, Parsed, usage};
use crate::ctx::{self, Ctx};
use crate::{help, http};
use anyhow::Result;
use nixa_protocol::paths;
use serde_json::{Map, Value, json};
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::DEEP.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.first().is_some_and(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::DEEP.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "join" => join(ctx, rest, out),
        "leave" => leave(ctx, rest, out),
        "decide" | "reply" => decide(ctx, rest, out),
        "ask" => ask(ctx, rest, out),
        "start" => start(ctx, rest, out),
        "ls" | "list" | "show" => show(ctx, rest, out),
        "digest" => digest(ctx, rest, out),
        "goals" => goals(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa deep {other}"))),
    }
}

fn terminal(p: &mut Parsed) -> Result<String> {
    if let Some(id) = p.value(&["--terminal"]) {
        return Ok(crate::term::term_id(&id)?.to_string());
    }
    paths::current_term_id()
        .map(|id| id.to_string())
        .ok_or_else(|| usage("run it inside a Nixa terminal (NIXA_TERM_ID is not set), or give --terminal ID"))
}

fn put(body: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(v) = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        body.insert(key.to_string(), Value::String(v));
    }
}

fn put_caller(body: &mut Map<String, Value>, p: &mut Parsed) -> Result<()> {
    let terminal = match p.value(&["--terminal"]) {
        Some(id) => Some(crate::term::term_id(&id)?.to_string()),
        None => paths::current_term_id().map(|id| id.to_string()),
    };
    put(body, "terminal", terminal);
    Ok(())
}

fn print(ctx: &Ctx, out: &mut dyn Write, reply: &Value, line: String) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{line}")?;
    }
    Ok(0)
}

fn text(v: &Value, path: &[&str]) -> String {
    let mut at = v;
    for key in path {
        at = match at.get(*key) {
            Some(next) => next,
            None => return String::new(),
        };
    }
    match at {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn join(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--as", "--name", "--space", "--terminal"], false)?;
    let mut body = Map::new();
    body.insert("terminal".into(), Value::String(terminal(&mut p)?));
    put(&mut body, "as", p.value(&["--as"]));
    put(&mut body, "name", p.value(&["--name"]));
    put(&mut body, "space", p.value(&["--space"]));
    p.finish(0, false)?;
    let reply = http::post("/deep/join", &Value::Object(body))?;
    let line = format!(
        "Joined: deep brain {} ({}) of space {}.",
        text(&reply, &["seat", "id"]),
        text(&reply, &["seat", "name"]),
        text(&reply, &["seat", "spaceName"])
    );
    print(&ctx, out, &reply, line)
}

fn leave(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--terminal"], false)?;
    let body = json!({"terminal": terminal(&mut p)?});
    p.finish(0, false)?;
    let reply = http::post("/deep/leave", &body)?;
    let line = format!("Left: deep brain {}.", text(&reply, &["seat", "id"]));
    print(&ctx, out, &reply, line)
}

fn ask_id(id: &str) -> Result<String> {
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(usage(format!("bad request id {id:?}, ids look like d12")));
    }
    Ok(id.to_string())
}

fn decide(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--pick", "--why", "--learn", "--terminal"], false)?;
    let id = ask_id(p.pos.first().ok_or_else(|| usage("nixa deep decide needs a request id"))?)?;
    let pick = p.value(&["--pick"]).ok_or_else(|| usage("nixa deep decide needs --pick"))?;
    let why = p.value(&["--why"]).ok_or_else(|| usage("nixa deep decide needs --why"))?;
    let mut body = Map::new();
    body.insert("terminal".into(), Value::String(terminal(&mut p)?));
    body.insert("pick".into(), Value::String(pick));
    body.insert("why".into(), Value::String(why));
    put(&mut body, "learn", p.value(&["--learn"]));
    p.finish(1, false)?;
    let reply = http::post(&format!("/deep/{id}/reply"), &Value::Object(body))?;
    print(&ctx, out, &reply, format!("Answered {id}."))
}

fn ask(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let names = ["--option", "-o", "--notes", "--space", "--kind", "--wait", "--brain", "--terminal"];
    let (mut p, ctx) = ctx::command(ctx, argv, &names, false)?;
    let question = p.pos.iter().chain(p.rest.iter()).cloned().collect::<Vec<_>>().join(" ");
    if question.trim().is_empty() {
        return Err(usage("nixa deep ask needs a question"));
    }
    let mut body = Map::new();
    body.insert("question".into(), Value::String(question));
    put_caller(&mut body, &mut p)?;
    let options = p.values(&["--option", "-o"]);
    if !options.is_empty() {
        body.insert("options".into(), json!(options));
    }
    put(&mut body, "notes", p.value(&["--notes"]));
    put(&mut body, "space", p.value(&["--space"]));
    put(&mut body, "kind", p.value(&["--kind"]));
    put(&mut body, "brain", p.value(&["--brain"]));
    if let Some(wait) = p.value(&["--wait"]) {
        body.insert("wait".into(), json!(args::duration_ms(&wait)?));
    }
    p.finish(usize::MAX, true)?;
    let reply = http::post("/deep/ask", &Value::Object(body))?;
    let line = format!("Asked {}.", text(&reply, &["ask", "id"]));
    print(&ctx, out, &reply, line)
}

fn start(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--space", "--brain", "--terminal"], false)?;
    let mut body = Map::new();
    put_caller(&mut body, &mut p)?;
    put(&mut body, "space", p.value(&["--space"]));
    put(&mut body, "brain", p.value(&["--brain"]));
    p.finish(0, false)?;
    let reply = http::post("/deep/start", &Value::Object(body))?;
    let line = format!(
        "Starting a {} deep brain in space {}.",
        text(&reply, &["brain"]),
        text(&reply, &["space"])
    );
    print(&ctx, out, &reply, line)
}

fn show(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::get("/deep")?;
    let line = text(&reply, &["text"]);
    print(&ctx, out, &reply, if line.trim().is_empty() { "No deep brains.".into() } else { line })
}

fn digest(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--space", "--terminal"], false)?;
    let mut body = Map::new();
    put_caller(&mut body, &mut p)?;
    put(&mut body, "space", p.value(&["--space"]));
    p.finish(0, false)?;
    let reply = http::post("/deep/digest", &Value::Object(body))?;
    let line = text(&reply, &["text"]);
    print(&ctx, out, &reply, line)
}

pub const GOAL_DAYS_MAX: u64 = 30;

pub fn goal_days(word: Option<String>) -> Result<u64> {
    let Some(word) = word else { return Ok(7) };
    let days: u64 = args::number(word.trim(), "--days")?;
    if !(1..=GOAL_DAYS_MAX).contains(&days) {
        return Err(usage(format!("--days must be from 1 to {GOAL_DAYS_MAX}. Goal records are kept {GOAL_DAYS_MAX} days")));
    }
    Ok(days)
}

pub fn goal_lines(reply: &Value, days: u64) -> Vec<String> {
    let goals = reply.get("goals").and_then(Value::as_array).cloned().unwrap_or_default();
    let rows = reply.get("rows").and_then(Value::as_u64);
    let mut lines = vec![format!("Goals, last {days} days{}:", rows.map(|n| format!(", from {n} records")).unwrap_or_default())];
    if rows == Some(0) {
        lines.push("Nixa has no goal records in this time. Nothing to judge yet.".to_string());
    }
    let (mut pass, mut fail, mut open) = (0, 0, 0);
    for goal in &goals {
        let user = goal.get("user").and_then(Value::as_bool).unwrap_or(false);
        let verdict = match (user, goal.get("pass").and_then(Value::as_bool)) {
            (true, _) => {
                open += 1;
                "USER"
            }
            (false, Some(true)) => {
                pass += 1;
                "PASS"
            }
            (false, Some(false)) => {
                fail += 1;
                "FAIL"
            }
            (false, None) => {
                open += 1;
                "NO DATA"
            }
        };
        lines.push(format!("{verdict:<7} {}: {}. Target: {}.", text(goal, &["name"]), text(goal, &["value"]), text(goal, &["target"])));
    }
    let judged = pass + fail;
    let rate = if judged == 0 { "no goal could be judged".to_string() } else { format!("{pass} of {judged} pass ({}%)", pass * 100 / judged) };
    lines.push(format!("Pass rate: {rate}. {fail} fail, {open} without a verdict (USER means the user checks it)."));
    lines
}

fn goals(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--days"], false)?;
    let days = goal_days(p.value(&["--days"]))?;
    p.finish(0, false)?;
    let reply = http::window_call("GET", &format!("/extras/goals?days={days}"), None)?;
    let shown = reply.get("days").and_then(Value::as_u64).unwrap_or(7);
    let mut lines = goal_lines(&reply, shown);
    if shown != days {
        lines.push(format!("This nixa-brain is older and shows only {shown} days. Restart it after an update for --days."));
    }
    print(&ctx, out, &reply, lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_days_are_checked() {
        assert_eq!(goal_days(None).unwrap(), 7);
        assert_eq!(goal_days(Some("14".into())).unwrap(), 14);
        assert!(goal_days(Some("0".into())).is_err());
        assert!(goal_days(Some("31".into())).is_err());
        assert!(goal_days(Some("x".into())).is_err());
    }

    #[test]
    fn goal_lines_show_verdicts_and_rate() {
        let reply = json!({"rows": 9, "goals": [
            {"name": "Duplicate work", "value": "0 sends held", "target": "no real clash", "user": true},
            {"name": "Answer time", "value": "slowest 30 seconds", "target": "under 2 minutes", "pass": true},
            {"name": "Overruled", "value": "2 of 5", "target": "at most 1 in 10", "pass": false},
            {"name": "Spoken reply length", "value": "no spoken reply yet", "target": "about 20 words"}
        ]});
        let lines = goal_lines(&reply, 3);
        assert_eq!(lines[0], "Goals, last 3 days, from 9 records:");
        assert_eq!(lines[1], "USER    Duplicate work: 0 sends held. Target: no real clash.");
        assert_eq!(lines[2], "PASS    Answer time: slowest 30 seconds. Target: under 2 minutes.");
        assert_eq!(lines[3], "FAIL    Overruled: 2 of 5. Target: at most 1 in 10.");
        assert!(lines[4].starts_with("NO DATA Spoken reply length"));
        assert_eq!(lines[5], "Pass rate: 1 of 2 pass (50%). 1 fail, 2 without a verdict (USER means the user checks it).");
        let empty = goal_lines(&json!({"rows": 0, "goals": []}), 7);
        assert!(empty[1].contains("no goal records"));
        assert!(empty[2].starts_with("Pass rate: no goal could be judged."));
    }
}
