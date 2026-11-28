use super::print;
use crate::ctx::Ctx;
use crate::http;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::io::Write;

const TEXT_MAX: usize = 90;

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn clip(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TEXT_MAX {
        return line;
    }
    format!("{}...", line.chars().take(TEXT_MAX).collect::<String>())
}

fn step_line(step: &Value) -> String {
    let (at, pick, to) = (text_at(step, "at"), text_at(step, "pick"), text_at(step, "to"));
    let pick = if pick.is_empty() { "(no answer)" } else { pick };
    let head = format!("  {at}: {} -> {pick}", clip(text_at(step, "text")));
    if to.is_empty() { format!("{head}, no line for this answer, the walk stops") } else { format!("{head} -> {to}") }
}

fn end_line(walk: &Value) -> String {
    let end = text_at(walk, "end");
    match walk.get("finding").filter(|f| f.is_object()) {
        Some(found) => format!("  end {end}: report: {}", clip(text_at(found, "text"))),
        None => match clip(text_at(walk, "end_text")) {
            box_text if box_text.is_empty() => format!("  end {end}: no report."),
            box_text => format!("  end {end} ({box_text}): no report."),
        },
    }
}

pub fn walk_lines(reply: &Value, file: Option<&str>) -> Vec<String> {
    let walks = reply.get("walks").and_then(Value::as_array).cloned().unwrap_or_default();
    let tree = text_at(reply, "tree");
    let mut lines = vec![format!("Dry run of rule tree {tree} on the terminals Nixa watches now. Nothing was saved or sent.")];
    if let Some(path) = file {
        lines.push(format!("Tested the tree in {path}. It is not saved."));
    }
    if walks.is_empty() {
        lines.push("No terminal to walk. Decision: no report.".to_string());
        return lines;
    }
    for walk in &walks {
        lines.push(format!("{}:", text_at(walk, "key")));
        if let Some(steps) = walk.get("steps").and_then(Value::as_array) {
            lines.extend(steps.iter().map(step_line));
        }
        lines.push(end_line(walk));
    }
    let found: Vec<String> = walks
        .iter()
        .filter(|w| w.get("finding").is_some_and(Value::is_object))
        .map(|w| format!("{} at {}", text_at(w, "key"), text_at(w, "end")))
        .collect();
    let decision = if found.is_empty() {
        format!("Decision: no report for any of the {} terminals.", walks.len())
    } else {
        format!("Decision: {} of {} terminals get a report: {}.", found.len(), walks.len(), found.join(", "))
    };
    lines.push(decision);
    lines
}

pub fn run(ctx: &Ctx, file: Option<String>, out: &mut dyn Write) -> Result<i32> {
    let body = match &file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| anyhow!("cannot read {path}: {e}"))?;
            let tree: Value = serde_json::from_str(&text).map_err(|e| anyhow!("{path} is not valid JSON: {e}"))?;
            json!({ "tree": tree })
        }
        None => json!({}),
    };
    let reply = http::post("/watch/dry", &body)?;
    if reply.get("walks").is_none() {
        let tested = file.as_deref().map(|path| format!("\nTested the tree in {path}. It is not saved.")).unwrap_or_default();
        let text = text_at(&reply, "text");
        return print(ctx, out, &reply, &format!("{text}{tested}\nThis nixa-brain is older and sends no steps. Restart it after an update to see each step."));
    }
    print(ctx, out, &reply, &walk_lines(&reply, file.as_deref()).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply() -> Value {
        json!({
            "tree": "watch-agents",
            "walks": [
                {
                    "key": "terminal m3",
                    "steps": [
                        { "at": "r1", "text": "Is the agent blocked?", "pick": "Yes", "to": "r2" }
                    ],
                    "end": "r2",
                    "end_text": "Report: {item} is blocked",
                    "finding": { "rule": "r2", "text": "terminal m3 is blocked" }
                },
                {
                    "key": "terminal m4",
                    "steps": [
                        { "at": "r1", "text": "Is the agent blocked?", "pick": "No", "to": "r3" },
                        { "at": "r3", "text": "Is it slow?", "pick": "Maybe", "to": "" }
                    ],
                    "end": "r3",
                    "end_text": "Is it slow?",
                    "finding": null
                }
            ]
        })
    }

    #[test]
    fn walk_lines_show_steps_and_decision() {
        let lines = walk_lines(&reply(), Some("new.json"));
        assert_eq!(lines[1], "Tested the tree in new.json. It is not saved.");
        assert_eq!(lines[2], "terminal m3:");
        assert_eq!(lines[3], "  r1: Is the agent blocked? -> Yes -> r2");
        assert_eq!(lines[4], "  end r2: report: terminal m3 is blocked");
        assert_eq!(lines[7], "  r3: Is it slow? -> Maybe, no line for this answer, the walk stops");
        assert_eq!(lines[8], "  end r3 (Is it slow?): no report.");
        assert_eq!(lines.last().unwrap(), "Decision: 1 of 2 terminals get a report: terminal m3 at r2.");
    }

    #[test]
    fn empty_walks_and_long_text() {
        let lines = walk_lines(&json!({ "tree": "watch-agents", "walks": [] }), None);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "No terminal to walk. Decision: no report.");
        let long = "word ".repeat(40);
        assert!(clip(&long).ends_with("..."));
        assert_eq!(clip(" a\n b "), "a b");
    }
}
