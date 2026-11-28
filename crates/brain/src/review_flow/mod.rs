pub mod auto_checks;
pub mod check_tool;
pub mod checks;
pub mod decide_review;
pub mod done;
mod fix_delivery;
pub mod handoff;
pub mod review_job;
pub mod settle;

#[cfg(test)]
mod tests;

use crate::app::{App, lock};
use crate::tools::{Args, ToolResult};
use crate::turns::ToolSpec;
use axum::Router;
use axum::extract::{Path as AxPath, State as AxState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nixa_store::agents::{REVIEWER_ROLE, agent_by_terminal, agent_state, agents_with_role, cannot_resume};
use nixa_store::review::ReviewSeat;
use nixa_store::tasks::{Task, must_task};
use serde_json::{Value, json};
use settle::{Effect, Facts, Settled, Team};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const NAMES: [&str; 2] = [check_tool::NAME, review_job::NAME];
pub const DONE_GRACE_MS: u64 = 8000;
pub const TICK_MS: u64 = 2000;
pub const EVENT_KIND: &str = "review_flow";
pub const REVIEW_STALL_MS: u64 = 30 * 60 * 1000;
pub const REVIEW_ASK_KEY: &str = "review_ask:";

pub struct State {
    pub done: Mutex<done::DoneQueue>,
    pub check: Mutex<review_job::Check>,
    pub settling: Mutex<HashSet<i64>>,
}

impl Default for State {
    fn default() -> State {
        State { done: Mutex::new(done::DoneQueue::new(DONE_GRACE_MS)), check: Mutex::new(review_job::Check::default()), settling: Mutex::new(HashSet::new()) }
    }
}

pub fn tools() -> Vec<ToolSpec> {
    vec![check_tool::spec_of(), review_job::spec_of()]
}

pub async fn call(app: &Arc<App>, name: &str, args: &Args) -> Option<ToolResult> {
    match name {
        check_tool::NAME => match crate::git_tools::checks_held(app, args) {
            Some(held) => Some(Ok(held.to_string())),
            None => Some(check_tool::call(&app.spaces(), args).await),
        },
        review_job::NAME => {
            let numbers = lock(&app.announce.numbers).clone();
            let checking: Vec<String> = lock(&app.review_flow.check).checking.iter().cloned().collect();
            let now = crate::util::now_ms();
            let picks: Vec<(String, Option<String>)> = {
                let deep = lock(&app.deep);
                checking.into_iter().map(|t| { let pick = deep.check_pick(&t, now); (t, pick) }).collect()
            };
            let mut check = lock(&app.review_flow.check);
            for (terminal, pick) in picks {
                match pick {
                    Some(pick) => check.picks.insert(terminal, pick),
                    None => check.picks.remove(&terminal),
                };
            }
            Some(review_job::record(&mut check, &numbers, &app.name(), args))
        }
        _ => None,
    }
}

fn open(app: &App) -> anyhow::Result<nixa_store::Connection> {
    nixa_store::open(&app.db_file)
}

pub fn reviewer_seats(conn: &nixa_store::Connection, terminals: &[crate::app::Seen], space: &str) -> Vec<ReviewSeat> {
    let held = nixa_store::tasks::held_tasks(conn).unwrap_or_default();
    agents_with_role(conn, REVIEWER_ROLE, space)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| !a.off)
        .map(|a| {
            let seen = terminals.iter().find(|s| !a.terminal.is_empty() && s.info.id.to_string() == a.terminal);
            let live = seen.is_some_and(|s| s.info.running);
            ReviewSeat {
                state: nixa_store::agent_busy::with_claims(agent_state(live, seen.and_then(|s| s.info.status)), &nixa_store::agent_busy::held_in(&held, &a.terminal)),
                can_resume: cannot_resume(&a).is_none(),
                terminal: if live { a.terminal.clone() } else { String::new() },
                id: a.id,
                title: a.title,
                role: a.role,
                space: a.space,
            }
        })
        .collect()
}

