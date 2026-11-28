use super::{asked, db, deep_seat, inner, log, status_event};
use crate::app::{App, lock};
use nixa_protocol::Status;
use nixa_store::Connection;
use nixa_store::tasks::{self, Actor, AuthorKind, BlockedOn, CommentInput, CommentKind, Task, TaskComment, TaskFilter, TaskStatus};
use serde_json::json;
use std::sync::Arc;

pub const SENT_KEY: &str = "task_notes_sent:";
pub const NOTE_CHARS: usize = 1500;

pub fn claim_at(task: &Task, comments: &[TaskComment]) -> i64 {
    comments
        .iter()
        .filter(|c| c.kind == CommentKind::Claim)
        .map(|c| c.at)
        .fold(task.claimed_at, i64::max)
}

pub fn since_claim<'a>(task: &Task, comments: &'a [TaskComment], sent_up_to: i64) -> Vec<&'a TaskComment> {
    let from = claim_at(task, comments);
    comments
        .iter()
        .filter(|c| {
            c.id > sent_up_to
                && c.at >= from
                && c.kind == CommentKind::Note
                && matches!(c.author_kind, AuthorKind::User | AuthorKind::Nixa)
                && c.terminal_id != task.terminal_id
        })
        .collect()
}

pub fn message(task: &Task, notes: &[&TaskComment], assistant: &str) -> String {
    let mut lines = vec![format!(
        "New notes on your task #{}, added while you worked. They change or add to the job. Read them, do what they add, and say in your report how you covered them:",
        task.id
    )];
    for note in notes {
        let who = if note.author_kind == AuthorKind::User { "the user" } else { assistant };
        let text = nixa_store::text::clip(&nixa_store::text::one_line(&note.text), NOTE_CHARS);
        lines.push(format!("- From {who}: {text}"));
    }
    lines.push(format!("The full task: nixa task show {}", task.id));
    lines.join("\n")
}

pub fn waits_for_answer(task: &Task, comments: &[TaskComment]) -> bool {
    task.status == TaskStatus::Blocked && (task.needs_user || task.blocked_on == Some(BlockedOn::Decision)
        || (task.blocked_on == Some(BlockedOn::Other) && !task.terminal_id.is_empty()
            && comments.iter().rev().find(|c| c.terminal_id == task.terminal_id && c.at >= task.claimed_at)
                .is_some_and(|c| c.kind == CommentKind::Question)))
}

pub fn question_at(task: &Task, comments: &[TaskComment]) -> i64 {
    let from = claim_at(task, comments);
    let since: Vec<&TaskComment> = comments.iter().filter(|c| c.at >= from).collect();
    since
        .iter()
        .filter(|c| c.kind == CommentKind::Question)
        .map(|c| c.id)
        .max()
        .or_else(|| since.iter().filter(|c| !task.terminal_id.is_empty() && c.terminal_id == task.terminal_id).map(|c| c.id).max())
        .unwrap_or(0)
}

pub fn answers<'a>(task: &Task, comments: &'a [TaskComment], sent_up_to: i64, deep: &dyn Fn(&str) -> bool) -> Vec<&'a TaskComment> {
    let from = claim_at(task, comments);
    let after = sent_up_to.max(question_at(task, comments));
    comments
        .iter()
        .filter(|c| {
            let by_user = c.author_kind == AuthorKind::User && matches!(c.kind, CommentKind::Answer | CommentKind::Note);
            let by_deep = c.author_kind == AuthorKind::Agent && c.kind == CommentKind::Answer && deep(&c.terminal_id);
            let by_nixa = c.author_kind == AuthorKind::Nixa && c.kind == CommentKind::Answer;
            c.id > after && c.at >= from && c.terminal_id != task.terminal_id && (by_user || by_deep || by_nixa)
        })
        .collect()
}

pub fn answer_message(task: &Task, lines_of: &[&TaskComment], assistant: &str) -> String {
    let mut lines = vec![format!("Your task #{} waited for an answer. It has an answer now and is back in doing. Read it and go on with the job:", task.id)];
    for comment in lines_of {
        let who = match comment.author_kind {
            AuthorKind::User => "the user",
            AuthorKind::Agent => "the deep brain",
            _ => assistant,
        };
        let text = nixa_store::text::clip(&nixa_store::text::one_line(&comment.text), NOTE_CHARS);
        lines.push(format!("- From {who}: {text}"));
    }
    lines.push(format!("The full task: nixa task show {}", task.id));
    lines.join("\n")
}

