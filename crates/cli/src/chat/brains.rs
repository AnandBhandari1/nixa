use super::{print, text, words};
use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

pub fn list(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::get("/brains")?;
    print(&ctx, out, &reply, &brain_lines(&reply))
}

fn rows(reply: &Value, key: &str) -> Vec<Value> {
    reply.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

pub fn brain_lines(reply: &Value) -> String {
    let chosen = text(reply, "choice");
    let mark = |choice: &str| if !chosen.is_empty() && choice == chosen { "*" } else { " " };
    let mut lines = vec!["Saved brains:".to_string()];
    let saved = rows(reply, "brains");
    if saved.is_empty() {
        lines.push("  none".into());
    }
    for b in &saved {
        let model = text(b, "model");
        lines.push(format!(
            "{} {}  {} ({}){}  key: {}",
            mark(&text(b, "choice")),
            text(b, "choice"),
            text(b, "name"),
            text(b, "kindTitle"),
            if model.is_empty() { String::new() } else { format!(" model {model}") },
            text(b, "keySource")
        ));
    }
    lines.push("Agent programs:".into());
    for c in rows(reply, "cli") {
        let model = text(&c, "model");
        let installed = c.get("installed").and_then(Value::as_bool) == Some(true);
        lines.push(format!(
            "{} {}  {}{}{}",
            mark(&text(&c, "choice")),
            text(&c, "choice"),
            text(&c, "name"),
            if model.is_empty() { String::new() } else { format!(" model {model}") },
            if installed { "" } else { "  (not installed)" }
        ));
    }
    if chosen.is_empty() {
        lines.push("No brain is picked yet. Pick one: nixa chat brain use ID".into());
    }
    lines.join("\n")
}

pub const OLLAMA_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

pub fn brain(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--url"], false)?;
    let url = p.value(&["--url"]);
    p.finish(2, false)?;
    match p.pos.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["use", id] => {
            let choice = choice_for(&http::get("/brains")?, id);
            let reply = http::post("/brain/choice", &json!({"choice": choice}))?;
            print(&ctx, out, &reply, &format!("The chat brain is {} now.", text(&reply, "choice")))
        }
        ["ollama"] => {
            let reply = http::post_within("/brain/ollama", &ollama_body(url.as_deref()), OLLAMA_WAIT)?;
            print(&ctx, out, &reply, &ollama_line(&reply))
        }
        _ => Err(usage("use: nixa chat brain use ID (see nixa chat brains), or nixa chat brain ollama [--url URL]")),
    }
}

pub fn ollama_body(url: Option<&str>) -> Value {
    match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => json!({"baseUrl": url}),
        None => json!({}),
    }
}

pub fn ollama_line(reply: &Value) -> String {
    let made = reply.get("made").and_then(Value::as_bool) == Some(true);
    let model = text(reply, "model");
    format!(
        "{} {} ({}) at {}. The chat brain is {} now.{}",
        if made { "Made the brain" } else { "Found the brain" },
        text(reply, "name"),
        text(reply, "choice"),
        text(reply, "baseUrl"),
        text(reply, "name"),
        if model.is_empty() { " No model is set yet: nixa chat model NAME".to_string() } else { format!(" Model {model}.") }
    )
}

pub fn choice_for(reply: &Value, id: &str) -> String {
    let id = id.trim();
    rows(reply, "brains")
        .iter()
        .find(|b| text(b, "id") == id || text(b, "name").eq_ignore_ascii_case(id))
        .map(|b| text(b, "choice"))
        .unwrap_or_else(|| id.to_string())
}

pub fn model(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--effort"], false)?;
    let effort = p.value(&["--effort"]);
    p.finish(1, false)?;
    if let Some(name) = p.pos.first() {
        let reply = http::post("/brain/model", &model_body(name, effort.as_deref()))?;
        return print(&ctx, out, &reply, &format!("The chat model is {name} now."));
    }
    if effort.is_some() {
        return Err(usage("--effort needs a model name: nixa chat model NAME --effort LEVEL"));
    }
    let health = http::get("/health")?;
    let brain = health.get("brain").cloned().unwrap_or(Value::Null);
    let mut lines = vec![if brain.is_null() {
        "No brain is picked yet.".to_string()
    } else {
        let effort = text(&brain, "effort");
        format!("Now: {} model {}{}", text(&brain, "name"), text(&brain, "model"), if effort.is_empty() { String::new() } else { format!(" effort {effort}") })
    }];
    let models = match http::get("/brain/models") {
        Ok(reply) => rows(&reply, "models"),
        Err(e) => {
            lines.push(format!("The model list could not be read: {e}"));
            Vec::new()
        }
    };
    let now = text(&brain, "model");
    for m in &models {
        let id = text(m, "id");
        let name = text(m, "name");
        let mark = if id == now { "*" } else { " " };
        lines.push(if name.is_empty() || name == id { format!("{mark} {id}") } else { format!("{mark} {id}  {name}") });
    }
    print(&ctx, out, &json!({"ok": true, "brain": brain, "models": models}), &lines.join("\n"))
}

