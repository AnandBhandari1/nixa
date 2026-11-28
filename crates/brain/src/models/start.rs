use super::{changed, open};
use crate::app::App;
use crate::schedules::runner::{agent_list, default_dirs};
use nixa_store::models::{Prepared, Program, prepare_start, record_start};

pub fn programs(app: &App) -> Vec<Program> {
    agent_list(&app.dir, &default_dirs())
        .into_iter()
        .map(|(key, _, run, installed)| Program { key, run, installed })
        .collect()
}

pub fn prepare(app: &App, argv: &[String], task: &str, named: Option<&str>) -> Result<Option<Prepared>, String> {
    let conn = open(app)?;
    let order = nixa_store::agent_map::fallover(&conn).map_err(|e| format!("{e:#}"))?;
    let list = programs(app);
    prepare_start(&conn, argv, task, named, Some(&list), &order).map_err(|e| format!("{e:#}"))
}

pub fn known_agent(conn: &nixa_store::Connection, argv: &[String]) -> bool {
    let Some(agent) = nixa_store::models::start::agent_of_argv(argv) else { return false };
    nixa_store::models::load(conn).map(|cache| nixa_store::models::model_rows(&cache).iter().any(|r| r.agent == agent)).unwrap_or(false)
}

pub fn pick(app: &App, argv: &[String], task: &str) -> Result<Option<Prepared>, String> {
    if task.trim().is_empty() || argv.is_empty() {
        return Ok(None);
    }
    match prepare(app, argv, task, None) {
        Ok(found) => Ok(found),
        Err(error) => {
            if open(app).map(|conn| known_agent(&conn, argv)).unwrap_or(false) {
                return Err(error);
            }
            crate::util::log("models", &format!("no model list for this agent, it starts as asked: {error}"));
            Ok(None)
        }
    }
}

pub fn record(app: &App, prepared: &Prepared, terminal: &str, task: &str, folder: &str, space: &str, name: &str) -> Result<(), String> {
    let conn = open(app)?;
    record_start(&conn, prepared, terminal, task, folder, space, name).map_err(|e| format!("{e:#}"))?;
    changed(app);
    Ok(())
}
