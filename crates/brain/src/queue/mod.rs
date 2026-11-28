mod api;
pub mod dispatch;
pub mod git;
mod launch;
pub(crate) mod lock;
pub mod marks;
pub mod notes;
pub mod pick;
pub mod plan;
pub mod prompt;
pub mod scope;
mod start;
mod sweep;
#[cfg(test)]
mod dispatch_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod waits_tests;
mod tool;
pub mod waits;
pub mod window;

pub use api::{Role, WS_KIND, action, caller, is_ws, routes, ws_request};
pub use git::{Changes, Print, base_of, changed_paths, changes, fingerprint};
pub use launch::{Outcome, Seat, Started, seat_of};
pub use lock::{Locked, auto_space, folder_mismatch, locked_text, task_locked};
pub(crate) use lock::{locked_space, unlocked};
pub use prompt::{AgentPlan, FinalWord, LastWorker, final_word, named_task, task_prompt};
pub use scope::{Scope, read_only_task, scope_of};
pub use start::{
    Continued, How, JobInput, Seen, checkpoint, continue_task, job_started, resume_todos, start_now,
};
pub use tool::{NAMES, call, specs};

use crate::app::{App, lock};
use crate::spaces::Space;
use futures_util::future::BoxFuture;
use nixa_store::Connection;
use nixa_store::spaces::Space as Row;
use nixa_store::tasks::{self, Actor, AuthorKind, CommentInput, CommentKind, Task};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub const QUEUE_MS: u64 = 60_000;
pub const FIRST_TICK_MS: u64 = 5_000;
pub const STALE_MS: i64 = 60 * 60 * 1000;
pub const RUN_STALE_MS: i64 = 6 * 60 * 60 * 1000;
pub const BRAIN_INBOX_MS: u64 = 10 * 60 * 1000;
const QUESTION_RETRY_MS: u64 = 10 * 60 * 1000;
pub const START_RETRY_MS: u64 = 30 * 60 * 1000;
pub const PLAN_KEY: &str = "task_plan:";
pub const LAST_KEY: &str = "task_last:";
pub const HANDOFF_START_KEY: &str = "handoff_start:";
pub const CHANGED: &str = "tasks_changed";
pub const TASK_LABEL: &str = "nixa.task";

#[derive(Default)]
pub struct Inner {
    pub screens: HashMap<String, String>,
    pub brain_asked: HashMap<i64, u64>,
    pub start_failed: HashMap<i64, u64>,
    pub spoken: HashSet<String>,
    question_checks: HashMap<i64, (String, u64)>,
    pub starting: HashMap<i64, String>,
    pub launches: HashMap<i64, scope::Launch>,
    pub queued: HashMap<i64, String>,
    pub seating: HashMap<String, i64>,
    pub noting: HashSet<String>,
    pub raised: HashMap<i64, String>,
    pub marks: HashMap<String, marks::Mark>,
}

#[derive(Default)]
pub struct State {
    pub inner: Mutex<Inner>,
    pub ticking: AtomicBool,
    pub marking: tokio::sync::Mutex<()>,
}

pub type ReviewHook = fn(Arc<App>, i64, String) -> BoxFuture<'static, ()>;

static REVIEW: OnceLock<ReviewHook> = OnceLock::new();

pub fn set_review(hook: ReviewHook) -> bool {
    REVIEW.set(hook).is_ok()
}

pub fn inner(app: &App) -> std::sync::MutexGuard<'_, Inner> {
    lock(&app.queue.inner)
}

pub fn db(app: &App) -> Result<Connection, String> {
    nixa_store::open(&app.db_file).map_err(|e| format!("The task store did not open: {e}"))
}

pub fn nixa() -> Actor {
    Actor::new(AuthorKind::System, "Nixa")
}

pub fn note(conn: &Connection, id: i64, text: &str, kind: CommentKind) {
    if let Err(error) = tasks::add_comment(conn, id, &CommentInput::new(kind, text), &nixa()) {
        crate::util::log("queue", &format!("comment on #{id} failed: {error}"));
    }
}

pub fn changed(app: &App, why: &str, id: Option<i64>) {
    let waiting = db(app)
        .ok()
        .and_then(|conn| {
            let rows = nixa_store::spaces::list_spaces(&conn).ok()?;
            Some(lock::unlocked(tasks::waiting_tasks(&conn).ok()?, &rows).len())
        })
        .unwrap_or(0);
    app.send_out(json!({"type": CHANGED, "why": why, "id": id, "waiting": waiting}));
}

pub fn log(event: &str, data: Value) {
    crate::util::log("queue", &format!("{event} {data}"));
}

pub struct Places {
    pub spaces: Vec<Space>,
    pub rows: Vec<Row>,
}

