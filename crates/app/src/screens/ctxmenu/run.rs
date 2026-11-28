use super::model::{Checkout, RIGHT_CLICK_LABEL, TO_MENU, TO_PROGRAM};
use crate::links::brain::{Link, http};
use crate::links::engine;
use anyhow::{Context as _, Result, bail};
use nixa_protocol::TermId;
use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SAVE_WAIT: Duration = Duration::from_secs(15);
pub const SAVE_PATH: &str = "/custom-agents/save";
pub const SAVE_TITLE: &str = "Save as custom agent";
pub const SAVING: &str = "Saving…";
pub const NEED_TITLE: &str = "Give the agent a title.";
pub const NEED_PURPOSE: &str = "Say in a few words what this agent is for.";
pub const NO_BRAIN: &str = "Nixa's brain is not linked. Start the brain and try again.";

fn git(folder: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn checkout(folder: &Path) -> Option<Checkout> {
    let top = git(folder, &["rev-parse", "--show-toplevel"])?;
    let common = git(
        folder,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if top.is_empty() || common.is_empty() {
        return None;
    }
    Some(if Path::new(&common) == Path::new(&top).join(".git") {
        Checkout::Main
    } else {
        Checkout::Linked
    })
}

pub fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

fn safe_arg(path: &str) -> String {
    if path.starts_with('-') {
        format!("./{path}")
    } else {
        path.to_string()
    }
}

pub fn reveal(path: &str) -> Result<()> {
    let status = Command::new("/usr/bin/open")
        .arg("-R")
        .arg(safe_arg(path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("Could not start open")?;
    if !status.success() {
        bail!("Finder could not show {path}");
    }
    Ok(())
}

pub fn right_click_value(to_program: bool) -> &'static str {
    if to_program { TO_PROGRAM } else { TO_MENU }
}

pub fn set_right_clicks(terminal: TermId, to_program: bool) -> Result<()> {
    engine::label(terminal, RIGHT_CLICK_LABEL, right_click_value(to_program))
}

pub fn save_missing(title: &str, purpose: &str) -> Option<&'static str> {
    if title.trim().is_empty() {
        Some(NEED_TITLE)
    } else if purpose.trim().is_empty() {
        Some(NEED_PURPOSE)
    } else {
        None
    }
}

pub fn save_body(terminal: &TermId, title: &str, purpose: &str) -> Value {
    json!({
        "terminal": terminal.to_string(),
        "title": title.trim(),
        "purpose": purpose.trim(),
    })
}

pub fn save_agent(link: &Link, body: &Value) -> Result<String> {
    let addr = link.addr().to_string();
    let reply = http(&addr, "POST", SAVE_PATH, Some(body), SAVE_WAIT)?;
    Ok(reply
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("Saved.")
        .to_string())
}
