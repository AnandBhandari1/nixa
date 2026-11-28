use crate::links::brain::Link;
use crate::screens::menu::switches::OFF_FILE;
use nixa_git::ai::{AiError, ErrorKind};
use serde_json::{Value, json};
use std::path::Path;

pub const KEY: &str = "git.speak";
pub const MISSING_TIP: &str = "Right-click ✧ to pick another agent.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub title: String,
    pub body: String,
    pub level: Level,
    pub spoken: String,
}

pub fn report(title: impl Into<String>, body: impl Into<String>, level: Level) -> Report {
    let title = title.into();
    Report {
        spoken: title.clone(),
        title,
        body: body.into(),
        level,
    }
}

impl Report {
    pub fn spoken(mut self, words: impl Into<String>) -> Report {
        self.spoken = words.into();
        self
    }
}

pub fn allowed(git_speak: bool, nixa_off: bool) -> bool {
    git_speak && !nixa_off
}

pub fn enabled(data: &Path) -> bool {
    let speak = nixa_store::open_default()
        .ok()
        .and_then(|conn| nixa_store::settings::setting_bool(&conn, KEY, true).ok())
        .unwrap_or(true);
    allowed(speak, data.join(OFF_FILE).exists())
}

pub fn message(text: &str) -> Value {
    json!({"text": text, "agent": "nixa", "kind": "git"})
}

pub fn say(link: &Link, data: &Path, r: &Report) -> anyhow::Result<bool> {
    let words = r.spoken.trim();
    if words.is_empty() || !enabled(data) {
        return Ok(false);
    }
    link.request("phone_say", message(words))?;
    Ok(true)
}

pub fn commit_ready(summary: &str) -> Report {
    report("Commit message ready", summary, Level::Success)
}

pub fn commit_stopped() -> Report {
    report("Stopped", "No commit message was written.", Level::Info)
}

pub fn commit_failed(e: &AiError) -> Report {
    let body = if e.kind == ErrorKind::Missing {
        format!("{}\n{MISSING_TIP}", e.message)
    } else {
        e.message.clone()
    };
    report("Commit message failed", body, Level::Error)
}

pub fn commit_result(result: &Result<nixa_git::ai::CommitText, AiError>) -> Report {
    match result {
        Ok(m) => commit_ready(&m.summary),
        Err(e) if e.cancelled() => commit_stopped(),
        Err(e) => commit_failed(e),
    }
}

pub fn wait_for(job: Option<&str>) -> Report {
    let title = match job {
        Some(j) if !j.is_empty() => format!("Wait for the {} to finish", j.to_lowercase()),
        _ => "Wait for the current git work to finish".into(),
    };
    report(title, "Then try again.", Level::Info).spoken("Wait a moment")
}

pub fn pick_profile(kind: &str) -> Report {
    let what = match kind {
        "commit" => "Commit",
        "push" => "Push",
        "pull" => "Pull",
        "op" => "The next git step",
        _ => "Git work",
    };
    report(
        format!("{what} not done"),
        "This repo needs a profile first. Your global git name and email are never used.",
        Level::Info,
    )
    .spoken("Pick a profile first")
}

pub fn fix_conflicts() -> Report {
    report(
        "Fix the conflicted files first",
        "Commit all would save the conflict markers. Pick a side or mark each file resolved, then commit.",
        Level::Warning,
    )
    .spoken("Fix the conflicts first")
}

pub fn open_repo_first(name: &str) -> Report {
    report(
        "Open that repo first",
        format!("The Changes panel shows another repo now. Open {name} in the panel, then try again."),
        Level::Warning,
    )
}

pub fn amend_off() -> Report {
    report("Amend turned off", "The last commit changed. Tick Amend again to load it.", Level::Info)
}

pub fn resolved(title: &str) -> Report {
    report(title, "", Level::Success).spoken("Marked resolved")
}

pub fn side_taken(path: &str, ours: bool) -> Report {
    let title = if ours { "Kept current" } else { "Took incoming" };
    report(title, path, Level::Success)
}

pub fn discarded_all(files: usize) -> Report {
    let what = if files == 1 { "1 file".to_string() } else { format!("{files} files") };
    report("Discarded all changes", what, Level::Success)
}

pub fn stash_back() -> Report {
    report("Changes brought back", "Your stashed changes are in the files again.", Level::Success)
}

pub fn done(ok: &str, body: &str) -> Report {
    report(ok, body, Level::Success)
}

pub fn failed(fail: &str, error: &str) -> Report {
    report(fail, error, Level::Error)
}
