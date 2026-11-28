use super::launch::{self, Outcome, Seat, custom_of, saved_seat, seat_of};
use super::lock::{auto_space, folder_mismatch, folder_owner, locked_text, space_locked, task_locked};
use super::prompt::{
    self, AgentPlan, LastWorker, PlanInput, PromptInput, agent_for, build_plan, cut, go_on_text,
    named_task, read_last, screen_tail, task_prompt,
};
use super::scope::{Launch, Scope, Wait, scope_of, start_wait};
use super::{
    HANDOFF_START_KEY, LAST_KEY, PLAN_KEY, START_RETRY_MS, changed, db, inner,
    live, log, note, places, screen,
};
use crate::app::App;
use crate::spaces::Space;
use crate::util::now_ms;
use nixa_store::Connection;
use nixa_store::claims::{self, FileHolder};
use nixa_store::tasks::{
    self, Actor, AuthorKind, CommentKind, Task, TaskFilter, TaskInput, TaskSource,
    TaskStatus, is_run_holder, is_starting, starting_holder,
};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    Auto,
    Now,
    Continue,
    HandOver,
    HandOff,
}

pub fn start_action(how: How) -> &'static str {
    match how {
        How::HandOff | How::HandOver => "hand-off",
        How::Auto => "auto-start",
        How::Now | How::Continue => "start-task",
    }
}

