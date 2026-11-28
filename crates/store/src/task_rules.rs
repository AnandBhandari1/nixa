use crate::spaces::{get_space, list_spaces};
use crate::tasks::{BlockedOn, Task, TaskStatus};
use anyhow::{Result, anyhow, bail};
use rusqlite::Connection;
use std::path::Path;

pub const AGENT_STATUSES: [TaskStatus; 4] = [TaskStatus::Doing, TaskStatus::Blocked, TaskStatus::Review, TaskStatus::Cancelled];
pub const AGENT_ADD_STATUSES: [TaskStatus; 2] = [TaskStatus::Inbox, TaskStatus::Todo];
pub const DONE_NOTE: &str = "The agent reports the work as done.";
pub const DONE_TO_REVIEW: &str = "Your done was set as review with your note. Nixa checks the work and closes the task. Go on with your next work.";
pub const AUTO_NEXT: &str = "Do not wait: ask Nixa with nixa deep ask (MCP: nixa_deep_ask) to open it again or to add the rest as a new task, then go on with your work.";
pub const HELD_NEXT: &str = "Do not wait: send your change to the holder terminal with nixa term send (MCP: nixa_term_send), or add it as a comment on the task with nixa task comment (MCP: nixa_task_comment), or ask Nixa with nixa deep ask (MCP: nixa_deep_ask) to hand the task over or add your part as a new task, then go on with your work.";

pub fn agent_statuses() -> String {
    AGENT_STATUSES.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
}

pub fn space_for_folder(conn: &Connection, folder: &str) -> Option<String> {
    let folder = folder.trim();
    if folder.is_empty() {
        return None;
    }
    list_spaces(conn)
        .ok()?
        .into_iter()
        .filter(|s| !s.folder.trim().is_empty() && Path::new(folder).starts_with(s.folder.trim()))
        .max_by_key(|s| s.folder.trim().len())
        .map(|s| s.space_id)
}

pub const LOCKED_MOVED: &str = "The space of this task is locked, so Nixa moved the task out of it. Nixa places it in an open space. Go on with the work.";

pub fn move_off_locked(conn: &Connection, task: &Task) -> Result<bool> {
    let space = task.space_id.trim();
    if space.is_empty() {
        return Ok(false);
    }
    if !get_space(conn, space)?.is_some_and(|s| s.locked) {
        return Ok(false);
    }
    let patch = crate::tasks::TaskPatch { space_id: Some(String::new()), space_name: Some(String::new()), ..crate::tasks::TaskPatch::default() };
    crate::tasks::update_task(conn, task.id, &patch)?;
    let note = format!("Space {} is locked, so Nixa moved this task out of it. Nixa places it in an open space.", task.space_name);
    let by = crate::tasks::Actor::new(crate::tasks::AuthorKind::Nixa, "nixa");
    crate::tasks::add_comment(conn, task.id, &crate::tasks::CommentInput::new(crate::tasks::CommentKind::Note, &note), &by)?;
    Ok(true)
}

pub fn auto_blocked(on: Option<BlockedOn>, needs_user: bool) -> (Option<BlockedOn>, bool) {
    let on = match on {
        Some(BlockedOn::User) => Some(BlockedOn::Decision),
        None | Some(BlockedOn::Other) if needs_user => Some(BlockedOn::Decision),
        other => other,
    };
    (on, false)
}

pub fn check_open(task: &Task) -> Result<()> {
    if task.status.is_closed() {
        bail!(
            "Task #{} is {}. A closed task is opened again by Nixa. You can still add a comment. Nothing was changed.",
            task.id,
            task.status.as_str()
        );
    }
    Ok(())
}

pub fn check_held(task: &Task, terminal: &str) -> Result<()> {
    check_open(task)?;
    let terminal = terminal.trim();
    if terminal.is_empty() || task.terminal_id != terminal {
        let holder = if task.terminal_id.is_empty() { "Claim it first.".to_string() } else { format!("Terminal {} holds it.", task.terminal_id) };
        bail!("Task #{} is not yours. {holder} Nothing was changed.", task.id);
    }
    if !task.status.is_held() {
        bail!(
            "Task #{} is {}, so your claim has ended. Nixa opens it again. Nothing was changed.",
            task.id,
            task.status.as_str()
        );
    }
    Ok(())
}

pub fn agent_done(status: TaskStatus, note: &str) -> (TaskStatus, String) {
    if status == TaskStatus::Done {
        let note = if note.trim().is_empty() { DONE_NOTE } else { note };
        return (TaskStatus::Review, note.to_string());
    }
    (status, note.to_string())
}

fn with_next(result: Result<()>, next: bool) -> Result<()> {
    match result {
        Err(error) if next => Err(anyhow!("{error} {AUTO_NEXT}")),
        other => other,
    }
}

