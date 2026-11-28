use super::NIXA;
use crate::app::{App, lock};
use nixa_protocol::TermId;
use nixa_store::tasks::{Task, TaskFilter, get_task, list_tasks};

fn is_term(text: &str) -> bool {
    !text.is_empty() && text != NIXA && text.parse::<TermId>().is_ok()
}

pub fn handoff(before: Option<&str>, task: &Task) -> Option<(String, String)> {
    let to = task.terminal_id.trim();
    if !is_term(to) {
        return None;
    }
    match before.map(str::trim).filter(|t| is_term(t)) {
        Some(was) if was == to => None,
        Some(was) => Some((was.to_string(), to.to_string())),
        None => {
            let maker = task.created_terminal.trim();
            let from = if is_term(maker) && maker != to { maker } else { NIXA };
            Some((from.to_string(), to.to_string()))
        }
    }
}

fn load(app: &App, id: Option<i64>) -> Vec<Task> {
    let Ok(conn) = nixa_store::open(&app.db_file) else { return Vec::new() };
    match id {
        Some(id) => get_task(&conn, id).ok().flatten().into_iter().collect(),
        None => list_tasks(&conn, &TaskFilter::default()).unwrap_or_default(),
    }
}

pub fn seed(app: &App) {
    let list = load(app, None);
    let mut seen = lock(&app.universe.tasks);
    for task in list {
        seen.insert(task.id, task.terminal_id);
    }
}

pub fn moves(seen: &mut std::collections::HashMap<i64, String>, list: &[Task]) -> Vec<(String, String)> {
    list.iter()
        .filter_map(|task| {
            let before = seen.insert(task.id, task.terminal_id.clone());
            handoff(before.as_deref(), task)
        })
        .collect()
}

pub fn changed(app: &App, id: Option<i64>) {
    let list = load(app, id);
    let found = moves(&mut lock(&app.universe.tasks), &list);
    for (from, to) in found {
        super::flow(app, &from, &to);
    }
}
