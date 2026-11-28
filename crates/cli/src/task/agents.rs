use super::caller;
use super::{list_agents, print};
use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use std::io::Write;

pub const ACTIONS: [&str; 5] = ["save", "add", "use", "edit", "forget"];

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    match argv.first().map(String::as_str) {
        Some(action) if ACTIONS.contains(&action) => change(ctx, action, &argv[1..], out),
        Some("list" | "find") => list_agents(ctx, &argv[1..], out),
        _ => list_agents(ctx, argv, out),
    }
}

pub fn fields(action: &str) -> &'static [&'static str] {
    match action {
        "save" => &["title", "purpose", "instructions", "provider", "session", "role"],
        "add" => &["title", "purpose", "instructions", "provider", "session", "space", "role", "job", "agent", "model", "effort", "off", "workflow", "skills"],
        "use" => &["message", "space"],
        "edit" => &["title", "purpose", "instructions", "session", "role", "job", "agent", "model", "effort", "off", "workflow", "skills"],
        _ => &[],
    }
}

pub fn is_agent_id(word: &str) -> bool {
    word.strip_prefix('a').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn change(ctx: &Ctx, action: &str, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let mut names: Vec<String> = fields(action).iter().map(|f| format!("--{f}")).collect();
    if action == "save" {
        names.push("--terminal".into());
    }
    let with_value: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut p, ctx) = ctx::command(ctx, argv, &with_value, false)?;
    let mut body = Map::new();
    for field in fields(action) {
        if let Some(value) = p.value(&[format!("--{field}").as_str()]) {
            body.insert(field.to_string(), json!(value));
        }
    }
    let show = action == "use" && p.flag(&["--show"]);
    let independent = action == "use" && p.flag(&["--independent"]);
    let given = if action == "save" { p.value(&["--terminal"]) } else { None };
    let rest = p.pos.iter().chain(p.rest.iter()).cloned().collect::<Vec<_>>().join(" ").trim().to_string();
    p.finish(usize::MAX, true)?;
    match action {
        "use" | "edit" | "forget" if rest.is_empty() => {
            return Err(usage(format!("nixa task agents {action} needs a custom agent id, for example a1")));
        }
        "use" => {
            let key = if is_agent_id(&rest) { "id" } else { "query" };
            body.insert(key.into(), json!(rest));
        }
        "edit" | "forget" => {
            body.insert("id".into(), json!(rest));
        }
        _ if !rest.is_empty() => return Err(usage(format!("unexpected argument {rest:?}"))),
        _ => {}
    }
    if show {
        body.insert("show".into(), json!(true));
    }
    if independent {
        body.insert("independent".into(), json!(true));
    }
    let who = caller::who(&ctx)?;
    if action == "save" {
        let terminal = match (who.terminal(), given) {
            (Some(own), Some(other)) if other.trim() != own => {
                bail!("An agent saves only its own terminal {own}, not {other}. Nothing was saved.")
            }
            (Some(own), _) => own.to_string(),
            (None, Some(given)) => given.trim().to_string(),
            (None, None) => return Err(usage("nixa task agents save needs --terminal ID outside a Nixa terminal")),
        };
        body.insert("terminal".into(), json!(terminal));
    }
    let reply = match (action, who.terminal()) {
        ("use", Some(own)) => {
            body.insert("terminal".into(), json!(own));
            http::post("/custom-agents/use", &Value::Object(body))?
        }
        _ => http::window_call("POST", &format!("/custom-agents/{action}"), Some(&Value::Object(body)))?,
    };
    let text = reply.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or("Done.").to_string();
    print(&ctx, out, &reply, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_ids_and_fields() {
        assert!(is_agent_id("a1"));
        assert!(is_agent_id("a42"));
        assert!(!is_agent_id("a"));
        assert!(!is_agent_id("news research"));
        assert!(!is_agent_id("ab1"));
        assert!(fields("add").contains(&"workflow"));
        assert!(!fields("use").contains(&"title"));
        assert!(fields("forget").is_empty());
    }
}
