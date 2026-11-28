use super::{emit, flag_at, on_off, text_at};
use super::schedules::id_body;
use crate::args::{self, Parsed};
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

const VALUES: [&str; 4] = ["--every", "--unit", "--text", "--id"];

pub fn row_line(r: &Value) -> String {
    let view = r.get("view").cloned().unwrap_or(Value::Null);
    let mut line = format!(
        "{}  [{}]  {}  next {}  sent {}",
        text_at(r, "id"),
        on_off(flag_at(r, "enabled")),
        text_at(&view, "every"),
        text_at(&view, "next"),
        r.get("sent").and_then(Value::as_u64).unwrap_or(0)
    );
    if let Some(note) = r.get("last_note").and_then(Value::as_str) {
        line.push_str(&format!("  last note: {note}"));
    }
    let text: String = text_at(r, "text").lines().next().unwrap_or("").chars().take(60).collect();
    line.push_str(&format!("\n    {text}"));
    line
}

pub fn list_lines(snap: &Value) -> Vec<String> {
    let rows = snap.get("repeats").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut lines = vec![format!("{} repeat messages saved.", rows.len())];
    lines.extend(rows.iter().map(row_line));
    if let Some(error) = snap.get("error").and_then(Value::as_str) {
        lines.push(error.to_string());
    }
    lines
}

pub fn save_body(p: &mut Parsed) -> Result<Value> {
    let every = p.value(&["--every"]).ok_or_else(|| args::usage("repeats add needs --every N"))?;
    let every: i64 = every.trim().parse().map_err(|_| args::usage(format!("--every must be a whole number, not {every:?}")))?;
    let unit = p.value(&["--unit"]).unwrap_or_else(|| "minutes".into());
    let text = p.value(&["--text"]).unwrap_or_else(|| p.pos.join(" "));
    if text.trim().is_empty() {
        return Err(args::usage("repeats add needs the message: --text TEXT or TEXT..."));
    }
    let mut repeat = json!({"text": text, "every": every, "unit": unit, "enabled": !p.flag(&["--off"])});
    if let Some(id) = p.value(&["--id"]) {
        repeat["id"] = json!(id);
    }
    Ok(json!({"action": "repeat.save", "payload": {"repeat": repeat}}))
}

fn post(ctx: &Ctx, out: &mut dyn Write, body: &Value, done: &str) -> Result<i32> {
    let snap = http::window_call("POST", "/schedules", Some(body))?;
    emit(ctx, out, &snap, done)
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("ls");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (mut p, ctx) = ctx::command(ctx, rest, &VALUES, false)?;
    match what {
        "ls" | "list" => {
            p.finish(0, false)?;
            let snap = http::window_call("GET", "/schedules", None)?;
            emit(&ctx, out, &snap, &list_lines(&snap).join("\n"))
        }
        "add" | "edit" => {
            if what == "edit" && p.value(&["--id"]).is_none() {
                return Err(args::usage("repeats edit needs --id ID"));
            }
            let body = save_body(&mut p)?;
            p.pos.clear();
            p.finish(0, false)?;
            post(&ctx, out, &body, "Repeat message saved. See it with: nixa settings repeats")
        }
        "rm" | "remove" | "on" | "off" | "send" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage(format!("repeats {what} needs a repeat ID")))?;
            let (action, on, done) = match what {
                "on" => ("repeat.toggle", Some(true), format!("Repeat {id} is on.")),
                "off" => ("repeat.toggle", Some(false), format!("Repeat {id} is off.")),
                "send" => ("repeat.send", None, format!("Repeat {id} is sent now.")),
                _ => ("repeat.delete", None, format!("Repeat {id} is removed.")),
            };
            post(&ctx, out, &id_body(action, id, on), &done)
        }
        other => Err(args::usage(format!("repeats takes ls, add, edit, rm, on, off or send, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(words: &[&str]) -> Parsed {
        let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        args::parse(&argv, &VALUES, false).unwrap()
    }

    #[test]
    fn save_body_shape() {
        let body = save_body(&mut parsed(&["--every", "15", "check", "idle", "agents"])).unwrap();
        assert_eq!(body["action"], "repeat.save");
        let r = &body["payload"]["repeat"];
        assert_eq!(r["text"], "check idle agents");
        assert_eq!(r["every"], 15);
        assert_eq!(r["unit"], "minutes");
        assert_eq!(r["enabled"], true);
        assert!(r.get("id").is_none());
        let body = save_body(&mut parsed(&["--every", "2", "--unit", "hours", "--id", "rep-1", "--off", "--text", "hi"])).unwrap();
        assert_eq!(body["payload"]["repeat"]["id"], "rep-1");
        assert_eq!(body["payload"]["repeat"]["enabled"], false);
        assert!(save_body(&mut parsed(&["--every", "x", "hi"])).is_err());
        assert!(save_body(&mut parsed(&["hi"])).is_err());
        assert!(save_body(&mut parsed(&["--every", "5"])).is_err());
    }

    #[test]
    fn list_lines_shape() {
        let snap = json!({"repeats": [{"id": "rep-1", "enabled": true, "text": "Check-in.\nmore", "sent": 3, "view": {"every": "Every 15 minutes", "next": "15:30"}}]});
        let lines = list_lines(&snap);
        assert_eq!(lines[0], "1 repeat messages saved.");
        assert_eq!(lines[1], "rep-1  [on]  Every 15 minutes  next 15:30  sent 3\n    Check-in.");
    }
}
