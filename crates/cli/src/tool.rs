use crate::args::usage;
use crate::ctx::Ctx;
use crate::{help, http};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::io::Write;
use std::time::Duration;

const CALL_WAIT: Duration = Duration::from_secs(630);

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let sub = argv.first().map(String::as_str).unwrap_or("list");
    if matches!(sub, "help" | "-h" | "--help") {
        out.write_all(help::TOOL.as_bytes())?;
        return Ok(0);
    }
    let rest = if argv.is_empty() { &[][..] } else { &argv[1..] };
    let (mut args, ctx) = crate::ctx::command(ctx, rest, &["--args"], false)?;
    match sub {
        "list" => list(&ctx, args.pos.first().map(String::as_str), out),
        "show" => {
            let name = args.pos.first().cloned().ok_or_else(|| usage("nixa tool show NAME"))?;
            show(&ctx, &name, out)
        }
        "call" => {
            let name = args.pos.first().cloned().ok_or_else(|| usage("nixa tool call NAME [JSON] [--args JSON]"))?;
            let raw = args.value(&["--args"]).or_else(|| args.pos.get(1).cloned()).unwrap_or_else(|| "{}".into());
            let input: Value = serde_json::from_str(&raw).map_err(|e| usage(format!("the tool input must be a JSON object: {e}")))?;
            if !input.is_object() {
                return Err(usage("the tool input must be a JSON object"));
            }
            call(&ctx, &name, input, out)
        }
        other => Err(usage(format!("unknown command nixa tool {other}"))),
    }
}

pub fn screenshot(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help")) {
        out.write_all(help::SCREENSHOT.as_bytes())?;
        return Ok(0);
    }
    let (mut args, ctx) = crate::ctx::command(ctx, argv, &["--window", "--name", "--question"], false)?;
    let input = screenshot_input(args.value(&["--window"]), args.value(&["--name"]), args.value(&["--question"]));
    call(&ctx, "nixa_eyes", input, out)
}

pub fn flow(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let sub = argv.first().map(String::as_str).unwrap_or("list");
    if matches!(sub, "help" | "-h" | "--help") {
        out.write_all(help::FLOW.as_bytes())?;
        return Ok(0);
    }
    let rest = if argv.is_empty() { &[][..] } else { &argv[1..] };
    let (args, ctx) = crate::ctx::command(ctx, rest, &[], false)?;
    let input = flow_input(sub, &args.pos, &std::env::current_dir().unwrap_or_default())?;
    call(&ctx, "nixa_flowman", input, out)
}

pub fn flow_input(sub: &str, pos: &[String], here: &std::path::Path) -> Result<Value> {
    let first = pos.first().cloned();
    Ok(match sub {
        "list" | "active" => json!({"action": sub}),
        "export" => match first {
            Some(flow) => json!({"action": "export", "flow": flow}),
            None => json!({"action": "export"}),
        },
        "show" | "open" | "apply" => {
            let flow = first.ok_or_else(|| usage(format!("nixa flow {sub} FLOW (tree/NAME, drones or workflow/NAME)")))?;
            json!({"action": sub, "flow": flow})
        }
        "blueprint" => {
            let dir = first.map(|d| here.join(d)).unwrap_or_else(|| here.to_path_buf());
            let mut input = json!({"action": "blueprint", "dir": dir.to_string_lossy()});
            if let Some(file) = pos.get(1) {
                input["file"] = json!(file);
            }
            input
        }
        other => return Err(usage(format!("unknown command nixa flow {other}. Run nixa flow help."))),
    })
}

pub fn screenshot_input(window: Option<String>, name: Option<String>, question: Option<String>) -> Value {
    if let Some(name) = name {
        let mut input = json!({"action": "shot", "name": name});
        if let Some(q) = question {
            input = json!({"action": "vision", "name": input["name"], "question": q});
        }
        return input;
    }
    let window = match window.as_deref() {
        None | Some("screen") => json!("screen"),
        Some("nixa") => json!("nixa"),
        Some(id) => id.parse::<u64>().map(|n| json!(n)).unwrap_or_else(|_| json!(id)),
    };
    json!({"action": "read", "window": window})
}