pub fn ready(running: bool, status: Option<Status>, answer: bool) -> bool {
    running && (matches!(status, Some(Status::Idle) | Some(Status::Done)) || (answer && status == Some(Status::Blocked)))
}

pub fn free(app: &App, terminal: &str) -> bool {
    lock(&app.watch).world.get(terminal).is_some_and(|s| ready(s.info.running, s.info.status, false))
}

fn ready_for(app: &App, terminal: &str, answer: bool) -> bool {
    lock(&app.watch).world.get(terminal).is_some_and(|s| ready(s.info.running, s.info.status, answer))
}

pub(super) fn run_active(conn: &Connection, id: i64) -> bool {
    crate::workflows::driver::loaded(conn, id).ok().flatten().is_some_and(|(run, _)| run.state.is_active())
}

pub fn terminal_waits_for_answer(app: &App, terminal: &str) -> bool {
    let Ok(conn) = db(app) else { return false };
    let filter = TaskFilter { terminal: terminal.into(), status: vec![TaskStatus::Blocked], ..TaskFilter::default() };
    tasks::list_tasks(&conn, &filter).unwrap_or_default().iter().any(|task| {
        waits_for_answer(task, &tasks::task_comments(&conn, task.id).unwrap_or_default())
    })
}

fn sent_key(id: i64) -> String {
    format!("{SENT_KEY}{id}")
}

pub struct Due {
    pub task: i64,
    pub text: String,
    pub ids: Vec<i64>,
    pub answer: bool,
    pub question: i64,
}

fn due_of(conn: &Connection, task: &Task, assistant: &str, deep: &dyn Fn(&str) -> bool) -> Option<Due> {
    let sent_up_to = tasks::task_meta(conn, &sent_key(task.id))
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let comments = tasks::task_comments(conn, task.id).ok()?;
    let notes = since_claim(task, &comments, sent_up_to);
    if !waits_for_answer(task, &comments) {
        if task.status != TaskStatus::Doing {
            return None;
        }
        return (!notes.is_empty()).then(|| Due { task: task.id, text: message(task, &notes, assistant), ids: notes.iter().map(|c| c.id).collect(), answer: false, question: 0 });
    }
    if run_active(conn, task.id) {
        return None;
    }
    let replies = answers(task, &comments, sent_up_to, deep);
    if replies.is_empty() {
        return None;
    }
    let mut all: Vec<&TaskComment> = replies.into_iter().chain(notes).collect();
    all.sort_by_key(|c| c.id);
    all.dedup_by_key(|c| c.id);
    Some(Due { task: task.id, text: answer_message(task, &all, assistant), ids: all.iter().map(|c| c.id).collect(), answer: true, question: question_at(task, &comments) })
}

pub fn pending(conn: &Connection, terminal: &str, assistant: &str, deep: &dyn Fn(&str) -> bool, notes_ok: bool) -> Option<Due> {
    let filter = TaskFilter {
        terminal: terminal.to_string(),
        status: vec![TaskStatus::Doing, TaskStatus::Blocked],
        ..TaskFilter::default()
    };
    tasks::list_tasks(conn, &filter)
        .ok()?
        .iter()
        .filter_map(|t| due_of(conn, t, assistant, deep))
        .find(|d| d.answer || notes_ok)
}

pub fn mark_sent(conn: &Connection, due: &Due) -> Result<(), String> {
    let before = tasks::task_meta(conn, &sent_key(due.task)).ok().flatten().and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);
    let top = due.ids.iter().copied().max().unwrap_or(0).max(before);
    tasks::set_task_meta(conn, &sent_key(due.task), &top.to_string()).map_err(|e| e.to_string())
}

pub fn settle_answer(conn: &Connection, due: &Due, terminal: &str) -> Result<bool, String> {
    let fail = |e: anyhow::Error| e.to_string();
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    let done = (|| -> Result<bool, String> {
        let task = tasks::must_task(conn, due.task).map_err(fail)?;
        if task.terminal_id != terminal {
            return Ok(false);
        }
        mark_sent(conn, due)?;
        let comments = tasks::task_comments(conn, due.task).map_err(fail)?;
        if !waits_for_answer(&task, &comments) || question_at(&task, &comments) != due.question {
            return Ok(false);
        }
        tasks::set_status(conn, due.task, TaskStatus::Doing, None, false).map_err(fail)?;
        let line = format!("Nixa sent the answer to terminal {terminal}. The task is back in doing.");
        tasks::add_comment(conn, due.task, &CommentInput::new(CommentKind::Progress, &line), &Actor::new(AuthorKind::Nixa, "Nixa")).map_err(fail)?;
        Ok(true)
    })();
    let end = if done.is_ok() { "COMMIT" } else { "ROLLBACK" };
    conn.execute_batch(end).map_err(|e| e.to_string())?;
    done
}