pub fn team_of(app: &App, conn: &nixa_store::Connection, task: &Task, people: Vec<handoff::Person>) -> Team {
    let spaces = app.spaces();
    let worker = if task.terminal_id.is_empty() { None } else { agent_by_terminal(conn, &task.terminal_id).ok().flatten().map(|a| a.id) };
    Team {
        people,
        seats: reviewer_seats(conn, &app.terminals(), &task.space_id),
        worker,
        deep_first: true,
        folders: spaces.into_iter().map(|s| (s.id, s.folder)).collect(),
    }
}

pub async fn facts_of(dir: &Path, folder: &str, run_checks: bool) -> Facts {
    facts_with(dir, folder, run_checks, None).await.0
}

pub async fn facts_with(dir: &Path, folder: &str, run_checks: bool, base: Option<(&str, &str)>) -> (Facts, Option<checks::ChecksOutcome>) {
    let folder_path = Path::new(folder);
    let Some(mut changed) = check_tool::changed_in(folder_path).await else {
        return (Facts { unread: true, ..Facts::default() }, None);
    };
    let mut no_base = false;
    if let Some((base, work)) = base {
        match crate::queue::git::changes(base, work).await {
            crate::queue::git::Changes::Files(since) => {
                let root = check_tool::git(Path::new(work), &["rev-parse", "--show-toplevel"]).await.map(|top| top.trim().to_string()).filter(|top| !top.is_empty()).unwrap_or_else(|| work.to_string());
                let root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.into());
                let since: Vec<std::path::PathBuf> = since.iter().map(|path| root.join(path)).collect();
                let here = std::fs::canonicalize(folder_path).unwrap_or_else(|_| folder_path.to_path_buf());
                changed.retain(|file| since.contains(&here.join(file)));
            }
            _ => no_base = true,
        }
    }
    if no_base || !run_checks || !auto_checks::on(dir) || changed.is_empty() {
        return (Facts { changed, no_base, ..Facts::default() }, None);
    }
    let outcome = checks::run_checks(folder_path, &changed, &checks::RunOptions::default()).await;
    let line = outcome.none.is_empty().then(|| settle::CheckLine { failed: outcome.failed, text: checks::short_checks(&outcome) });
    (Facts { changed, unread: false, checks: line, no_base }, Some(outcome))
}

pub fn check_state(line: Option<&settle::CheckLine>) -> crate::modes::scores::Checks {
    match line {
        Some(line) if line.failed => crate::modes::scores::Checks::Failed,
        Some(_) => crate::modes::scores::Checks::Passed,
        None => crate::modes::scores::Checks::None,
    }
}

pub fn closed_word(outcome: &settle::Outcome, auto: bool) -> Option<&'static str> {
    match outcome {
        settle::Outcome::Done => Some("done"),
        settle::Outcome::Held if auto => None,
        settle::Outcome::User | settle::Outcome::Held => Some("needs-user"),
        settle::Outcome::Back | settle::Outcome::Moved => None,
    }
}

fn joined(app: &App, terminal: &str, task: &Task, facts: &Facts, settled: &Settled) {
    if terminal.is_empty() {
        return;
    }
    let checks = check_state(facts.checks.as_ref());
    if checks != crate::modes::scores::Checks::None {
        crate::modes::checked(app, terminal, checks, facts.checks.as_ref().map(|c| c.text.as_str()));
    }
    let Some(word) = closed_word(&settled.outcome, true) else { return };
    let close = crate::modes::scores::JobClose { outcome: word.into(), checks: Some(checks), spent: false, short: facts.checks.as_ref().map(|c| c.text.clone()) };
    let changes = crate::modes::journal::ChangeInput { terminal: terminal.to_string(), project: Some(task.space_id.clone()).filter(|s| !s.is_empty()).and_then(|id| app.spaces().into_iter().find(|s| s.id == id).map(|s| s.folder)), files: facts.changed.clone(), outcome: word.into(), report: None };
    crate::modes::job_closed(app, terminal, &close, &changes);
    crate::modes::note(app, "job", &format!("task #{} {word}", task.id), Some(terminal), None);
}

