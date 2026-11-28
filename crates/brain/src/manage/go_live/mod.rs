pub mod outcome;
pub mod runner;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::tools::{Args, ToolResult, need, number, schema, spec, text};
use crate::turns::ToolSpec;
use crate::util::{log, now_ms};
use nixa_store::tasks::{Actor, AuthorKind, CommentInput, CommentKind};
use outcome::{NOT_LIVE, Outcome, Stage, State};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const NAME: &str = "nixa_go_live";
pub const REPO_KEY: &str = "golive.repo";
pub const WATCH_GAP_MS: u64 = 3_000;
pub const SHOWN_LINES: usize = 12;
pub const NOTHING: &str = "Nothing was built, installed or restarted.";

static GUARD: Mutex<()> = Mutex::new(());

pub fn spec_of() -> ToolSpec {
    spec(
        NAME,
        "Stage a Nixa release locally, activate its verified candidate with automatic rollback, or read status. Automatic does not wait for approval. Manual retains a named confirmation. Review source before staging because building executes it. activate installs the last staged signed candidate and restarts only the app and brain on the existing terminal engine. The original engine and its terminals stay running. The user can stop before activation or turn Nixa off. Never call activation until staging has succeeded.",
        schema(
            json!({
                "action": {"type": "string", "enum": ["stage", "activate", "status"]},
                "task": {"type": "integer", "minimum": 1, "description": "stage only, optional: the task to record the result on."},
                "repo": {"type": "string", "description": "stage only, optional: the reviewed Nixa source folder. Default: the folder used last time."}
            }),
            &["action"],
        ),
    )
}

pub fn call(app: &Arc<App>, args: &Args) -> ToolResult {
    match need(args, "action")?.as_str() {
        "status" => Ok(status(app)),
        "activate" | "install" | "restart" => activate(app),
        "stage" => stage(app, args),
        "start" => Ok(refused("The legacy installer is disabled. Use stage to build a candidate without touching the installed app.".into())),
        other => Err(format!("action {other} is not known. Use stage or status.")),
    }
}

pub fn saved_repo(app: &App) -> Option<PathBuf> {
    let conn = nixa_store::open(&app.db_file).ok()?;
    let text = nixa_store::settings::setting_text(&conn, REPO_KEY, "").ok()?;
    Some(PathBuf::from(text.trim())).filter(|p| !p.as_os_str().is_empty())
}

fn save_repo(app: &App, repo: &str) {
    let done = nixa_store::open(&app.db_file).and_then(|conn| nixa_store::settings::set_setting_text(&conn, REPO_KEY, repo));
    if let Err(error) = done {
        log("go-live", &format!("the source folder was not saved: {error:#}"));
    }
}

pub fn pick_repo(given: Option<&str>, saved: Option<PathBuf>, dev: Option<PathBuf>, home: Option<&Path>) -> Result<PathBuf, String> {
    let found = match given {
        Some(text) => runner::expand(text, home),
        None => saved.filter(|p| runner::is_repo(p)).or(dev).ok_or_else(|| format!("Give repo: the Nixa source folder, as an absolute path. {NOTHING}"))?,
    };
    let found = std::fs::canonicalize(&found).unwrap_or(found);
    if !runner::is_repo(&found) {
        return Err(format!("{} is not the Nixa source folder: it needs Cargo.lock, crates/xtask and crates/app. {NOTHING}", found.display()));
    }
    Ok(found)
}

fn check_task(app: &App, id: i64) -> Result<(), String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("The store did not open: {e:#}"))?;
    match nixa_store::tasks::get_task(&conn, id) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("Task #{id} was not found. {NOTHING}")),
        Err(error) => Err(format!("Task #{id} could not be read: {error:#}")),
    }
}

fn refused(reason: String) -> String {
    json!({"status": "refused", "reason": reason, "changed": NOTHING}).to_string()
}