fn tools() -> Result<Vec<Value>> {
    let reply = http::get("/agent/tools")?;
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        bail!("{}", reply.get("reason").and_then(Value::as_str).unwrap_or("Nixa did not list its tools"));
    }
    Ok(reply.get("tools").and_then(Value::as_array).cloned().unwrap_or_default())
}

fn first_line(text: &str) -> String {
    let line = text.split(". ").next().unwrap_or(text).trim().trim_end_matches('.');
    line.chars().take(110).collect()
}

fn list(ctx: &Ctx, filter: Option<&str>, out: &mut dyn Write) -> Result<i32> {
    let all = tools()?;
    let want = filter.map(str::to_lowercase);
    let shown: Vec<&Value> = all
        .iter()
        .filter(|t| want.as_ref().is_none_or(|w| t.to_string().to_lowercase().contains(w.as_str())))
        .collect();
    if ctx.json {
        writeln!(out, "{}", Value::Array(shown.into_iter().cloned().collect()))?;
        return Ok(0);
    }
    for t in shown {
        let name = t["name"].as_str().unwrap_or("");
        writeln!(out, "{name:<28} {}", first_line(t["description"].as_str().unwrap_or("")))?;
    }
    writeln!(out, "\nRun one: nixa tool call NAME '{{\"action\": \"...\"}}'. Details: nixa tool show NAME.")?;
    Ok(0)
}

fn show(ctx: &Ctx, name: &str, out: &mut dyn Write) -> Result<i32> {
    let all = tools()?;
    let Some(t) = all.iter().find(|t| t["name"].as_str() == Some(name)) else {
        bail!("No Nixa tool named {name}. Run nixa tool list.");
    };
    if ctx.json {
        writeln!(out, "{t}")?;
    } else {
        writeln!(out, "{name}\n\n{}\n\nInput:\n{}", t["description"].as_str().unwrap_or(""), serde_json::to_string_pretty(&t["inputSchema"])?)?;
    }
    Ok(0)
}

pub(crate) fn call(ctx: &Ctx, name: &str, input: Value, out: &mut dyn Write) -> Result<i32> {
    let reply = http::post_within("/agent/tool", &json!({"name": name, "args": input}), CALL_WAIT)?;
    let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
    if ctx.json {
        writeln!(out, "{reply}")?;
    } else {
        let text = reply.get("text").or_else(|| reply.get("reason")).and_then(Value::as_str).unwrap_or("");
        writeln!(out, "{text}")?;
    }
    Ok(if ok { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_picks_the_right_eyes_action() {
        assert_eq!(screenshot_input(None, None, None), json!({"action": "read", "window": "screen"}));
        assert_eq!(screenshot_input(Some("42".into()), None, None), json!({"action": "read", "window": 42}));
        assert_eq!(screenshot_input(Some("nixa".into()), None, None), json!({"action": "read", "window": "nixa"}));
        assert_eq!(screenshot_input(None, Some("Tasks".into()), None), json!({"action": "shot", "name": "Tasks"}));
        assert_eq!(screenshot_input(None, Some("main".into()), Some("is the mic off?".into())), json!({"action": "vision", "name": "main", "question": "is the mic off?"}));
    }

    #[test]
    fn flow_commands_map_to_the_tool() {
        let here = std::path::Path::new("/p");
        assert_eq!(flow_input("open", &["drones".into()], here).unwrap(), json!({"action": "open", "flow": "drones"}));
        assert_eq!(flow_input("export", &[], here).unwrap(), json!({"action": "export"}));
        assert_eq!(flow_input("blueprint", &["app".into(), "01-a.mmd".into()], here).unwrap(), json!({"action": "blueprint", "dir": "/p/app", "file": "01-a.mmd"}));
        assert!(flow_input("apply", &[], here).is_err());
    }

    #[test]
    fn list_lines_are_short() {
        assert_eq!(first_line("Nixa's eyes on the Mac. More text."), "Nixa's eyes on the Mac");
    }
}