pub fn model_body(name: &str, effort: Option<&str>) -> Value {
    match effort {
        Some(e) => json!({"model": name.trim(), "effort": e.trim()}),
        None => json!({"model": name.trim()}),
    }
}

pub fn memory(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    let (sub, rest) = p.pos.split_first().map(|(s, r)| (s.as_str(), r)).unwrap_or(("ls", &[]));
    match sub {
        "ls" | "list" => {
            p.finish(1, false)?;
            let reply = http::get("/memory")?;
            let lines: Vec<String> = rows(&reply, "lines").iter().map(memory_line).collect();
            let line = if lines.is_empty() { "No memory notes.".to_string() } else { lines.join("\n") };
            print(&ctx, out, &reply, &line)
        }
        "add" => {
            let note = words(rest, &p.rest)?;
            let reply = http::post("/memory", &json!({"text": note}))?;
            print(&ctx, out, &reply, &text(&reply, "message"))
        }
        "rm" | "remove" => {
            p.finish(2, false)?;
            let id = rest.first().ok_or_else(|| usage("give the memory note id, see nixa chat memory"))?;
            let id: i64 = args::number(id, "the memory note id")?;
            let reply = http::window_call("DELETE", &format!("/memory/{id}"), None)?;
            print(&ctx, out, &reply, &format!("Removed memory note {id}."))
        }
        "search" => {
            p.finish(usize::MAX, false)?;
            super::find::memory_search(&ctx, rest, out)
        }
        "forget" => {
            p.finish(usize::MAX, false)?;
            super::find::memory_forget(&ctx, rest, out)
        }
        other => Err(usage(format!("unknown command nixa chat memory {other}, use ls, add, rm, search or forget"))),
    }
}

pub fn memory_line(line: &Value) -> String {
    let held = line.get("held").and_then(Value::as_array).map(|h| h.len()).unwrap_or(0);
    let extra = if held > 0 { format!(" (held: {held})") } else { String::new() };
    format!("{} {} {}{extra}", text(line, "id"), text(line, "day"), text(line, "text").trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brain_choice_and_lines() {
        let reply = json!({"choice": "claude", "brains": [{"id": "b1", "choice": "api:b1", "name": "Work", "kindTitle": "OpenAI", "model": "m1", "keySource": "keychain"}],
            "cli": [{"choice": "claude", "name": "Claude", "installed": true}, {"choice": "codex", "name": "Codex", "installed": false}]});
        assert_eq!(choice_for(&reply, "b1"), "api:b1");
        assert_eq!(choice_for(&reply, "work"), "api:b1");
        assert_eq!(choice_for(&reply, "codex"), "codex");
        let lines = brain_lines(&reply);
        assert!(lines.contains("  api:b1  Work (OpenAI) model m1  key: keychain"), "{lines}");
        assert!(lines.contains("* claude  Claude"), "{lines}");
        assert!(lines.contains("codex  Codex  (not installed)"), "{lines}");
    }

    #[test]
    fn ollama_shapes() {
        assert_eq!(ollama_body(None), json!({}));
        assert_eq!(ollama_body(Some(" ")), json!({}));
        assert_eq!(ollama_body(Some("http://localhost:11434/v1")), json!({"baseUrl": "http://localhost:11434/v1"}));
        let made = json!({"ok": true, "choice": "api:0a0b0c0d", "name": "Ollama", "model": "small:1b", "baseUrl": "http://127.0.0.1:11434/v1", "made": true});
        assert_eq!(ollama_line(&made), "Made the brain Ollama (api:0a0b0c0d) at http://127.0.0.1:11434/v1. The chat brain is Ollama now. Model small:1b.");
        let found = json!({"ok": true, "choice": "api:0a0b0c0d", "name": "Ollama", "model": "", "made": false});
        assert!(ollama_line(&found).starts_with("Found the brain Ollama"));
        assert!(ollama_line(&found).ends_with("nixa chat model NAME"));
    }

    #[test]
    fn model_and_memory_shapes() {
        assert_eq!(model_body(" m2 ", None), json!({"model": "m2"}));
        assert_eq!(model_body("m2", Some("high")), json!({"model": "m2", "effort": "high"}));
        assert_eq!(memory_line(&json!({"id": 3, "day": "2026-01-02", "text": "likes tea", "held": []})), "3 2026-01-02 likes tea");
    }
}