async fn second_opinion(app: &Arc<App>, task: &Task, folder: &str, facts: &Facts, outcome: Option<checks::ChecksOutcome>) {
    let Some(outcome) = outcome.filter(|o| o.failed) else { return };
    if task.terminal_id.is_empty() {
        return;
    }
    let reply = crate::followup::flow::reply_of(app, &task.terminal_id).unwrap_or_default();
    let input = crate::extras::opinions::Input { terminal: task.terminal_id.clone(), folder: folder.to_string(), reply, outcome, changed: facts.changed.clone(), space: task.space_id.clone() };
    crate::extras::opinions::ask(app, input).await;
}

pub fn check_line(app: &App, terminal: &str) -> String {
    crate::extras::opinions::line_for(app, terminal)
}

pub fn never_text(paths: &[String]) -> String {
    format!("changed files on the project's never-touch list: {}. The job is not done until that change is undone by editing, not with git", paths.join(", "))
}

pub fn never_touched(mut facts: Facts, folder: &str) -> Facts {
    let rules = crate::knowledge::rules::read(Path::new(folder));
    let never = crate::knowledge::rules::touches_never(rules.as_ref(), &facts.changed);
    if never.is_empty() {
        return facts;
    }
    let line = never_text(&never);
    let text = match facts.checks.take() {
        Some(old) if !old.text.trim().is_empty() => format!("{}; {line}", old.text.trim_end_matches(['.', ' '])),
        _ => line,
    };
    facts.checks = Some(settle::CheckLine { failed: true, text });
    facts
}

pub fn rules_of(folder: &str) -> String {
    if folder.is_empty() {
        return String::new();
    }
    crate::knowledge::rules::rules_line(crate::knowledge::rules::read(Path::new(folder)).as_ref())
}

fn folder_of_event(app: &App, event: &done::DoneEvent) -> String {
    if !event.cwd.trim().is_empty() {
        return event.cwd.clone();
    }
    app.space_of_terminal(&event.terminal).map(|s| s.folder).unwrap_or_default()
}

pub fn ask_chat(app: &Arc<App>, task: &Task, question: &str) {
    let terminal = Some(task.terminal_id.as_str()).filter(|t| !t.is_empty());
    crate::modes::note(app, "review", &format!("task #{}: Nixa decides without the user", task.id), terminal, Some(question));
    crate::watch::tell_event(app, &decide_event(app, task.id, question));
}

async fn apply(app: &Arc<App>, task: &Task, settled: &mut Settled, facts: Option<&Facts>) -> Result<(), String> {
    for effect in &mut settled.effects {
        match effect {
            Effect::Send { terminal, text } => {
                if let Some(note) = fix_delivery::send(app, task, terminal, text).await.map_err(|e| format!("{e:#}"))? {
                    settled.outcome = settle::Outcome::Held;
                    app.info(&note);
                    *effect = Effect::Info { text: note };
                }
            }
            Effect::Stop { terminal } => {
                if let Ok(id) = crate::engine::term_id(terminal)
                    && let Err(e) = crate::engine::remove(&id).await
                {
                    crate::util::log("review_flow", &format!("could not stop {terminal}: {e:#}"));
                }
            }
            Effect::Info { text } => app.info(text),
            Effect::Speak { .. } => {}
            Effect::AskDeep { task: id, question } => {
                if *id == task.id && decide_review::take(app, task, facts, question) {
                    continue;
                }
                ask_chat(app, task, question);
            }
        }
    }
    if !settled.effects.is_empty() {
        app.send_out(json!({"type": EVENT_KIND, "changed": true}));
    }
    Ok(())
}

pub fn decide_line(deep: bool, id: i64, question: &str) -> String {
    if deep {
        return format!("Event: Check of task #{id} needs a decision. Ask the deep brain of its space with nixa_deep_ask and act on its pick. If no deep brain answers, pick the recommended or safest option yourself and act on it. Do not wait for the user. The question: {question}");
    }
    format!("Event: Check of task #{id} needs a decision. Nixa decides: pick the recommended or safest option yourself and act on it now. Do not wait for the user. The question: {question}")
}

