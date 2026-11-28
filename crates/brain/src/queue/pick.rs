use super::launch::{Seat, custom_of};
use super::start::{How, start_now};
use super::{changed, db, inner, log, note, paused, tick_soon};
use crate::app::{App, lock};
use crate::deep::{Ask, AskState, TIMEOUT_MS};
use crate::extras::model_pick::{self, ASK_KEY, Hold, PICK_KEY, Pick};
use crate::spaces::Space;
use crate::util::now_ms;
use nixa_store::Connection;
use nixa_store::tasks::{self, CommentKind, Task, TaskPatch, TaskStatus};
use serde_json::json;
use std::sync::Arc;

pub const START_KEY: &str = "model_pick.start:";
pub const CLOSED: &str = "the deep brain request closed with no pick";
pub const LATE: &str = "the deep brain gave no pick in time";

pub fn wanted(seat: &Seat) -> bool {
    matches!(seat, Seat::Program(_))
}

fn key(prefix: &str, id: i64) -> String {
    format!("{prefix}{id}")
}

pub fn late(ask: &Ask, now: u64) -> bool {
    matches!(ask.state, AskState::Waiting | AskState::Sent) && now.saturating_sub(ask.at) > ask.wait.unwrap_or(TIMEOUT_MS)
}

pub fn has_pick(conn: &Connection, id: i64) -> bool {
    tasks::task_meta(conn, &key(PICK_KEY, id)).ok().flatten().is_some_and(|v| !v.is_empty())
}

pub fn ask_of(conn: &Connection, id: i64) -> Option<String> {
    let wanted = id.to_string();
    let plain: String = ASK_KEY.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    tasks::task_meta_list(conn, &plain).ok()?.into_iter().find(|(k, v)| *v == wanted && k.starts_with(ASK_KEY)).and_then(|(k, _)| k.strip_prefix(ASK_KEY).map(str::to_string))
}

pub fn save_none(conn: &Connection, id: i64, ask: &str, why: &str) {
    let pick = Pick { none: true, ask: ask.to_string(), why: why.to_string(), ..Pick::default() };
    let _ = tasks::set_task_meta(conn, &key(PICK_KEY, id), &serde_json::to_string(&pick).unwrap_or_default());
}

pub fn usable(app: &App, task: &Task) -> Option<Pick> {
    model_pick::saved_pick(app, task.id).filter(|p| !p.none && p.assigned == task.assigned)
}

pub fn want(pick: &Pick) -> Option<crate::schedules::runner::Want> {
    if pick.model.is_empty() && pick.effort.is_empty() {
        return None;
    }
    if let Some((agent, model)) = pick.model.split_once("::")
        && (agent.is_empty() || model.is_empty())
    {
        return None;
    }
    Some(crate::schedules::runner::Want { model: pick.model.clone(), effort: pick.effort.clone() })
}

pub fn model_args(pick: &Pick) -> Vec<String> {
    let Some((agent, model)) = pick.model.split_once("::") else { return Vec::new() };
    crate::schedules::runner::model_args(agent, model, &pick.effort)
}

pub fn start_mark(how: How, by_user: bool) -> Option<&'static str> {
    match how {
        How::Now | How::Continue | How::HandOver => Some(if by_user { "user" } else { "nixa" }),
        How::Auto | How::HandOff => None,
    }
}