impl How {
    pub fn label(self) -> &'static str {
        match self {
            How::Auto => "Auto-start",
            How::Now => "Start now",
            How::Continue => "Continue",
            How::HandOver => "Hand-over",
            How::HandOff => "Hand-off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Read,
    Nothing,
    Text(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Continued {
    pub task: Task,
    pub how: &'static str,
    pub terminal: String,
}

fn fail(e: anyhow::Error) -> String {
    e.to_string()
}

fn get(conn: &Connection, id: i64) -> Result<Task, String> {
    tasks::get_task(conn, id)
        .map_err(fail)?
        .ok_or_else(|| format!("No task #{id}."))
}

fn fresh_or(app: &App, task: &Task) -> Task {
    db(app)
        .ok()
        .and_then(|c| tasks::get_task(&c, task.id).ok().flatten())
        .unwrap_or_else(|| task.clone())
}

fn plan_of(conn: &Connection, id: i64) -> Option<AgentPlan> {
    let text = tasks::task_meta(conn, &format!("{PLAN_KEY}{id}")).ok()??;
    serde_json::from_str(&text).ok()
}

fn holders(conn: &Connection) -> Vec<FileHolder> {
    claims::file_holders(conn, now_ms() as i64).unwrap_or_default()
}

fn scope_for(conn: &Connection, task: &Task, folder: &str, holders: &[FileHolder]) -> Scope {
    let comments = tasks::task_comments(conn, task.id).unwrap_or_default();
    scope_of(
        task,
        folder,
        &comments,
        holders,
        plan_of(conn, task.id).as_ref(),
    )
}

fn wait_reason(
    app: &App,
    conn: &Connection,
    task: &Task,
    space: &Space,
) -> (Option<String>, Scope) {
    let pl = places(conn);
    let held = holders(conn);
    let me = scope_for(conn, task, &space.folder, &held);
    let active: Vec<(i64, Scope)> = tasks::held_tasks(conn)
        .unwrap_or_default()
        .into_iter()
        .filter(|o| o.id != task.id && matches!(o.status, TaskStatus::Doing | TaskStatus::Blocked))
        .filter_map(|o| {
            let folder = pl.space(&o.space_id)?.folder.clone();
            Some((o.id, scope_for(conn, &o, &folder, &held)))
        })
        .collect();
    let filter = TaskFilter {
        space: Some(space.id.clone()),
        status: vec![TaskStatus::Doing, TaskStatus::Todo],
        ..TaskFilter::default()
    };
    let doing: Vec<i64> = tasks::list_tasks(conn, &filter)
        .unwrap_or_default()
        .iter()
        .filter(|t| t.status == TaskStatus::Doing || super::pick::waits(app, conn, t.id))
        .map(|t| t.id)
        .collect();
    let state = inner(app);
    let why = start_wait(&Wait {
        task: task.id,
        space: &space.id,
        scope: &me,
        active: &active,
        launches: &state.launches,
        doing: &doing,
        max_doing: pl.max_doing(&space.id),
    });
    (why, me)
}

pub fn wait_to_start(app: &App, task: &Task, how: How, why: &str) -> Task {
    let first = inner(app)
        .queued
        .insert(task.id, why.to_string())
        .as_deref()
        != Some(why);
    if first && let Ok(conn) = db(app) {
        note(
            &conn,
            task.id,
            &format!(
                "{} queued: {why}. Nixa starts it when that clears.",
                how.label()
            ),
            CommentKind::Note,
        );
        changed(app, "queue", Some(task.id));
    }
    fresh_or(app, task)
}

pub fn moved_line(task: &Task, space: &Space, owner: Option<&Space>) -> String {
    let head = format!("Job folder \"{}\" is outside space \"{}\".", task.repo.trim(), space.name);
    match owner {
        Some(other) => format!("{head} Nixa moved the task to space \"{}\", which owns that folder. It starts there.", other.name),
        None => format!("{head} Nixa cleared the task's folder, and the task runs in the folder of space \"{}\".", space.name),
    }
}

fn move_mismatch(app: &Arc<App>, conn: &Connection, task: &Task, space: &Space, known: &[Space], found: &str) -> Result<Task, String> {
    let owner = folder_owner(known, space, &task.repo);
    let patch = match owner {
        Some(other) => tasks::TaskPatch { space_id: Some(other.id.clone()), space_name: Some(other.name.clone()), ..tasks::TaskPatch::default() },
        None => tasks::TaskPatch { repo: Some(String::new()), ..tasks::TaskPatch::default() },
    };
    let (owner, done) = match tasks::update_task(conn, task.id, &patch) {
        Err(_) if owner.is_some() => (None, tasks::update_task(conn, task.id, &tasks::TaskPatch { repo: Some(String::new()), ..tasks::TaskPatch::default() })),
        other => (owner, other),
    };
    let line = moved_line(task, space, owner);
    match done {
        Ok(moved) => {
            note(conn, task.id, &line, CommentKind::Note);
            crate::modes::note(app, "task", &format!("task #{}: Nixa fixed the job folder", task.id), None, Some(found));
            log("folder mismatch moved", json!({"task": task.id, "space": task.space_id, "repo": task.repo}));
            changed(app, "queue", Some(task.id));
            Ok(moved)
        }
        Err(error) => {
            log("folder mismatch not moved", json!({"task": task.id, "error": error.to_string()}));
            Err(format!("{found} Nixa could not fix the job folder: {error}"))
        }
    }
}

fn mismatch_of(app: &Arc<App>, task: &Task, space: &Space) -> Result<Option<(Task, Space)>, String> {
    let Ok(conn) = db(app) else { return Ok(None) };
    let pl = places(&conn);
    let Some(found) = folder_mismatch(&pl.spaces, space, &task.repo) else { return Ok(None) };
    let moved = move_mismatch(app, &conn, task, space, &pl.spaces, &found)?;
    let next = pl.space(&moved.space_id).cloned().ok_or_else(|| format!("The space of task #{} is not known.", moved.id))?;
    Ok(Some((moved, next)))
}

fn refuse_locked(conn: &Connection, task: &Task, space: &Space) -> Result<(), String> {
    let pl = places(conn);
    if space_locked(space, &pl.rows) {
        return Err(locked_text(&space.name));
    }
    match task_locked(task, &pl.rows) {
        Some(found) => Err(locked_text(&found.name)),
        None => Ok(()),
    }
}

fn person_of(app: &App, task: &Task) -> (String, bool) {
    let Some(id) = custom_of(task) else {
        return (String::new(), false);
    };
    match db(app)
        .ok()
        .and_then(|c| nixa_store::agents::get_agent(&c, &id).ok().flatten())
    {
        Some(agent) => (
            if agent.title.is_empty() {
                agent.id
            } else {
                agent.title
            },
            agent.role.eq_ignore_ascii_case("reviewer"),
        ),
        None => (format!("custom agent {id}"), false),
    }
}

struct Seating<'a> {
    app: &'a App,
    seat: Option<String>,
    task: i64,
}

impl Drop for Seating<'_> {
    fn drop(&mut self) {
        let mut state = inner(self.app);
        if let Some(seat) = &self.seat
            && state.seating.get(seat) == Some(&self.task)
        {
            state.seating.remove(seat);
        }
        state.launches.remove(&self.task);
    }
}

pub async fn start_task(
    app: &Arc<App>,
    task: Task,
    space: &Space,
    how: How,
    by_user: bool,
) -> Result<Task, String> {
    if let Some(why) = crate::workflows::open_db(app).ok().and_then(|conn| crate::workflows::held(&conn, task.id)) {
        return Ok(wait_to_start(app, &task, how, &why));
    }
    let task = match crate::workflows::begin(app, task.id).await? {
        Some(done) if !done.go => return Ok(wait_to_start(app, &task, how, &done.text)),
        Some(_) => get(&db(app)?, task.id)?,
        None => task,
    };
    let task = match seat_of(&task) {
        Some(Seat::Terminal(terminal)) if !launch::terminal_open(&terminal).await => launch::replace_gone(&db(app)?, &task, &terminal)?,
        _ => task,
    };
    if task.status == TaskStatus::Inbox {
        return Ok(task);
    }
    let conn = db(app)?;
    refuse_locked(&conn, &task, space)?;
    let seat = seat_of(&task).ok_or_else(|| {
        format!(
            "Task #{} has no one assigned. Pick a custom agent, a terminal or an agent type first.",
            task.id
        )
    })?;
    let (why, scope) = wait_reason(app, &conn, &task, space);
    drop(conn);
    if let Some(why) = why {
        return Ok(wait_to_start(app, &task, how, &why));
    }
    if !by_user
        && let Some(held) = crate::term_tools::work_hold(app, start_action(how), &task.title, None)
    {
        return Ok(wait_to_start(app, &task, how, &held));
    }
    let custom = custom_of(&task);
    if let Some(id) = &custom {
        let taken = inner(app).seating.get(id).copied();
        if let Some(other) = taken.filter(|other| *other != task.id) {
            return Ok(wait_to_start(
                app,
                &task,
                how,
                &format!("the custom agent {id} is starting task #{other}"),
            ));
        }
    }
    if super::pick::hold(app, &task, space, &seat, how, by_user) {
        return Ok(db(app).ok().and_then(|c| get(&c, task.id).ok()).unwrap_or(task));
    }
    let _guard = {
        let mut state = inner(app);
        if let Some(id) = &custom {
            state.seating.insert(id.clone(), task.id);
        }
        state.launches.insert(
            task.id,
            Launch {
                space: space.id.clone(),
                scope: scope.clone(),
            },
        );
        Seating {
            app,
            seat: custom.clone(),
            task: task.id,
        }
    };
    let started = launch_task(app, task, space, how, by_user, &seat, &scope).await;
    if let Ok(task) = &started {
        super::marks::sync(app, false, Some(task.id)).await;
    }
    started
}

async fn launch_task(
    app: &Arc<App>,
    task: Task,
    space: &Space,
    how: How,
    by_user: bool,
    seat: &Seat,
    scope: &Scope,
) -> Result<Task, String> {
    let task = match mismatch_of(app, &task, space)? {
        Some((moved, next)) if next.id != space.id => return Err(format!("Task #{} moved to space \"{}\", which owns its folder. Nixa starts it there.", moved.id, next.name)),
        Some((moved, _)) => moved,
        None => task,
    };
    let (person, review) = person_of(app, &task);
    let pick = if super::pick::wanted(seat) || matches!(seat, Seat::Custom(_)) { super::pick::usable(app, &task) } else { None };
    let want = pick.as_ref().and_then(super::pick::want);
    let plan = build_plan(
        &task,
        &PlanInput {
            member: person.clone(),
            model: pick.as_ref().map(|p| p.model.clone()).unwrap_or_default(),
            space: space.name.clone(),
            folder: space.folder.clone(),
            allowed_files: scope.files.clone(),
            review,
            waits: db(app).map(|c| nixa_store::task_waits::waits_of(&c, task.id)).unwrap_or_default(),
        },
    );
    let text = {
        let conn = db(app)?;
        let _ = tasks::set_task_meta(
            &conn,
            &format!("{PLAN_KEY}{}", task.id),
            &serde_json::to_string(&plan).unwrap_or_default(),
        );
        let comments = tasks::task_comments(&conn, task.id).unwrap_or_default();
        let held = holders(&conn);
        let member = if person.is_empty() {
            None
        } else {
            Some(person.as_str())
        };
        crate::joins::stage_text(
            app,
            task.id,
            with_rules(
                task_prompt(&PromptInput {
                    task: &task,
                    comments: &comments,
                    member,
                    review,
                    others: &held,
                    plan: &plan,
                }),
                &super::scope::work_folder(&task, &space.folder),
            ),
        )
    };
    let base = super::git::base_of(&super::scope::work_folder(&task, &space.folder)).await;
    let conn = db(app)?;
    let latest = get(&conn, task.id)?;
    refuse_locked(&conn, &latest, space)?;
    if matches!(how, How::Auto | How::HandOff) || !by_user {
        super::waits::refuse_open(&conn, task.id)?;
    }
    let holder = starting_holder(task.id);
    let who = if task.assigned.is_empty() {
        agent_for(&task)
    } else {
        task.assigned.clone()
    };
    let mut actor = Actor::in_terminal(
        AuthorKind::System,
        &format!("Nixa, starting {who}"),
        &holder,
    );
    actor.space_id = space.id.clone();
    let claim = tasks::claim_task(&conn, task.id, &actor, &base).map_err(fail)?;
    if !claim.ok {
        return Err(format!(
            "Nixa did not start an agent for task #{}: {}",
            task.id, claim.reason
        ));
    }
    drop(conn);
    changed(app, "queue", Some(task.id));
    let mut used = pick.clone();
    let outcome = match launch::open(app, &latest, space, seat, &text, by_user, want.as_ref()).await {
        Outcome::Failed(why) if want.is_some() && pick.is_some() => {
            if let (Ok(conn), Some(p)) = (db(app), pick.as_ref()) {
                super::pick::failed_pick(&conn, task.id, p, how, &why);
            }
            used = None;
            launch::open(app, &latest, space, seat, &text, by_user, None).await
        }
        other => other,
    };
    let started = match outcome {
        Outcome::Started(started) => started,
        Outcome::Busy(why) => {
            if let Ok(conn) = db(app)
                && tasks::drop_claim(&conn, task.id, &holder)
                    .ok()
                    .flatten()
                    .is_some()
            {
                changed(app, "queue", Some(task.id));
            }
            let why = cut(why.trim_end_matches(['.', ' ']), 200);
            return Ok(wait_to_start(app, &task, how, &why));
        }
        Outcome::Failed(why) => {
            if let Ok(conn) = db(app)
                && tasks::drop_claim(&conn, task.id, &holder)
                    .ok()
                    .flatten()
                    .is_some()
            {
                changed(app, "queue", Some(task.id));
            }
            return Err(why);
        }
    };
    let conn = db(app)?;
    let Some(claimed) =
        tasks::move_claim(&conn, task.id, &holder, &started.terminal, &started.name)
            .map_err(fail)?
    else {
        let now = tasks::get_task(&conn, task.id)
            .ok()
            .flatten()
            .map(|t| t.status.as_str())
            .unwrap_or("deleted");
        drop(conn);
        if started.fresh {
            launch::stop(&started.terminal).await;
            return Err(format!(
                "Task #{} changed while {} was starting (it is {now} now), so Nixa closed terminal {}.",
                task.id, started.name, started.terminal
            ));
        }
        return Err(format!(
            "Task #{} changed while {} was starting (it is {now} now), so Nixa gave it no work.",
            task.id, started.name
        ));
    };
    super::pick::started(&conn, task.id);
    let picked = used
        .map(|p| {
            let whose = if p.ask == crate::workflows::driver::SAVED_ASK {
                "the workflow stage's"
            } else if p.ask == super::plan::PICK_ASK {
                "the step plan's"
            } else {
                "the deep brain's"
            };
            format!(" It uses {whose} pick: {}.", p.label)
        })
        .unwrap_or_default();
    let extra = if started.note.is_empty() {
        String::new()
    } else {
        format!(" {}", started.note)
    };
    note(
        &conn,
        task.id,
        &format!(
            "{}: {} started in terminal {} and holds the claim.{picked} Plan: goal=\"{}\", first=\"{}\".{extra}",
            how.label(),
            started.name,
            started.terminal,
            cut(&plan.goal, 80),
            cut(&plan.first_action, 120)
        ),
        CommentKind::Claim,
    );
    if tasks::task_meta(&conn, &format!("{HANDOFF_START_KEY}{}", task.id))
        .ok()
        .flatten()
        .is_some_and(|v| !v.is_empty())
    {
        let _ = tasks::set_task_meta(&conn, &format!("{HANDOFF_START_KEY}{}", task.id), "");
    }
    {
        let mut state = inner(app);
        state.queued.remove(&task.id);
        state.start_failed.remove(&task.id);
    }
    app.info(&format!(
        "Task #{} started: {} in {}.{extra}",
        task.id, started.name, space.name
    ));
    log(
        "start",
        json!({"task": task.id, "how": how.label(), "terminal": started.terminal, "agent": agent_for(&task)}),
    );
    changed(app, "queue", Some(task.id));
    Ok(get(&conn, task.id).unwrap_or(claimed))
}

pub fn with_rules(prompt: String, folder: &str) -> String {
    if folder.is_empty() {
        return prompt;
    }
    let rules = crate::knowledge::rules::read(std::path::Path::new(folder));
    let line = crate::knowledge::rules::rules_for_agent(rules.as_ref());
    if line.trim().is_empty() { prompt } else { format!("{prompt}\n{}", line.trim()) }
}

fn startable(app: &App, id: i64, mine: bool) -> Result<Task, String> {
    let conn = db(app)?;
    let task = get(&conn, id)?;
    if !mine && inner(app).starting.contains_key(&id) {
        return Err(format!("Task #{id} is already starting."));
    }
    if !task.terminal_id.is_empty() {
        return Err(format!(
            "Task #{id} is already claimed by terminal {}.",
            task.terminal_id
        ));
    }
    if !matches!(task.status, TaskStatus::Inbox | TaskStatus::Todo) {
        return Err(format!(
            "Task #{id} is {}. Only an inbox or todo task can start.",
            task.status.as_str()
        ));
    }
    if task.archived_at > 0 {
        return Err(format!("Task #{id} is archived. Unarchive it first."));
    }
    if task.space_id.is_empty() {
        return Err(format!("Task #{id} has no space. Pick a space first."));
    }
    if task.assigned.is_empty() {
        return Err(format!(
            "Task #{id} has no one assigned. Pick a custom agent, a terminal or an agent type first."
        ));
    }
    Ok(task)
}

struct Starting<'a> {
    app: &'a App,
    task: i64,
}

