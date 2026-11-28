use super::print;
use crate::args::usage;
use crate::ctx::{self, Ctx};
use crate::http;
use anyhow::{Result, anyhow};
use nixa_store::workflows::valid_name;
use serde_json::{Value, json};
use std::io::Write;

pub const WORDS: &str = "list, read NAME or check NAME [--file F]";

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn info_line(info: &Value) -> String {
    let name = text_at(info, "name");
    if !text_at(info, "broken").is_empty() {
        return format!("{name}: broken: {}", text_at(info, "broken"));
    }
    let boxes = info.get("nodes").and_then(Value::as_u64).unwrap_or(0);
    let mut line = format!("{name}: {} ({boxes} boxes)", text_at(info, "title"));
    if !text_at(info, "when").is_empty() {
        line.push_str(&format!(", when {}", text_at(info, "when")));
    }
    line
}

pub fn check_lines(name: &str, problems: &[String]) -> Vec<String> {
    if problems.is_empty() {
        return vec![format!("Workflow {name} passes the check.")];
    }
    let count = if problems.len() == 1 { "1 problem".to_string() } else { format!("{} problems", problems.len()) };
    let mut lines = vec![format!("Workflow {name} fails the check: {count}.")];
    lines.extend(problems.iter().map(|p| format!("- {p}")));
    lines
}

fn need_name(name: Option<&String>, what: &str) -> Result<String> {
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| usage(format!("nixa task workflows {what} needs a workflow name. Run nixa task workflows list")))?;
    if !valid_name(&name) {
        return Err(usage(format!("{name} is not a workflow name: small letters, numbers and dashes, up to 60")));
    }
    Ok(name)
}

fn saved_list() -> Result<Vec<Value>> {
    let reply = http::window_call("GET", "/workflows", None)?;
    Ok(reply.get("workflows").and_then(Value::as_array).cloned().unwrap_or_default())
}

fn find(list: &[Value], name: &str) -> Result<Value> {
    list.iter().find(|w| text_at(w, "name") == name).cloned().ok_or_else(|| {
        let names: Vec<&str> = list.iter().map(|w| text_at(w, "name")).collect();
        anyhow!("There is no workflow named {name}. Saved workflows: {}.", if names.is_empty() { "none".to_string() } else { names.join(", ") })
    })
}

fn read(name: &str) -> Result<Value> {
    let reply = http::window_call("POST", "/workflows", Some(&json!({ "action": "read", "name": name })))?;
    reply.get("workflow").cloned().filter(Value::is_object).ok_or_else(|| anyhow!("nixa-brain sent no workflow {name}."))
}

fn problems_of(reply: &Value) -> Vec<String> {
    reply.get("problems").and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn check(name: &str, file: Option<String>) -> Result<Vec<String>> {
    let flow = match file {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("cannot read {path}: {e}"))?;
            serde_json::from_str::<Value>(&text).map_err(|e| anyhow!("{path} is not valid JSON: {e}"))?
        }
        None => {
            let info = find(&saved_list()?, name)?;
            if !text_at(&info, "broken").is_empty() {
                return Ok(vec![text_at(&info, "broken").to_string()]);
            }
            read(name)?
        }
    };
    let reply = http::window_call("POST", "/workflows", Some(&json!({ "action": "check", "name": name, "workflow": flow })))?;
    Ok(problems_of(&reply))
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    let (mut p, ctx) = ctx::command(ctx, argv, &["--file"], false)?;
    let what = p.pos.first().map(|w| w.trim().to_lowercase()).unwrap_or_else(|| "list".into());
    let file = p.value(&["--file"]);
    p.finish(2, false)?;
    let name = p.pos.get(1);
    if file.is_some() && what != "check" {
        return Err(usage("--file works only with nixa task workflows check"));
    }
    match what.as_str() {
        "list" | "ls" => {
            if name.is_some() {
                return Err(usage("nixa task workflows list takes no name"));
            }
            let list = saved_list()?;
            let text = if list.is_empty() { "No saved workflows.".to_string() } else { list.iter().map(info_line).collect::<Vec<_>>().join("\n") };
            print(&ctx, out, &list, &text)
        }
        "read" | "show" => {
            let name = need_name(name, "read")?;
            let flow = read(&name)?;
            print(&ctx, out, &flow, &serde_json::to_string_pretty(&flow)?)
        }
        "check" => {
            let name = need_name(name, "check")?;
            let problems = check(&name, file)?;
            let pass = problems.is_empty();
            print(&ctx, out, &json!({ "name": name, "pass": pass, "problems": problems }), &check_lines(&name, &problems).join("\n"))?;
            Ok(if pass { 0 } else { 1 })
        }
        other => Err(usage(format!("nixa task workflows {other} is not known. Use {WORDS}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_lines_read_the_list() {
        let good = json!({ "name": "build-review", "title": "Build then review", "when": "a code task", "nodes": 4 });
        assert_eq!(info_line(&good), "build-review: Build then review (4 boxes), when a code task");
        let plain = json!({ "name": "x", "title": "X", "when": "", "nodes": 2 });
        assert_eq!(info_line(&plain), "x: X (2 boxes)");
        let broken = json!({ "name": "old", "title": "old", "nodes": 0, "broken": "bad JSON" });
        assert_eq!(info_line(&broken), "old: broken: bad JSON");
    }

    #[test]
    fn check_lines_say_pass_or_fail() {
        assert_eq!(check_lines("a", &[]), vec!["Workflow a passes the check."]);
        let one = check_lines("a", &["There is no end box.".into()]);
        assert_eq!(one, vec!["Workflow a fails the check: 1 problem.", "- There is no end box."]);
        assert_eq!(check_lines("a", &["x".into(), "y".into()])[0], "Workflow a fails the check: 2 problems.");
    }

    #[test]
    fn problems_and_find() {
        assert_eq!(problems_of(&json!({ "problems": ["a", 3, "b"] })), vec!["a", "b"]);
        assert!(problems_of(&json!({})).is_empty());
        let list = vec![json!({ "name": "one" }), json!({ "name": "two" })];
        assert_eq!(find(&list, "two").unwrap()["name"], json!("two"));
        assert!(find(&list, "three").unwrap_err().to_string().contains("Saved workflows: one, two."));
    }
}
