pub mod answer;
pub mod driver;
pub mod effects;
pub mod model;
pub mod pick;
pub mod routes;
pub mod run;
#[cfg(test)]
mod tests;
pub mod tool;
pub mod who;

pub use driver::{ASK_KIND, Done, Effect, Settle, StageInfo, held, stage_info};
pub use routes::{WS_KIND, is_ws, request, routes, ws_request};
pub use tool::{NAME, call, specs};

use crate::app::App;
use driver::Ctx;
use nixa_store::Connection;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use who::{Custom, WhoOptions};

pub const CHANGED: &str = "workflows_changed";
pub const RUNS_CHANGED: &str = "workflow_runs_changed";
pub const WINDOW_WHO: &str = "the user in the Workflows window";
pub const BASE_AGENTS: [&str; 10] = ["claude", "codex", "agy", "omp", "cursor", "opencode", "hermes", "dim", "kimi", "gemini"];

pub fn changed(app: &App, name: &str) {
    app.send_out(json!({"type": CHANGED, "name": name}));
}

pub fn open_db(app: &App) -> Result<Connection, String> {
    let conn = nixa_store::open(&app.db_file).map_err(|e| format!("The workflow store did not open: {e:#}"))?;
    conn.execute_batch(nixa_store::workflows::SQL).map_err(|e| format!("The workflow tables are missing: {e}"))?;
    Ok(conn)
}

pub fn customs(conn: &Connection) -> Vec<Custom> {
    nixa_store::agents::list_agents(conn)
        .unwrap_or_default()
        .into_iter()
        .map(|a| Custom {
            claude: nixa_store::agents::claude_based(&a.provider, a.start.as_ref()),
            id: a.id,
            title: a.title,
            space: a.space,
            off: a.off,
        })
        .collect()
}

pub fn installed_programs(dir: &Path) -> Vec<String> {
    let dirs = crate::schedules::runner::default_dirs();
    crate::schedules::runner::agent_list(dir, &dirs).into_iter().filter(|(_, _, _, ok)| *ok).map(|(key, _, _, _)| key).collect()
}

pub fn model_rows(conn: &Connection) -> Vec<nixa_store::models::ModelRow> {
    nixa_store::models::load(conn).map(|cache| nixa_store::models::model_rows(&cache)).unwrap_or_default()
}

pub fn who_options(conn: &Connection, programs: Vec<String>) -> WhoOptions {
    WhoOptions {
        map_rows: nixa_store::agent_map::map_rows(conn).unwrap_or_default(),
        programs,
        customs: customs(conn),
        models: model_rows(conn).into_iter().map(|row| (row.agent, row.model)).collect(),
        space: String::new(),
        fallback: false,
    }
}

fn deep_first(app: &App) -> bool {
    app.decision.deep_mode()
}