impl Drop for Starting<'_> {
    fn drop(&mut self) {
        inner(self.app).starting.remove(&self.task);
    }
}

fn mark_starting<'a>(app: &'a App, task: &Task) -> Starting<'a> {
    inner(app).starting.insert(task.id, task.space_id.clone());
    Starting { app, task: task.id }
}

fn space_of(app: &App, task: &Task) -> Result<Space, String> {
    let conn = db(app)?;
    places(&conn)
        .space(&task.space_id)
        .cloned()
        .ok_or_else(|| format!("The space of task #{} is not known.", task.id))
}

pub async fn start_now(app: &Arc<App>, id: i64, by_user: bool) -> Result<Task, String> {
    let task = startable(app, id, false)?;
    let _mark = mark_starting(app, &task);
    let space = space_of(app, &task)?;
    let fresh = startable(app, id, true)?;
    if fresh.space_id != space.id {
        return Err(format!("Task #{id} moved to another space. Try again."));
    }
    refuse_locked(&db(app)?, &fresh, &space)?;
    if !by_user {
        super::waits::refuse_and_hold(&db(app)?, id, super::waits::START)?;
    }
    let (fresh, space) = match mismatch_of(app, &fresh, &space)? {
        Some(moved) => moved,
        None => (fresh, space),
    };
    refuse_locked(&db(app)?, &fresh, &space)?;
    let ready = if fresh.status == TaskStatus::Todo {
        fresh
    } else {
        tasks::set_status(&db(app)?, id, TaskStatus::Todo, None, false).map_err(fail)?
    };
    match start_task(app, ready.clone(), &space, How::Now, by_user).await {
        Ok(task) => Ok(task),
        Err(error) => {
            if let Ok(conn) = db(app)
                && get(&conn, id)
                    .map(|t| t.status != TaskStatus::Blocked)
                    .unwrap_or(false)
            {
                note(
                    &conn,
                    id,
                    &format!(
                        "Start now failed: {}.",
                        cut(error.trim_end_matches('.'), 300)
                    ),
                    CommentKind::Error,
                );
            }
            changed(app, "queue", Some(id));
            Err(error)
        }
    }
}

