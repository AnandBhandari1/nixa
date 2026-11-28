mod brains;
mod cards;
mod find;
mod say;

use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::{help, http};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{Read, Write};

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::CHAT.as_bytes())?;
        return Ok(2);
    };
    let asks_help = matches!(sub.as_str(), "help" | "-h" | "--help")
        || rest.first().is_some_and(|a| a == "-h" || a == "--help");
    if asks_help {
        out.write_all(help::CHAT.as_bytes())?;
        return Ok(0);
    }
    match sub.as_str() {
        "say" => say::run(ctx, rest, out),
        "history" | "ls" => match rest.split_first() {
            Some((word, more)) if matches!(word.as_str(), "search" | "get" | "page") => find::history(ctx, word, more, out),
            _ => history(ctx, rest, out),
        },
        "search" => find::knowledge(ctx, rest, out),
        "clean" => simple(ctx, rest, out, "/chat/clean", |_| "The chat is clean.".into()),
        "stop" => simple(ctx, rest, out, "/chat/interrupt", |v| {
            if v.get("stopped").and_then(Value::as_bool) == Some(true) { "Stopped the turn.".into() } else { "Nothing was running.".into() }
        }),
        "cards" => cards::cards(ctx, rest, out),
        "approve" => cards::confirm(ctx, rest, out, true),
        "deny" => cards::confirm(ctx, rest, out, false),
        "questions" => cards::questions(ctx, rest, out),
        "answer" => cards::answer(ctx, rest, out),
        "brains" => brains::list(ctx, rest, out),
        "brain" => brains::brain(ctx, rest, out),
        "model" => brains::model(ctx, rest, out),
        "context" => context(ctx, rest, out),
        "compact" => simple(ctx, rest, out, "/compact", |v| format!("Compact: {}.", text(v, "outcome"))),
        "memory" => brains::memory(ctx, rest, out),
        other => Err(usage(format!("unknown command nixa chat {other}"))),
    }
}

pub(crate) fn print(ctx: &Ctx, out: &mut dyn Write, reply: &Value, line: &str) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(0)
}

pub(crate) fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub(crate) fn words(pos: &[String], rest: &[String]) -> Result<String> {
    let all: Vec<&str> = pos.iter().chain(rest).map(String::as_str).collect();
    let joined = if all == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        all.join(" ")
    };
    let joined = joined.trim().to_string();
    if joined.is_empty() {
        return Err(usage("give the text, or - to read it from stdin"));
    }
    Ok(joined)
}

pub(crate) fn item_line(item: &Value) -> String {
    let who = match item.get("who").and_then(Value::as_str).unwrap_or("") {
        "user" => "You".to_string(),
        "nixa" => "Nixa".to_string(),
        "info" => "Info".to_string(),
        "agent" => item.get("label").and_then(Value::as_str).unwrap_or("Agent").to_string(),
        other => other.to_string(),
    };
    format!("#{} {who}: {}", text(item, "id"), text(item, "text").trim())
}

pub(crate) fn last_items(reply: &Value, last: usize) -> Vec<Value> {
    let items = reply.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    let skip = items.len().saturating_sub(last);
    items.into_iter().skip(skip).collect()
}

fn history(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--last", "-n"], false)?;
    let last = p.value(&["--last", "-n"]).map(|v| args::number::<usize>(&v, "--last")).transpose()?.unwrap_or(20);
    p.finish(0, false)?;
    let reply = http::get("/chat")?;
    let items = last_items(&reply, last);
    let busy = reply.get("busy").and_then(Value::as_bool) == Some(true);
    let mut lines: Vec<String> = items.iter().map(item_line).collect();
    if lines.is_empty() {
        lines.push("The chat is empty.".into());
    }
    if busy {
        lines.push("Nixa is working on a turn now.".into());
    }
    print(&ctx, out, &json!({"ok": true, "items": items, "busy": busy}), &lines.join("\n"))
}

fn simple(ctx: &Ctx, argv: &[String], out: &mut dyn Write, path: &str, line: fn(&Value) -> String) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::post(path, &json!({}))?;
    print(&ctx, out, &reply, &line(&reply))
}

fn context(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::get("/context")?;
    print(&ctx, out, &reply, &text(&reply, "text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn words_join_and_refuse_empty() {
        assert_eq!(words(&strs(&["hi", "there"]), &strs(&["-x"])).unwrap(), "hi there -x");
        assert!(words(&[], &[]).is_err());
        assert!(words(&strs(&["  "]), &[]).is_err());
    }

    #[test]
    fn lines_and_last_items() {
        let reply = json!({"items": [
            {"id": 1, "who": "user", "text": "hello"},
            {"id": 2, "who": "nixa", "text": "hi\n"},
            {"id": 3, "who": "agent", "label": "Codex", "text": "done"},
        ]});
        let items = last_items(&reply, 2);
        assert_eq!(items.len(), 2);
        assert_eq!(item_line(&items[0]), "#2 Nixa: hi");
        assert_eq!(item_line(&items[1]), "#3 Codex: done");
        assert_eq!(last_items(&reply, 9).len(), 3);
        assert!(last_items(&json!({}), 3).is_empty());
    }
}
