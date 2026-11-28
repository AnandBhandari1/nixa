use super::lock::{space_locked, task_locked, unlocked};
use super::sweep::same_title;
use super::start::{How, retry_ok, start_one};
use super::{changed, db, inner, log, note, places};
use crate::app::{App, lock};
use crate::spaces::Space;
use futures_util::future::BoxFuture;
use nixa_store::Connection;
use nixa_store::agents::{CustomAgent, list_agents, match_score};
use nixa_store::tasks::{self, CommentKind, Task, TaskFilter, TaskStatus};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

pub const KEY: &str = "deep_dispatch:";
pub const AUTO: &str = "auto";
pub const MANUAL: &str = "manual";
pub const HELD: &str = "held";
pub const LEAD: i64 = 2;
pub const SHOWN: usize = 4;
pub const RETRY: &str = "retry:";
pub const RETRY_MS: u64 = 10 * 60 * 1000;

pub type Starter = fn(Arc<App>, Task, Space) -> BoxFuture<'static, ()>;

#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    One(Box<CustomAgent>),
    Nobody,
    Several(Vec<CustomAgent>),
}

pub fn queue_start(app: Arc<App>, task: Task, space: Space) -> BoxFuture<'static, ()> {
    Box::pin(async move { start_one(&app, task, space, How::Auto).await })
}

pub fn fit(agents: &[CustomAgent], title: &str) -> Fit {
    let mut scored: Vec<(i64, &CustomAgent)> = agents
        .iter()
        .filter(|a| !a.off)
        .map(|a| (match_score(a, title), a))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    let Some(best) = scored.first().map(|(score, _)| *score) else {
        return Fit::Nobody;
    };
    let second = scored.get(1).map(|(score, _)| *score).unwrap_or(0);
    if best >= LEAD && best - second >= LEAD {
        return Fit::One(Box::new(scored[0].1.clone()));
    }
    Fit::Several(scored.iter().filter(|(score, _)| best - score < LEAD).take(SHOWN).map(|(_, a)| (*a).clone()).collect())
}

pub fn mark_of(conn: &Connection, id: i64) -> Option<String> {
    tasks::task_meta(conn, &format!("{KEY}{id}")).ok().flatten().filter(|v| !v.is_empty())
}

fn set_mark(conn: &Connection, id: i64, mark: &str) {
    let _ = tasks::set_task_meta(conn, &format!("{KEY}{id}"), mark);
}

pub fn deep_terminals(app: &App) -> HashSet<String> {
    lock(&app.deep).seats.iter().filter(|s| s.on).filter_map(|s| s.terminal.clone()).collect()
}

fn listed(conn: &Connection, status: &[TaskStatus]) -> Vec<Task> {
    let filter = TaskFilter { status: status.to_vec(), ..TaskFilter::default() };
    tasks::list_tasks(conn, &filter).unwrap_or_default()
}

fn open_tasks(conn: &Connection) -> Vec<Task> {
    listed(conn, &[TaskStatus::Inbox, TaskStatus::Todo])
        .into_iter()
        .filter(|t| t.terminal_id.is_empty() && t.archived_at == 0)
        .collect()
}

pub fn stale_mark(mark: Option<&str>) -> bool {
    matches!(mark, Some(MANUAL) | Some(HELD))
}

fn named(agent: &CustomAgent) -> String {
    format!("{} ({})", agent.title, agent.id)
}

fn who_of(conn: &Connection, task: &Task) -> String {
    match super::launch::custom_of(task).and_then(|id| nixa_store::agents::get_agent(conn, &id).ok().flatten()) {
        Some(agent) => named(&agent),
        None => task.assigned.clone(),
    }
}

fn current(conn: &Connection, task: &Task) -> Option<Task> {
    tasks::get_task(conn, task.id)
        .ok()
        .flatten()
        .filter(|t| t.status == task.status && t.terminal_id.is_empty() && t.assigned == task.assigned)
}

fn twin_of(open: &[Task], task: &Task) -> bool {
    task.status == TaskStatus::Inbox
        && !task.space_id.is_empty()
        && open.iter().any(|o| o.id < task.id && o.space_id == task.space_id && same_title(&o.title) == same_title(&task.title))
}

fn to_status(conn: &Connection, task: &Task, status: TaskStatus) -> Option<Task> {
    let tx = conn.unchecked_transaction().ok()?;
    current(&tx, task)?;
    let moved = tasks::set_status(&tx, task.id, status, None, false).ok()?;
    tx.commit().ok()?;
    Some(moved)
}

