//! nixa_hooks: the status hooks of the coding agents (`nixa hooks`), so Nixa chat can see, install and remove them by itself.

use crate::tools::{Args, ToolResult, flag, schema, spec, strings, text};
use crate::turns::ToolSpec;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

pub const NAME: &str = "nixa_hooks";
pub const AGENTS: [&str; 4] = ["claude", "codex", "omp", "cursor"];
const WAIT: Duration = Duration::from_secs(30);

pub fn specs() -> Vec<ToolSpec> {
    vec![spec(
        NAME,
        "See and set the status hooks of the coding agents, so every agent reports working, blocked, done or error to Nixa. action status says per agent whether its hooks are installed, out of date or missing. install and remove are a dry run by default: they show the change and write nothing. apply true writes it; each changed file is backed up first. agents picks from claude, codex, omp, cursor (default: all). Without hooks an agent shows no status until it reports by itself.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["status", "install", "remove"]},
                "agents": {"type": "array", "items": {"type": "string", "enum": AGENTS}, "description": "claude, codex, omp or cursor. Default: all four."},
                "apply": {"type": "boolean", "default": false, "description": "With install or remove: true writes the change. Default false only shows it."},
            }),
            &["action"],
        ),
    )]
}

pub async fn call(args: &Args) -> ToolResult {
    let Some(bin) = nixa_protocol::paths::nixa_bin() else {
        return Err("The nixa command was not found next to Nixa, so the hooks cannot be read. Nothing was done.".into());
    };
    call_with(args, &bin).await
}

pub async fn call_with(args: &Args, bin: &Path) -> ToolResult {
    let action = need_action(args)?;
    let agents = pick_agents(&strings(args, "agents"))?;
    let apply = flag(args, "apply").unwrap_or(false) && action != "status";
    let before = run(bin, &action, &agents, false).await?;
    if action == "status" || !apply {
        let note = if action == "status" {
            "Nothing was changed.".to_string()
        } else {
            "Dry run: nothing was written. Show the change to the user; apply true writes it.".to_string()
        };
        return Ok(json!({ "action": action, "apply": false, "agents": before, "note": note }).to_string());
    }
    let written = run(bin, &action, &agents, true).await?;
    let after = run(bin, "status", &agents, false).await?;
    let failed: Vec<String> = written.iter().filter(|r| r.get("error").is_some()).filter_map(|r| r["agent"].as_str().map(str::to_string)).collect();
    let note = if failed.is_empty() {
        format!("Applied to {}. Each changed file was backed up first. Codex: run /hooks inside Codex and trust them. Cursor: restart Cursor and cursor-agent.", agents.join(", "))
    } else {
        format!("Applied where it could. Not changed: {}. Read their error lines.", failed.join(", "))
    };
    Ok(json!({ "action": action, "status": if failed.is_empty() { "applied" } else { "partial" }, "before": before, "written": written, "agents": after, "note": note }).to_string())
}

fn need_action(args: &Args) -> Result<String, String> {
    match text(args, "action").as_deref() {
        Some(action @ ("status" | "install" | "remove")) => Ok(action.to_string()),
        Some(other) => Err(format!("nixa_hooks has no action {other:?}. Use status, install or remove. Nothing was done.")),
        None => Err("action is needed: status, install or remove. Nothing was done.".into()),
    }
}

/// The agents to act on: the asked ones (any case), or all four.
pub fn pick_agents(asked: &[String]) -> Result<Vec<String>, String> {
    let mut picked: Vec<String> = Vec::new();
    for agent in asked.iter().map(|a| a.trim().to_lowercase()) {
        if !AGENTS.contains(&agent.as_str()) {
            return Err(format!("{agent:?} is not one of {}. Nothing was done.", AGENTS.join(", ")));
        }
        if !picked.contains(&agent) {
            picked.push(agent);
        }
    }
    Ok(if picked.is_empty() { AGENTS.iter().map(|a| a.to_string()).collect() } else { picked })
}

/// The change the dry run showed, as one preview line per agent.
pub fn preview(action: &str, report: &[Value]) -> String {
    let mut lines = vec![format!("{} Nixa status hooks:", if action == "install" { "Install" } else { "Remove" })];
    for row in report {
        let agent = row["agent"].as_str().unwrap_or("?");
        if let Some(error) = row["error"].as_str() {
            lines.push(format!("- {agent}: cannot, {error}"));
        } else if !row["found"].as_bool().unwrap_or(true) {
            lines.push(format!("- {agent}: not on this Mac, skipped"));
        } else if !row["changed"].as_bool().unwrap_or(false) {
            lines.push(format!("- {agent}: already as asked, no change ({})", row["file"].as_str().unwrap_or("")));
        } else {
            let file = row["file"].as_str().unwrap_or("");
            let mut changes: Vec<String> = row["changes"].as_array().map(|c| c.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            changes.extend(row["added"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| format!("+ {s}"))).collect::<Vec<_>>()).unwrap_or_default());
            changes.extend(row["removed"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| format!("- {s}"))).collect::<Vec<_>>()).unwrap_or_default());
            let what = if changes.is_empty() { "the Nixa hooks move inside the file".to_string() } else { changes.join("; ") };
            lines.push(format!("- {agent}: {file}: {what}"));
        }
    }
    lines.join("\n")
}

/// Run `nixa hooks <action> [--apply] --json` and return its per-agent report.
pub async fn run(bin: &Path, action: &str, agents: &[String], apply: bool) -> Result<Vec<Value>, String> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("hooks").arg(action).arg("--json").args(agents);
    if apply {
        cmd.arg("--apply");
    }
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(WAIT, cmd.output())
        .await
        .map_err(|_| "The nixa command did not answer in 30 seconds. Nothing was changed.".to_string())?
        .map_err(|error| format!("The nixa command could not run: {error}. Nothing was changed."))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let report: Vec<Value> = serde_json::from_str(stdout.trim()).unwrap_or_default();
    if report.is_empty() {
        let why = crate::util::clip_flat(stderr.trim(), 300);
        return Err(format!("nixa hooks {action} gave no report{}.", if why.is_empty() { String::new() } else { format!(": {why}") }));
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