pub fn check_open_in(task: &Task) -> Result<()> {
    with_next(check_open(task), task.status.is_closed())
}

fn held_by_other(task: &Task, terminal: &str) -> bool {
    !task.terminal_id.is_empty() && task.terminal_id != terminal.trim()
}

pub fn check_held_in(task: &Task, terminal: &str) -> Result<()> {
    let result = check_held(task, terminal);
    if result.is_ok() {
        return result;
    }
    if !task.status.is_closed() && task.status.is_held() && held_by_other(task, terminal) {
        return result.map_err(|error| anyhow!("{error} {HELD_NEXT}"));
    }
    with_next(result, task.status.is_closed() || !task.status.is_held())
}

pub fn check_agent_status_in(task: &Task, terminal: &str, status: TaskStatus, note: &str) -> Result<()> {
    let (status, note) = agent_done(status, note);
    check_held_in(task, terminal)?;
    check_agent_status(task, terminal, status, &note)
}

pub fn check_agent_status(task: &Task, terminal: &str, status: TaskStatus, note: &str) -> Result<()> {
    if status == TaskStatus::Done {
        bail!("An agent cannot set a task to done. Set review with a result note. Then Nixa checks the work and sets done. Nothing was changed.");
    }
    check_held(task, terminal)?;
    if !AGENT_STATUSES.contains(&status) {
        bail!("An agent can set {}. To free your claim, release the task. Nothing was changed.", agent_statuses());
    }
    if status == TaskStatus::Review && note.trim().is_empty() {
        bail!("Moving to review needs a result note, for example \"Tests pass 12 of 12\". Nothing was changed.");
    }
    Ok(())
}

pub fn check_agent_release(task: &Task, terminal: &str) -> Result<()> {
    check_held_in(task, terminal)
}