pub fn hold(app: &Arc<App>, task: &Task, space: &Space, seat: &Seat, how: How, by_user: bool) -> bool {
    if !wanted(seat) {
        return false;
    }
    if app.decision.nixa_decides() {
        log("model pick skipped", json!({"task": task.id, "why": "Nixa decides"}));
        return false;
    }
    if crate::deep_service::ensure_open(app, space).is_err() {
        return false;
    }
    let Ok(conn) = db(app) else { return false };
    if has_pick(&conn, task.id) {
        return false;
    }
    if let Some(ask) = ask_of(&conn, task.id) {
        let found = lock(&app.deep).asks.iter().find(|a| a.id == ask).map(|a| late(a, now_ms()));
        if found == Some(true) {
            let _ = tasks::remove_task_meta(&conn, &format!("{ASK_KEY}{ask}"));
            save_none(&conn, task.id, &ask, LATE);
            note(&conn, task.id, &format!("The deep brain {ask} gave no agent and model pick in time. Nixa starts the task with its own pick."), CommentKind::Note);
            log("model pick late", json!({"task": task.id, "ask": ask}));
            return false;
        }
        if found.is_some() {
            if let Some(mark) = start_mark(how, by_user) {
                let _ = tasks::set_task_meta(&conn, &key(START_KEY, task.id), mark);
            }
            return true;
        }
        let _ = tasks::remove_task_meta(&conn, &format!("{ASK_KEY}{ask}"));
        save_none(&conn, task.id, &ask, CLOSED);
        return false;
    }
    match model_pick::ask_for_task(app, task.id) {
        Ok(made) => {
            if let Some(mark) = start_mark(how, by_user) {
                let _ = tasks::set_task_meta(&conn, &key(START_KEY, task.id), mark);
            }
            note(&conn, task.id, &format!("Nixa asked the deep brain {made} to pick the agent and model for this task. It starts on that pick, or on Nixa's own pick if no answer comes in time."), CommentKind::Note);
            log("model asked", json!({"task": task.id, "ask": made}));
            changed(app, "queue", Some(task.id));
            true
        }
        Err(error) => {
            save_none(&conn, task.id, "", &error);
            note(&conn, task.id, &format!("Nixa uses its own agent and model pick: {error}"), CommentKind::Note);
            log("model ask failed", json!({"task": task.id, "error": crate::util::clip(&error, 200)}));
            false
        }
    }
}

pub fn waits(app: &App, conn: &Connection, id: i64) -> bool {
    let Some(ask) = ask_of(conn, id) else { return false };
    let now = now_ms();
    lock(&app.deep).asks.iter().any(|a| a.id == ask && matches!(a.state, AskState::Waiting | AskState::Sent) && !late(a, now))
}

fn open(task: &Task) -> bool {
    task.terminal_id.is_empty() && matches!(task.status, TaskStatus::Todo | TaskStatus::Inbox)
}

pub fn apply(app: &App, conn: &Connection, id: i64, pick: &Pick) -> Option<Task> {
    let task = tasks::get_task(conn, id).ok().flatten()?;
    if let Some(hold) = pick.hold {
        let _ = tasks::remove_task_meta(conn, &key(PICK_KEY, id));
        let _ = tasks::remove_task_meta(conn, &key(START_KEY, id));
        let task = match (open(&task), hold) {
            (true, Hold::Cancel) => tasks::set_status(conn, id, TaskStatus::Cancelled, None, false).ok().unwrap_or(task),
            (true, Hold::Done) => tasks::set_status(conn, id, TaskStatus::Done, None, false).ok().unwrap_or(task),
            (true, Hold::Wait) => {
                inner(app).start_failed.insert(id, now_ms());
                task
            }
            (false, _) => task,
        };
        note(conn, id, &model_pick::pick_line(pick), CommentKind::Note);
        return Some(task);
    }
    let mut task = task;
    if !pick.none && open(&task) && custom_of(&task).is_none() && (task.assigned != pick.assigned || task.assigned_kind != pick.assigned_kind) {
        let patch = TaskPatch { assigned_kind: Some(pick.assigned_kind.clone()), assigned: Some(pick.assigned.clone()), ..TaskPatch::default() };
        if let Ok(moved) = tasks::update_task(conn, id, &patch) {
            task = moved;
        }
    }
    note(conn, id, &model_pick::pick_line(pick), CommentKind::Note);
    Some(task)
}

