pub mod changes;
pub mod crew;
pub mod flow;
pub mod migrate;
pub mod model;
pub mod prompt;
pub mod routes;
pub mod runs;
pub mod sources;
pub mod tool;
pub mod watch;

#[cfg(test)]
mod tests;

pub use routes::{routes, ws};
pub use runs::{RunStart, start_for_schedule};
pub use tool::{NAME, call, specs};

use crate::app::{App, lock};
use flow::Live;
use model::{RUNS_FILE, Run, TEAMS_FILE};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TICK_MS: u64 = 2000;
pub const TEAM_LABEL: &str = "nixa.team_run";
pub const CHANGED: &str = "team_runs";

pub struct State {
    pub runs_file: PathBuf,
    pub teams_file: PathBuf,
    pub runs: Mutex<Vec<Run>>,
    pub live: Mutex<HashMap<String, Live>>,
    pub busy: Mutex<HashSet<String>>,
    pub last_check: Mutex<u64>,
    pub watch: Mutex<watch::Memory>,
    pub watching: Mutex<bool>,
    pub made: Mutex<u64>,
}

impl State {
    pub fn open(dir: &Path) -> State {
        let runs_file = dir.join(RUNS_FILE);
        let mut runs = model::parse_runs(std::fs::read_to_string(&runs_file).ok().as_deref());
        if model::cut_running(&mut runs)
            && let Err(e) = crate::util::atomic_write(&runs_file, &model::runs_text(&runs))
        {
            crate::util::log("teams", &format!("could not save {}: {e}", runs_file.display()));
        }
        State {
            runs_file,
            teams_file: dir.join(TEAMS_FILE),
            runs: Mutex::new(runs),
            live: Mutex::new(HashMap::new()),
            busy: Mutex::new(HashSet::new()),
            last_check: Mutex::new(0),
            watch: Mutex::new(watch::Memory::default()),
            watching: Mutex::new(false),
            made: Mutex::new(0),
        }
    }
}

pub fn list(state: &State) -> Vec<Run> {
    lock(&state.runs).clone()
}

pub fn get(state: &State, id: &str) -> Option<Run> {
    lock(&state.runs).iter().find(|r| r.id == id.trim()).cloned()
}

pub fn save(state: &State) {
    let text = {
        let mut runs = lock(&state.runs);
        let skip = runs.len().saturating_sub(model::RUNS_KEEP);
        if skip > 0 {
            runs.drain(..skip);
        }
        model::runs_text(&runs)
    };
    if let Err(e) = crate::util::atomic_write(&state.runs_file, &text) {
        crate::util::log("teams", &format!("could not save {}: {e}", state.runs_file.display()));
    }
}

pub fn update<R>(state: &State, id: &str, change: impl FnOnce(&mut Run) -> R) -> Option<R> {
    let out = {
        let mut runs = lock(&state.runs);
        let run = runs.iter_mut().find(|r| r.id == id)?;
        change(run)
    };
    save(state);
    Some(out)
}

pub fn changed(app: &App) {
    let runs: Vec<serde_json::Value> = list(&app.teams).iter().rev().take(20).map(prompt::run_brief).collect();
    app.send_out(json!({"type": CHANGED, "runs": runs}));
}

pub fn line(app: &App, text: &str) {
    crate::util::log("teams", text);
    app.info(text);
}

pub fn say(app: &Arc<App>, text: &str) {
    crate::voice::say(app, text, None);
}

pub fn choices(app: &App) -> Vec<crate::schedules::runner::Choice> {
    runs::teams_file(app)
        .map(|f| f.teams)
        .unwrap_or_default()
        .into_iter()
        .map(|t| crate::schedules::runner::Choice { id: format!("{}{}", crate::schedules::runner::TEAM_PREFIX, t.id), name: t.name, flow: t.flow })
        .collect()
}

pub fn owns(state: &State, terminal: &str) -> bool {
    lock(&state.runs).iter().any(|r| r.state == model::RunState::Running && r.steps.iter().any(|s| s.terminal == terminal))
}

pub fn replace_terminal(state: &State, previous: &str, terminal: &str) {
    let mut changed = false;
    {
        let mut runs = lock(&state.runs);
        for run in runs.iter_mut().filter(|r| r.state == model::RunState::Running) {
            for step in run.steps.iter_mut().filter(|s| s.terminal == previous) {
                step.terminal = terminal.to_string();
                changed = true;
            }
        }
    }
    if changed {
        save(state);
    }
}

pub async fn run(app: Arc<App>) {
    let mut timer = tokio::time::interval(Duration::from_millis(TICK_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        runs::tick(&app);
        runs::watch_tick(&app);
    }
}