fn due(app: &App, terminal: &str) -> Option<Due> {
    let conn = db(app).ok()?;
    let deep = |t: &str| !t.is_empty() && lock(&app.deep).owns(t);
    pending(&conn, terminal, &app.name(), &deep, free(app, terminal))
}

pub async fn deliver(app: &Arc<App>, terminal: &str) {
    if terminal.is_empty() || !ready_for(app, terminal, true) || deep_seat(app, terminal) || asked(app, terminal) {
        return;
    }
    if !inner(app).noting.insert(terminal.to_string()) {
        return;
    }
    send(app, terminal).await;
    inner(app).noting.remove(terminal);
}

async fn send(app: &Arc<App>, terminal: &str) {
    let Some(found) = due(app, terminal) else { return };
    let Ok(id) = crate::engine::term_id(terminal) else { return };
    if let Err(error) = crate::engine::send(&id, &found.text, true, false).await {
        log(if found.answer { "answer not sent" } else { "note not sent" }, json!({"task": found.task, "terminal": terminal, "error": error.to_string()}));
        return;
    }
    let Ok(conn) = db(app) else { return };
    if !found.answer {
        if let Err(error) = mark_sent(&conn, &found) {
            log("notes mark failed", json!({"task": found.task, "error": error}));
        }
        log("notes sent", json!({"task": found.task, "terminal": terminal, "notes": found.ids}));
        return;
    }
    let moved = settle_answer(&conn, &found, terminal);
    drop(conn);
    log("answer sent", json!({"task": found.task, "terminal": terminal, "comments": found.ids, "doing": moved.as_ref().ok()}));
    match moved {
        Ok(true) => {
            status_event(app, found.task, "status:doing", "nixa").await;
        }
        Ok(false) => {}
        Err(error) => log("answer status failed", json!({"task": found.task, "error": error})),
    }
}

pub fn soon(app: &Arc<App>, id: i64) {
    let terminal = db(app)
        .ok()
        .and_then(|conn| tasks::get_task(&conn, id).ok().flatten())
        .filter(|t| matches!(t.status, TaskStatus::Doing | TaskStatus::Blocked))
        .map(|t| t.terminal_id)
        .filter(|t| !t.is_empty());
    let Some(terminal) = terminal else { return };
    let app = app.clone();
    tokio::spawn(async move { deliver(&app, &terminal).await });
}