fn decide_event(app: &App, id: i64, question: &str) -> String {
    decide_line(app.decision.deep_mode(), id, question)
}

struct Settling<'a>(&'a State, i64);

impl Drop for Settling<'_> {
    fn drop(&mut self) {
        lock(&self.0.settling).remove(&self.1);
    }
}

pub async fn recover(app: &Arc<App>) {
    for (task, mut settled) in recover_now(app, Vec::new()) {
        if let Err(error) = apply(app, &task, &mut settled, None).await {
            crate::util::log("review_flow", &format!("recovery of task {} was not applied: {error}", task.id));
        }
    }
}

pub fn recover_now(app: &Arc<App>, people: Vec<handoff::Person>) -> Vec<(Task, Settled)> {
    let Ok(conn) = open(app) else { return Vec::new() };
    let filter = nixa_store::tasks::TaskFilter { status: vec![nixa_store::tasks::TaskStatus::Todo, nixa_store::tasks::TaskStatus::Blocked], ..Default::default() };
    let mut out = Vec::new();
    for task in nixa_store::tasks::list_tasks(&conn, &filter).unwrap_or_default() {
        let Some(why) = stuck(&conn, &task) else { continue };
        if task.space_id.is_empty() {
            continue;
        }
        let terminal = Some(task.terminal_id.as_str()).filter(|t| !t.is_empty());
        if crate::modes::gate(app, "check", terminal).is_some() {
            continue;
        }
        if !lock(&app.review_flow.settling).insert(task.id) {
            continue;
        }
        let _settling = Settling(&app.review_flow, task.id);
        let team = team_of(app, &conn, &task, people.clone());
        let moved = conn.unchecked_transaction().map_err(anyhow::Error::from).and_then(|tx| {
            let fresh = must_task(&tx, task.id)?;
            if fresh.updated_at != task.updated_at || fresh.terminal_id != task.terminal_id {
                return Ok(None);
            }
            let moved = match task.status {
                nixa_store::tasks::TaskStatus::Blocked => settle::escalate_blocked(&tx, task.id, &[], &team)?,
                _ => settle::seat_gone(&tx, task.id, &why, &team)?,
            };
            tx.commit()?;
            Ok(moved)
        });
        match moved {
            Ok(Some(settled)) => {
                crate::util::log("review_flow", &format!("recovered task {}: {:?}", task.id, settled.outcome));
                out.push((task, settled));
            }
            Ok(None) => {}
            Err(error) => crate::util::log("review_flow", &format!("recovery of task {} failed: {error:#}", task.id)),
        }
    }
    out.extend(stale_reviews(app, &conn, crate::util::now_ms()));
    out
}

pub fn to_home_space(conn: &nixa_store::Connection, task: &Task, home: &crate::spaces::Space) -> anyhow::Result<Task> {
    if !task.space_id.is_empty() {
        return Ok(task.clone());
    }
    let patch = nixa_store::tasks::TaskPatch { space_id: Some(home.id.clone()), space_name: Some(home.name.clone()), ..Default::default() };
    nixa_store::tasks::update_task(conn, task.id, &patch)?;
    let line = format!("The task had no space, so Nixa put it in the home space {} to check it.", home.name);
    nixa_store::tasks::add_comment(conn, task.id, &nixa_store::tasks::CommentInput::new(nixa_store::tasks::CommentKind::Note, &line), &settle::nixa())?;
    must_task(conn, task.id)
}

pub fn review_stalled(updated_at: i64, last_ask: u64, now: u64) -> bool {
    let since = (updated_at.max(0) as u64).max(last_ask);
    now.saturating_sub(since) >= REVIEW_STALL_MS
}

pub fn stalled_question(task: &Task, minutes: u64) -> String {
    format!("Task #{} in {} has waited in review for {minutes} minutes with no change. Pick one and act on it: mark it done when its checks passed, send a fix to the worker, or give it to another agent.", task.id, settle::place(task))
}

