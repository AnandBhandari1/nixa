use crate::args::{Parsed, usage};
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::Write;

const WITH_VALUE: [&str; 4] = ["--until", "--note", "--to", "--terminal"];

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (sub, rest) = split(argv);
    if matches!(sub, "help" | "-h" | "--help") {
        out.write_all(help::AGENT.as_bytes())?;
        return Ok(0);
    }
    let (args, ctx) = crate::ctx::command(ctx, rest, &WITH_VALUE, false)?;
    let own = std::env::var(nixa_protocol::paths::ENV_TERM_ID).ok();
    let (path, body) = parse(sub, args, own.as_deref())?;
    let reply = match body {
        None => http::get(path)?,
        Some(body) => http::post(path, &Value::Object(body))?,
    };
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else if let Some(text) = reply.get("text").and_then(Value::as_str) {
        writeln!(out, "{text}")?;
    } else {
        writeln!(out, "{}", serde_json::to_string_pretty(&reply)?)?;
    }
    Ok(0)
}

pub fn split(argv: &[String]) -> (&str, &[String]) {
    match argv.first() {
        None => ("leave", argv),
        Some(first) if first.starts_with('-') && !matches!(first.as_str(), "-h" | "--help") => ("leave", argv),
        Some(first) => (first.as_str(), &argv[1..]),
    }
}

pub fn parse(sub: &str, mut args: Parsed, own: Option<&str>) -> Result<(&'static str, Option<Map<String, Value>>)> {
    let path = match sub {
        "leave" | "ls" | "list" | "status" => {
            args.finish(0, false)?;
            return Ok(("/agents/leave", None));
        }
        "limit" => "/agents/limit",
        "handover" | "hand-over" | "handoff" => "/agents/handover",
        other => return Err(usage(format!("unknown command nixa agent {other}"))),
    };
    let terminal = args
        .value(&["--terminal"])
        .or_else(|| own.map(str::to_string))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| usage("run it inside a Nixa terminal (NIXA_TERM_ID) or give --terminal ID"))?;
    let mut body = Map::new();
    body.insert("terminal".into(), json!(terminal));
    for (option, key) in [("--until", "until"), ("--note", "note"), ("--to", "to")] {
        if let Some(value) = args.value(&[option]) {
            body.insert(key.into(), json!(value));
        }
    }
    if path == "/agents/limit" {
        if !args.pos.is_empty() && !body.contains_key("note") {
            body.insert("note".into(), json!(args.pos.join(" ")));
            args.pos.clear();
        }
    } else if !body.get("note").and_then(Value::as_str).is_some_and(|n| !n.trim().is_empty()) {
        return Err(usage("handover needs --note \"what is done and what is left\""));
    }
    args.finish(0, false)?;
    Ok((path, Some(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_parse(sub: &str, words: &[&str], own: Option<&str>) -> Result<(&'static str, Option<Map<String, Value>>)> {
        let rest: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        parse(sub, crate::args::parse(&rest, &WITH_VALUE, false)?, own)
    }

    #[test]
    fn limit_uses_the_own_terminal_and_options() {
        let (path, body) = run_parse("limit", &["--until", "5pm", "--note", "tests left"], Some("nixa-codex-3")).unwrap();
        let body = body.unwrap();
        assert_eq!(path, "/agents/limit");
        assert_eq!(body["terminal"], json!("nixa-codex-3"));
        assert_eq!(body["until"], json!("5pm"));
        assert_eq!(body["note"], json!("tests left"));
        let (_, body) = run_parse("limit", &[], Some("m2")).unwrap();
        assert_eq!(body.unwrap().get("until"), None);
        let (_, body) = run_parse("limit", &["out", "of", "tokens"], Some("m2")).unwrap();
        assert_eq!(body.unwrap()["note"], json!("out of tokens"));
        assert!(run_parse("limit", &[], None).is_err());
        let (_, body) = run_parse("limit", &["--terminal", "m9"], None).unwrap();
        assert_eq!(body.unwrap()["terminal"], json!("m9"));
    }

    #[test]
    fn handover_needs_a_note_and_takes_an_engine() {
        assert!(run_parse("handover", &["--to", "claude"], Some("m2")).is_err());
        let (path, body) = run_parse("handover", &["--to", "claude", "--note", "half done"], Some("m2")).unwrap();
        let body = body.unwrap();
        assert_eq!(path, "/agents/handover");
        assert_eq!(body["to"], json!("claude"));
        assert_eq!(body["note"], json!("half done"));
    }

    #[test]
    fn a_flag_first_means_the_leave_list() {
        let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        let json = words(&["--json"]);
        assert_eq!(split(&json), ("leave", &json[..]));
        let term = words(&["--terminal", "m1"]);
        assert_eq!(split(&term).0, "leave");
        let limit = words(&["limit", "--note", "x"]);
        assert_eq!(split(&limit), ("limit", &limit[1..]));
        assert_eq!(split(&words(&["--help"])).0, "--help");
        assert_eq!(split(&[]).0, "leave");
    }

    #[test]
    fn leave_lists_and_bad_words_fail() {
        assert_eq!(run_parse("leave", &[], None).unwrap(), ("/agents/leave", None));
        assert!(run_parse("leave", &["x"], None).is_err());
        assert!(run_parse("bogus", &[], Some("m1")).is_err());
        assert!(run_parse("limit", &["--wrong", "x"], Some("m1")).is_err());
    }
}