pub fn retry_due(mark: &str, now: u64) -> bool {
    mark.strip_prefix(RETRY).and_then(|at| at.trim().parse::<u64>().ok()).is_some_and(|at| now.saturating_sub(at) >= RETRY_MS)
}

fn hold_auto(app: &Arc<App>, conn: &Connection, task: &Task, why: &str, first: bool) {
    set_mark(conn, task.id, &format!("{RETRY}{}", crate::util::now_ms()));
    if !first {
        return;
    }
    note(conn, task.id, &format!("Nixa could not dispatch this deep brain task yet: {why}. Nixa asked the brain to pick the way on, and tries the dispatch again every 10 minutes."), CommentKind::Note);
    crate::modes::note(app, "task", &format!("task #{}: dispatch tried again later", task.id), None, Some(why));
    crate::watch::tell_event(app, &format!("Event: Deep brain task #{} ({}) was not dispatched: {why}. Do not wait for the user. Pick an agent or a space yourself, or ask the deep brain with nixa_deep_ask, then start it with {} (id {}, assign the agent). Nixa also tries the dispatch again every 10 minutes.", task.id, task.title, super::tool::START, task.id));
    log("dispatch retry", json!({"task": task.id, "why": why}));
    changed(app, "queue", Some(task.id));
}

fn best_space(app: &App, task: &Task, pl: &super::Places) -> Option<Space> {
    let open: Vec<Space> = pl.spaces.iter().filter(|s| !space_locked(s, &pl.rows)).cloned().collect();
    let seat = lock(&app.deep).seats.iter().find(|s| s.terminal.as_deref() == Some(task.created_terminal.as_str())).map(|s| s.space.clone());
    seat.and_then(|id| open.iter().find(|s| s.id == id).cloned())
        .or_else(|| app.space_of_terminal(&task.created_terminal).filter(|s| open.iter().any(|o| o.id == s.id)))
        .or_else(|| Some(task.repo.trim()).filter(|r| !r.is_empty()).and_then(|r| crate::spaces::for_folder(&open, r).cloned()))
        .or_else(|| open.first().cloned())
}

fn adopt_space(app: &App, conn: &Connection, task: &Task, pl: &super::Places) -> Option<(Task, Space)> {
    let space = best_space(app, task, pl)?;
    let patch = tasks::TaskPatch { space_id: Some(space.id.clone()), space_name: Some(space.name.clone()), ..tasks::TaskPatch::default() };
    let moved = tasks::update_task(conn, task.id, &patch).ok()?;
    note(conn, task.id, &format!("This deep brain task had no space. Nixa put it in {}.", space.name), CommentKind::Note);
    crate::modes::note(app, "task", &format!("task #{}: Nixa put it in the space {}", task.id, space.name), None, Some("a deep brain task had no space"));
    log("dispatch space", json!({"task": task.id, "space": space.id}));
    changed(app, "queue", Some(task.id));
    Some((moved, space))
}

fn assign_to(conn: &Connection, task: &Task, kind: &str, who: &str, line: &str) -> Result<Task, String> {
    let changed = || "the task changed while Nixa read it".to_string();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    current(&tx, task).ok_or_else(changed)?;
    let moved = tasks::assign_task(&tx, task.id, kind, who).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    note(conn, task.id, line, CommentKind::Note);
    Ok(moved)
}

fn fallback_program(conn: &Connection, dir: &std::path::Path) -> Option<String> {
    let away = crate::leave::away_in(dir);
    let free = |a: &String| !a.trim().is_empty() && !away.iter().any(|e| crate::leave::book::same_engine(e, a));
    nixa_store::agent_map::fallover(conn)
        .unwrap_or_default()
        .into_iter()
        .find(free)
        .or_else(|| crate::workflows::installed_programs(dir).into_iter().find(free))
}

fn assign(conn: &Connection, dir: &std::path::Path, task: &Task, agents: &[CustomAgent]) -> Result<Task, String> {
    match fit(agents, &task.title) {
        Fit::One(agent) => assign_to(conn, task, crate::tasks::KIND_CUSTOM, &agent.id, &format!("Nixa assigned this deep brain task to {}, the custom agent that fits its title best.", named(&agent))),
        Fit::Several(list) if !list.is_empty() => {
            let line = format!("More than one custom agent fits its title. Nixa picked {}, the best fit.", named(&list[0]));
            assign_to(conn, task, crate::tasks::KIND_CUSTOM, &list[0].id, &line)
        }
        Fit::Nobody => match fallback_program(conn, dir) {
            Some(program) => {
                let line = format!("No custom agent fits its title. Nixa gave it to the agent {program}.");
                assign_to(conn, task, crate::tasks::KIND_TERMINAL, &program, &line)
            }
            None => Err("no custom agent fits its title, no agent is set in the fall-over order and no agent program is installed".into()),
        },
        Fit::Several(list) => {
            let names: Vec<String> = list.iter().map(named).collect();
            Err(format!("more than one custom agent fits its title ({})", names.join(", ")))
        }
    }
}