pub fn retry_ok(app: &App, id: i64) -> bool {
    let at = inner(app).start_failed.get(&id).copied().unwrap_or(0);
    now_ms().saturating_sub(at) >= START_RETRY_MS
}

pub async fn start_one(app: &Arc<App>, task: Task, space: Space, how: How) {
    if crate::modes::gate(app, start_action(how), None).is_some() {
        return;
    }
    let Ok(conn) = db(app) else { return };
    let Ok(fresh) = get(&conn, task.id) else {
        return;
    };
    drop(conn);
    if fresh.status != TaskStatus::Todo
        || !fresh.terminal_id.is_empty()
        || inner(app).starting.contains_key(&task.id)
    {
        return;
    }
    let Ok(conn) = db(app) else { return };
    if refuse_locked(&conn, &fresh, &space).is_err() || super::waits::waits_on_open(&conn, fresh.id) {
        return;
    }
    drop(conn);
    let (fresh, space) = match mismatch_of(app, &fresh, &space) {
        Ok(Some(moved)) => moved,
        Ok(None) => (fresh, space),
        Err(_) => return,
    };
    let Ok(conn) = db(app) else { return };
    if refuse_locked(&conn, &fresh, &space).is_err() {
        return;
    }
    drop(conn);
    let _mark = mark_starting(app, &fresh);
    if let Err(error) = start_task(app, fresh, &space, how, true).await {
        if let Ok(conn) = db(app)
            && get(&conn, task.id)
                .map(|t| t.status != TaskStatus::Blocked)
                .unwrap_or(false)
        {
            inner(app).start_failed.insert(task.id, now_ms());
            let what = if how == How::HandOff {
                "Hand-off start"
            } else {
                "Auto-start"
            };
            note(
                &conn,
                task.id,
                &format!(
                    "{what} failed: {}. Nixa tries again in 30 minutes.",
                    cut(error.trim_end_matches('.'), 300)
                ),
                CommentKind::Error,
            );
            log(
                "auto-start failed",
                json!({"task": task.id, "how": how.label(), "error": cut(&error, 200)}),
            );
        }
        changed(app, "queue", Some(task.id));
    }
}