impl Places {
    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.space_id == id)
    }

    pub fn max_doing(&self, id: &str) -> i64 {
        self.row(id)
            .map(|r| r.max_doing)
            .unwrap_or(nixa_store::spaces::MAX_DOING)
    }

    pub fn folder_of(&self, task: &Task) -> Option<String> {
        self.space(&task.space_id)
            .map(|s| scope::work_folder(task, &s.folder))
            .filter(|f| !f.is_empty())
            .or_else(|| Some(task.repo.clone()).filter(|r| !r.is_empty()))
    }
}

pub fn places(conn: &Connection) -> Places {
    let spaces = crate::spaces::from_store(conn);
    Places {
        spaces,
        rows: nixa_store::spaces::list_spaces(conn).unwrap_or_default(),
    }
}

pub fn paused(app: &App) -> bool {
    !crate::settings_api::nixa_on(&app.dir)
}

pub fn deep_first(app: &App, _task: &Task) -> bool {
    app.decision.deep_mode()
}

pub fn asked(app: &App, terminal: &str) -> bool {
    lock(&app.prompts.book)
        .items
        .iter()
        .any(|p| p.terminal == terminal)
}

pub fn deep_seat(app: &App, terminal: &str) -> bool {
    lock(&app.deep).owns(terminal)
}

pub fn brain_free(app: &App) -> bool {
    !crate::agent::busy(app)
}

pub async fn live() -> Option<HashSet<String>> {
    let list = crate::engine::list().await.ok()?;
    Some(
        list.into_iter()
            .filter(|t| t.running)
            .map(|t| t.id.to_string())
            .collect(),
    )
}

pub async fn screen(terminal: &str) -> Option<(String, Option<nixa_protocol::Status>)> {
    let id = crate::engine::term_id(terminal).ok()?;
    let info = crate::engine::get(&id).await.ok()?;
    let text = crate::engine::read(&id, Some(60), false).await.ok()?;
    Some((text, info.status))
}

pub async fn tick(app: &Arc<App>) {
    if paused(app) || app.queue.ticking.swap(true, Ordering::SeqCst) {
        return;
    }
    sweep::fix_unclaimed(app);
    sweep::clear_committed(app).await;
    waits::sweep(app).await;
    revisit_questions(app);
    tokio::join!(crate::prompts::auto::revisit(app), crate::workflows::revisit_waiting(app));
    sweep::free_claims(app).await;
    sweep::sweep_claims(app).await;
    crate::review_flow::recover(app).await;
    dispatch::tick(app, dispatch::queue_start).await;
    sweep::check_inbox(app);
    futures_util::future::join3(start::auto_start(app), start::start_handoffs(app), start::start_picked(app)).await;
    plan::tick(app).await;
    notes::sweep(app).await;
    marks::sync(app, true, None).await;
    app.queue.ticking.store(false, Ordering::SeqCst);
}

pub fn tick_soon(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move { tick(&app).await });
}

