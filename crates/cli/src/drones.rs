//! CLI for Nixa's drones (`nixa drones ...`).

use crate::args::{Parsed, number, usage};
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::Write;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let sub = argv.first().map(String::as_str).unwrap_or("read");
    if matches!(sub, "help" | "-h" | "--help") {
        out.write_all(help::DRONES.as_bytes())?;
        return Ok(0);
    }
    let rest = if argv.is_empty() { &[][..] } else { &argv[1..] };
    let (args, ctx) = crate::ctx::command(ctx, rest, &["--step", "--every", "--remind"], false)?;
    let (action, body) = parse(sub, args)?;
    let who = std::env::var("NIXA_TERM_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|id| format!("cli ({id})"))
        .unwrap_or_else(|| "cli".into());
    let mut payload = body;
    payload.insert("who".into(), json!(who));
    let path = format!("/drones/{action}");
    let reply = if matches!(action, "read" | "facts" | "history") {
        http::get(&path)?
    } else {
        http::post(&path, &Value::Object(payload))?
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

fn parse(sub: &str, mut args: Parsed) -> Result<(&str, Map<String, Value>)> {
    let action = match sub {
        "read" | "facts" | "run" | "add" | "change" | "remove" | "on" | "off" | "timing"
        | "undo" | "history" => sub,
        "dry" | "dry_run" | "dry-run" => "dry_run",
        other => return Err(usage(format!("unknown command nixa drones {other}"))),
    };
    let mut body = Map::new();
    match action {
        "add" | "change" | "dry_run" => {
            if let Some(raw) = args.value(&["--step"]) {
                let step: Map<String, Value> = serde_json::from_str(&raw)
                    .map_err(|e| usage(format!("--step must be a JSON object: {e}")))?;
                body.insert("step".into(), Value::Object(step));
            }
            if (action != "dry_run" || !args.pos.is_empty() || !body.is_empty())
                && (args.pos.is_empty() || !body.contains_key("step"))
            {
                return Err(usage(format!("{sub} needs ID and --step JSON")));
            }
        }
        "timing" => {
            for (option, key) in [("--every", "every"), ("--remind", "remind")] {
                if let Some(raw) = args.value(&[option]) {
                    body.insert(key.into(), json!(number::<u64>(&raw, option)?));
                }
            }
            if body.is_empty() {
                return Err(usage("timing needs --every or --remind"));
            }
        }
        "remove" if args.pos.is_empty() => return Err(usage("remove needs a step ID")),
        _ => {}
    }
    let takes_id = matches!(
        action,
        "add" | "change" | "remove" | "on" | "off" | "dry_run"
    );
    args.finish(usize::from(takes_id), false)?;
    if let Some(id) = args.pos.into_iter().next() {
        body.insert("id".into(), json!(id));
    }
    Ok((action, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args<'a>(sub: &'a str, rest: &[String]) -> Result<(&'a str, Map<String, Value>)> {
        let args = crate::args::parse(rest, &["--step", "--every", "--remind"], false)?;
        parse(sub, args)
    }

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_dry_timing_and_toggle() {
        let (act, body) = parse_args(
            "dry",
            &[
                "b-inbox".into(),
                "--step".into(),
                r#"{"kind":"wait"}"#.into(),
            ],
        )
        .unwrap();
        assert_eq!(act, "dry_run");
        assert_eq!(body["id"], json!("b-inbox"));
        assert_eq!(body["step"]["kind"], json!("wait"));

        let (act, body) = parse_args("timing", &words("--every 5 --remind 30")).unwrap();
        assert_eq!(act, "timing");
        assert_eq!(body["every"], json!(5));
        assert_eq!(body["remind"], json!(30));

        let (act, body) = parse_args("off", &words("d-idle")).unwrap();
        assert_eq!(act, "off");
        assert_eq!(body["id"], json!("d-idle"));

        assert!(parse_args("bogus", &[]).is_err());
        assert!(parse_args("off", &words("--remind 30")).is_err());
        assert!(parse_args("read", &words("some-id")).is_err());
        assert!(parse_args("dry", &words("some-id")).is_err());
        assert!(parse_args("change", &words("some-id --step []")).is_err());
    }
}