pub async fn sweep(app: &Arc<App>) {
    let held = db(app)
        .ok()
        .and_then(|conn| tasks::held_tasks(&conn).ok())
        .unwrap_or_default();
    for task in held.into_iter().filter(|t| matches!(t.status, TaskStatus::Doing | TaskStatus::Blocked)) {
        deliver(app, &task.terminal_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixa_store::tasks::{Actor, CommentInput, TaskInput};

    fn setup() -> (nixa_store::Connection, Task) {
        let conn = nixa_store::open_memory().unwrap();
        let input = TaskInput { title: "Fix the chat".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let task = tasks::add_task(&conn, &input, &Actor::new(AuthorKind::User, "owner")).unwrap();
        (conn, task)
    }

    fn comment(conn: &nixa_store::Connection, id: i64, kind: CommentKind, text: &str, by: &Actor) {
        tasks::add_comment(conn, id, &CommentInput::new(kind, text), by).unwrap();
    }

    #[test]
    fn only_new_notes_from_the_user_or_nixa_after_the_claim_go_out() {
        let (conn, task) = setup();
        let user = Actor::new(AuthorKind::User, "owner");
        let nixa = Actor::new(AuthorKind::Nixa, "Nixa");
        comment(&conn, task.id, CommentKind::Note, "old note before the claim", &user);
        let worker = Actor::in_terminal(AuthorKind::Agent, "worker", "m4");
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(tasks::claim_task(&conn, task.id, &worker, "").unwrap().ok);
        std::thread::sleep(std::time::Duration::from_millis(5));
        comment(&conn, task.id, CommentKind::Note, "also check the dark theme", &user);
        comment(&conn, task.id, CommentKind::Progress, "half done", &worker);
        comment(&conn, task.id, CommentKind::Note, "my own note", &worker);
        comment(&conn, task.id, CommentKind::Question, "which theme?", &user);
        comment(&conn, task.id, CommentKind::Note, "keep the old colours", &nixa);
        comment(&conn, task.id, CommentKind::Note, "system line", &Actor::new(AuthorKind::System, "Nixa"));
        let task = tasks::get_task(&conn, task.id).unwrap().unwrap();
        let comments = tasks::task_comments(&conn, task.id).unwrap();
        let notes = since_claim(&task, &comments, 0);
        let texts: Vec<&str> = notes.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["also check the dark theme", "keep the old colours"]);
        let first = notes[0].id;
        let later = since_claim(&task, &comments, first);
        assert_eq!(later.len(), 1);
        assert!(since_claim(&task, &comments, later[0].id).is_empty());
    }

    fn blocked_task(conn: &nixa_store::Connection, terminal: &str) -> Task {
        let input = TaskInput { title: "Fix the login".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let task = tasks::add_task(conn, &input, &Actor::new(AuthorKind::User, "owner")).unwrap();
        let worker = Actor::in_terminal(AuthorKind::Agent, "worker", terminal);
        assert!(tasks::claim_task(conn, task.id, &worker, "").unwrap().ok);
        std::thread::sleep(std::time::Duration::from_millis(5));
        tasks::set_status(conn, task.id, TaskStatus::Blocked, Some(tasks::BlockedOn::User), false).unwrap();
        comment(conn, task.id, CommentKind::Question, "Which login page?", &worker);
        tasks::get_task(conn, task.id).unwrap().unwrap()
    }

    #[test]
    fn an_answer_to_a_blocked_task_goes_out_once_and_moves_it_back_to_doing() {
        let conn = nixa_store::open_memory().unwrap();
        let task = blocked_task(&conn, "m4");
        assert!(waits_for_answer(&task, &[]));
        let no_deep = |_: &str| false;
        assert!(pending(&conn, "m4", "Nixa", &no_deep, true).is_none(), "no answer yet");
        comment(&conn, task.id, CommentKind::Note, "Nixa line", &Actor::new(AuthorKind::Nixa, "Nixa"));
        assert!(pending(&conn, "m4", "Nixa", &no_deep, true).is_none(), "a Nixa note is not an answer");
        comment(&conn, task.id, CommentKind::Answer, "the admin login", &Actor::new(AuthorKind::User, "owner"));
        let due = pending(&conn, "m4", "Nixa", &no_deep, true).expect("the answer is due");
        assert!(due.answer && due.ids.len() == 2, "the answer and the waiting Nixa note go out together");
        assert!(due.text.contains("- From the user: the admin login") && due.text.contains("- From Nixa: Nixa line") && due.text.contains("back in doing"), "{}", due.text);
        assert!(settle_answer(&conn, &due, "m4").unwrap());
        let now = tasks::get_task(&conn, task.id).unwrap().unwrap();
        assert_eq!(now.status, TaskStatus::Doing);
        assert!(!now.needs_user && now.terminal_id == "m4");
        assert!(pending(&conn, "m4", "Nixa", &no_deep, true).is_none(), "nothing is sent again");
        assert!(!settle_answer(&conn, &due, "m4").unwrap(), "a task in doing is not moved again");
    }

    #[test]
    fn automatic_and_existing_question_blocks_resume_after_nixa_answers() {
        let conn = nixa_store::open_memory().unwrap();
        for (terminal, on) in [("m4", BlockedOn::Decision), ("m5", BlockedOn::Other)] {
            let task = blocked_task(&conn, terminal);
            let task = tasks::set_status(&conn, task.id, TaskStatus::Blocked, Some(on), false).unwrap();
            let worker = Actor::in_terminal(AuthorKind::Agent, "worker", terminal);
            comment(&conn, task.id, CommentKind::Question, "Which theme?", &worker);
            assert!(!task.needs_user);
            assert!(pending(&conn, terminal, "Nixa", &|_| false, true).is_none());
            comment(&conn, task.id, CommentKind::Answer, "Use the existing theme.", &Actor::new(AuthorKind::Nixa, "Nixa"));
            let due = pending(&conn, terminal, "Nixa", &|_| false, false).expect("the answer is due while the agent is blocked");
            assert!(due.answer && due.text.contains("Use the existing theme."));
            assert!(settle_answer(&conn, &due, terminal).unwrap());
            assert_eq!(tasks::must_task(&conn, task.id).unwrap().status, TaskStatus::Doing);
            assert!(pending(&conn, terminal, "Nixa", &|_| false, true).is_none());
        }
    }

    #[test]
    fn a_stale_answer_does_not_clear_a_newer_question_and_notes_wait_for_an_idle_agent() {
        let conn = nixa_store::open_memory().unwrap();
        let task = blocked_task(&conn, "m4");
        comment(&conn, task.id, CommentKind::Answer, "the admin login", &Actor::new(AuthorKind::User, "owner"));
        let due = pending(&conn, "m4", "Nixa", &|_| false, false).expect("an answer goes to a blocked agent");
        comment(&conn, task.id, CommentKind::Question, "And which theme?", &Actor::in_terminal(AuthorKind::Agent, "worker", "m4"));
        assert!(!settle_answer(&conn, &due, "m4").unwrap(), "a newer question keeps the task waiting");
        assert!(waits_for_answer(&tasks::get_task(&conn, task.id).unwrap().unwrap(), &[]));
        assert!(pending(&conn, "m4", "Nixa", &|_| false, false).is_none(), "the sent answer is not sent again");
        let other = TaskInput { title: "Doing work".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let other = tasks::add_task(&conn, &other, &Actor::new(AuthorKind::User, "owner")).unwrap();
        assert!(tasks::claim_task(&conn, other.id, &Actor::in_terminal(AuthorKind::Agent, "w", "m6"), "").unwrap().ok);
        std::thread::sleep(std::time::Duration::from_millis(5));
        comment(&conn, other.id, CommentKind::Note, "add tests", &Actor::new(AuthorKind::User, "owner"));
        assert!(pending(&conn, "m6", "Nixa", &|_| false, false).is_none(), "a note waits until the agent is free");
        assert!(pending(&conn, "m6", "Nixa", &|_| false, true).is_some());
    }

    #[test]
    fn without_a_question_the_agents_last_line_marks_the_wait() {
        let conn = nixa_store::open_memory().unwrap();
        let input = TaskInput { title: "Fix the login".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let task = tasks::add_task(&conn, &input, &Actor::new(AuthorKind::User, "owner")).unwrap();
        let worker = Actor::in_terminal(AuthorKind::Agent, "worker", "m4");
        assert!(tasks::claim_task(&conn, task.id, &worker, "").unwrap().ok);
        std::thread::sleep(std::time::Duration::from_millis(5));
        comment(&conn, task.id, CommentKind::Note, "old note", &Actor::new(AuthorKind::User, "owner"));
        comment(&conn, task.id, CommentKind::Progress, "stuck on the page name", &worker);
        tasks::set_status(&conn, task.id, TaskStatus::Blocked, Some(tasks::BlockedOn::User), false).unwrap();
        assert!(pending(&conn, "m4", "Nixa", &|_| false, false).is_none());
        comment(&conn, task.id, CommentKind::Answer, "the admin page", &Actor::new(AuthorKind::User, "owner"));
        assert!(pending(&conn, "m4", "Nixa", &|_| false, false).is_some());
    }

    #[test]
    fn a_note_from_before_the_question_is_not_an_answer() {
        let conn = nixa_store::open_memory().unwrap();
        let input = TaskInput { title: "Fix the login".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let task = tasks::add_task(&conn, &input, &Actor::new(AuthorKind::User, "owner")).unwrap();
        let worker = Actor::in_terminal(AuthorKind::Agent, "worker", "m4");
        assert!(tasks::claim_task(&conn, task.id, &worker, "").unwrap().ok);
        std::thread::sleep(std::time::Duration::from_millis(5));
        comment(&conn, task.id, CommentKind::Note, "use the dark theme", &Actor::new(AuthorKind::User, "owner"));
        tasks::set_status(&conn, task.id, TaskStatus::Blocked, Some(tasks::BlockedOn::User), false).unwrap();
        comment(&conn, task.id, CommentKind::Question, "Which page?", &Actor::new(AuthorKind::Nixa, "Nixa"));
        assert!(pending(&conn, "m4", "Nixa", &|_| false, true).is_none(), "the older note does not resume the task");
        comment(&conn, task.id, CommentKind::Note, "the admin page", &Actor::new(AuthorKind::User, "owner"));
        let due = pending(&conn, "m4", "Nixa", &|_| false, true).unwrap();
        assert_eq!(due.ids.len(), 2, "the answer goes out with the note the agent has not seen yet");
    }

    #[test]
    fn the_deep_brain_may_answer_and_other_agents_may_not() {
        let conn = nixa_store::open_memory().unwrap();
        let task = blocked_task(&conn, "m4");
        comment(&conn, task.id, CommentKind::Answer, "from a helper", &Actor::in_terminal(AuthorKind::Agent, "helper", "m7"));
        comment(&conn, task.id, CommentKind::Answer, "use the admin page", &Actor::in_terminal(AuthorKind::Agent, "deep", "m9"));
        let deep = |t: &str| t == "m9";
        let due = pending(&conn, "m4", "Nixa", &deep, true).unwrap();
        assert_eq!(due.ids.len(), 1);
        assert!(due.text.contains("- From the deep brain: use the admin page"), "{}", due.text);
    }

    #[test]
    fn nixa_may_answer_a_question() {
        let conn = nixa_store::open_memory().unwrap();
        let task = blocked_task(&conn, "m4");
        comment(&conn, task.id, CommentKind::Answer, "use the admin page", &Actor::new(AuthorKind::Nixa, "Nixa"));
        let due = pending(&conn, "m4", "Nixa", &|_| false, true).unwrap();
        assert!(due.answer && due.text.contains("- From Nixa: use the admin page"), "{}", due.text);
    }

    #[test]
    fn a_closed_or_busy_terminal_gets_no_answer_and_the_task_stays_blocked() {
        assert!(!ready(false, Some(Status::Idle), true), "a closed terminal");
        assert!(!ready(true, Some(Status::Working), true), "a working agent");
        assert!(ready(true, Some(Status::Blocked), true) && ready(true, Some(Status::Idle), true));
        assert!(!ready(true, Some(Status::Blocked), false), "notes still wait for an idle agent");
        let conn = nixa_store::open_memory().unwrap();
        let task = blocked_task(&conn, "m4");
        comment(&conn, task.id, CommentKind::Answer, "admin", &Actor::new(AuthorKind::User, "owner"));
        let due = pending(&conn, "m4", "Nixa", &|_| false, true).unwrap();
        assert!(!settle_answer(&conn, &due, "m8").unwrap(), "another terminal does not move it");
        let other = TaskInput { title: "Waits on a task".into(), status: Some(TaskStatus::Todo), ..TaskInput::default() };
        let other = tasks::add_task(&conn, &other, &Actor::new(AuthorKind::User, "owner")).unwrap();
        assert!(tasks::claim_task(&conn, other.id, &Actor::in_terminal(AuthorKind::Agent, "w", "m5"), "").unwrap().ok);
        tasks::set_status(&conn, other.id, TaskStatus::Blocked, Some(tasks::BlockedOn::Task), false).unwrap();
        comment(&conn, other.id, CommentKind::Answer, "done", &Actor::new(AuthorKind::User, "owner"));
        assert!(pending(&conn, "m5", "Nixa", &|_| false, true).is_none(), "only tasks blocked on the user get answers");
        assert_eq!(tasks::get_task(&conn, task.id).unwrap().unwrap().status, TaskStatus::Blocked);
    }

    #[test]
    fn the_message_names_who_wrote_each_note() {
        let (conn, task) = setup();
        comment(&conn, task.id, CommentKind::Note, "use   the\nnew API", &Actor::new(AuthorKind::User, "owner"));
        comment(&conn, task.id, CommentKind::Note, "tests too", &Actor::new(AuthorKind::Nixa, "Nixa"));
        let comments = tasks::task_comments(&conn, task.id).unwrap();
        let notes: Vec<&TaskComment> = comments.iter().collect();
        let text = message(&task, &notes, "Nixa");
        assert!(text.starts_with(&format!("New notes on your task #{}", task.id)), "{text}");
        assert!(text.contains("- From the user: use the new API"), "{text}");
        assert!(text.contains("- From Nixa: tests too"), "{text}");
        assert!(text.ends_with(&format!("The full task: nixa task show {}", task.id)), "{text}");
    }
}