async fn with_ctx(app: &Arc<App>, work: impl FnOnce(&Ctx) -> Result<Option<Done>, String> + Send + 'static) -> Result<Option<Done>, String> {
    let worker = app.clone();
    let done = tokio::task::spawn_blocking(move || {
        let _guard = crate::app::lock(&worker.workflow_writes);
        let conn = open_db(&worker)?;
        let programs = installed_programs(&worker.dir);
        let ctx = Ctx { conn: &conn, dir: &worker.dir, now: crate::util::now_ms(), options: who_options(&conn, programs), deep_first: deep_first(&worker) };
        work(&ctx)
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(done) = &done {
        effects::apply(app, done.effects.clone()).await;
    }
    Ok(done)
}

pub async fn begin(app: &Arc<App>, task: i64) -> Result<Option<Done>, String> {
    with_ctx(app, move |ctx| driver::begin(ctx, task)).await
}

pub async fn settle(app: &Arc<App>, task: i64, facts: Settle) -> Result<Done, String> {
    with_ctx(app, move |ctx| driver::settle(ctx, task, &facts).map(Some)).await.map(Option::unwrap_or_default)
}

pub async fn act(app: &Arc<App>, task: i64, action: &str, by: &str, text: &str) -> Result<Done, String> {
    let (action, by, text) = (action.to_string(), by.to_string(), text.to_string());
    with_ctx(app, move |ctx| driver::act(ctx, task, &action, &by, &text).map(Some)).await.map(Option::unwrap_or_default)
}

pub async fn on_change(app: &Arc<App>, task: i64, why: &str, role: &str) -> Result<Option<Done>, String> {
    let (why, role) = (why.to_string(), role.to_string());
    with_ctx(app, move |ctx| driver::on_change(ctx, task, &why, &role)).await
}

pub async fn deep_answered(app: &Arc<App>, ask: &crate::deep::Ask) -> Result<Option<Done>, String> {
    if ask.kind.as_deref() != Some(ASK_KIND) || crate::queue::paused(app) {
        return Ok(None);
    }
    let answered = ask.state == crate::deep::AskState::Answered;
    let (id, pick) = (ask.id.clone(), ask.pick.clone().unwrap_or_default());
    let worker = app.clone();
    let ask_id = id.clone();
    let task = tokio::task::spawn_blocking(move || driver::waiting_task(&open_db(&worker)?, &ask_id))
        .await
        .map_err(|error| error.to_string())??;
    let Some(task) = task else { return Ok(None) };
    Box::pin(with_ctx(app, move |ctx| driver::deep_answered(ctx, task, &id, answered, &pick))).await
}

pub async fn revisit_waiting(app: &Arc<App>) {
    if crate::queue::paused(app) {
        return;
    }
    let worker = app.clone();
    let outcomes = tokio::task::spawn_blocking(move || {
        let _guard = crate::app::lock(&worker.workflow_writes);
        let conn = open_db(&worker)?;
        let rows = nixa_store::workflows::list_runs(&conn, true).map_err(|e| e.to_string())?;
        let spaces = nixa_store::spaces::list_spaces(&conn).map_err(|e| e.to_string())?;
        let now = crate::util::now_ms();
        let pending: Vec<_> = rows.iter().filter(|r| r.state == "waiting" && now.saturating_sub(r.updated_at.max(0) as u64) >= driver::ASK_WAIT_MS).collect();
        if pending.is_empty() {
            return Ok::<_, String>(Vec::new());
        }
        let options = who_options(&conn, installed_programs(&worker.dir));
        let mut outcomes = Vec::new();
        for row in pending {
            let Ok(Some(task)) = nixa_store::tasks::get_task(&conn, row.task_id) else { continue };
            if task.archived_at != 0 || task.status != nixa_store::tasks::TaskStatus::Blocked
                || crate::queue::task_locked(&task, &spaces).is_some() {
                continue;
            }
            let Some(question) = run::read_run(&row.run).and_then(|r| r.ask) else { continue };
            let deep = if question.deep.is_empty() { None } else {
                crate::app::lock(&worker.deep).asks.iter().find(|ask| ask.id == question.deep).cloned()
            };
            let ctx = Ctx { conn: &conn, dir: &worker.dir, now, options: options.clone(), deep_first: false };
            let outcome = match deep {
                Some(ask) if matches!(ask.state, crate::deep::AskState::Waiting | crate::deep::AskState::Sent)
                    && now.saturating_sub(ask.at) < driver::ASK_WAIT_MS + crate::deep::JOIN_MS => continue,
                Some(ask) => driver::deep_answered(&ctx, task.id, &ask.id, ask.state == crate::deep::AskState::Answered, ask.pick.as_deref().unwrap_or_default()),
                None if !question.deep.is_empty() => driver::deep_answered(&ctx, task.id, &question.deep, false, ""),
                None => driver::recover_unrouted(&ctx, &task, &question),
            };
            match outcome {
                Ok(Some(done)) => outcomes.push(done),
                Ok(None) => {}
                Err(error) => crate::util::log("workflow", &format!("could not revisit task #{}: {error}", task.id)),
            }
        }
        Ok(outcomes)
    }).await;
    match outcomes {
        Ok(Ok(outcomes)) => {
            for done in outcomes {
                effects::apply(app, done.effects).await;
            }
        }
        Ok(Err(error)) => crate::util::log("workflow", &error),
        Err(error) => crate::util::log("workflow", &error.to_string()),
    }
}