fn stale_reviews(app: &Arc<App>, conn: &nixa_store::Connection, now: u64) -> Vec<(Task, Settled)> {
    let filter = nixa_store::tasks::TaskFilter { status: vec![nixa_store::tasks::TaskStatus::Review], ..Default::default() };
    let mut out = Vec::new();
    for task in nixa_store::tasks::list_tasks(conn, &filter).unwrap_or_default() {
        if task.archived_at > 0 || task.space_id.is_empty() {
            continue;
        }
        let key = format!("{REVIEW_ASK_KEY}{}", task.id);
        let last = nixa_store::tasks::task_meta(conn, &key).ok().flatten().and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(0);
        if !review_stalled(task.updated_at, last, now) || crate::deep::review_held(conn, &task) || lock(&app.review_flow.settling).contains(&task.id) {
            continue;
        }
        let terminal = Some(task.terminal_id.as_str()).filter(|t| !t.is_empty());
        if crate::modes::gate(app, "check", terminal).is_some() {
            continue;
        }
        let _ = nixa_store::tasks::set_task_meta(conn, &key, &now.to_string());
        let question = stalled_question(&task, REVIEW_STALL_MS / 60_000);
        let id = task.id;
        crate::util::log("review_flow", &format!("task {id} waited in review, asked again"));
        out.push((task, Settled { outcome: settle::Outcome::Held, effects: vec![Effect::AskDeep { task: id, question }] }));
    }
    out
}

fn stuck(conn: &nixa_store::Connection, task: &Task) -> Option<String> {
    let state = nixa_store::review::handoff_state(conn, task.id).ok()?;
    match task.status {
        nixa_store::tasks::TaskStatus::Blocked => {
            let worker = !task.blocked_on.is_some_and(nixa_store::tasks::BlockedOn::is_question) && !task.terminal_id.is_empty();
            (worker && nixa_store::review::in_chain(&state) && !state.escalated).then(String::new)
        }
        nixa_store::tasks::TaskStatus::Todo if task.terminal_id.is_empty() && state.auto && task.assigned_kind == "agent" => {
            let id = nixa_store::review::custom_id_of(&task.assigned)?;
            match nixa_store::agents::get_agent(conn, &id).ok()? {
                None => Some(format!("The saved agent {id} for this step is gone.")),
                Some(agent) if agent.off => Some(format!("The saved agent {} ({id}) for this step is off.", if agent.title.is_empty() { &agent.id } else { &agent.title })),
                Some(_) => None,
            }
        }
        _ => None,
    }
}

pub async fn settle_task(app: &Arc<App>, id: i64, run_checks: bool, people: Vec<handoff::Person>) -> Result<Settled, String> {
    if !lock(&app.review_flow.settling).insert(id) {
        return Err(format!("Task #{id} is already being checked."));
    }
    let _settling = Settling(&app.review_flow, id);
    let (mut settled, task, facts) = settle_once(app, id, run_checks, people).await?;
    let outcome = settled.outcome;
    apply(app, &task, &mut settled, Some(&facts)).await?;
    learn(app, &task.terminal_id, &outcome);
    joined(app, &task.terminal_id, &task, &facts, &settled);
    Ok(settled)
}

fn learn(app: &App, terminal: &str, outcome: &settle::Outcome) {
    let good = match outcome {
        settle::Outcome::Done => true,
        settle::Outcome::Back => false,
        _ => return,
    };
    if terminal.is_empty() {
        return;
    }
    if let Err(error) = crate::models::launch_result(app, terminal, good) {
        crate::util::log("review", &format!("model score for {terminal} not saved: {error}"));
    }
}

