use super::{emit, flag_at, full_path, here, on_off, text_at};
use crate::args::{self, Parsed};
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

pub const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const ADD_VALUES: [&str; 8] = ["--name", "--agent", "--folder", "--prompt", "--repeat", "--time", "--date", "--days"];

pub fn row_line(s: &Value) -> String {
    let view = s.get("view").cloned().unwrap_or(Value::Null);
    let mut line = format!(
        "{}  {}  [{}]  {}  agent {}",
        text_at(s, "id"),
        text_at(s, "name"),
        on_off(flag_at(s, "enabled")),
        text_at(&view, "when"),
        text_at(&view, "agent_name")
    );
    let next = text_at(&view, "next");
    if !next.is_empty() {
        line.push_str(&format!("  next {next}"));
    }
    if flag_at(&view, "failed") {
        line.push_str("  last run failed");
    }
    line
}

pub fn list_lines(snap: &Value) -> Vec<String> {
    let rows = snap.get("schedules").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut lines = vec![format!(
        "Schedules are {}. {} saved. Time zone {}.",
        on_off(flag_at(snap, "enabled")),
        rows.len(),
        super::switches::or_unknown(text_at(snap, "zone"))
    )];
    lines.extend(rows.iter().map(row_line));
    if let Some(error) = snap.get("error").and_then(Value::as_str) {
        lines.push(error.to_string());
    }
    lines
}

pub fn days_of(text: &str) -> Result<Vec<i64>> {
    let mut days = Vec::new();
    for part in text.split(',').map(|d| d.trim().to_lowercase()).filter(|d| !d.is_empty()) {
        let day = match DAYS.iter().position(|d| part.starts_with(d)) {
            Some(i) => i as i64,
            None => part.parse::<i64>().ok().filter(|d| (0..=6).contains(d)).ok_or_else(|| args::usage(format!("bad day {part:?}: use sun..sat or 0..6")))?,
        };
        if !days.contains(&day) {
            days.push(day);
        }
    }
    days.sort();
    Ok(days)
}

pub fn add_body(p: &mut Parsed) -> Result<Value> {
    let mut need = |name: &str| p.value(&[name]).ok_or_else(|| args::usage(format!("schedules add needs {name}")));
    let name = need("--name")?;
    let agent = need("--agent")?;
    let time = need("--time")?;
    let repeat = p.value(&["--repeat"]).unwrap_or_else(|| "daily".into());
    let folder = full_path(&p.value(&["--folder"]).unwrap_or_else(here));
    let prompt = p.value(&["--prompt"]).unwrap_or_else(|| p.pos.join(" "));
    let mut schedule = json!({
        "name": name, "agent": agent, "folder": folder, "prompt": prompt,
        "repeat": repeat, "time": time, "enabled": !p.flag(&["--off"]),
    });
    if let Some(date) = p.value(&["--date"]) {
        schedule["date"] = json!(date);
    }
    if let Some(days) = p.value(&["--days"]) {
        schedule["days"] = json!(days_of(&days)?);
    }
    Ok(json!({"action": "schedule.save", "payload": {"schedule": schedule}}))
}

pub fn id_body(action: &str, id: &str, on: Option<bool>) -> Value {
    let mut payload = json!({"id": id});
    if let Some(on) = on {
        payload["enabled"] = json!(on);
    }
    json!({"action": action, "payload": payload})
}

fn post(ctx: &Ctx, out: &mut dyn Write, body: &Value, done: &str) -> Result<i32> {
    let snap = http::window_call("POST", "/schedules", Some(body))?;
    emit(ctx, out, &snap, done)
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("ls");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (mut p, ctx) = ctx::command(ctx, rest, &ADD_VALUES, false)?;
    match what {
        "ls" | "list" => {
            p.finish(0, false)?;
            let snap = http::window_call("GET", "/schedules", None)?;
            emit(&ctx, out, &snap, &list_lines(&snap).join("\n"))
        }
        "add" => {
            let body = add_body(&mut p)?;
            p.pos.clear();
            p.finish(0, false)?;
            post(&ctx, out, &body, "Schedule saved. See it with: nixa settings schedules")
        }
        "rm" | "remove" | "on" | "off" | "run" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage(format!("schedules {what} needs a schedule ID")))?;
            let (action, on, done) = match what {
                "on" => ("schedule.toggle", Some(true), format!("Schedule {id} is on.")),
                "off" => ("schedule.toggle", Some(false), format!("Schedule {id} is off.")),
                "run" => ("schedule.run", None, format!("Schedule {id} starts now.")),
                _ => ("schedule.delete", None, format!("Schedule {id} is removed.")),
            };
            post(&ctx, out, &id_body(action, id, on), &done)
        }
        other => Err(args::usage(format!("schedules takes ls, add, rm, on, off or run, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(words: &[&str]) -> Parsed {
        let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        args::parse(&argv, &ADD_VALUES, false).unwrap()
    }

    #[test]
    fn days_parse() {
        assert_eq!(days_of("mon, Wed,1,6").unwrap(), vec![1, 3, 6]);
        assert!(days_of("8").is_err());
    }

    #[test]
    fn add_body_shape() {
        let mut p = parsed(&["--name", "Night", "--agent", "claude", "--time", "22:00", "--folder", "/tmp", "--repeat", "weekly", "--days", "mon,fri", "--off", "check", "tests"]);
        let body = add_body(&mut p).unwrap();
        assert_eq!(body["action"], "schedule.save");
        let s = &body["payload"]["schedule"];
        assert_eq!(s["name"], "Night");
        assert_eq!(s["prompt"], "check tests");
        assert_eq!(s["days"], json!([1, 5]));
        assert_eq!(s["enabled"], false);
        assert_eq!(s["folder"], "/tmp");
        assert!(add_body(&mut parsed(&["--name", "x"])).is_err());
    }

    #[test]
    fn id_bodies_and_lines() {
        assert_eq!(id_body("schedule.toggle", "a1", Some(true)), json!({"action": "schedule.toggle", "payload": {"id": "a1", "enabled": true}}));
        let snap = json!({"enabled": true, "zone": "UTC", "schedules": [{"id": "a1", "name": "Night", "enabled": false, "view": {"when": "daily 22:00", "agent_name": "claude", "next": ""}}]});
        let lines = list_lines(&snap);
        assert_eq!(lines[0], "Schedules are on. 1 saved. Time zone UTC.");
        assert_eq!(lines[1], "a1  Night  [off]  daily 22:00  agent claude");
    }
}
