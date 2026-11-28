use super::{changed, open, programs};
use crate::app::App;
use nixa_store::models::public::PUBLIC_SCORES_URL;
use nixa_store::models::lists::{LIST_AGENTS, LIST_TIMEOUT, command_for, models_stale};
use nixa_store::models::start::agent_of_argv;
use nixa_store::models::{Program, load, model_rows, rank, refresh_list, run_list_command, take_public_wish};
use nixa_store::time::now_ms;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const CHECK_EVERY: Duration = Duration::from_secs(60);
pub const FIRST_SCAN: Duration = Duration::from_secs(5 * 60);
pub const SCAN_EVERY: Duration = Duration::from_secs(60 * 60);

pub type Runner = fn(&[&str]) -> anyhow::Result<String>;

fn run_command(argv: &[&str]) -> anyhow::Result<String> {
    run_list_command(argv, LIST_TIMEOUT)
}

pub fn list_agents(programs: &[Program]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for program in programs.iter().filter(|p| p.installed) {
        if let Some(agent) = agent_of_argv(&program.run)
            && LIST_AGENTS.contains(&agent)
            && command_for(agent).is_some()
            && !out.iter().any(|a| a == agent)
        {
            out.push(agent.to_string());
        }
    }
    out
}

pub fn picked(app: &App) -> Vec<String> {
    crate::cli_brains::chosen(app).map(|kind| vec![kind.word().to_string()]).unwrap_or_default()
}

pub fn only_picked(programs: Vec<Program>, picked: &[String]) -> Vec<Program> {
    programs.into_iter().filter(|p| agent_of_argv(&p.run).is_some_and(|agent| picked.iter().any(|w| w == agent))).collect()
}

pub async fn inventory_with(app: &Arc<App>, list: Vec<Program>, runner: Runner, scores_url: &str) -> usize {
    let db = app.db_file.clone();
    let found = tokio::task::spawn_blocking(move || {
        let conn = nixa_store::open(&db).ok()?;
        let before: HashSet<String> = model_rows(&load(&conn).ok()?).into_iter().map(|r| r.id).collect();
        for agent in list_agents(&list) {
            let cache = load(&conn).ok()?;
            if models_stale(cache.agents.get(&agent), now_ms()) {
                let _ = refresh_list(&conn, &agent, now_ms(), &runner);
            }
        }
        let after = model_rows(&load(&conn).ok()?);
        let new = after.iter().filter(|r| !before.contains(&r.id)).count();
        if new > 0 {
            rank(&conn, &[]).ok()?;
        }
        Some(new)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(0);
    if found > 0 {
        changed(app);
        super::public::refresh_from(app, scores_url, true).await;
    }
    found
}

pub async fn inventory(app: &Arc<App>) -> usize {
    inventory_with(app, programs(app), run_command, PUBLIC_SCORES_URL).await
}

pub type Lister = fn(&App) -> Vec<Program>;

pub struct Timing {
    pub check: Duration,
    pub first: Duration,
    pub every: Duration,
}

pub const TIMING: Timing = Timing { check: CHECK_EVERY, first: FIRST_SCAN, every: SCAN_EVERY };

pub async fn run(app: Arc<App>) {
    run_with(app, programs, run_command, PUBLIC_SCORES_URL, TIMING).await
}

pub async fn run_with(app: Arc<App>, lister: Lister, runner: Runner, scores_url: &str, timing: Timing) {
    let born = Instant::now();
    let mut scanned: Option<Instant> = None;
    loop {
        tokio::time::sleep(timing.check).await;
        if crate::privacy::privacy_on(&app) {
            continue;
        }
        let wish = open(&app).ok().and_then(|conn| take_public_wish(&conn).ok()).unwrap_or(false);
        if wish && super::online::on(&app) {
            super::public::refresh_from(&app, scores_url, true).await;
        }
        let due = born.elapsed() >= timing.first && scanned.is_none_or(|at| at.elapsed() >= timing.every);
        if due {
            scanned = Some(Instant::now());
            let list = only_picked(lister(&app), &picked(&app));
            if !list.is_empty() {
                inventory_with(&app, list, runner, scores_url).await;
            }
            if super::online::on(&app) {
                super::public::refresh_from(&app, scores_url, false).await;
            }
        }
    }
}