async fn settle_once(app: &Arc<App>, id: i64, run_checks: bool, people: Vec<handoff::Person>) -> Result<(Settled, Task, Facts), String> {
    let fail = |e: anyhow::Error| format!("{e:#}");
    let (task, team) = {
        let conn = open(app).map_err(fail)?;
        let task = must_task(&conn, id).map_err(fail)?;
        let task = match crate::spaces::home(&app.spaces()).filter(|_| task.space_id.is_empty()) {
            Some(home) => to_home_space(&conn, &task, home).map_err(fail)?,
            None => task,
        };
        let team = team_of(app, &conn, &task, people);
        (task, team)
    };
    let Some(folder) = settle::folder_of(&task, &team) else {
        let conn = open(app).map_err(fail)?;
        if team.deep_first {
            let held = nixa_store::tasks::set_status(&conn, id, nixa_store::tasks::TaskStatus::Review, None, false).map_err(fail)?;
            let text = "Nixa could not check the work: the task has no space folder.";
            nixa_store::tasks::add_comment(&conn, id, &nixa_store::tasks::CommentInput::new(nixa_store::tasks::CommentKind::Note, &format!("{text} Nixa asked for a decision.")), &settle::nixa()).map_err(fail)?;
            let question = format!("Task #{id} in {}: {text} Pick: done, send a fix to an agent, or move the task to a space with a folder.", settle::place(&held));
            let settled = Settled { outcome: settle::Outcome::Held, effects: vec![Effect::AskDeep { task: id, question }] };
            return Ok((settled, task, Facts::default()));
        }
        let held = nixa_store::tasks::set_status(&conn, id, nixa_store::tasks::TaskStatus::Review, None, true).map_err(fail)?;
        nixa_store::tasks::add_comment(
            &conn,
            id,
            &nixa_store::tasks::CommentInput::new(nixa_store::tasks::CommentKind::Result, "Nixa could not check the work: the task has no space folder. Waiting for the user."),
            &settle::nixa(),
        )
        .map_err(fail)?;
        let settled = Settled { outcome: settle::Outcome::Held, effects: vec![Effect::Info { text: format!("Task #{id} in {} waits for you: it has no space folder.", settle::place(&held)) }] };
        return Ok((settled, task, Facts::default()));
    };
    let (facts, outcome) = facts_with(&app.dir, &folder, run_checks, Some((&task.base, &crate::queue::scope::work_folder(&task, &folder)))).await;
    let facts = never_touched(facts, &folder);
    second_opinion(app, &task, &folder, &facts, outcome).await;
    let conn = open(app).map_err(fail)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let fresh = must_task(&tx, id).map_err(fail)?;
    if fresh.terminal_id != task.terminal_id || fresh.status != task.status || fresh.updated_at != task.updated_at {
        return Err(format!("Task #{id} changed during the check, so Nixa skipped it."));
    }
    let settled = settle::settle(&tx, id, &folder, &facts, &team).map_err(fail)?;
    let task = if settled.outcome == settle::Outcome::Back { must_task(&tx, id).map_err(fail)? } else { task };
    tx.commit().map_err(|e| e.to_string())?;
    Ok((settled, task, facts))
}

pub fn on_done(app: &App, event: done::DoneEvent) -> bool {
    lock(&app.review_flow.done).add(event, crate::util::now_ms())
}

pub fn due_done(app: &App) -> Option<String> {
    let now = crate::util::now_ms();
    let due = lock(&app.review_flow.done).due(now, usize::MAX, |e| !crate::extras::busy(app, &e.terminal));
    let (due, stale): (Vec<done::DoneEvent>, Vec<done::DoneEvent>) = due.into_iter().partition(|e| crate::followup::flow::still_fresh(app, e));
    for event in stale {
        crate::util::log("review_flow", &format!("done fallback for {} dropped: its job or report changed", event.terminal));
    }
    let ready = due;
    for event in &ready {
        crate::followup::standalone::queue(app, event, now);
    }
    if ready.is_empty() {
        return None;
    }
    let numbers = lock(&app.announce.numbers).clone();
    let jobs = done_jobs(app, &ready);
    let places = done_places(app, &ready);
    let text = done::describe_done(&ready, &numbers, &jobs, &places);
    Some(with_opinions(app, text, &ready))
}

fn with_opinions(app: &App, text: String, ready: &[done::DoneEvent]) -> String {
    let lines: Vec<String> = ready
        .iter()
        .filter_map(|e| {
            let line = format!("{}{}", rules_of(&folder_of_event(app, e)), check_line(app, &e.terminal));
            (!line.is_empty()).then(|| format!("{}:{line}", crate::followup::flow::place_of(app, &e.terminal)))
        })
        .collect();
    if lines.is_empty() { text } else { format!("{text}\n{}", lines.join("\n")) }
}