pub fn route(app: &Arc<App>, conn: &Connection, task: &Task, space: &Space, agents: &[CustomAgent], first: bool) -> Option<Task> {
    let mut task = task.clone();
    if task.assigned.trim().is_empty() {
        task = match assign(conn, &app.dir, &task, agents) {
            Ok(moved) => moved,
            Err(why) => {
                hold_auto(app, conn, &task, &why, first);
                return None;
            }
        };
    }
    if task.status == TaskStatus::Inbox {
        task = to_status(conn, &task, TaskStatus::Todo)?;
    }
    set_mark(conn, task.id, AUTO);
    note(conn, task.id, &format!("In {}, Nixa starts this deep brain task with {} once the queue checks pass.", space.name, who_of(conn, &task)), CommentKind::Note);
    log("dispatch auto", json!({"task": task.id, "space": space.id, "assigned": task.assigned}));
    changed(app, "queue", Some(task.id));
    Some(task)
}

fn agent_locked(conn: &Connection, task: &Task, pl: &super::Places) -> bool {
    super::launch::custom_of(task)
        .and_then(|id| nixa_store::agents::get_agent(conn, &id).ok().flatten())
        .and_then(|agent| pl.space(&agent.space).cloned())
        .is_some_and(|space| space_locked(&space, &pl.rows))
}

pub fn ready(app: &App, conn: &Connection, task: &Task, pl: &super::Places) -> bool {
    if task.status != TaskStatus::Todo || task.assigned.trim().is_empty() || agent_locked(conn, task, pl) {
        return false;
    }
    !inner(app).starting.contains_key(&task.id)
        && retry_ok(app, task.id)
        && !super::pick::waits(app, conn, task.id)
        && super::plan::auto_ok(conn, task.id)
        && !super::waits::waits_on_open(conn, task.id)
}

pub async fn tick(app: &Arc<App>, start: Starter) {
    let Ok(conn) = db(app) else { return };
    let pl = places(&conn);
    let deep = deep_terminals(app);
    let agents: Vec<CustomAgent> = list_agents(&conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.space.is_empty() || pl.space(&a.space).is_none_or(|s| !space_locked(s, &pl.rows)))
        .collect();
    let others = unlocked(listed(&conn, &[TaskStatus::Inbox, TaskStatus::Todo, TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review]), &pl.rows);
    let mut starts = Vec::new();
    let now = crate::util::now_ms();
    for task in open_tasks(&conn) {
        let mark = mark_of(&conn, task.id);
        let retry = mark.as_deref().is_some_and(|m| m.starts_with(RETRY));
        if retry && !mark.as_deref().is_some_and(|m| retry_due(m, now)) {
            continue;
        }
        let stale = stale_mark(mark.as_deref());
        let again = retry || stale;
        if mark.is_none() && !deep.contains(&task.created_terminal) {
            continue;
        }
        if task_locked(&task, &pl.rows).is_some() || (mark.is_none() && twin_of(&others, &task)) {
            continue;
        }
        let found = match pl.space(&task.space_id).cloned() {
            Some(space) => Some((task.clone(), space)),
            None if mark.is_none() || again => adopt_space(app, &conn, &task, &pl),
            None => None,
        };
        let Some((task, space)) = found else {
            if mark.is_none() || again {
                hold_auto(app, &conn, &task, "it has no space and no open space was found", mark.is_none() || stale);
            }
            continue;
        };
        let task = match mark.as_deref() {
            None => match route(app, &conn, &task, &space, &agents, true) {
                Some(task) => task,
                None => continue,
            },
            Some(_) if again => match route(app, &conn, &task, &space, &agents, stale) {
                Some(task) => task,
                None => continue,
            },
            Some(AUTO) => task,
            Some(_) => continue,
        };
        if ready(app, &conn, &task, &pl) {
            starts.push(start(app.clone(), task, space));
        }
    }
    drop(conn);
    futures_util::future::join_all(starts).await;
}