pub const AUTO_OFF_KEY: &str = "auto_start_off:";

pub fn user_turned_off(conn: &Connection, space: &str) -> bool {
    tasks::task_meta(conn, &format!("{AUTO_OFF_KEY}{space}")).ok().flatten().is_some_and(|v| !v.is_empty())
}

pub fn mark_auto_start(conn: &Connection, space: &str, on: bool) {
    let _ = tasks::set_task_meta(conn, &format!("{AUTO_OFF_KEY}{space}"), if on { "" } else { "1" });
}

pub fn auto_on(conn: &Connection, row: &nixa_store::spaces::Space) -> bool {
    row.auto_start || !user_turned_off(conn, &row.space_id)
}

pub async fn auto_start(app: &Arc<App>) {
    let Ok(conn) = db(app) else { return };
    let pl = places(&conn);
    let mut jobs = Vec::new();
    for row in pl.rows.iter().filter(|r| !r.locked && auto_on(&conn, r)) {
        let Some(space) = pl.space(&row.space_id) else {
            continue;
        };
        if !auto_space(space) {
            continue;
        }
        let filter = TaskFilter {
            space: Some(space.id.clone()),
            status: vec![TaskStatus::Todo],
            ..TaskFilter::default()
        };
        for task in tasks::list_tasks(&conn, &filter).unwrap_or_default() {
            if task.assigned.is_empty()
                || !task.terminal_id.is_empty()
                || task.archived_at > 0
                || inner(app).starting.contains_key(&task.id)
                || !retry_ok(app, task.id)
                || super::pick::waits(app, &conn, task.id)
                || !super::plan::auto_ok(&conn, task.id)
            {
                continue;
            }
            jobs.push(start_one(app, task, space.clone(), How::Auto));
        }
    }
    drop(conn);
    futures_util::future::join_all(jobs).await;
}

pub async fn start_picked(app: &Arc<App>) {
    super::pick::start_due(app, |id| retry_ok(app, id)).await;
}

pub async fn start_handoffs(app: &Arc<App>) {
    let Ok(conn) = db(app) else { return };
    let pl = places(&conn);
    let plain: String = HANDOFF_START_KEY.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    let waiting: Vec<(String, String)> =
        tasks::task_meta_list(&conn, &plain).unwrap_or_default();
    let mut jobs = Vec::new();
    for (key, value) in waiting {
        if value != "1" {
            continue;
        }
        let Some(id) = key
            .strip_prefix(HANDOFF_START_KEY)
            .and_then(|n| n.parse::<i64>().ok())
        else {
            continue;
        };
        let Ok(task) = get(&conn, id) else { continue };
        if task.status != TaskStatus::Todo
            || task.assigned.is_empty()
            || !task.terminal_id.is_empty()
            || inner(app).starting.contains_key(&id)
            || !retry_ok(app, id)
            || !super::plan::auto_ok(&conn, id)
        {
            continue;
        }
        let Some(space) = pl.space(&task.space_id).cloned() else {
            continue;
        };
        if space_locked(&space, &pl.rows) {
            continue;
        }
        jobs.push(start_one(app, task, space, How::HandOff));
    }
    drop(conn);
    futures_util::future::join_all(jobs).await;
}