fn done_jobs(app: &App, events: &[done::DoneEvent]) -> HashMap<String, String> {
    let Ok(conn) = open(app) else {
        return HashMap::new();
    };
    events
        .iter()
        .filter_map(|e| {
            let filter = nixa_store::tasks::TaskFilter { terminal: e.terminal.clone(), ..Default::default() };
            let title = nixa_store::tasks::list_tasks(&conn, &filter).ok()?.into_iter().max_by_key(|t| t.id)?.title;
            Some((e.terminal.clone(), title))
        })
        .collect()
}

fn done_places(app: &App, events: &[done::DoneEvent]) -> HashMap<String, String> {
    events.iter().filter_map(|e| app.space_of_terminal(&e.terminal).map(|s| (e.terminal.clone(), s.name))).collect()
}

pub async fn tick(app: &Arc<App>) {
    crate::followup::flow::recheck_queued(app).await;
    if let Some(text) = due_done(app) {
        crate::watch::tell_event(app, &text);
    }
    crate::followup::standalone::tick(app);
    let notes: Vec<(String, String)> = std::mem::take(&mut lock(&app.review_flow.done).notes);
    for (terminal, why) in notes {
        crate::util::log("review_flow", &format!("done held for {terminal}: {why}"));
    }
}

pub async fn run(app: Arc<App>) {
    let mut timer = tokio::time::interval(Duration::from_millis(TICK_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        tick(&app).await;
    }
}

fn reply(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

fn bad(error: String) -> Response {
    reply(StatusCode::BAD_REQUEST, json!({"ok": false, "error": error}))
}

pub fn view(app: &App) -> Value {
    let check = lock(&app.review_flow.check);
    let mut checking: Vec<&String> = check.checking.iter().collect();
    checking.sort();
    json!({
        "ok": true,
        "auto_checks": auto_checks::on(&app.dir),
        "checking": checking,
        "done_waiting": lock(&app.review_flow.done).size(),
        "settling": lock(&app.review_flow.settling).len(),
    })
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/review-flow", get(view_get))
        .route("/review-flow/auto-checks", post(auto_post))
        .route("/review-flow/checks", post(checks_post))
        .route("/review-flow/settle/{id}", post(settle_post))
}

async fn view_get(AxState(app): AxState<Arc<App>>, headers: HeaderMap) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    reply(StatusCode::OK, view(&app))
}

async fn auto_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let Some(on) = body.get("on").and_then(Value::as_bool) else {
        return bad("give on: true or false".into());
    };
    if let Err(e) = auto_checks::set(&app.dir, on) {
        return bad(format!("could not save the auto checks switch: {e}"));
    }
    app.send_out(json!({"type": EVENT_KIND, "auto_checks": on}));
    reply(StatusCode::OK, view(&app))
}

async fn checks_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, axum::Json(body): axum::Json<Value>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let args = body.as_object().cloned().unwrap_or_default();
    match check_tool::call(&app.spaces(), &args).await {
        Ok(text) => reply(StatusCode::OK, serde_json::from_str::<Value>(&text).map(|v| json!({"ok": true, "data": v})).unwrap_or_else(|_| json!({"ok": true, "text": text}))),
        Err(e) => bad(e),
    }
}

async fn settle_post(AxState(app): AxState<Arc<App>>, headers: HeaderMap, AxPath(id): AxPath<i64>, body: Option<axum::Json<Value>>) -> Response {
    if !crate::window_key::allowed(&app, &headers) {
        return crate::window_key::refused();
    }
    let run = body.as_ref().and_then(|b| b.get("checks")).and_then(Value::as_bool).unwrap_or(true);
    match settle_task(&app, id, run, Vec::new()).await {
        Ok(settled) => reply(StatusCode::OK, json!({"ok": true, "data": settled})),
        Err(e) => bad(e),
    }
}
