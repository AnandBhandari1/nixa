use crate::agent;
use crate::app::{App, lock};
use crate::decide::{self, Answer, Answers};
use crate::queue::lock::task_locked;
use crate::util::log;
use nixa_protocol::{Status, TermInfo};
use std::collections::BTreeSet;
use nixa_store::spaces::list_spaces;
use nixa_store::tasks::{TaskFilter, TaskStatus, list_tasks};
use std::sync::Arc;

pub const IDLE_MS: u64 = 10 * 60_000;
pub const SUGGEST_MS: u64 = 60 * 60_000;
const MAX_SUGGEST: usize = 3;
const MAX_CLOSE_CHECKS: usize = 6;
pub const ACTION: &str = "action";
pub const CLOSE: &str = "close";
pub const KEEP: &str = "keep";
pub const FINISHED: &str = "finished";

#[derive(Default)]
pub struct Idle {
    quiet_since: Option<u64>,
    last_nudge: Option<u64>,
    last_suggest: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Open {
    pub inbox: usize,
    pub todo: usize,
    pub doing: usize,
    pub review: usize,
    pub blocked: usize,
    pub blocked_terminals: usize,
    pub asks: usize,
    pub waiting_user: usize,
    pub finished: Vec<String>,
}

impl Open {
    pub fn total(&self) -> usize {
        self.inbox + self.todo + self.doing + self.review + self.blocked + self.blocked_terminals + self.asks + self.finished.len()
    }
}

pub fn due(idle: &mut Idle, busy: bool, now: u64) -> bool {
    if busy {
        idle.quiet_since = None;
        return false;
    }
    let since = *idle.quiet_since.get_or_insert(now);
    let quiet_long = now.saturating_sub(since) >= IDLE_MS;
    let nudged_long_ago = idle.last_nudge.is_none_or(|at| now.saturating_sub(at) >= IDLE_MS);
    quiet_long && nudged_long_ago
}

fn part(n: usize, one: &str, many: &str) -> Option<String> {
    match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        _ => Some(format!("{n} {many}")),
    }
}

fn close_text(open: &Open) -> Option<String> {
    (!open.finished.is_empty()).then(|| {
        format!(
            "These agent terminals finished and hold no open task: {}. Close each one with nixa_term_remove. Keep a terminal open only when its last output shows work that is not done.",
            open.finished.join(", ")
        )
    })
}

pub fn suggest_due(idle: &Idle, open: &Open, now: u64) -> bool {
    open.total() == open.finished.len() && open.waiting_user == 0 && idle.last_suggest.is_none_or(|at| now.saturating_sub(at) >= SUGGEST_MS)
}

fn suggest_text(idle_min: u64) -> String {
    format!(
        "Event: Nixa was idle for {idle_min} minutes and no task is open. Find new work, but ask the user first. For each unlocked space, read docs/PRD.md (Roadmap and open items, Known gaps) and docs/lessons.md with your tools, and check the old tasks (nixa_task list with old true) so you do not repeat one. Suggest at most {MAX_SUGGEST} useful tasks: add each with nixa_task add, status inbox and needs_user true, with a clear title and a short description of what done means. Do not assign or start them. Then say the suggestions in one short chat line and ask the user to approve. When they say yes, set the task to todo."
    )
}

