use crate::ctx::{self, Ctx};
use crate::{help, http};
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::io::Write;

const THIS_MAC: &str = "this Mac";

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn path_line(path: &Value) -> String {
    let on = path.get("in_use").and_then(Value::as_bool).unwrap_or(false);
    let reach = text_at(path, "reach");
    let mark = if on { "[on] " } else { "[off]" };
    let leaves = match (on, reach) {
        (false, _) | (true, THIS_MAC) => "",
        (true, "unknown") => ", data may leave",
        _ => ", data can leave",
    };
    let detail = text_at(path, "detail").trim();
    let mut line = format!("  {mark} {} -> {reach}{leaves}.", text_at(path, "name"));
    if !detail.is_empty() {
        line.push(' ');
        line.push_str(detail);
    }
    line
}

pub fn report_lines(report: &Value) -> Vec<String> {
    let paths = report.get("paths").and_then(Value::as_array).cloned().unwrap_or_default();
    let mode = if report.get("privacy_mode").and_then(Value::as_bool).unwrap_or(false) { "on" } else { "off" };
    let leaves = report.get("leaves").and_then(Value::as_u64).unwrap_or(0);
    let mut lines = vec![
        format!("Privacy mode is {mode}. Read from settings only, nothing was sent."),
        format!("{leaves} of {} paths are on and can send data off this Mac.", paths.len()),
    ];
    let mut areas: Vec<&str> = Vec::new();
    for p in &paths {
        let area = text_at(p, "area");
        if !areas.contains(&area) {
            areas.push(area);
        }
    }
    for area in areas {
        lines.push(String::new());
        lines.push(area.to_string());
        lines.extend(paths.iter().filter(|p| text_at(p, "area") == area).map(path_line));
    }
    lines
}

pub fn run(ctx: &Ctx, argv: &[String], out: &mut dyn Write) -> Result<i32> {
    if argv.first().is_some_and(|a| matches!(a.as_str(), "help" | "-h" | "--help")) {
        out.write_all(help::PRIVACY.as_bytes())?;
        return Ok(0);
    }
    let (p, ctx) = ctx::command(ctx, argv, &[], false)?;
    p.finish(0, false)?;
    let reply = http::window_call("GET", "/privacy", None)?;
    let report = reply.get("report").cloned().filter(Value::is_object).ok_or_else(|| anyhow!("nixa-brain sent no privacy report."))?;
    if ctx.json {
        writeln!(out, "{}", serde_json::to_string(&report)?)?;
    } else {
        writeln!(out, "{}", report_lines(&report).join("\n"))?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "privacy_mode": false,
            "leaves": 1,
            "paths": [
                { "area": "Chat brain", "name": "Claude", "reach": "internet", "in_use": true, "detail": "Anthropic API." },
                { "area": "MCP servers", "name": "nixa-mcp (Nixa's own)", "reach": "this Mac", "in_use": true, "detail": "" },
                { "area": "Chat brain", "name": "Local", "reach": "this Mac", "in_use": false, "detail": "Ollama." },
                { "area": "Phone", "name": "Phone door", "reach": "local network", "in_use": false, "detail": "Off." }
            ]
        })
    }

    #[test]
    fn paths_are_marked_on_or_off() {
        let paths = sample()["paths"].clone();
        assert_eq!(path_line(&paths[0]), "  [on]  Claude -> internet, data can leave. Anthropic API.");
        assert_eq!(path_line(&paths[1]), "  [on]  nixa-mcp (Nixa's own) -> this Mac.");
        assert_eq!(path_line(&paths[3]), "  [off] Phone door -> local network. Off.");
        let unknown = json!({ "name": "tool", "reach": "unknown", "in_use": true, "detail": "" });
        assert_eq!(path_line(&unknown), "  [on]  tool -> unknown, data may leave.");
    }

    #[test]
    fn report_groups_areas_in_order() {
        let lines = report_lines(&sample());
        assert_eq!(lines[0], "Privacy mode is off. Read from settings only, nothing was sent.");
        assert_eq!(lines[1], "1 of 4 paths are on and can send data off this Mac.");
        assert_eq!(lines[3], "Chat brain");
        assert!(lines[5].contains("Local -> this Mac"));
        assert_eq!(lines[7], "MCP servers");
        assert_eq!(lines.last().unwrap(), "  [off] Phone door -> local network. Off.");
    }
}
