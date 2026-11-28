mod knowledge;
mod repeats;
pub(crate) mod rules;
mod schedules;
mod secret;
mod switches;
mod web;

use crate::args;
use crate::ctx::{self, Ctx};
use crate::{help, http};
use anyhow::Result;
use serde_json::Value;
use std::io::Write;

pub fn emit(ctx: &Ctx, out: &mut dyn Write, value: &Value, text: &str) -> Result<i32> {
    if ctx.json {
        writeln!(out, "{}", serde_json::to_string(value)?)?;
    } else {
        writeln!(out, "{text}")?;
    }
    Ok(0)
}

pub fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn flag_at(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn on_word(word: &str, what: &str) -> Result<bool> {
    match word {
        "on" | "yes" | "true" => Ok(true),
        "off" | "no" | "false" => Ok(false),
        _ => Err(args::usage(format!("{what} is on or off"))),
    }
}

pub fn here() -> String {
    std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default()
}

pub fn full_path(dir: &str) -> String {
    let path = std::path::Path::new(dir);
    if path.is_absolute() || dir.starts_with('~') {
        return dir.to_string();
    }
    std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or_else(|_| format!("{}/{dir}", here()))
}

fn wants_help(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help"))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if wants_help(argv) {
        out.write_all(help::SETTINGS.as_bytes())?;
        return Ok(0);
    }
    let what = argv.first().map(String::as_str).unwrap_or("show");
    let rest = argv.get(1..).unwrap_or(&[]);
    match what {
        "show" | "ls" => show(ctx, rest, out),
        "privacy" => switches::privacy(ctx, rest, out),
        "keyword-clear" => switches::keyword_clear(ctx, rest, out),
        "web" => web::run(ctx, rest, out),
        "schedules" | "schedule" => schedules::run(ctx, rest, out),
        "repeats" | "repeat" => repeats::run(ctx, rest, out),
        "answer-rules" | "rules" => rules::run(ctx, rest, out),
        "knowledge" => knowledge::run(ctx, rest, out),
        "--json" => show(ctx, argv, out),
        other => Err(args::usage(format!("unknown settings part {other:?}, see nixa help settings"))),
    }
}

pub fn show_lines(switches: &Value, web: Option<&Value>, decision: Option<&Value>) -> Vec<String> {
    let work = switches.get("work").cloned().unwrap_or(Value::Null);
    let mut lines = vec![
        format!("Privacy: {}", on_off(flag_at(switches, "privacy"))),
    ];
    if let Some(topic) = work.get("topic").filter(|t| t.is_object()) {
        lines.push(format!("Topic: {} {}", text_at(topic, "kind"), text_at(topic, "text")));
    }
    if work.get("hold").is_some_and(Value::is_object) {
        lines.push("Hold: on. Nixa waits until you say go on.".into());
    }
    lines.push(match web {
        Some(web) => web::line(web),
        None => "Web search: unknown".into(),
    });
    lines.push(match decision {
        Some(d) => format!("Who decides: {}", switches::or_unknown(text_at(d, "name"))),
        None => "Who decides: unknown".into(),
    });
    lines.push(format!("Git voice: {}", on_off(flag_at(switches, "gitVoice"))));
    lines
}

fn show(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::window_call("GET", "/switches", None)?;
    let switches = reply.get("switches").cloned().unwrap_or(Value::Null);
    let web = http::window_call("GET", "/web", None).ok().and_then(|r| r.get("web").cloned());
    let decision = http::get("/decision").ok();
    let all = serde_json::json!({"switches": switches, "web": web, "decision": decision});
    emit(&ctx, out, &all, &show_lines(&switches, web.as_ref(), decision.as_ref()).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn show_has_the_main_switches() {
        let switches = json!({"privacy": true, "gitVoice": false, "work": {"hold": {"at": 1}}});
        let web = json!({"on": false, "provider": "searxng", "exa_key": false, "searxng": {"installed": true, "running": false}});
        let decision = json!({"full": "deep", "name": "The deep brain decides"});
        let lines = show_lines(&switches, Some(&web), Some(&decision));
        assert_eq!(lines[0], "Privacy: on");
        assert!(lines[1].starts_with("Hold: on"));
        assert!(lines[2].starts_with("Web search: off"));
        assert_eq!(lines[3], "Who decides: The deep brain decides");
        let bare = show_lines(&json!({}), None, None);
        assert_eq!(bare[1], "Web search: unknown");
        assert_eq!(bare[2], "Who decides: unknown");
    }

    #[test]
    fn on_words() {
        assert!(on_word("on", "x").unwrap());
        assert!(!on_word("off", "x").unwrap());
        assert!(on_word("maybe", "x").is_err());
    }
}