fn stage(app: &Arc<App>, args: &Args) -> ToolResult {
    let home = nixa_protocol::paths::home_dir();
    let repo = pick_repo(text(args, "repo").as_deref(), saved_repo(app), nixa_protocol::paths::dev_root(), home.as_deref())?;
    let task = number(args, "task").and_then(|n| i64::try_from(n).ok()).filter(|n| *n > 0);
    if let Some(id) = task {
        check_task(app, id)?;
    }
    let dir = app.dir.join(outcome::FOLDER);
    let repo_text = repo.display().to_string();
    let guard = lock(&GUARD);
    if let Some(last) = outcome::read(&dir)
        && outcome::running(&last, outcome::runner_pid(&dir), now_ms(), &runner::alive)
    {
        return Ok(refused(format!("A go-live{} is already running. Read it with action status.", outcome::task_words(last.task))));
    }
    nixa_protocol::paths::ensure_private_dir(&dir).map_err(|e| format!("cannot make {}: {e}. {NOTHING}", dir.display()))?;
    let staging = tempfile::Builder::new().prefix("candidate-").tempdir_in(&dir)
        .map_err(|error| format!("Could not make a private candidate folder: {error}. {NOTHING}"))?;
    let bundle = staging.path().join("Nixa.app");
    let first = Outcome { stage: Stage::Staging, task, repo: repo_text.clone(), candidate: Some(bundle.clone()), started_at: now_ms(), pid: std::process::id(), lines: vec![format!("Building Nixa from {repo_text}.")], ..Outcome::default() };
    outcome::write(&dir, &first).map_err(|e| format!("cannot write the go-live state: {e}. {NOTHING}"))?;
    drop(guard);
    save_repo(app, &repo_text);
    let worker = app.clone();
    let place = dir.clone();
    let output = staging.keep();
    let cancel = lock(&app.brain).cancel.clone();
    tokio::spawn(async move {
        match cancel {
            Some(cancel) => tokio::select! {
                biased;
                _ = cancel.cancelled() => stop(&worker, &place, "Stopped by the user before installation."),
                _ = flow(&worker, &place, &repo, &output) => {}
            },
            None => flow(&worker, &place, &repo, &output).await,
        }
    });
    log("go-live", "staging started");
    Ok(json!({
        "status": "staging",
        "task": task,
        "repo": repo_text,
        "candidate": bundle,
        "next": "The build takes minutes. Read the result with status. Nothing has been installed or restarted."
    })
    .to_string())
}