pub async fn run(app: Arc<App>) {
    tokio::time::sleep(Duration::from_millis(FIRST_TICK_MS)).await;
    let mut timer = tokio::time::interval(Duration::from_millis(QUEUE_MS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        tick(&app).await;
    }
}

pub fn on_change(app: &Arc<App>, id: i64, why: &str, role: &str) {
    let Ok(conn) = db(app) else { return };
    let Ok(Some(task)) = tasks::get_task(&conn, id) else {
        return;
    };
    drop(conn);
    match why {
        "status:review" if role != "user" => {
            if let Some(hook) = REVIEW.get() {
                tokio::spawn(hook(app.clone(), id, why.to_string()));
            }
        }
        "status:blocked" => blocked_on_user(app, &task),
        "added" if task.status == tasks::TaskStatus::Inbox => tick_soon(app),
        "status:todo" | "released" => tick_soon(app),
        "status:done" | "status:cancelled" => plan::soon(app),
        _ => {}
    }
}

pub async fn status_event(app: &Arc<App>, id: i64, why: &str, role: &str) -> bool {
    let stamp = db(app).ok().and_then(|conn| tasks::get_task(&conn, id).ok().flatten()).map(|task| format!("{why}:{}", task.updated_at));
    if let Some(stamp) = stamp
        && inner(app).raised.insert(id, stamp.clone()).as_deref() == Some(stamp.as_str())
    {
        return false;
    }
    on_change(app, id, why, role);
    if let Err(error) = crate::workflows::on_change(app, id, why, role).await {
        app.info(&format!("Task #{id}: {error}"));
    }
    changed(app, why, Some(id));
    marks::sync(app, false, Some(id)).await;
    true
}

fn revisit_questions(app: &Arc<App>) {
    if !crate::agent::has_brain(app) {
        return;
    }
    let Ok(conn) = db(app) else { return };
    let Ok(held) = tasks::held_tasks(&conn) else { return };
    let blocked: HashSet<_> = held.iter().filter(|task| task.status == tasks::TaskStatus::Blocked).map(|task| task.id).collect();
    inner(app).question_checks.retain(|id, _| blocked.contains(id));
    for task in held.iter().filter(|task| blocked.contains(&task.id)) {
        blocked_on_user(app, task);
    }
}

fn check_question(inner: &mut Inner, id: i64, key: String, now: u64) -> bool {
    if inner.question_checks.get(&id).is_some_and(|(previous, at)| previous == &key && now.saturating_sub(*at) < QUESTION_RETRY_MS) {
        return false;
    }
    inner.question_checks.insert(id, (key, now));
    true
}

pub fn blocked_on_user(app: &Arc<App>, task: &Task) {
    let Ok(conn) = db(app) else { return };
    let rows = nixa_store::spaces::list_spaces(&conn).unwrap_or_default();
    if task_locked(task, &rows).is_some() {
        return;
    }
    let comments = tasks::task_comments(&conn, task.id).unwrap_or_default();
    if !notes::waits_for_answer(task, &comments) || notes::run_active(&conn, task.id) {
        return;
    }
    let question_id = notes::question_at(task, &comments);
    let deep_seat = |terminal: &str| !terminal.is_empty() && deep_seat(app, terminal);
    if paused(app) || !crate::agent::has_brain(app)
        || !notes::answers(task, &comments, 0, &deep_seat).is_empty() {
        return;
    }
    let key = format!("{}:{}:{question_id}", task.id, task.claimed_at);
    if !check_question(&mut inner(app), task.id, key, crate::util::now_ms()) {
        return;
    }
    let question = comments
        .iter()
        .rev()
        .find(|c| c.id == question_id && c.kind == CommentKind::Question);
    let asked_text = question.map(|c| c.text.clone()).unwrap_or_else(|| "(no question text; read the task)".into());
    let terminal = Some(task.terminal_id.as_str()).filter(|t| !t.is_empty());
    crate::modes::note(app, "task", &format!("task #{}: Nixa answers the agent's question", task.id), terminal, Some(&prompt::cut(&asked_text, 300)));
    app.info(&format!("Task #{} in {} asks a question. Nixa answers it.", task.id, prompt::place(task)));
    let event = prompt::answer_event(task, &asked_text);
    let deep = app.decision.deep_mode()
        && crate::deep_service::resolve(app, Some(&task.space_id), None).ok().is_some_and(|space| {
            let handoff = format!("{}: {}", crate::followup::hard::BLOCKER_EVENT, event.trim_start_matches("Event: "));
            crate::followup::flow::relay_in_space(app, &handoff, Some(&space))
        });
    if !deep {
        crate::watch::tell_event(app, &event);
    }
}

pub fn waiting_lines(app: &App) -> Vec<String> {
    let Ok(conn) = db(app) else { return Vec::new() };
    let rows = nixa_store::spaces::list_spaces(&conn).unwrap_or_default();
    let mut out: Vec<String> =
        lock::unlocked(tasks::waiting_tasks(&conn).unwrap_or_default(), &rows)
            .iter()
            .map(|t| prompt::waiting_line(t, &tasks::task_comments(&conn, t.id).unwrap_or_default()))
            .collect();
    let mut groups: Vec<(String, Vec<Task>)> = Vec::new();
    for task in lock::unlocked(tasks::uncommitted_tasks(&conn).unwrap_or_default(), &rows) {
        let name = prompt::place(&task);
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, list)) => list.push(task),
            None => groups.push((name, vec![task])),
        }
    }
    for (name, list) in groups {
        let files: HashSet<&String> = list.iter().flat_map(|t| t.uncommitted.iter()).collect();
        let ids: Vec<String> = list.iter().map(|t| format!("#{}", t.id)).collect();
        let s = |n: usize| if n == 1 { "" } else { "s" };
        out.push(format!(
            "{name}: {} finished task{} not committed ({}), {} file{}",
            list.len(),
            s(list.len()),
            ids.join(", "),
            files.len(),
            s(files.len())
        ));
    }
    out
}

pub fn view(app: &App) -> Value {
    let inner = inner(app);
    let mut starting: Vec<&i64> = inner.starting.keys().collect();
    starting.sort();
    let queued: HashMap<String, &String> = inner
        .queued
        .iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let failed: HashMap<String, &u64> = inner
        .start_failed
        .iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    json!({"starting": starting, "queued": queued, "failed": failed, "paused": paused(app)})
}
