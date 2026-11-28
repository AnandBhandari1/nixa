use super::{print, text, words};
use crate::args::{self, usage};
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::io::Write;

pub fn cards(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::window_call("GET", "/talk", None)?;
    let card = reply.pointer("/talk/confirm").cloned().unwrap_or(Value::Null);
    let line = card_line(&card);
    print(&ctx, out, &json!({"ok": true, "cards": if card.is_null() { vec![] } else { vec![card] }}), &line)
}

pub fn card_line(card: &Value) -> String {
    if card.is_null() {
        return "No waiting cards.".into();
    }
    let why = text(card, "why");
    let why = if why.trim().is_empty() { String::new() } else { format!(" Why: {}", why.trim()) };
    format!("Card {}: {}.{why}\nAnswer: nixa chat approve {0} or nixa chat deny {0}", text(card, "id"), text(card, "message").trim().trim_end_matches('.'))
}

pub fn confirm(ctx: &Ctx, argv: &[String], out: &mut dyn Write, approve: bool) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(1, false)?;
    let id = p.pos.first().ok_or_else(|| usage("give the card id, see nixa chat cards"))?;
    let reply = http::window_call("POST", "/talk/confirm", Some(&confirm_body(id, approve)?))?;
    let line = if approve { format!("Approved card {id}.") } else { format!("Denied card {id}.") };
    print(&ctx, out, &reply, &line)
}

pub fn confirm_body(id: &str, approve: bool) -> Result<Value> {
    let id: u64 = args::number(id, "the card id")?;
    Ok(json!({"approve": approve, "id": id}))
}

pub fn questions(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::get("/prompts")?;
    let prompts = reply.get("prompts").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut lines = Vec::new();
    if prompts.is_empty() {
        lines.push("No waiting questions.".into());
    }
    lines.extend(prompts.iter().map(prompt_lines));
    print(&ctx, out, &reply, &lines.join("\n"))
}

pub fn labels(prompt: &Value) -> Vec<String> {
    match text(prompt, "kind").as_str() {
        "permission" => vec!["yes".into(), "yes, always".into(), "no".into()],
        "plan" => vec!["yes, approve".into(), "no, reject".into()],
        _ => {
            let index = prompt.get("question_index").and_then(Value::as_u64).unwrap_or(0) as usize;
            prompt
                .get("questions")
                .and_then(Value::as_array)
                .and_then(|q| q.get(index))
                .and_then(|q| q.get("options"))
                .and_then(Value::as_array)
                .map(|o| o.iter().map(|v| text(v, "label")).collect())
                .unwrap_or_default()
        }
    }
}

fn flag(prompt: &Value, key: &str) -> bool {
    prompt.get(key).and_then(Value::as_bool) == Some(true)
}