pub async fn checkpoint(app: &Arc<App>, task: &Task, why: &str, seen: Seen) -> Task {
    let terminal = task.terminal_id.clone();
    if terminal.is_empty() || is_starting(&terminal) || is_run_holder(&terminal) {
        return task.clone();
    }
    let folder = db(app).ok().and_then(|c| places(&c).folder_of(task));
    let found = match (&folder, task.base.is_empty()) {
        (Some(folder), false) => super::git::changes(&task.base, folder).await,
        _ => super::git::Changes::NoBase,
    };
    let changed_now: Vec<String> = match &found {
        super::git::Changes::Files(files) => files.clone(),
        _ => Vec::new(),
    };
    let mut files: Vec<String> = task.uncommitted.clone();
    for file in &changed_now {
        if !files.contains(file) {
            files.push(file.clone());
        }
    }
    let text = match seen {
        Seen::Read => screen(&terminal).await.map(|s| s.0).unwrap_or_default(),
        Seen::Nothing => String::new(),
        Seen::Text(text) => text,
    };
    let shown = screen_tail(&text, prompt::CHECKPOINT_LINES);
    let name = if task.claimed_by.is_empty() {
        terminal.clone()
    } else {
        task.claimed_by.clone()
    };
    let Ok(conn) = db(app) else {
        return task.clone();
    };
    if changed_now.iter().any(|f| !task.uncommitted.contains(f)) {
        let _ = tasks::set_uncommitted(&conn, task.id, &files);
    }
    let last = LastWorker {
        terminal: terminal.clone(),
        agent: agent_for(task),
        name: name.clone(),
        at: now_ms(),
    };
    let _ = tasks::set_task_meta(
        &conn,
        &format!("{LAST_KEY}{}", task.id),
        &serde_json::to_string(&last).unwrap_or_default(),
    );
    let files_line = if !files.is_empty() {
        format!(
            "Files changed so far, not committed: {}.",
            cut(&files.join(", "), 800)
        )
    } else if found == super::git::Changes::Unread {
        "Nixa could not read the git changes, so the changed files are not known.".to_string()
    } else if !task.base.is_empty() {
        "No changed files yet.".to_string()
    } else {
        "Nixa has no start point for this claim, so it cannot list the changed files.".to_string()
    };
    let mut lines = vec![
        format!(
            "{}{name} in terminal {terminal} ({why}).",
            prompt::CHECKPOINT_PREFIX
        ),
        files_line,
    ];
    if !shown.is_empty() {
        lines.push(format!("Last screen:\n\"\"\"{shown}\"\"\""));
    }
    note(&conn, task.id, &lines.join("\n"), CommentKind::Handoff);
    log(
        "checkpoint",
        json!({"task": task.id, "terminal": terminal, "files": files.len(), "why": why}),
    );
    get(&conn, task.id).unwrap_or_else(|_| task.clone())
}

fn busy_elsewhere(conn: &Connection, terminal: &str, id: i64) -> bool {
    let filter = TaskFilter {
        terminal: terminal.to_string(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review],
        old: true,
        ..TaskFilter::default()
    };
    tasks::list_tasks(conn, &filter)
        .unwrap_or_default()
        .iter()
        .any(|t| t.id != id)
}

async fn agent_runs(terminal: &str) -> bool {
    let Ok(id) = crate::engine::term_id(terminal) else {
        return false;
    };
    crate::engine::get(&id)
        .await
        .is_ok_and(|info| info.running && !info.cmd.is_empty())
}

async fn tell_same(app: &Arc<App>, task: &Task) -> Option<Continued> {
    let terminal = task.terminal_id.clone();
    if !agent_runs(&terminal).await {
        return None;
    }
    let text = {
        let conn = db(app).ok()?;
        go_on_text(
            task,
            &tasks::task_comments(&conn, task.id).unwrap_or_default(),
        )
    };
    let id = crate::engine::term_id(&terminal).ok()?;
    crate::engine::send(&id, &text, true, false).await.ok()?;
    let conn = db(app).ok()?;
    let fresh = get(&conn, task.id).ok()?;
    if fresh.terminal_id != terminal {
        return None;
    }
    if fresh.status != TaskStatus::Doing {
        tasks::set_status(&conn, task.id, TaskStatus::Doing, None, false).ok()?;
    }
    let _ = tasks::touch_terminal(&conn, &terminal);
    let name = if task.claimed_by.is_empty() {
        terminal.clone()
    } else {
        task.claimed_by.clone()
    };
    note(
        &conn,
        task.id,
        &format!("Continue: the user asked {name} in terminal {terminal} to go on with this task."),
        CommentKind::Claim,
    );
    log(
        "continue",
        json!({"task": task.id, "how": "told", "terminal": terminal}),
    );
    changed(app, "continue", Some(task.id));
    Some(Continued {
        task: get(&conn, task.id).ok()?,
        how: "told",
        terminal,
    })
}

