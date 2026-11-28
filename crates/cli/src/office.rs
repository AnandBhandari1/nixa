use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::{help, http};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;
use std::time::Duration;

const TEST_WAIT: Duration = Duration::from_secs(45);
const FUN_WAIT: Duration = Duration::from_secs(15);
const WORDS_MAX: usize = 64;

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    match argv.split_first() {
        Some((first, rest)) if first == "soap" => soap(ctx, rest, out),
        Some((first, rest)) if first == "fun" => fun(ctx, rest, out),
        Some((first, rest)) if first == "talk" => talk(ctx, rest, out),
        Some((first, _)) if matches!(first.as_str(), "help" | "-h" | "--help") => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(0)
        }
        Some((other, _)) => Err(usage(format!("unknown command nixa office {other}"))),
        None => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(2)
        }
    }
}

fn soap(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let Some((sub, rest)) = argv.split_first() else {
        out.write_all(help::OFFICE.as_bytes())?;
        return Ok(2);
    };
    match sub.as_str() {
        "status" => {
            let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/office/soap", None)?;
            print(out, ctx.json, &reply, status_lines)
        }
        "model" => {
            let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
            p.finish(1, false)?;
            let Some(name) = p.pos.first() else {
                return Err(usage("nixa office soap model NAME: the Ollama model name is needed"));
            };
            let reply = http::window_call("POST", "/office/soap/model", Some(&json!({"model": name})))?;
            print(out, ctx.json, &reply, |r| {
                let mut lines = vec![format!("Office soap model is now {}.", text(r, "model"))];
                lines.extend(status_lines(r).into_iter().skip(1));
                lines
            })
        }
        "test" => {
            let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
            p.finish(0, false)?;
            let reply = http::window_call_within("POST", "/office/soap/test", None, TEST_WAIT)?;
            print(out, ctx.json, &reply, scene_lines)
        }
        "help" | "-h" | "--help" => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(0)
        }
        other => Err(usage(format!("unknown command nixa office soap {other}"))),
    }
}

fn talk(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    match argv.split_first() {
        Some((sub, rest)) if sub == "status" => {
            let (p, ctx) = ctx::command(ctx, rest, &[], false)?;
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/office/talk", None)?;
            print(out, ctx.json, &reply, talk_lines)
        }
        Some((sub, _)) if matches!(sub.as_str(), "help" | "-h" | "--help") => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(0)
        }
        Some((other, _)) => Err(usage(format!("unknown command nixa office talk {other}"))),
        None => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(2)
        }
    }
}

pub fn talk_lines(reply: &Value) -> Vec<String> {
    let on = reply.get("voice_on").and_then(Value::as_bool) == Some(true);
    let num = |k: &str| reply.get(k).and_then(Value::as_u64).unwrap_or(0);
    let line = |v: &Value| format!("{} ({}): {}", text(v, "who"), text(v, "src"), text(v, "text"));
    let mut lines = vec![format!("Office voice: {}", if on { "on" } else { "off" })];
    match reply.get("speaking").filter(|s| !s.is_null()) {
        Some(s) => lines.push(format!("Speaking now: {}", line(s))),
        None => lines.push("Speaking now: nothing".into()),
    }
    if let Some(last) = reply.get("last").filter(|l| !l.is_null()) {
        lines.push(format!("Last line {}: {}", text(last, "state"), line(last)));
    }
    lines.push(format!("Lines started: {}, ended: {}, skipped: {}", num("started"), num("ended"), num("skipped")));
    lines
}

fn fun(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    match argv.split_first() {
        Some((sub, rest)) if sub == "test" => {
            let (mut p, ctx) = ctx::command(ctx, rest, &["--who"], false)?;
            let who = p.value(&["--who"]);
            p.finish(WORDS_MAX, false)?;
            let line = p.pos.join(" ");
            if line.trim().is_empty() {
                return Err(usage("nixa office fun test LINE: the line to rewrite is needed"));
            }
            let body = json!({"text": line, "who": who});
            let reply = http::window_call_within("POST", "/office/fun/test", Some(&body), FUN_WAIT)?;
            print(out, ctx.json, &reply, fun_lines)
        }
        Some((sub, _)) if matches!(sub.as_str(), "help" | "-h" | "--help") => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(0)
        }
        Some((other, _)) => Err(usage(format!("unknown command nixa office fun {other}"))),
        None => {
            out.write_all(help::OFFICE.as_bytes())?;
            Ok(2)
        }
    }
}

pub fn fun_lines(reply: &Value) -> Vec<String> {
    let line = reply.get("line").cloned().unwrap_or(Value::Null);
    let mut lines = vec![format!("{} said: {}", text(reply, "who"), text(reply, "original"))];
    if line.get("changed").and_then(Value::as_bool) == Some(true) {
        lines.push(format!("Office line: {}", text(&line, "text")));
    } else {
        let reason = line.get("reason").and_then(Value::as_str).unwrap_or("the model kept it as it was");
        lines.push(format!("Kept the real line: {reason}"));
    }
    let serious = reply.get("serious").and_then(Value::as_bool) == Some(true);
    lines.push(format!("Serious: {}. Model: {}", if serious { "yes, no jokes" } else { "no" }, text(reply, "model")));
    lines
}

pub fn fun_status(fun: &Value) -> Vec<String> {
    if fun.is_null() {
        return Vec::new();
    }
    let flag = |k: &str| fun.get(k).and_then(Value::as_bool) == Some(true);
    let num = |k: &str| fun.get(k).and_then(Value::as_u64).unwrap_or(0);
    let mut lines = vec![format!(
        "Office fun layer: {} (soap switch {}, Office view {})",
        if flag("on") { "on" } else { "off" },
        if flag("switch") { "on" } else { "off" },
        if flag("office") { "open" } else { "closed" }
    )];
    lines.push(format!("Rewrites: {}, kept real lines: {}", num("rewrites"), num("fallbacks")));
    if let Some(last) = fun.get("last").filter(|l| !l.is_null()) {
        lines.push(format!("Last rewrite: {} -> {}", text(last, "original"), text(last, "text")));
    }
    if let Some(reason) = fun.get("last_reason").and_then(Value::as_str) {
        lines.push(format!("Last fallback: {reason}"));
    }
    lines
}