pub fn picked(app: &Arc<App>, id: i64, pick: &Pick) {
    let Ok(conn) = db(app) else { return };
    if apply(app, &conn, id, pick).is_none() {
        return;
    }
    log("model picked", json!({"task": id, "ask": pick.ask, "none": pick.none, "assigned": pick.assigned, "model": pick.model}));
    changed(app, "queue", Some(id));
    drop(conn);
    if pick.hold.is_none() {
        tick_soon(app);
    }
}

pub fn marks(conn: &Connection) -> Vec<(i64, String)> {
    let plain: String = START_KEY.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    tasks::task_meta_list(conn, &plain)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(START_KEY)?.parse().ok()?, v)))
        .collect()
}

pub fn due_starts(app: &App, conn: &Connection) -> Vec<(i64, bool)> {
    let mut due = Vec::new();
    for (id, by) in marks(conn) {
        if !tasks::get_task(conn, id).ok().flatten().is_some_and(|t| open(&t)) {
            started(conn, id);
            continue;
        }
        if waits(app, conn, id) || inner(app).starting.contains_key(&id) {
            continue;
        }
        due.push((id, by == "user"));
    }
    due
}

pub async fn start_due(app: &Arc<App>, ready: impl Fn(i64) -> bool) {
    if paused(app) {
        return;
    }
    let due = match db(app) {
        Ok(conn) => due_starts(app, &conn),
        Err(_) => return,
    };
    let jobs = due.into_iter().filter(|(id, _)| ready(*id)).map(|(id, by_user)| async move {
        if let Err(error) = start_now(app, id, by_user).await {
            if let Ok(conn) = db(app) {
                started(&conn, id);
            }
            log("start after pick failed", json!({"task": id, "error": crate::util::clip(&error, 200)}));
        }
    });
    futures_util::future::join_all(jobs).await;
}

pub fn started(conn: &Connection, id: i64) {
    let _ = tasks::remove_task_meta(conn, &key(START_KEY, id));
}

pub fn failed_pick(conn: &Connection, id: i64, pick: &Pick, how: How, why: &str) {
    let gone = Pick { none: true, ask: pick.ask.clone(), why: format!("the start on {} failed", pick.label), ..Pick::default() };
    let _ = tasks::set_task_meta(conn, &key(PICK_KEY, id), &serde_json::to_string(&gone).unwrap_or_default());
    note(conn, id, &format!("{} on the deep brain's pick ({}) failed: {}. Nixa starts it with its own pick.", how.label(), pick.label, super::prompt::cut(why.trim_end_matches('.'), 300)), CommentKind::Error);
}

#[cfg(test)]
mod late_tests {
    use super::*;
    use crate::deep::AskInput;
    use crate::extras::model_pick::{ASK_KEY, read_pick};
    use nixa_store::spaces::SpacePatch;
    use nixa_store::tasks::TaskInput;

