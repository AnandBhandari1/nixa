use super::{db, inner};
use crate::app::{App, lock};
use crate::engine;
use nixa_protocol::{Request, Status, TermInfo};
use nixa_store::tasks::{Archived, Task, TaskFilter, TaskStatus};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

pub const STUCK_MS: u64 = 20 * 60 * 1000;
pub const STUCK_NOTE: &str = "looks stuck: no screen change for 20 minutes";
const SCREEN_LINES: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub task: i64,
    pub word: &'static str,
    pub screen: u64,
    pub changed_at: u64,
    pub stuck: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub status: Status,
    pub note: String,
}

pub fn wanted(status: TaskStatus, id: i64) -> Option<Set> {
    let (status, note) = match status {
        TaskStatus::Doing => (Status::Working, format!("task #{id}")),
        TaskStatus::Blocked => (Status::Blocked, format!("task #{id} is blocked")),
        TaskStatus::Review => (Status::Done, format!("task #{id} is in review")),
        TaskStatus::Done => (Status::Done, format!("task #{id} is done")),
        TaskStatus::Cancelled => (Status::Idle, format!("task #{id} was cancelled")),
        TaskStatus::Inbox | TaskStatus::Todo => return None,
    };
    Some(Set { status, note })
}

pub struct Seen<'a> {
    pub task: i64,
    pub status: TaskStatus,
    pub live: Option<Status>,
    pub screen: Option<u64>,
    pub now: u64,
    pub prev: Option<&'a Mark>,
    pub event: bool,
    pub ours_stuck: bool,
}

fn agent_says_more(live: Option<Status>) -> bool {
    matches!(live, Some(Status::Blocked | Status::Error | Status::Done))
}

pub fn step(s: Seen) -> (Mark, Option<Set>) {
    let word = s.status.as_str();
    let screen = s.screen.unwrap_or_default();
    let fresh = Mark { task: s.task, word, screen, changed_at: s.now, stuck: false };
    let want = wanted(s.status, s.task);
    let Some(prev) = s.prev.filter(|p| p.task == s.task && p.word == word && !s.event) else {
        if s.status == TaskStatus::Doing && s.ours_stuck && !s.event {
            return (Mark { stuck: true, ..fresh }, None);
        }
        let first = s.prev.is_none() && !s.event;
        let set = match s.status {
            TaskStatus::Doing if first && agent_says_more(s.live) => None,
            TaskStatus::Blocked if first && matches!(s.live, Some(Status::Error)) => None,
            TaskStatus::Doing | TaskStatus::Blocked => want,
            _ if first => None,
            _ => want,
        };
        return (fresh, set.filter(|w| s.live != Some(w.status)));
    };
    if s.status != TaskStatus::Doing {
        return (prev.clone(), None);
    }
    let Some(screen) = s.screen else { return (prev.clone(), None) };
    if screen != prev.screen {
        let back = (prev.stuck && s.ours_stuck).then(|| want.clone()).flatten();
        return (Mark { screen, changed_at: s.now, stuck: false, ..prev.clone() }, back);
    }
    if !prev.stuck && s.live == Some(Status::Working) && s.now.saturating_sub(prev.changed_at) >= STUCK_MS {
        return (Mark { stuck: true, ..prev.clone() }, Some(Set { status: Status::Error, note: format!("task #{}: {STUCK_NOTE}", s.task) }));
    }
    (prev.clone(), None)
}

fn rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::Doing | TaskStatus::Blocked => 2,
        TaskStatus::Review => 1,
        _ => 0,
    }
}

pub fn current_by_terminal(tasks: Vec<Task>) -> HashMap<String, Task> {
    let mut out: HashMap<String, Task> = HashMap::new();
    for task in tasks.into_iter().filter(|t| !t.terminal_id.is_empty()) {
        let key = |t: &Task| (rank(t.status), t.claimed_at, t.id);
        let newer = out.get(&task.terminal_id).is_none_or(|old| key(&task) > key(old));
        if newer {
            out.insert(task.terminal_id.clone(), task);
        }
    }
    out
}