pub fn nudge_text(open: &Open, idle_min: u64, suggest: bool) -> Option<String> {
    let work = open.total() - open.finished.len();
    if work == 0 {
        let close = close_text(open);
        return match (suggest, close) {
            (true, Some(close)) => Some(format!("{} {close}", suggest_text(idle_min))),
            (true, None) => Some(suggest_text(idle_min)),
            (false, Some(close)) => Some(format!("Event: Nixa was idle for {idle_min} minutes. {close} Then report in one short line.")),
            (false, None) => None,
        };
    }
    let parts: Vec<String> = [
        part(open.review, "task in review", "tasks in review"),
        part(open.blocked, "blocked task", "blocked tasks"),
        part(open.doing, "task in doing", "tasks in doing"),
        part(open.todo, "task in todo", "tasks in todo"),
        part(open.inbox, "task in the inbox", "tasks in the inbox"),
        part(open.blocked_terminals, "blocked terminal", "blocked terminals"),
        part(open.asks, "waiting deep brain request", "waiting deep brain requests"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut text = format!(
        "Event: Nixa was idle for {idle_min} minutes while work waits: {}. Do not stay idle. Check them now with your tools or nixa_cli (task ls, term ls, deep list). Move the oldest item forward yourself: close a review task that has proof, send a fix or give it to another agent, unblock what you can, answer what waits. Give a big or unclear item to the space's deep brain with nixa_deep_ask. Take one or two steps, then report in one short line.",
        parts.join(", ")
    );
    if let Some(close) = close_text(open) {
        text = format!("{text} {close}");
    }
    Some(text)
}

pub fn finished_agents(terms: &[TermInfo], held: &BTreeSet<String>) -> Vec<String> {
    terms
        .iter()
        .filter(|t| t.running && t.status == Some(Status::Done) && t.parent.is_some() && !t.keep && !t.draft)
        .filter(|t| t.labels.get(crate::deep::ROLE_LABEL).is_none_or(|role| role != crate::deep::ROLE_VALUE))
        .filter(|t| !held.contains(t.id.as_str()))
        .map(|t| t.id.as_str().to_string())
        .collect()
}

fn count_tasks(conn: &nixa_store::Connection) -> (Open, BTreeSet<String>) {
    let mut open = Open::default();
    let mut held = BTreeSet::new();
    let filter = TaskFilter { status: vec![TaskStatus::Inbox, TaskStatus::Todo, TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review], ..TaskFilter::default() };
    let rows = list_spaces(conn).ok();
    for task in list_tasks(conn, &filter).unwrap_or_default() {
        if !task.terminal_id.is_empty() {
            held.insert(task.terminal_id.clone());
        }
        let Some(rows) = rows.as_deref() else { continue };
        if task_locked(&task, rows).is_some() {
            continue;
        }
        if task.needs_user {
            open.waiting_user += 1;
            continue;
        }
        match task.status {
            TaskStatus::Inbox => open.inbox += 1,
            TaskStatus::Todo => open.todo += 1,
            TaskStatus::Doing => open.doing += 1,
            TaskStatus::Blocked => open.blocked += 1,
            TaskStatus::Review => open.review += 1,
            _ => {}
        }
    }
    (open, held)
}

async fn open_work(app: &Arc<App>) -> Open {
    let (mut open, held) = nixa_store::open(&app.db_file).map(|conn| count_tasks(&conn)).unwrap_or_default();
    if let Ok(terms) = crate::engine::list().await {
        open.blocked_terminals = terms.iter().filter(|t| t.running && t.status == Some(Status::Blocked)).count();
        open.finished = finished_agents(&terms, &held);
    }
    open.asks = lock(&app.deep).open_asks().len();
    open
}

pub fn close_questions() -> Vec<serde_json::Value> {
    vec![
        decide::choice(ACTION, "Should Nixa close this agent terminal now? close = its last output shows the job is finished and nothing waits. keep = work is not done, or an error, a question or a request waits.", &[(CLOSE, ""), (KEEP, "")]),
        decide::predicate(FINISHED, "The last output shows the agent finished its job and nothing is left for it to do."),
    ]
}

pub fn close_input(id: &str, name: &str, lines: &str) -> String {
    let shown = if lines.trim().is_empty() { "(nothing on screen)".to_string() } else { crate::agent::data_block(lines) };
    format!("Agent terminal {id} ({name}) says it is done and holds no open task.\nLast lines:\n{shown}")
}

pub fn sure_close(answers: &Answers, level: f64) -> Result<(), String> {
    let finished = match answers.get(FINISHED) {
        Some(Answer::Predicate { probability, .. }) => *probability,
        _ => 0.0,
    };
    let (action, confidence) = match answers.get(ACTION) {
        Some(Answer::Choice { choice, confidence, .. }) => (choice.as_str().unwrap_or_default().to_string(), *confidence),
        _ => return Err("it gave no action".into()),
    };
    let note = format!("action {action} ({confidence:.2}), finished {finished:.2}");
    if action != CLOSE {
        return Err(format!("it says {note}"));
    }
    if confidence < level || finished < level {
        return Err(format!("it was not sure enough: {note}, level {level:.2}"));
    }
    Ok(())
}

async fn close_one(app: &Arc<App>, id: &str, set: &decide::settings::Settings) -> Result<String, String> {
    let term = crate::engine::term_id(id).map_err(|e| format!("{e:#}"))?;
    let info = crate::engine::get(&term).await.map_err(|e| format!("{e:#}"))?;
    if !info.running || info.status != Some(Status::Done) || info.keep {
        return Err("it changed before the check".into());
    }
    let screen = crate::engine::read(&term, Some(crate::kid_watch::READ_LINES), false).await.map_err(|e| format!("{e:#}"))?;
    let name = crate::util::clip_flat(&info.name, 40);
    let input = close_input(id, &name, &crate::kid_watch::last_lines(&screen));
    let answers = decide::ask(app, &set.model, vec![decide::text(&input)], close_questions()).await?;
    sure_close(&answers, set.level)?;
    crate::engine::remove(&term).await.map_err(|e| format!("{e:#}"))?;
    Ok(answers.answers.iter().map(Answer::short).collect::<Vec<_>>().join(", "))
}

async fn close_by_model(app: &Arc<App>, finished: Vec<String>) -> Vec<String> {
    let set = decide::settings::load(app);
    if !set.uses("idle") || finished.is_empty() {
        return finished;
    }
    let mut kept = Vec::new();
    for (i, id) in finished.into_iter().enumerate() {
        if i >= MAX_CLOSE_CHECKS {
            kept.push(id);
            continue;
        }
        match close_one(app, &id, &set).await {
            Ok(note) => {
                log("idle", &format!("the decision model closed finished terminal {id}: {note}"));
                app.info(&format!("The decision model {} closed finished terminal {id} ({note}).", set.model));
            }
            Err(why) => {
                log("idle", &format!("finished terminal {id} goes to the chat brain: {why}"));
                kept.push(id);
            }
        }
    }
    kept
}

pub async fn tick(app: &Arc<App>, idle: &mut Idle, now: u64) {
    if !crate::settings_api::nixa_on(&app.dir) || !agent::has_brain(app) {
        idle.quiet_since = None;
        return;
    }
    if !due(idle, agent::busy(app), now) {
        return;
    }
    let mut open = open_work(app).await;
    open.finished = close_by_model(app, std::mem::take(&mut open.finished)).await;
    let suggest = suggest_due(idle, &open, now);
    let Some(text) = nudge_text(&open, IDLE_MS / 60_000, suggest) else {
        return;
    };
    if suggest {
        idle.last_suggest = Some(now);
    }
    idle.last_nudge = Some(now);
    idle.quiet_since = None;
    log("idle", &format!("woke Nixa: {} open items, {} finished agents, suggest {suggest}", open.total() - open.finished.len(), open.finished.len()));
    crate::watch::tell_event(app, &text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakes_only_after_ten_quiet_minutes_and_not_twice_in_a_row() {
        let mut idle = Idle::default();
        assert!(!due(&mut idle, false, 0));
        assert!(!due(&mut idle, false, IDLE_MS - 1));
        assert!(due(&mut idle, false, IDLE_MS));
        idle.last_nudge = Some(IDLE_MS);
        idle.quiet_since = None;
        assert!(!due(&mut idle, false, IDLE_MS + 1));
        assert!(!due(&mut idle, false, 2 * IDLE_MS));
        assert!(due(&mut idle, false, 2 * IDLE_MS + 1));
        assert!(!due(&mut idle, true, 5 * IDLE_MS));
        assert!(!due(&mut idle, false, 5 * IDLE_MS + 1));
    }

    #[test]
    fn the_wake_text_lists_the_open_work() {
        assert_eq!(nudge_text(&Open::default(), 10, false), None);
        let text = nudge_text(&Open { review: 29, blocked: 1, ..Open::default() }, 10, false).unwrap();
        assert!(text.contains("29 tasks in review, 1 blocked task"), "{text}");
        assert!(text.contains("nixa_deep_ask") && text.contains("Do not stay idle"));
    }

    #[test]
    fn a_locked_space_task_is_not_counted_as_open_work() {
        use nixa_store::spaces::{SpacePatch, set_space};
        use nixa_store::tasks::{Actor, AuthorKind, TaskInput, add_task};
        let conn = nixa_store::open_memory().unwrap();
        set_space(&conn, "ace", &SpacePatch { name: Some("ACE".into()), locked: Some(true), ..SpacePatch::default() }).unwrap();
        set_space(&conn, "shop", &SpacePatch { name: Some("Shop".into()), ..SpacePatch::default() }).unwrap();
        let by = Actor::new(AuthorKind::User, "test");
        let input = |title: &str, space: &str, status: TaskStatus| TaskInput { title: title.into(), space_id: space.into(), space_name: space.into(), status: Some(status), ..TaskInput::default() };
        add_task(&conn, &input("locked todo", "ace", TaskStatus::Todo), &by).unwrap();
        add_task(&conn, &input("locked inbox", "ace", TaskStatus::Inbox), &by).unwrap();
        add_task(&conn, &input("open todo", "shop", TaskStatus::Todo), &by).unwrap();
        let (open, _) = count_tasks(&conn);
        assert_eq!(open.todo, 1);
        assert_eq!(open.inbox, 0);
        assert_eq!(open.total(), 1);
    }

    #[test]
    fn an_empty_list_asks_for_suggestions_once_an_hour() {
        let mut idle = Idle::default();
        let empty = Open::default();
        assert!(suggest_due(&idle, &empty, 0));
        let text = nudge_text(&empty, 10, true).unwrap();
        assert!(text.contains("needs_user true") && text.contains("ask the user first") && text.contains("ask the user to approve"), "{text}");
        idle.last_suggest = Some(0);
        assert!(!suggest_due(&idle, &empty, SUGGEST_MS - 1));
        assert!(suggest_due(&idle, &empty, SUGGEST_MS));
        assert!(!suggest_due(&Idle::default(), &Open { waiting_user: 1, ..Open::default() }, 0));
        assert!(!suggest_due(&Idle::default(), &Open { todo: 1, ..Open::default() }, 0));
    }

    #[test]
    fn finished_children_without_a_task_are_closed() {
        let term = |id: &str, parent: bool, status: Status, keep: bool, deep: bool| {
            let mut labels = std::collections::BTreeMap::new();
            if deep {
                labels.insert(crate::deep::ROLE_LABEL.to_string(), crate::deep::ROLE_VALUE.to_string());
            }
            TermInfo {
                id: id.parse().unwrap(), name: id.into(), labels, parent: parent.then(|| "nixa-claude-9".parse().unwrap()), children: vec![], cmd: vec![], cwd: String::new(),
                pid: None, running: true, exit_code: None, status: Some(status), status_note: None, keep, draft: false, title: None, rows: 24, cols: 80, created_ms: 0, seq: 0,
            }
        };
        let terms = vec![
            term("nixa-claude-1", true, Status::Done, false, false),
            term("nixa-claude-2", true, Status::Done, false, false),
            term("nixa-claude-3", false, Status::Done, false, false),
            term("nixa-claude-4", true, Status::Done, true, false),
            term("nixa-claude-5", true, Status::Done, false, true),
            term("nixa-claude-6", true, Status::Working, false, false),
        ];
        let held = BTreeSet::from(["nixa-claude-2".to_string()]);
        assert_eq!(finished_agents(&terms, &held), vec!["nixa-claude-1".to_string()]);
        let text = nudge_text(&Open { finished: vec!["nixa-claude-1".into()], ..Open::default() }, 10, false).unwrap();
        assert!(text.contains("nixa-claude-1") && text.contains("nixa_term_remove"), "{text}");
    }

    fn answers(action: &str, confidence: f64, finished: f64) -> Answers {
        Answers {
            answers: vec![
                Answer::Choice { name: ACTION.into(), choice: serde_json::json!(action), confidence, probabilities: Vec::new() },
                Answer::Predicate { name: FINISHED.into(), probability: finished },
            ],
            ..Answers::default()
        }
    }

    #[test]
    fn the_model_closes_a_terminal_only_when_sure_it_is_finished() {
        assert!(sure_close(&answers(CLOSE, 0.95, 0.92), 0.85).is_ok());
        assert!(sure_close(&answers(CLOSE, 0.80, 0.95), 0.85).is_err());
        assert!(sure_close(&answers(CLOSE, 0.95, 0.60), 0.85).is_err());
        assert!(sure_close(&answers(KEEP, 0.99, 0.10), 0.85).is_err());
        assert!(sure_close(&Answers::default(), 0.85).is_err());
        let names: Vec<String> = close_questions().iter().map(|q| q["name"].as_str().unwrap_or_default().to_string()).collect();
        assert_eq!(names, vec![ACTION, FINISHED]);
        assert!(close_input("t1", "Tester", "").contains("(nothing on screen)"));
    }
}