fn activate(app: &Arc<App>) -> ToolResult {
    let dir = app.dir.join(outcome::FOLDER);
    let guard = lock(&GUARD);
    let mut record = outcome::read(&dir).ok_or("Stage a release before activating it.")?;
    if record.state != State::Staged {
        return Err("Only a completed staged candidate can be activated.".into());
    }
    let bundle = record.candidate.clone().ok_or("The staged candidate is missing.")?;
    let repo = PathBuf::from(&record.repo);
    if !runner::is_repo(&repo) || !bundle.is_dir() {
        return Err("The staged candidate or source folder is no longer available.".into());
    }
    for file in [outcome::END_FILE, outcome::STEPS_FILE, outcome::PID_FILE, "cancel"] {
        match std::fs::remove_file(dir.join(file)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    record.state = State::Running;
    record.stage = Stage::Restart;
    record.started_at = now_ms();
    record.launched_at = now_ms();
    record.ended_at = 0;
    record.reported = false;
    record.lines.push("Verifying the staged candidate before automatic activation.".into());
    outcome::write(&dir, &record).map_err(|e| e.to_string())?;
    drop(guard);
    let worker = app.clone();
    let cancel = lock(&app.brain).cancel.clone();
    let cancel_dir = dir.clone();
    if let Some(cancel) = cancel {
        tokio::spawn(async move {
            cancel.cancelled().await;
            let _ = std::fs::write(cancel_dir.join("cancel"), "stopped by the user\n");
        });
    }
    tokio::spawn(async move {
        if let Err(error) = runner::activate(&worker, &repo, &bundle, &dir).await {
            stop(&worker, &dir, &format!("{NOT_LIVE} {error}"));
        }
    });
    Ok(json!({"status":"activating", "rollback":true, "engine":"preserved", "next":"Read status after the window and brain restart."}).to_string())
}

fn stop(app: &App, dir: &Path, line: &str) {
    {
        let _guard = lock(&GUARD);
        if let Some(mut now) = outcome::read(dir) {
            now.lines.push(line.to_string());
            now.state = State::NotLive;
            now.ok = false;
            now.ended_at = now_ms();
            if let Err(error) = outcome::write(dir, &now) {
                log("go-live", &format!("the go-live state was not saved: {error}"));
            }
        }
    }
    settle_now(app, dir);
}

async fn flow(app: &App, dir: &Path, repo: &Path, output: &Path) {
    if let Err(reason) = runner::build(repo, output).await {
        return stop(app, dir, &format!("{NOT_LIVE} {reason}. Nothing was installed or restarted."));
    }
    let bundle = output.join("Nixa.app");
    if !bundle.join("Contents/MacOS/nixa-app").is_file() {
        return stop(app, dir, &format!("{NOT_LIVE} the expected bundle was not produced. Nothing was installed or restarted."));
    }
    let text = format!("Staged {}. Not installed or restarted.", bundle.display());
    let done = {
        let _guard = lock(&GUARD);
        let Some(mut now) = outcome::read(dir) else { return };
        now.state = State::Staged;
        now.ended_at = now_ms();
        now.lines.push(text);
        now.reported = true;
        if let Err(error) = outcome::write(dir, &now) {
            log("go-live", &format!("could not save staged result: {error}"));
            return;
        }
        now
    };
    app.info(&outcome::text(&done));
    if let Some(id) = done.task {
        comment(app, id, true, &outcome::text(&done));
    }
}

fn comment(app: &App, id: i64, ok: bool, text: &str) {
    let kind = if ok { CommentKind::Result } else { CommentKind::Error };
    let done = nixa_store::open(&app.db_file).and_then(|conn| nixa_store::tasks::add_comment(&conn, id, &CommentInput::new(kind, text), &Actor::new(AuthorKind::System, &app.name())));
    if let Err(error) = done {
        log("go-live", &format!("the result was not added to task #{id}: {error:#}"));
    }
}

fn tell(app: &App, done: &Outcome) {
    let text = outcome::text(done);
    let input = crate::modes::journal::DecisionInput {
        kind: "tool".into(), choice: NAME.into(), reason: Some("verified self-update outcome".into()),
        outcome: Some(if done.ok { "live" } else { "not live" }.into()),
        ..Default::default()
    };
    if let Err(error) = crate::modes::store(app).and_then(|conn| crate::modes::journal::record(&conn, &input, now_ms())) {
        log("go-live", &format!("could not journal release outcome: {error}"));
    }
    app.info(&text);
    if let Some(id) = done.task {
        let steps = done.lines.iter().rev().take(SHOWN_LINES).rev().cloned().collect::<Vec<_>>().join(" ");
        comment(app, id, done.ok, &format!("{text} Steps: {steps}"));
    }
    log("go-live", if done.ok { "live" } else { "not live" });
}

pub fn settle_now(app: &App, dir: &Path) -> Option<Outcome> {
    let done = {
        let _guard = lock(&GUARD);
        outcome::settle(dir, now_ms(), &runner::alive)?
    };
    tell(app, &done);
    Some(done)
}

async fn watch_dir(app: &App, dir: &Path) {
    loop {
        let Some(now) = outcome::read(dir) else { return };
        if now.reported || settle_now(app, dir).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(WATCH_GAP_MS)).await;
    }
}

pub async fn watch(app: Arc<App>) {
    watch_dir(&app, &app.dir.join(outcome::FOLDER)).await;
}

pub async fn wait_result(app: &App) -> ToolResult {
    let dir = app.dir.join(outcome::FOLDER);
    loop {
        if !crate::settings_api::nixa_on(&app.dir) {
            let _ = std::fs::write(dir.join("cancel"), "Nixa is off\n");
            return Err("Self update was stopped because Nixa is off.".into());
        }
        let record = outcome::read(&dir).ok_or("The release result could not be read.")?;
        if record.state != State::Running {
            return Ok(status(app));
        }
        if let Some(done) = settle_now(app, &dir) {
            return if done.ok || done.state == State::Staged { Ok(status(app)) } else { Err(outcome::text(&done)) };
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn status(app: &App) -> String {
    let dir = app.dir.join(outcome::FOLDER);
    let now = now_ms();
    let last = outcome::read(&dir).and_then(|record| {
        outcome::observe(&dir, &record, now, &runner::alive).or(Some(record))
    });
    let running = last.as_ref().is_some_and(|o| outcome::running(o, outcome::runner_pid(&dir), now, &runner::alive));
    let stamp = |at: u64| if at == 0 { None } else { Some(crate::modes::clock::stamp(at)) };
    json!({
        "observedAt": crate::modes::clock::stamp(now),
        "running": running,
        "last": last.map(|o| json!({
            "task": o.task,
            "state": o.state,
            "stage": o.stage,
            "ok": o.ok,
            "repo": o.repo,
            "candidate": o.candidate,
            "startedAt": stamp(o.started_at),
            "endedAt": stamp(o.ended_at),
            "text": outcome::text(&o),
            "lines": o.lines.iter().rev().take(SHOWN_LINES).rev().collect::<Vec<_>>(),
            "reported": o.reported,
        })),
        "activationAvailable": true,
        "files": dir.display().to_string(),
    })
    .to_string()
}