async fn take_back(
    app: &Arc<App>,
    task: &Task,
    last: &LastWorker,
    space: &Space,
) -> Result<Option<Continued>, String> {
    let conn = db(app)?;
    refuse_locked(&conn, task, space)?;
    let ready = if task.status == TaskStatus::Todo {
        task.clone()
    } else {
        tasks::set_status(&conn, task.id, TaskStatus::Todo, None, false).map_err(fail)?
    };
    let (why, scope) = wait_reason(app, &conn, &ready, space);
    drop(conn);
    if let Some(why) = why {
        return Ok(Some(Continued {
            task: wait_to_start(app, &ready, How::Continue, &why),
            how: "queued",
            terminal: String::new(),
        }));
    }
    let _guard = {
        let mut state = inner(app);
        state.launches.insert(
            task.id,
            Launch {
                space: space.id.clone(),
                scope: scope.clone(),
            },
        );
        Seating {
            app,
            seat: None,
            task: task.id,
        }
    };
    let base = super::git::base_of(&super::scope::work_folder(task, &space.folder)).await;
    let conn = db(app)?;
    let latest = get(&conn, task.id)?;
    refuse_locked(&conn, &latest, space)?;
    if busy_elsewhere(&conn, &last.terminal, task.id) {
        return Ok(None);
    }
    let actor = Actor::in_terminal(AuthorKind::Agent, &last.name, &last.terminal);
    let claim = tasks::claim_task(&conn, task.id, &actor, &base).map_err(fail)?;
    if !claim.ok {
        return Ok(None);
    }
    let (person, review) = person_of(app, &latest);
    let plan = build_plan(
        &latest,
        &PlanInput {
            member: last.name.clone(),
            model: String::new(),
            space: space.name.clone(),
            folder: space.folder.clone(),
            allowed_files: scope.files.clone(),
            review,
            waits: nixa_store::task_waits::waits_of(&conn, task.id),
        },
    );
    let comments = tasks::task_comments(&conn, task.id).unwrap_or_default();
    let held = holders(&conn);
    let member = if person.is_empty() {
        None
    } else {
        Some(person.as_str())
    };
    let text = task_prompt(&PromptInput {
        task: &latest,
        comments: &comments,
        member,
        review,
        others: &held,
        plan: &plan,
    });
    drop(conn);
    let sent = match crate::engine::term_id(&last.terminal) {
        Ok(id) => crate::engine::send(&id, &text, true, false).await.is_ok(),
        Err(_) => false,
    };
    let conn = db(app)?;
    if !sent {
        let _ = tasks::drop_claim(&conn, task.id, &last.terminal);
        return Ok(None);
    }
    note(
        &conn,
        task.id,
        &format!(
            "Continue: gave it back to {} in terminal {}, which worked on it before.",
            last.name, last.terminal
        ),
        CommentKind::Claim,
    );
    log(
        "continue",
        json!({"task": task.id, "how": "back", "terminal": last.terminal}),
    );
    changed(app, "continue", Some(task.id));
    Ok(Some(Continued {
        task: get(&conn, task.id)?,
        how: "back",
        terminal: last.terminal.clone(),
    }))
}

pub async fn continue_task(
    app: &Arc<App>,
    id: i64,
    assign: Option<(String, String)>,
) -> Result<Continued, String> {
    let conn = db(app)?;
    let task = get(&conn, id)?;
    drop(conn);
    if task.archived_at > 0 {
        return Err(format!("Task #{id} is archived. Unarchive it first."));
    }
    if inner(app).starting.contains_key(&id) || is_starting(&task.terminal_id) {
        return Err(format!("Task #{id} is starting now. Wait a moment."));
    }
    if is_run_holder(&task.terminal_id) {
        return Err(format!(
            "A Nixa run holds task #{id}. Wait for the run to end, or free the claim first."
        ));
    }
    let target = assign.or_else(|| {
        (!task.assigned.is_empty()).then(|| (task.assigned_kind.clone(), task.assigned.clone()))
    });
    let Some((kind, who)) = target else {
        return Err(format!(
            "Task #{id} has no one assigned. Pick a custom agent, a terminal or an agent type first."
        ));
    };
    if task.space_id.is_empty() {
        return Err(format!("Task #{id} has no space. Pick a space first."));
    }
    let other = who != task.assigned || kind != task.assigned_kind;
    let how = if other { How::HandOver } else { How::Continue };
    let _mark = mark_starting(app, &task);
    let space = space_of(app, &task)?;
    refuse_locked(&db(app)?, &task, &space)?;
    let (task, space) = match mismatch_of(app, &task, &space)? {
        Some(moved) => moved,
        None => (task, space),
    };
    refuse_locked(&db(app)?, &task, &space)?;
    let mut current = task.clone();
    if !current.terminal_id.is_empty() {
        let old = current.terminal_id.clone();
        let mine = !busy_elsewhere(&db(app)?, &old, id);
        let alive = mine && live().await.is_some_and(|l| l.contains(&old));
        if !other
            && alive
            && let Some(told) = tell_same(app, &current).await
        {
            return Ok(told);
        }
        if mine {
            let why = if other {
                format!("the user handed it to {who}")
            } else if alive {
                "Nixa could not send to its agent, so Nixa starts a new one".to_string()
            } else {
                "its terminal is closed".to_string()
            };
            checkpoint(
                app,
                &current,
                &why,
                if alive { Seen::Read } else { Seen::Nothing },
            )
            .await;
        }
        current = tasks::release_task(&db(app)?, id).map_err(fail)?;
        if other && alive && !saved_seat(app, &old) {
            launch::stop(&old).await;
        }
    } else if !other {
        let saved = tasks::task_meta(&db(app)?, &format!("{LAST_KEY}{id}"))
            .ok()
            .flatten();
        let last = read_last(saved.as_deref());
        if let Some(last) = last
            && last.agent == agent_for(&current)
            && !busy_elsewhere(&db(app)?, &last.terminal, id)
            && live().await.is_some_and(|l| l.contains(&last.terminal))
        {
            if let Some(back) = take_back(app, &current, &last, &space).await? {
                return Ok(back);
            }
            current = get(&db(app)?, id)?;
        }
    }
    let conn = db(app)?;
    if other {
        current = tasks::assign_task(&conn, id, &kind, &who).map_err(fail)?;
    }
    let ready = if current.status == TaskStatus::Todo && current.terminal_id.is_empty() {
        current
    } else {
        tasks::set_status(&conn, id, TaskStatus::Todo, None, false).map_err(fail)?
    };
    drop(conn);
    match start_task(app, ready.clone(), &space, how, true).await {
        Ok(started) => {
            let word = if started.terminal_id.is_empty() {
                "queued"
            } else if other {
                "handed"
            } else {
                "started"
            };
            let terminal = started.terminal_id.clone();
            changed(app, "continue", Some(id));
            Ok(Continued {
                task: started,
                how: word,
                terminal,
            })
        }
        Err(error) => {
            if let Ok(conn) = db(app)
                && get(&conn, id)
                    .map(|t| t.status != TaskStatus::Blocked)
                    .unwrap_or(false)
            {
                note(
                    &conn,
                    id,
                    &format!(
                        "{} failed: {}.",
                        how.label(),
                        cut(error.trim_end_matches('.'), 300)
                    ),
                    CommentKind::Error,
                );
            }
            changed(app, "continue", Some(id));
            Err(error)
        }
    }
}