    fn setup() -> (tempfile::TempDir, Arc<App>, Space, Task) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path()).unwrap();
        let folder = dir.path().join("work").display().to_string();
        std::fs::create_dir_all(&folder).unwrap();
        let conn = db(&app).unwrap();
        let patch = SpacePatch { name: Some("nixa".into()), folder: Some(folder.clone()), ..SpacePatch::default() };
        nixa_store::spaces::set_space(&conn, "nixa", &patch).unwrap();
        let input = TaskInput {
            title: "Fix the cart".into(),
            status: Some(TaskStatus::Todo),
            space_id: "nixa".into(),
            space_name: "nixa".into(),
            assigned_kind: crate::tasks::KIND_TERMINAL.into(),
            assigned: "claude".into(),
            ..TaskInput::default()
        };
        let task = tasks::add_task(&conn, &input, &super::super::nixa()).unwrap();
        let space = Space { id: "nixa".into(), name: "nixa".into(), folder };
        (dir, app, space, task)
    }

    fn sent_ask(app: &App, space: &Space, task: &Task, at: u64) -> String {
        let input = AskInput { question: "Which row?".into(), kind: Some("model".into()), wait: Some(60_000), ..AskInput::default() };
        let made = lock(&app.deep).ask(input, space, true, true, at).unwrap().0;
        {
            let mut deep = lock(&app.deep);
            let ask = deep.asks.iter_mut().find(|a| a.id == made.id).unwrap();
            ask.state = AskState::Sent;
            ask.terminal = Some("m5".into());
            ask.sent_at = Some(at);
        }
        let conn = db(app).unwrap();
        tasks::set_task_meta(&conn, &format!("{ASK_KEY}{}", made.id), &task.id.to_string()).unwrap();
        tasks::set_task_meta(&conn, &format!("{START_KEY}{}", task.id), "user").unwrap();
        made.id
    }

    #[test]
    fn nixa_decides_starts_on_its_own_pick_without_the_deep_brain() {
        use nixa_store::agent_map::{AgentMapRow, add_row};
        let (_dir, app, space, task) = setup();
        let conn = db(&app).unwrap();
        add_row(&conn, &AgentMapRow { job: "Build".into(), agent: "claude".into(), model: "claude::sonnet".into(), ..AgentMapRow::default() }).unwrap();
        app.decision.set_choice("nixa").unwrap();
        assert!(!hold(&app, &task, &space, &Seat::Program("claude".into()), How::Now, true));
        assert!(!has_pick(&conn, task.id), "no pick is saved, so a later deep mode still asks");
        assert!(ask_of(&conn, task.id).is_none());
        assert_eq!(model_pick::ask_for_task(&app, task.id), Err(model_pick::NIXA_DECIDES.to_string()));
        assert!(lock(&app.deep).asks.is_empty());
        app.decision.set_choice("deep").unwrap();
        assert!(hold(&app, &task, &space, &Seat::Program("claude".into()), How::Now, true));
        assert_eq!(lock(&app.deep).asks.len(), 1);
    }

    #[test]
    fn a_sent_pick_inside_its_wait_still_holds_the_start() {
        let (_dir, app, space, task) = setup();
        sent_ask(&app, &space, &task, now_ms());
        let conn = db(&app).unwrap();
        assert!(waits(&app, &conn, task.id));
        assert!(due_starts(&app, &conn).is_empty());
        assert!(hold(&app, &task, &space, &Seat::Program("claude".into()), How::Now, true));
        assert!(!has_pick(&conn, task.id));
    }

    #[test]
    fn a_late_pick_on_a_busy_seat_starts_on_nixas_own_pick() {
        let (_dir, app, space, task) = setup();
        let ask = sent_ask(&app, &space, &task, now_ms() - 10 * 60_000);
        let conn = db(&app).unwrap();
        assert!(!waits(&app, &conn, task.id), "a late pick no longer holds the task or a doing slot");
        assert_eq!(due_starts(&app, &conn), vec![(task.id, true)]);
        assert!(!hold(&app, &task, &space, &Seat::Program("claude".into()), How::Now, true));
        let saved = tasks::task_meta(&conn, &format!("{}{}", crate::extras::model_pick::PICK_KEY, task.id)).unwrap().and_then(|raw| read_pick(&raw)).unwrap();
        assert!(saved.none);
        assert_eq!(saved.ask, ask);
        assert_eq!(saved.why, LATE);
        assert!(usable(&app, &task).is_none());
        assert!(ask_of(&conn, task.id).is_none(), "a late answer does not change the task");
        assert!(tasks::task_comments(&conn, task.id).unwrap().iter().any(|c| c.text.contains("gave no agent and model pick in time")));
        assert!(!hold(&app, &task, &space, &Seat::Program("claude".into()), How::Now, true));
        assert_eq!(lock(&app.deep).asks.iter().find(|a| a.id == ask).map(|a| a.state), Some(AskState::Sent), "the deep brain keeps working on it");
    }
}