pub fn prompt_lines(prompt: &Value) -> String {
    let place = [text(prompt, "place"), text(prompt, "terminal_name")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    let agent = text(prompt, "agent");
    let mut ask = text(prompt, "text").trim().to_string();
    if text(prompt, "kind") == "permission" {
        let tool = text(prompt, "tool_name");
        let detail = text(prompt, "detail");
        ask = format!("may it use {}{}? {ask}", if tool.is_empty() { "a tool".into() } else { tool }, if detail.is_empty() { String::new() } else { format!(": {}", detail.trim()) });
    }
    let mut lines = vec![format!("{} {} {agent}{} (terminal {}): {}", text(prompt, "id"), text(prompt, "kind"), if place.is_empty() { String::new() } else { format!(" in {place}") }, text(prompt, "terminal"), ask.trim())];
    if flag(prompt, "unread") {
        lines.push("  Its options cannot be read. Answer it on its screen.".into());
    } else if flag(prompt, "plain") {
        lines.push("  Answer with words, for example yes or no.".into());
    } else {
        lines.extend(labels(prompt).iter().enumerate().map(|(i, l)| format!("  {}. {l}", i + 1)));
    }
    lines.join("\n")
}

pub fn answer(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(usize::MAX, true)?;
    let (id, said) = p.pos.split_first().ok_or_else(|| usage("give the question id and the answer, see nixa chat questions"))?;
    let said = words(said, &p.rest)?;
    let list = http::get("/prompts")?;
    let prompt = list
        .get("prompts")
        .and_then(Value::as_array)
        .and_then(|all| all.iter().find(|q| text(q, "id") == *id))
        .cloned()
        .ok_or_else(|| anyhow!("No waiting question {id}. See nixa chat questions"))?;
    let body = answer_body(&prompt, &said)?;
    let path = format!("/prompts/{}/answer", id.trim());
    let reply = if body.get("indexes").is_some() { http::window_call("POST", &path, Some(&body))? } else { http::post(&path, &body)? };
    let result = text(&reply, "result");
    if !result.starts_with("Sent") {
        return Err(anyhow!(if result.is_empty() { "The answer was not sent.".to_string() } else { result }));
    }
    print(&ctx, out, &reply, &result)
}

pub fn answer_body(prompt: &Value, said: &str) -> Result<Value> {
    let said = said.trim();
    let options = labels(prompt);
    if let Some(numbers) = numbers(said) {
        if flag(prompt, "plain") {
            return Ok(json!({"text": said}));
        }
        if let Some(bad) = numbers.iter().find(|n| **n == 0 || **n > options.len()) {
            return Err(usage(format!("option {bad} is not there, options are 1 to {}", options.len())));
        }
        return Ok(json!({"indexes": numbers.iter().map(|n| n - 1).collect::<Vec<_>>()}));
    }
    if flag(prompt, "plain") {
        return Ok(json!({"text": said}));
    }
    let low = said.to_lowercase();
    let index = match (text(prompt, "kind").as_str(), low.as_str()) {
        ("permission", "yes" | "y" | "allow") => Some(0),
        ("permission", "always") => Some(1),
        ("permission", "no" | "n" | "deny") => Some(2),
        ("plan", "yes" | "y" | "approve") => Some(0),
        ("plan", "no" | "n" | "reject") => Some(1),
        _ => options.iter().position(|o| o.to_lowercase() == low),
    };
    match index {
        Some(i) => Ok(json!({"indexes": [i]})),
        None if text(prompt, "kind") == "question" => Ok(json!({"text": said})),
        None => Err(usage("answer yes, always or no, or an option number")),
    }
}

fn numbers(said: &str) -> Option<Vec<usize>> {
    let parts: Vec<&str> = said.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    parts.iter().map(|s| s.parse::<usize>().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Value {
        json!({"id": "p2", "kind": "question", "agent": "claude", "terminal": "t1", "question_index": 0,
            "questions": [{"question": "Which?", "options": [{"label": "Fast"}, {"label": "Full"}], "multi_select": false}]})
    }

    #[test]
    fn card_bodies_and_lines() {
        assert_eq!(confirm_body("3", true).unwrap(), json!({"approve": true, "id": 3}));
        assert_eq!(confirm_body("4", false).unwrap(), json!({"approve": false, "id": 4}));
        assert!(confirm_body("x", true).is_err());
        assert_eq!(card_line(&Value::Null), "No waiting cards.");
        let line = card_line(&json!({"id": 3, "message": "Push the branch.", "why": ""}));
        assert!(line.starts_with("Card 3: Push the branch."), "{line}");
        assert!(line.contains("nixa chat approve 3"));
    }

    #[test]
    fn answers_map_to_clicks_or_text() {
        let perm = json!({"id": "p1", "kind": "permission"});
        assert_eq!(answer_body(&perm, "yes").unwrap(), json!({"indexes": [0]}));
        assert_eq!(answer_body(&perm, "Always").unwrap(), json!({"indexes": [1]}));
        assert_eq!(answer_body(&perm, "no").unwrap(), json!({"indexes": [2]}));
        assert_eq!(answer_body(&perm, "3").unwrap(), json!({"indexes": [2]}));
        assert!(answer_body(&perm, "4").is_err());
        assert!(answer_body(&perm, "maybe").is_err());
        let plan = json!({"id": "p3", "kind": "plan"});
        assert_eq!(answer_body(&plan, "no").unwrap(), json!({"indexes": [1]}));
        let q = question();
        assert_eq!(answer_body(&q, "full").unwrap(), json!({"indexes": [1]}));
        assert_eq!(answer_body(&q, "1").unwrap(), json!({"indexes": [0]}));
        assert_eq!(answer_body(&q, "1, 2").unwrap(), json!({"indexes": [0, 1]}));
        assert_eq!(answer_body(&q, "only the fast ones").unwrap(), json!({"text": "only the fast ones"}));
        let plain = json!({"id": "p4", "kind": "question", "plain": true});
        assert_eq!(answer_body(&plain, "yes").unwrap(), json!({"text": "yes"}));
    }

    #[test]
    fn prompt_lines_list_options() {
        let lines = prompt_lines(&question());
        assert!(lines.contains("p2 question claude"), "{lines}");
        assert!(lines.contains("  2. Full"), "{lines}");
        let perm = prompt_lines(&json!({"id": "p1", "kind": "permission", "agent": "codex", "terminal": "t2", "tool_name": "Bash", "detail": "ls"}));
        assert!(perm.contains("may it use Bash: ls?"), "{perm}");
        assert!(perm.contains("  2. yes, always"), "{perm}");
    }
}