fn print(out: &mut dyn Write, json: bool, reply: &Value, lines: impl Fn(&Value) -> Vec<String>) -> Result<i32> {
    if json {
        writeln!(out, "{}", serde_json::to_string(reply)?)?;
    } else {
        writeln!(out, "{}", lines(reply).join("\n"))?;
    }
    Ok(0)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn status_lines(reply: &Value) -> Vec<String> {
    let models: Vec<&str> = reply.get("models").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let model = text(reply, "model");
    let mut lines = vec![format!("Model: {model}"), format!("Url: {}", text(reply, "url"))];
    match reply.get("error").and_then(Value::as_str) {
        Some(error) => lines.push(format!("Reachable: no. {error}")),
        None => {
            lines.push("Reachable: yes".into());
            if !models.contains(&model) {
                lines.push(format!("The model {model} is not installed. Pick one below or run: ollama pull {model}"));
            }
            lines.push(format!("Installed models ({}):", models.len()));
            lines.extend(models.iter().map(|m| format!("  {}{m}", if *m == model { "* " } else { "  " })));
        }
    }
    lines.extend(fun_status(reply.get("fun").unwrap_or(&Value::Null)));
    lines
}

pub fn scene_lines(reply: &Value) -> Vec<String> {
    let made = reply.get("scene").cloned().unwrap_or(Value::Null);
    let name = |id: &str| reply.get("names").and_then(|n| n.get(id)).and_then(Value::as_str).unwrap_or(id).to_string();
    let mut lines = vec![format!(
        "Scene from {} at the {}, mood {}:",
        text(reply, "model"),
        text(&made, "where"),
        text(&made, "mood")
    )];
    for line in made.get("lines").and_then(Value::as_array).into_iter().flatten() {
        let act = text(line, "act");
        let act = if act.is_empty() || act == "none" { String::new() } else { format!(" ({act})") };
        lines.push(format!("  {}: {}{act}", name(text(line, "who")), text(line, "text")));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_shows_models_and_errors() {
        let ok = json!({"model": "a:1", "url": "http://127.0.0.1:11434", "models": ["a:1", "b:2"], "error": null});
        let lines = status_lines(&ok);
        assert_eq!(lines[2], "Reachable: yes");
        assert!(lines.contains(&"  * a:1".to_string()));
        let missing = status_lines(&json!({"model": "c:3", "url": "u", "models": ["a:1"], "error": null}));
        assert!(missing[3].contains("not installed"));
        let down = status_lines(&json!({"model": "a:1", "url": "u", "models": [], "error": "Ollama is not reachable at u."}));
        assert_eq!(down[2], "Reachable: no. Ollama is not reachable at u.");
    }

    #[test]
    fn fun_lines_show_rewrite_fallback_and_state() {
        let made = json!({"who": "Nova", "original": "Task #4 is done.", "model": "m", "serious": false, "line": {"text": "Task #4 is done and dusted!", "changed": true}});
        let lines = fun_lines(&made);
        assert_eq!(lines[1], "Office line: Task #4 is done and dusted!");
        let kept = json!({"who": "Nova", "original": "Build failed.", "serious": true, "line": {"text": "Build failed.", "changed": false, "reason": "Ollama took too long"}});
        let lines = fun_lines(&kept);
        assert_eq!(lines[1], "Kept the real line: Ollama took too long");
        assert!(lines[2].contains("no jokes"));
        let state = json!({"on": true, "switch": true, "office": true, "rewrites": 3, "fallbacks": 1, "last": {"original": "a", "text": "b"}, "last_reason": "the rewrite lost \"4\""});
        let lines = fun_status(&state);
        assert_eq!(lines[0], "Office fun layer: on (soap switch on, Office view open)");
        let talk = talk_lines(&json!({"voice_on": true, "speaking": {"who": "m1", "src": "office", "text": "On it!"},
            "last": {"who": "nixa", "src": "chat", "state": "end", "text": "Done."}, "started": 2, "ended": 1, "skipped": 3}));
        assert_eq!(talk, vec!["Office voice: on", "Speaking now: m1 (office): On it!", "Last line end: nixa (chat): Done.",
            "Lines started: 2, ended: 1, skipped: 3"]);
        assert_eq!(talk_lines(&json!({}))[1], "Speaking now: nothing");
        assert_eq!(lines[1], "Rewrites: 3, kept real lines: 1");
        assert_eq!(lines[2], "Last rewrite: a -> b");
        assert!(fun_status(&Value::Null).is_empty());
        let status = status_lines(&json!({"model": "a", "url": "u", "models": [], "error": "down", "fun": {"on": false}}));
        assert!(status.last().unwrap().starts_with("Rewrites"));
    }

    #[test]
    fn scene_uses_names() {
        let reply = json!({"model": "a:1", "names": {"m1": "Asha"}, "scene": {"where": "tea", "mood": "fun", "lines": [{"who": "m1", "text": "Hi", "act": "wave"}, {"who": "m2", "text": "Yo", "act": "none"}]}});
        let lines = scene_lines(&reply);
        assert_eq!(lines[0], "Scene from a:1 at the tea, mood fun:");
        assert_eq!(lines[1], "  Asha: Hi (wave)");
        assert_eq!(lines[2], "  m2: Yo");
    }
}