pub fn check_agent_add(status: Option<TaskStatus>) -> Result<()> {
    match status {
        Some(status) if !AGENT_ADD_STATUSES.contains(&status) => {
            bail!("An agent adds a task as inbox or todo, not {}. Nothing was added.", status.as_str())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::temp_db;
    use crate::tasks::{Actor, AuthorKind, TaskInput, add_task, claim_task, must_task, set_status};
    use rusqlite::Connection;

    fn todo(conn: &Connection, title: &str) -> Task {
        let input = TaskInput { title: title.to_string(), ..TaskInput::default() };
        add_task(conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap()
    }

    fn claimed(conn: &Connection, title: &str, terminal: &str) -> Task {
        let task = todo(conn, title);
        let by = Actor::in_terminal(AuthorKind::Agent, terminal, terminal);
        claim_task(conn, task.id, &by, "").unwrap().task.unwrap()
    }

    #[test]
    fn agents_never_set_done() {
        let conn = temp_db("rules-done");
        let task = claimed(&conn, "Build", "m1");
        let err = check_agent_status(&task, "m1", TaskStatus::Done, "all good").unwrap_err().to_string();
        assert!(err.contains("cannot set a task to done"), "{err}");
        assert!(check_agent_status(&task, "m1", TaskStatus::Blocked, "").is_ok());
        assert!(check_agent_status(&task, "m1", TaskStatus::Cancelled, "").is_ok());
        let err = check_agent_status(&task, "m1", TaskStatus::Todo, "").unwrap_err().to_string();
        assert!(err.contains("doing, blocked, review, cancelled"), "{err}");
    }

    #[test]
    fn automatic_turns_an_agent_done_into_review() {
        let conn = temp_db("rules-auto-done");
        let task = claimed(&conn, "Build", "m1");
        assert_eq!(agent_done(TaskStatus::Done, "all good"), (TaskStatus::Review, "all good".to_string()));
        assert_eq!(agent_done(TaskStatus::Done, " "), (TaskStatus::Review, DONE_NOTE.to_string()));
        assert_eq!(agent_done(TaskStatus::Blocked, ""), (TaskStatus::Blocked, String::new()));
        assert!(check_agent_status_in(&task, "m1", TaskStatus::Done, "").is_ok());
    }

    #[test]
    fn automatic_closed_task_errors_name_the_next_step() {
        let conn = temp_db("rules-auto-closed");
        let task = claimed(&conn, "Build", "m1");
        set_status(&conn, task.id, TaskStatus::Done, None, false).unwrap();
        let done = must_task(&conn, task.id).unwrap();
        let err = check_open_in(&done).unwrap_err().to_string();
        assert!(err.contains("nixa deep ask"), "{err}");
        let err = check_agent_status_in(&done, "m1", TaskStatus::Doing, "").unwrap_err().to_string();
        assert!(err.contains("nixa deep ask"), "{err}");
        let other = claimed(&conn, "Other", "m2");
        let err = check_held_in(&other, "m1").unwrap_err().to_string();
        assert!(err.contains("Terminal m2 holds it"), "{err}");
        assert!(err.contains("nixa_term_send"), "{err}");
        assert!(err.contains("nixa_task_comment"), "{err}");
        let err = check_agent_release(&other, "m1").unwrap_err().to_string();
        assert!(err.contains("nixa_term_send"), "{err}");
    }

    #[test]
    fn review_needs_a_note() {
        let conn = temp_db("rules-review");
        let task = claimed(&conn, "Build", "m1");
        let err = check_agent_status(&task, "m1", TaskStatus::Review, "  ").unwrap_err().to_string();
        assert!(err.contains("needs a result note"), "{err}");
        assert!(check_agent_status(&task, "m1", TaskStatus::Review, "Tests pass").is_ok());
    }

    #[test]
    fn only_the_holder_changes_or_frees_a_claim() {
        let conn = temp_db("rules-held");
        let task = claimed(&conn, "Build", "m1");
        let err = check_agent_release(&task, "m2").unwrap_err().to_string();
        assert!(err.contains("not yours. Terminal m1 holds it"), "{err}");
        assert!(check_agent_release(&task, "m1").is_ok());
        let free = todo(&conn, "Free");
        let err = check_agent_status(&free, "m1", TaskStatus::Blocked, "").unwrap_err().to_string();
        assert!(err.contains("Claim it first"), "{err}");
    }

    #[test]
    fn closed_tasks_stay_closed_for_agents() {
        let conn = temp_db("rules-closed");
        let task = claimed(&conn, "Build", "m1");
        set_status(&conn, task.id, TaskStatus::Done, None, false).unwrap();
        let done = must_task(&conn, task.id).unwrap();
        assert_eq!(done.terminal_id, "m1");
        for err in [check_open(&done), check_agent_release(&done, "m1"), check_agent_status(&done, "m1", TaskStatus::Doing, "")] {
            assert!(err.unwrap_err().to_string().contains("A closed task is opened again by Nixa"));
        }
        let cancelled = set_status(&conn, todo(&conn, "Old").id, TaskStatus::Cancelled, None, false).unwrap();
        assert!(check_open(&cancelled).is_err());
        assert!(check_open(&todo(&conn, "New")).is_ok());
    }

    #[test]
    fn space_for_folder_finds_the_deepest_space() {
        use crate::spaces::{SpacePatch, set_space};
        let conn = temp_db("rules-decision");
        set_space(&conn, "s1", &SpacePatch { name: Some("Shop".into()), folder: Some("/work/shop".into()), ..SpacePatch::default() }).unwrap();
        assert_eq!(space_for_folder(&conn, "/work/shop"), Some("s1".to_string()));
        assert_eq!(space_for_folder(&conn, "/work/shop/src"), Some("s1".to_string()));
        assert_eq!(space_for_folder(&conn, "/work/other"), None);
        assert_eq!(space_for_folder(&conn, ""), None);
    }

    #[test]
    fn automatic_moves_a_task_out_of_a_locked_space() {
        use crate::spaces::{SpacePatch, set_space};
        let conn = temp_db("rules-locked");
        set_space(&conn, "s1", &SpacePatch { name: Some("Shop".into()), folder: Some("/work/shop".into()), locked: Some(true), ..SpacePatch::default() }).unwrap();
        let input = TaskInput { title: "Fix".into(), space_id: "s1".into(), space_name: "Shop".into(), ..TaskInput::default() };
        let task = add_task(&conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap();
        assert!(move_off_locked(&conn, &task).unwrap());
        assert_eq!(must_task(&conn, task.id).unwrap().space_id, "");
        let second = add_task(&conn, &input, &Actor::new(AuthorKind::User, "Sam")).unwrap();
        crate::settings::set_setting_text(&conn, "decision.mode", "manual").unwrap();
        assert!(move_off_locked(&conn, &second).unwrap());
        assert_eq!(must_task(&conn, second.id).unwrap().space_id, "");
    }

    #[test]
    fn automatic_never_parks_a_task_on_the_user() {
        assert_eq!(auto_blocked(Some(BlockedOn::User), true), (Some(BlockedOn::Decision), false));
        assert_eq!(auto_blocked(None, true), (Some(BlockedOn::Decision), false));
        assert_eq!(auto_blocked(Some(BlockedOn::Task), false), (Some(BlockedOn::Task), false));
    }

    #[test]
    fn agents_add_only_open_work() {
        assert!(check_agent_add(None).is_ok());
        assert!(check_agent_add(Some(TaskStatus::Todo)).is_ok());
        assert!(check_agent_add(Some(TaskStatus::Inbox)).is_ok());
        assert!(check_agent_add(Some(TaskStatus::Done)).is_err());
        assert!(check_agent_add(Some(TaskStatus::Review)).is_err());
    }
}