fn hash_of(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

pub fn ours_stuck(info: &TermInfo) -> bool {
    info.status == Some(Status::Error) && info.status_note.as_deref().is_some_and(|n| n.contains(STUCK_NOTE))
}

fn active_tasks(app: &App, marked: &[i64]) -> Vec<Task> {
    let Ok(conn) = db(app) else { return Vec::new() };
    let filter = TaskFilter {
        archived: Archived::Hide,
        status: vec![TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review],
        ..Default::default()
    };
    let mut list = nixa_store::tasks::list_tasks(&conn, &filter).unwrap_or_default();
    for id in marked {
        if !list.iter().any(|t| t.id == *id)
            && let Ok(Some(task)) = nixa_store::tasks::get_task(&conn, *id)
        {
            list.push(task);
        }
    }
    list
}

fn skip_term(app: &App, t: &TermInfo) -> bool {
    !t.running
        || t.labels.get(crate::deep::ROLE_LABEL).map(String::as_str) == Some(crate::deep::ROLE_VALUE)
        || lock(&app.deep).owns(t.id.as_str())
}

pub async fn sync(app: &Arc<App>, read_screens: bool, event: Option<i64>) {
    let _one = app.queue.marking.lock().await;
    let Ok(terms) = engine::list().await else { return };
    let live: HashMap<String, TermInfo> = terms.into_iter().filter(|t| !skip_term(app, t)).map(|t| (t.id.to_string(), t)).collect();
    let mut marked: Vec<i64> = inner(app).marks.values().map(|m| m.task).collect();
    marked.extend(event);
    let tasks: Vec<(String, Task)> = current_by_terminal(active_tasks(app, &marked)).into_iter().filter(|(terminal, _)| live.contains_key(terminal)).collect();
    let reads = tasks.iter().map(|(terminal, task)| {
        let info = &live[terminal];
        let look = read_screens && task.status == TaskStatus::Doing && (info.status == Some(Status::Working) || ours_stuck(info));
        let id = info.id.clone();
        async move {
            if look { engine::read(&id, Some(SCREEN_LINES), false).await.ok().map(|text| hash_of(&text)) } else { None }
        }
    });
    let screens = futures_util::future::join_all(reads).await;
    let now = crate::util::now_ms();
    let mut sets: Vec<(String, Option<Status>, Set)> = Vec::new();
    {
        let mut state = inner(app);
        for ((terminal, task), screen) in tasks.iter().zip(screens) {
            let seen_live = live[terminal].status;
            let ours = ours_stuck(&live[terminal]);
            let (mark, set) = step(Seen { task: task.id, status: task.status, live: seen_live, screen, now, prev: state.marks.get(terminal), event: event == Some(task.id), ours_stuck: ours });
            state.marks.insert(terminal.clone(), mark);
            if let Some(set) = set {
                sets.push((terminal.clone(), seen_live, set));
            }
        }
        state.marks.retain(|terminal, _| live.contains_key(terminal));
    }
    for (terminal, seen_live, set) in sets {
        let Ok(id) = engine::term_id(&terminal) else { continue };
        let still = engine::get(&id).await.is_ok_and(|t| t.status == seen_live);
        let wrote = still && engine::quick(Request::StatusSet { id, status: Some(set.status), note: Some(set.note.clone()) }).await.is_ok();
        if !wrote {
            inner(app).marks.remove(&terminal);
            if still {
                crate::util::log("queue", &format!("could not mark {terminal} {}", set.status.as_str()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(status: TaskStatus, live: Option<Status>, screen: Option<u64>, now: u64, prev: Option<&Mark>) -> (Mark, Option<Set>) {
        step(Seen { task: 7, status, live, screen, now, prev, event: false, ours_stuck: live == Some(Status::Error) })
    }

    #[test]
    fn a_started_task_turns_its_terminal_to_working() {
        let (mark, set) = seen(TaskStatus::Doing, None, None, 0, None);
        assert_eq!(set, Some(Set { status: Status::Working, note: "task #7".into() }));
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Working), None, 1, None).1, None, "no write when it already shows working");
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Idle), None, 5, Some(&mark)).1, None, "between changes the agent's own status wins");
    }

    #[test]
    fn task_changes_move_the_mark() {
        let (doing, _) = seen(TaskStatus::Doing, None, None, 0, None);
        assert_eq!(seen(TaskStatus::Review, Some(Status::Working), None, 1, Some(&doing)).1.map(|s| s.status), Some(Status::Done));
        assert_eq!(seen(TaskStatus::Blocked, Some(Status::Working), None, 1, Some(&doing)).1.map(|s| s.status), Some(Status::Blocked));
        assert_eq!(seen(TaskStatus::Done, Some(Status::Working), None, 1, Some(&doing)).1.map(|s| s.status), Some(Status::Done));
        assert_eq!(seen(TaskStatus::Cancelled, Some(Status::Working), None, 1, Some(&doing)).1.map(|s| s.status), Some(Status::Idle));
        let other = Mark { task: 6, ..doing.clone() };
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Idle), None, 1, Some(&other)).1.map(|s| s.status), Some(Status::Working), "a new task on the same terminal marks it again");
    }

    #[test]
    fn old_finished_work_does_not_repaint_after_a_restart() {
        assert_eq!(seen(TaskStatus::Done, Some(Status::Working), None, 0, None).1, None);
        assert_eq!(seen(TaskStatus::Review, Some(Status::Idle), None, 0, None).1, None);
        assert_eq!(seen(TaskStatus::Blocked, Some(Status::Working), None, 0, None).1.map(|s| s.status), Some(Status::Blocked));
        let event = step(Seen { task: 7, status: TaskStatus::Review, live: Some(Status::Working), screen: None, now: 0, prev: None, event: true, ours_stuck: false });
        assert_eq!(event.1.map(|s| s.status), Some(Status::Done), "a real finish right after a restart still shows");
        let (quiet, _) = seen(TaskStatus::Review, Some(Status::Working), None, 0, None);
        let again = step(Seen { task: 7, status: TaskStatus::Review, live: Some(Status::Working), screen: None, now: 1, prev: Some(&quiet), event: true, ours_stuck: false });
        assert_eq!(again.1.map(|s| s.status), Some(Status::Done), "an event paints even when a tick saw the status first");
    }

    #[test]
    fn only_nixas_own_red_mark_is_cleared() {
        let (stuck, _) = seen(TaskStatus::Doing, Some(Status::Working), Some(1), 0, None);
        let stuck = Mark { stuck: true, ..stuck };
        let agent_error = step(Seen { task: 7, status: TaskStatus::Doing, live: Some(Status::Error), screen: Some(2), now: 5, prev: Some(&stuck), event: false, ours_stuck: false });
        assert_eq!(agent_error.1, None, "an error the agent reported stays");
        let restart = step(Seen { task: 7, status: TaskStatus::Doing, live: Some(Status::Error), screen: Some(1), now: 5, prev: None, event: false, ours_stuck: true });
        assert!(restart.0.stuck && restart.1.is_none(), "after a restart Nixa knows its own red mark");
        let moved = step(Seen { task: 7, status: TaskStatus::Doing, live: Some(Status::Error), screen: Some(2), now: 6, prev: Some(&restart.0), event: false, ours_stuck: true });
        assert_eq!(moved.1.map(|s| s.status), Some(Status::Working));
    }

    #[test]
    fn a_first_look_keeps_what_the_agent_itself_reports() {
        for live in [Status::Blocked, Status::Error, Status::Done] {
            assert_eq!(seen(TaskStatus::Doing, Some(live), None, 0, None).1, None, "{live:?}");
        }
        assert_eq!(seen(TaskStatus::Blocked, Some(Status::Error), None, 0, None).1, None);
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Idle), None, 0, None).1.map(|s| s.status), Some(Status::Working));
    }

    #[test]
    fn a_still_screen_while_working_turns_red_and_comes_back() {
        let (mark, _) = seen(TaskStatus::Doing, Some(Status::Working), Some(1), 0, None);
        let (mark, set) = seen(TaskStatus::Doing, Some(Status::Working), Some(1), STUCK_MS - 1, Some(&mark));
        assert_eq!(set, None);
        let (stuck, set) = seen(TaskStatus::Doing, Some(Status::Working), Some(1), STUCK_MS, Some(&mark));
        assert_eq!(set.as_ref().map(|s| s.status), Some(Status::Error));
        assert!(set.unwrap().note.contains(STUCK_NOTE) && stuck.stuck);
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Error), Some(1), STUCK_MS * 3, Some(&stuck)).1, None, "marked once");
        let (moved, set) = seen(TaskStatus::Doing, Some(Status::Error), Some(2), STUCK_MS * 3, Some(&stuck));
        assert_eq!(set.map(|s| s.status), Some(Status::Working));
        assert!(!moved.stuck && moved.changed_at == STUCK_MS * 3);
        let (idle, _) = seen(TaskStatus::Doing, Some(Status::Idle), Some(2), STUCK_MS * 3, Some(&moved));
        assert_eq!(seen(TaskStatus::Doing, Some(Status::Idle), Some(2), STUCK_MS * 9, Some(&idle)).1, None, "an idle agent waiting for input is not stuck");
    }

    #[test]
    fn each_terminal_follows_its_newest_task() {
        let conn = nixa_store::open_memory().unwrap();
        let by = nixa_store::tasks::Actor::new(nixa_store::tasks::AuthorKind::Nixa, "Nixa");
        for title in ["old", "new", "free"] {
            nixa_store::tasks::add_task(&conn, &nixa_store::tasks::TaskInput { title: title.into(), ..Default::default() }, &by).unwrap();
        }
        let mut old = nixa_store::tasks::get_task(&conn, 1).unwrap().unwrap();
        let mut new = nixa_store::tasks::get_task(&conn, 2).unwrap().unwrap();
        let free = nixa_store::tasks::get_task(&conn, 3).unwrap().unwrap();
        old.terminal_id = "m4".into();
        new.terminal_id = "m4".into();
        old.status = TaskStatus::Done;
        old.claimed_at = 30;
        new.status = TaskStatus::Doing;
        new.claimed_at = 20;
        let found = current_by_terminal(vec![old.clone(), new.clone(), free]);
        assert_eq!(found.len(), 1);
        assert_eq!(found["m4"].id, 2, "the active task wins over a newer finished one");
        old.status = TaskStatus::Review;
        new.status = TaskStatus::Done;
        assert_eq!(current_by_terminal(vec![old, new])["m4"].id, 1);
    }
}
