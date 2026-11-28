use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FOLDER: &str = "go-live";
pub const OUTCOME_FILE: &str = "outcome.json";
pub const STEPS_FILE: &str = "steps.log";
pub const END_FILE: &str = "end";
pub const PID_FILE: &str = "runner.pid";
pub const BUILD_LOG: &str = "build.log";
pub const RUNNER_LOG: &str = "runner.log";
pub const NOT_LIVE: &str = "NOT LIVE:";
pub const END_OK: &str = "ok";
pub const STALE_MS: u64 = 90 * 60 * 1000;
pub const START_MS: u64 = 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    #[default]
    Running,
    Done,
    Staged,
    NotLive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    #[default]
    Build,
    Staging,
    Quiet,
    Restart,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Outcome {
    pub state: State,
    pub stage: Stage,
    pub task: Option<i64>,
    pub repo: String,
    pub started_at: u64,
    pub launched_at: u64,
    pub ended_at: u64,
    pub pid: u32,
    pub candidate: Option<PathBuf>,
    pub ok: bool,
    pub lines: Vec<String>,
    pub reported: bool,
}

pub fn dir_now() -> PathBuf {
    nixa_protocol::paths::app_data_dir().join(FOLDER)
}

pub fn read(dir: &Path) -> Option<Outcome> {
    crate::util::read_json(&dir.join(OUTCOME_FILE))
}

pub fn write(dir: &Path, outcome: &Outcome) -> std::io::Result<()> {
    crate::util::write_json(&dir.join(OUTCOME_FILE), outcome)
}

pub fn steps(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(STEPS_FILE)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

pub fn end(dir: &Path) -> Option<bool> {
    std::fs::read_to_string(dir.join(END_FILE)).ok().map(|t| t.trim() == END_OK)
}

pub fn runner_pid(dir: &Path) -> Option<u32> {
    std::fs::read_to_string(dir.join(PID_FILE)).ok()?.trim().parse().ok().filter(|p| *p > 1)
}

pub fn task_words(task: Option<i64>) -> String {
    task.map(|id| format!(" for task #{id}")).unwrap_or_default()
}

pub fn running(outcome: &Outcome, runner: Option<u32>, now: u64, alive: &dyn Fn(u32) -> bool) -> bool {
    if outcome.state != State::Running || now.saturating_sub(outcome.started_at) > STALE_MS {
        return false;
    }
    match outcome.stage {
        Stage::Build | Stage::Staging | Stage::Quiet => outcome.pid > 1 && alive(outcome.pid),
        Stage::Restart => match runner {
            Some(pid) => alive(pid),
            None => now.saturating_sub(outcome.launched_at) <= START_MS,
        },
    }
}

pub fn next_state(outcome: &Outcome, end: Option<bool>, steps: Vec<String>, runner: Option<u32>, now: u64, alive: &dyn Fn(u32) -> bool, dir: &Path) -> Option<Outcome> {
    if outcome.state != State::Running {
        return Some(outcome.clone());
    }
    let mut lines = outcome.lines.clone();
    if let Some(ok) = end {
        lines.extend(steps);
        if !ok && !lines.iter().any(|l| l.starts_with(NOT_LIVE)) {
            lines.push(format!("{NOT_LIVE} the go-live runner gave no reason. See {}.", dir.join(RUNNER_LOG).display()));
        }
        return Some(Outcome { state: if ok { State::Done } else { State::NotLive }, ok, ended_at: now, lines, ..outcome.clone() });
    }
    if running(outcome, runner, now, alive) {
        return None;
    }
    lines.extend(steps);
    lines.push(format!("{NOT_LIVE} the go-live stopped without a result. See the files in {}.", dir.display()));
    Some(Outcome { state: State::NotLive, ok: false, ended_at: now, lines, ..outcome.clone() })
}

pub fn settle(dir: &Path, now: u64, alive: &dyn Fn(u32) -> bool) -> Option<Outcome> {
    let outcome = read(dir)?;
    if outcome.reported {
        return None;
    }
    let next = observe(dir, &outcome, now, alive)?;
    let done = Outcome { reported: true, ..next };
    write(dir, &done).ok()?;
    Some(done)
}

pub fn observe(dir: &Path, record: &Outcome, now: u64, alive: &dyn Fn(u32) -> bool) -> Option<Outcome> {
    if record.stage == Stage::Staging {
        next_state(record, None, Vec::new(), None, now, alive, dir)
    } else {
        next_state(record, end(dir), steps(dir), runner_pid(dir), now, alive, dir)
    }
}

pub fn text(outcome: &Outcome) -> String {
    let task = task_words(outcome.task);
    match outcome.state {
        State::Running if outcome.stage == Stage::Staging => "The release build is running. Nothing has been installed or restarted.".into(),
        State::Running => format!("The go-live{task} has not finished. Nothing is live yet."),
        State::Done => format!("Go-live done{task}: Nixa was built, installed into {} and restarted, and /health is ok. The verified previous bundle is retained for rollback. No git step.", super::runner::APP_DEST),
        State::Staged => outcome.lines.last().cloned().unwrap_or_else(|| "Candidate staged. Not installed or restarted.".into()),
        State::NotLive => {
            let reason = outcome.lines.iter().rev().find(|l| l.starts_with(NOT_LIVE)).cloned().unwrap_or_else(|| format!("{NOT_LIVE} the go-live gave no reason."));
            format!("Go-live{task} is not live. {reason}")
        }
    }
}
