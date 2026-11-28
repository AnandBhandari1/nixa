use super::{emit, full_path, here, text_at};
use crate::args;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::Write;

pub const ALL: &str = "*";
pub const STATUSES: [&str; 4] = ["trial", "offer", "live", "off"];

pub fn rule_line(r: &Value) -> String {
    let project = text_at(r, "project");
    let project = if project == ALL { "all projects" } else { project };
    format!(
        "#{} [{}] {}: \"{}\" -> \"{}\" (hits {})",
        r.get("id").and_then(Value::as_u64).unwrap_or(0),
        text_at(r, "status"),
        project,
        text_at(r, "ask"),
        text_at(r, "answer"),
        r.get("hits").and_then(Value::as_u64).unwrap_or(0)
    )
}

pub fn list_lines(reply: &Value) -> Vec<String> {
    let rules = reply.get("rules").and_then(Value::as_array).cloned().unwrap_or_default();
    let max = reply.get("max").and_then(Value::as_u64).unwrap_or(0);
    let mut lines = vec![format!("{} answer rules (at most {max}).", rules.len())];
    lines.extend(rules.iter().map(rule_line));
    lines
}

pub fn project_of(raw: Option<String>) -> String {
    match raw {
        Some(p) if p.trim() == ALL || p.trim() == "all" => ALL.into(),
        Some(p) => full_path(&p),
        None => here(),
    }
}

pub fn add_body(ask: &str, answer: &str, project: Option<String>, status: Option<String>) -> Result<Value> {
    let status = status.unwrap_or_else(|| "trial".into());
    if !STATUSES.contains(&status.as_str()) {
        return Err(args::usage("the status is trial, offer, live or off"));
    }
    if ask.trim().is_empty() || answer.trim().is_empty() {
        return Err(args::usage("answer-rules add needs --ask TEXT and --answer TEXT"));
    }
    Ok(json!({"project": project_of(project), "ask": ask, "answer": answer, "status": status}))
}

pub fn check_line(reply: &Value) -> String {
    match reply.get("rule").filter(|r| r.is_object()) {
        Some(rule) => {
            let act = if text_at(reply, "act") == "answer" { "answers" } else { "would answer (trial, watch only)" };
            format!("Rule {act}: {}", rule_line(rule))
        }
        None => "No answer rule fits this question.".into(),
    }
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let what = argv.first().map(String::as_str).filter(|w| !w.starts_with('-')).unwrap_or("ls");
    let rest = if argv.first().is_some_and(|w| w == what) { &argv[1..] } else { argv };
    let (mut p, ctx) = ctx::command(ctx, rest, &["--ask", "--answer", "--project", "--status", "--folder"], false)?;
    match what {
        "ls" | "list" => {
            p.finish(0, false)?;
            let reply = http::window_call("GET", "/answer-rules", None)?;
            emit(&ctx, out, &reply, &list_lines(&reply).join("\n"))
        }
        "add" => {
            let ask = p.value(&["--ask"]).unwrap_or_default();
            let answer = p.value(&["--answer"]).unwrap_or_default();
            let body = add_body(&ask, &answer, p.value(&["--project"]), p.value(&["--status"]))?;
            p.finish(0, false)?;
            let reply = http::window_call("POST", "/answer-rules", Some(&body))?;
            let rule = reply.get("rule").cloned().unwrap_or(Value::Null);
            emit(&ctx, out, &reply, &format!("Saved: {}", rule_line(&rule)))
        }
        "check" => {
            let folder = full_path(&p.value(&["--folder"]).unwrap_or_else(here));
            let question = p.pos.join(" ");
            if question.trim().is_empty() {
                return Err(args::usage("answer-rules check needs the question text"));
            }
            p.pos.clear();
            p.finish(0, false)?;
            let reply = http::window_call("POST", "/answer-rules/check", Some(&json!({"folder": folder, "question": question})))?;
            emit(&ctx, out, &reply, &check_line(&reply))
        }
        "rm" | "remove" => {
            p.finish(1, false)?;
            let id = p.pos.first().ok_or_else(|| args::usage("answer-rules rm needs a rule number"))?;
            let id: u64 = args::number(id.trim_start_matches('#'), "rule number")?;
            let reply = http::window_call("POST", &format!("/answer-rules/{id}/remove"), Some(&json!({})))?;
            emit(&ctx, out, &reply, &format!("Rule #{id} is removed."))
        }
        other => Err(args::usage(format!("answer-rules takes ls, add, check or rm, not {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_bodies() {
        let body = add_body("run the tests", "yes", Some("all".into()), None).unwrap();
        assert_eq!(body, json!({"project": "*", "ask": "run the tests", "answer": "yes", "status": "trial"}));
        let body = add_body("run the tests", "yes", Some("/tmp/x".into()), Some("live".into())).unwrap();
        assert_eq!(body["project"], "/tmp/x");
        assert!(add_body("a", "b", None, Some("always".into())).is_err());
        assert!(add_body("", "b", None, None).is_err());
    }

    #[test]
    fn lines() {
        let rule = json!({"id": 3, "status": "trial", "project": "*", "ask": "run tests", "answer": "yes", "hits": 2});
        assert_eq!(rule_line(&rule), "#3 [trial] all projects: \"run tests\" -> \"yes\" (hits 2)");
        assert!(check_line(&json!({"rule": rule, "act": "answer"})).starts_with("Rule answers: #3"));
        assert_eq!(check_line(&json!({"rule": null, "act": null})), "No answer rule fits this question.");
    }
}