pub struct JobInput {
    pub terminal: String,
    pub actor: Actor,
    pub request: String,
    pub folder: String,
    pub space: Option<Space>,
    pub base: String,
}

fn take_named(conn: &Connection, target: &Task, input: &JobInput) -> Option<Task> {
    if !matches!(target.status, TaskStatus::Todo | TaskStatus::Inbox)
        || !target.terminal_id.is_empty()
    {
        return None;
    }
    let mut patch = tasks::TaskPatch::default();
    if target.space_id.is_empty()
        && let Some(space) = &input.space
    {
        patch.space_id = Some(space.id.clone());
        patch.space_name = Some(space.name.clone());
    }
    if target.repo.is_empty() && !input.folder.is_empty() {
        patch.repo = Some(input.folder.clone());
    }
    if patch != tasks::TaskPatch::default() {
        tasks::update_task(conn, target.id, &patch).ok()?;
    }
    if target.status == TaskStatus::Inbox {
        tasks::set_status(conn, target.id, TaskStatus::Todo, None, false).ok()?;
    }
    let claim = tasks::claim_task(conn, target.id, &input.actor, &input.base).ok()?;
    if !claim.ok {
        return None;
    }
    note(
        conn,
        target.id,
        &format!(
            "Nixa sent this job to terminal {}. The job text names this task, so no new task was made. Job text: \"\"\"{}\"\"\"",
            input.terminal,
            cut(input.request.trim(), 600)
        ),
        CommentKind::Claim,
    );
    claim.task
}

pub fn job_started(app: &App, input: &JobInput) -> Option<i64> {
    let conn = db(app).ok()?;
    let filter = TaskFilter {
        terminal: input.terminal.clone(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked],
        ..TaskFilter::default()
    };
    if let Some(held) = tasks::list_tasks(&conn, &filter).ok()?.first() {
        return Some(held.id);
    }
    let target =
        named_task(&input.request).and_then(|id| tasks::get_task(&conn, id).ok().flatten());
    if let Some(target) = &target
        && let Some(taken) = take_named(&conn, target, input)
    {
        log(
            "job linked",
            json!({"task": taken.id, "terminal": input.terminal}),
        );
        changed(app, "job", Some(taken.id));
        return Some(taken.id);
    }
    let title = cut(
        &input
            .request
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        120,
    );
    if title.is_empty() {
        return None;
    }
    let (space_id, space_name) = input
        .space
        .as_ref()
        .map(|s| (s.id.clone(), s.name.clone()))
        .unwrap_or_default();
    let made = tasks::add_task(
        &conn,
        &TaskInput {
            title,
            description: input.request.clone(),
            status: Some(TaskStatus::Todo),
            source: Some(TaskSource::Nixa),
            space_id,
            space_name,
            repo: input.folder.clone(),
            assigned_kind: crate::tasks::KIND_TERMINAL.into(),
            assigned: input.terminal.clone(),
            ..TaskInput::default()
        },
        &super::nixa(),
    )
    .ok()?;
    if tasks::claim_task(&conn, made.id, &input.actor, &input.base)
        .ok()
        .is_some_and(|c| c.ok)
    {
        note(
            &conn,
            made.id,
            &format!("Nixa sent this job to terminal {}.", input.terminal),
            CommentKind::Claim,
        );
    }
    if let Some(target) = target {
        let why = if !target.terminal_id.is_empty() {
            format!(
                "{} in terminal {} holds it",
                if target.claimed_by.is_empty() {
                    &target.terminal_id
                } else {
                    &target.claimed_by
                },
                target.terminal_id
            )
        } else {
            format!("it is {}", target.status.as_str())
        };
        note(
            &conn,
            made.id,
            &format!(
                "Part of task #{}. That task was not given to this job because {why}.",
                target.id
            ),
            CommentKind::Note,
        );
        note(
            &conn,
            target.id,
            &format!(
                "Nixa sent part of this task to terminal {} as task #{}.",
                input.terminal, made.id
            ),
            CommentKind::Note,
        );
    }
    changed(app, "job", Some(made.id));
    Some(made.id)
}

pub async fn resume_todos(app: &Arc<App>, space: Option<&str>) -> (Vec<i64>, Vec<i64>) {
    let list = match db(app) {
        Ok(conn) => {
            let filter = TaskFilter {
                space: space.map(str::to_string),
                status: vec![TaskStatus::Todo],
                ..TaskFilter::default()
            };
            tasks::list_tasks(&conn, &filter).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    let mut started = Vec::new();
    let mut skipped = Vec::new();
    for task in list
        .into_iter()
        .filter(|t| !t.assigned.is_empty() && t.terminal_id.is_empty())
    {
        match start_now(app, task.id, true).await {
            Ok(_) => started.push(task.id),
            Err(error) => {
                skipped.push(task.id);
                log(
                    "resume skipped",
                    json!({"task": task.id, "error": cut(&error, 200)}),
                );
            }
        }
    }
    log("resume", json!({"started": started, "skipped": skipped}));
    changed(app, "resume", None);
    (started, skipped)
}
